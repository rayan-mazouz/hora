//! The status summary: turning stored checks into the page/API view model.
//!
//! [`build_summary`] gathers everything in batched queries; the view-model
//! types are in `view`, a monitor card's assembly in `monitor`, and the
//! status/label helpers shared with the other views in `status`.

mod monitor;
mod status;
#[cfg(test)]
mod tests;
mod view;

pub(crate) use view::{
    ChannelView, DayCell, GroupView, IncidentView, MaintenanceView, MonitorView, PeerView,
    StatusTemplate, Summary, VantageView,
};

use monitor::{MonitorData, build_groups, build_monitor_view};
use status::{iso, overall_label, worse};

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use hora_core::SECONDS_PER_DAY;
use hora_core::config::{Config, Monitor};
use hora_core::db::{self, Latest};
use hora_core::notifications::ChannelHealthEntry;

use crate::SPARK_BUCKETS;
use crate::visibility::Visibility;

/// Shared, read-only inputs for building each monitor's view.
pub(crate) struct SummaryCtx {
    now: DateTime<Utc>,
    timestamp: i64,
    since_24h: i64,
    since_history: i64,
    threshold: i64,
    cert_threshold: i64,
    history_days: u16,
}

/// Build the page/API view model for one audience. Monitors the audience may
/// not see are left out entirely - cards, groups and daily bars alike; failure
/// reasons collapse to safe categories (see `probe::public_reason`) without
/// detail; topology annotations never name a monitor the audience may not
/// see. Event markers and channel health are operator streams; every other
/// audience gets them empty.
pub(crate) async fn build_summary(
    pool: &SqlitePool,
    config: &Config,
    visibility: &Visibility<'_>,
    channel_health: &[ChannelHealthEntry],
    vantage: &HashMap<String, Vec<hora_core::mesh::vantage::PeerVantage>>,
) -> Summary {
    let now = Utc::now();
    let timestamp = now.timestamp();
    // The daily fetch also feeds the error-budget arithmetic, so it must cover
    // the largest configured SLO window, not just the displayed history.
    let slo_days = config
        .monitors
        .iter()
        .filter(|monitor| monitor.slo_uptime.is_some())
        .map(Monitor::slo_window_days)
        .max()
        .unwrap_or(0);
    let fetch_days = config.page.history_days.max(slo_days);
    let ctx = SummaryCtx {
        now,
        timestamp,
        since_24h: timestamp - SECONDS_PER_DAY,
        since_history: timestamp - i64::from(fetch_days) * SECONDS_PER_DAY,
        threshold: i64::from(config.alerts.fail_threshold.max(1)),
        cert_threshold: i64::from(config.alerts.cert_expiry_days),
        history_days: config.page.history_days,
    };
    let operator_streams = visibility.sees_operator_streams();

    let visible_monitors: Vec<&Monitor> = config
        .monitors
        .iter()
        .filter(|monitor| visibility.can_see_monitor(monitor))
        .collect();

    // The 24h/90d aggregates batch into one query each, and the batches are
    // independent, so they run concurrently: under WAL every read gets its own
    // pool connection and the rebuild costs the slowest query, not the sum of
    // all of them. A failed query degrades to empty data ("no data" cards)
    // rather than blacking out the page.
    let bucket_secs = (SECONDS_PER_DAY / SPARK_BUCKETS).max(1);
    // Latency is summarised in SQL: exact percentiles, plus a bucket-averaged
    // series for the sparkline. The raw 24h samples never enter memory or the
    // page, so both stay bounded by the monitor count, not the check frequency.
    let (availability, daily, percentiles, sparklines, certs, recent, events) = tokio::join!(
        db::availability_all(pool, ctx.since_24h),
        db::daily_all(pool, ctx.since_history, ctx.timestamp),
        db::latency_percentiles_all(pool, ctx.since_24h),
        db::latency_sparkline_all(pool, ctx.since_24h, bucket_secs),
        db::cert_all(pool),
        recent_checks_map(pool, &visible_monitors, ctx.threshold.max(1)),
        // Event markers overlay the sparklines - operator info (deploy titles),
        // so only the operator's view fetches them; every other stays bare.
        async {
            if operator_streams {
                db::events_since(pool, ctx.since_24h).await
            } else {
                Ok(Vec::new())
            }
        },
    );
    let availability = or_empty(availability, "availability");
    let daily = or_empty(daily, "daily");
    let percentiles = percentile_map(or_empty(percentiles, "latency percentiles"));
    let sparklines = or_empty(sparklines, "latency sparklines");
    let certs = or_empty(certs, "certificates");
    let events = or_empty(events, "events");

    let data = MonitorData {
        recent: &recent,
        availability: &availability,
        daily: &daily,
        percentiles: &percentiles,
        sparklines: &sparklines,
        certs: &certs,
        events: &events,
        vantage,
    };

    let monitors: Vec<Arc<MonitorView>> = visible_monitors
        .iter()
        .map(|monitor| {
            let mut view = build_monitor_view(monitor, &ctx, &data, &config.monitors, visibility);
            view.maintenance = config
                .active_maintenance(&monitor.id, now)
                .map(|window| window.title.clone());
            Arc::new(view)
        })
        .collect();

    let overall = monitors
        .iter()
        .fold("up", |worst, m| worse(worst, m.status));

    let groups = build_groups(&monitors, &config.monitors);

    // Watched peers form their own section; their state does not roll into the
    // overall badge (it tracks the monitored services, not the surveillance mesh).
    let peers = build_peers(pool, config, ctx.threshold).await;

    // Banner order: ad-hoc announcements (the fresh operational news) first,
    // then the config-declared ones. A read failure costs the ad-hoc banners
    // only, never the page.
    let mut incidents = announcement_banners(pool, timestamp).await;
    incidents.extend(build_incident_banners(config));

    // Active maintenance windows shown as a top banner (so a long reason never
    // changes a card's height and disturbs the grid).
    let maintenances = build_maintenances(config, now, None);

    Summary {
        title: config.page.title.clone(),
        overall,
        overall_label: overall_label(overall),
        generated_at: now.to_rfc3339(),
        updated_utc: now.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        incidents,
        maintenances,
        monitors,
        groups,
        peers,
        channels: if operator_streams {
            channel_views(channel_health)
        } else {
            Vec::new()
        },
    }
}

/// Build the banner view for the maintenance windows active at `now`, resolving
/// each covered monitor id to its display name (or "all monitors" when empty).
/// With `only_ids`, windows that touch none of those monitors are dropped (the
/// per-group page must not announce another client's maintenance).
fn build_maintenances(
    config: &Config,
    now: DateTime<Utc>,
    only_ids: Option<&std::collections::HashSet<&str>>,
) -> Vec<MaintenanceView> {
    config
        .maintenance
        .iter()
        .filter(|window| now >= window.start && now <= window.end)
        .filter(|window| {
            only_ids.is_none_or(|ids| {
                window.monitors.is_empty()
                    || window.monitors.iter().any(|id| ids.contains(id.as_str()))
            })
        })
        .map(|window| {
            let monitors = if window.monitors.is_empty() {
                "all monitors".to_owned()
            } else {
                window
                    .monitors
                    .iter()
                    .map(|id| {
                        config
                            .monitors
                            .iter()
                            .find(|monitor| &monitor.id == id)
                            .map_or(id.as_str(), |monitor| monitor.name.as_str())
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            MaintenanceView {
                reason: window.title.clone(),
                monitors,
            }
        })
        .collect()
}

/// Build the channel-health view model from the dispatcher's snapshot. Only
/// called for the operator's view; every other summary omits it.
fn channel_views(health: &[ChannelHealthEntry]) -> Vec<ChannelView> {
    health
        .iter()
        .map(|entry| ChannelView {
            name: entry.name.clone(),
            failing: entry.health.consecutive_failures > 0,
            consecutive_failures: entry.health.consecutive_failures,
            failing_for_secs: entry
                .health
                .first_failure_at
                .and_then(|since| since.elapsed().ok())
                .map(|d| d.as_secs()),
        })
        .collect()
}

/// Derive a single-group view from a built summary: the monitors of `group`
/// only, the overall badge recomputed from them, the peers section hidden
/// (the surveillance mesh is the operator's business, not a client's), and
/// the maintenance banners restricted to windows touching the group. `None`
/// when the summary holds no monitor of that group - an unknown group, or a
/// fully private one viewed anonymously, answers exactly like a missing page.
pub(crate) fn for_group(summary: &Summary, config: &Config, group: &str) -> Option<Summary> {
    let monitors: Vec<Arc<MonitorView>> = summary
        .monitors
        .iter()
        .filter(|monitor| monitor.group.as_deref() == Some(group))
        .map(Arc::clone)
        .collect();
    if monitors.is_empty() {
        return None;
    }

    let overall = monitors
        .iter()
        .fold("up", |worst, monitor| worse(worst, monitor.status));
    let ids: std::collections::HashSet<&str> =
        monitors.iter().map(|monitor| monitor.id.as_str()).collect();
    let maintenances = build_maintenances(config, Utc::now(), Some(&ids));

    Some(Summary {
        title: format!("{} · {group}", config.page.title),
        overall,
        overall_label: overall_label(overall),
        generated_at: summary.generated_at.clone(),
        updated_utc: summary.updated_utc.clone(),
        // The operator's announcements are page-wide communication; keep them
        // (the already-built list includes the ad-hoc `hora announce` ones).
        incidents: summary.incidents.clone(),
        maintenances,
        groups: vec![GroupView {
            name: group.to_owned(),
            ids: monitors.iter().map(|view| view.id.clone()).collect(),
            monitors: monitors.clone(),
        }],
        monitors,
        peers: Vec::new(),
        channels: Vec::new(),
    })
}

/// The ad-hoc announcements (`hora announce` / `POST /api/announce`),
/// rendered like the config-declared banners. Newest first.
async fn announcement_banners(pool: &SqlitePool, now: i64) -> Vec<IncidentView> {
    or_empty(db::active_announcements(pool, now).await, "announcements")
        .into_iter()
        .map(|announcement| IncidentView {
            title: announcement.title,
            body: announcement.body,
            severity: severity_label(&announcement.severity),
            at: chrono::DateTime::from_timestamp(announcement.created_at, 0)
                .map(|at| at.format("%Y-%m-%d %H:%M UTC").to_string()),
        })
        .collect()
}

/// The static severity label for a stored announcement (validated at insert;
/// anything unexpected degrades to `info`).
fn severity_label(severity: &str) -> &'static str {
    severity
        .parse::<hora_core::config::Severity>()
        .unwrap_or_default()
        .as_str()
}

/// The configured announcements, rendered for the status page banner.
fn build_incident_banners(config: &Config) -> Vec<IncidentView> {
    config
        .incidents
        .iter()
        .map(|incident| IncidentView {
            title: incident.title.clone(),
            body: incident.body.clone(),
            severity: incident.severity.as_str(),
            at: incident
                .at
                .map(|at| at.format("%Y-%m-%d %H:%M UTC").to_string()),
        })
        .collect()
}

/// Build the view for each watched peer (its current status and last-seen time),
/// rendered in a section of its own apart from the monitors.
async fn build_peers(pool: &SqlitePool, config: &Config, threshold: i64) -> Vec<PeerView> {
    let mut peers = Vec::new();
    for peer in config.peers.iter().filter(|peer| peer.is_watched()) {
        let recent = or_empty(
            db::recent_checks(pool, peer.listen_id(), threshold.max(1)).await,
            "peer recent checks",
        );
        peers.push(PeerView {
            status: db::derive_status(&recent, threshold),
            last_seen: recent.first().and_then(|latest| iso(latest.time)),
            id: peer.id.clone(),
            name: peer.name.clone(),
            pings: peer.ping_url.is_some(),
        });
    }
    peers
}

/// Unwrap a batch query, logging and using empty data on error so one failed
/// query degrades to "no data" cards instead of failing the whole page.
pub(crate) fn or_empty<T: Default>(result: sqlx::Result<T>, what: &str) -> T {
    result.unwrap_or_else(|err| {
        tracing::error!("summary: {what} query failed: {err:#}");
        T::default()
    })
}

/// Convert the raw `(p50, p95, p99)` tuples from SQL into [`Percentiles`].
pub(crate) fn percentile_map(
    raw: HashMap<String, (i64, i64, i64)>,
) -> HashMap<String, Percentiles> {
    raw.into_iter()
        .map(|(id, (p50, p95, p99))| (id, Percentiles { p50, p95, p99 }))
        .collect()
}

/// Fetch each monitor's recent checks. Deliberately per-monitor: the query is an
/// indexed `ORDER BY time DESC LIMIT N` (tiny), and unlike a single windowed query
/// it is correct for any interval - a monitor checked less than once a day (e.g. a
/// weekly push heartbeat) would be dropped by a 24h batch window and shown as
/// "unknown". On embedded `SQLite` these N statements cost microseconds each, far
/// less than scanning the whole history table to rank rows.
pub(crate) async fn recent_checks_map(
    pool: &SqlitePool,
    monitors: &[&Monitor],
    limit: i64,
) -> HashMap<String, Vec<Latest>> {
    let mut recent: HashMap<String, Vec<Latest>> = HashMap::new();
    for monitor in monitors {
        let checks = or_empty(
            db::recent_checks(pool, &monitor.id, limit).await,
            "recent checks",
        );
        recent.insert(monitor.id.clone(), checks);
    }
    recent
}

/// 24h latency percentiles for a monitor, computed in SQL by
/// [`db::latency_percentiles_all`] (nearest-rank).
#[derive(Clone, Copy)]
pub(crate) struct Percentiles {
    p50: i64,
    p95: i64,
    p99: i64,
}
