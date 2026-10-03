//! The server-rendered pages: status (whole and per group), history and its
//! Atom feed, incidents, the timeline and the monthly report.

use std::future::Future;

use askama::Template;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, header};
use axum::response::{Html, IntoResponse, Response};
use chrono::Utc;
use serde::Deserialize;

use hora_core::config::{Config, Kind};
use hora_core::db::{self};

use crate::AppState;
use crate::auth::Viewer;
use crate::error::AppError;
use crate::history;
use crate::layout::Chrome;
use crate::snapshot::Body;
use crate::status_page;
use crate::summary::{StatusTemplate, Summary};
use crate::text;
use crate::visibility::{Audience, Visibility};

/// Text clients (curl, wget, or an explicit text/plain Accept) get the aligned
/// plain-text rendering; everyone else the HTML page.
fn wants_text(headers: &HeaderMap) -> bool {
    headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|ua| ua.starts_with("curl/") || ua.starts_with("Wget/"))
        || headers
            .get(header::ACCEPT)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|accept| accept.contains("text/plain") && !accept.contains("text/html"))
}

/// The status page - whole, or one group's - as `audience` sees it: the HTML
/// page or, for text clients, plain text. Rendered once per snapshot and
/// audience, then served from memory; the page's per-request parts (the
/// viewer's `?token=` in its links, `?theme=`, `?refresh=`) are spliced into
/// the shared render per request, so a token never enters the cache (see
/// `crate::layout`). A group this audience sees nothing of answers 404.
async fn status_response(
    state: &AppState,
    viewer: &Viewer,
    audience: &Audience,
    group: Option<&str>,
    chrome: Chrome,
    headers: &HeaderMap,
) -> Result<Response, AppError> {
    let config = &viewer.config;
    let snapshot = state.snapshot().await;
    let operator = *audience == Audience::Operator;
    let scoped = |whole: &Summary| -> Option<Summary> {
        group.map(|group| crate::summary::for_group(whole, config, group))?
    };
    if wants_text(headers) {
        let body = snapshot
            .body(
                config,
                Body::Text(audience.clone(), group.map(str::to_owned)),
                || {
                    let whole = snapshot.summary(config, audience);
                    let body = match group {
                        None => text::render(&whole, operator),
                        Some(_) => match scoped(&whole) {
                            Some(view) => text::render(&view, operator),
                            None => return Ok(None),
                        },
                    };
                    Ok::<_, AppError>(Some(Bytes::from(body)))
                },
            )?
            .ok_or(AppError::NotFound("unknown group"))?;
        return Ok(([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response());
    }
    // Only the anonymous view can be the private door, so only it varies
    // with whether a token came.
    let bad_token = chrome.req.has_token && *audience == Audience::Public;
    let req = chrome.req.clone();
    let page = snapshot
        .page(
            config,
            Body::Page(audience.clone(), group.map(str::to_owned), bad_token),
            || {
                let whole = snapshot.summary(config, audience);
                let view;
                let summary: &Summary = match group {
                    None => &whole,
                    Some(_) => match scoped(&whole) {
                        Some(found) => {
                            view = found;
                            &view
                        }
                        None => return Ok(None),
                    },
                };
                let ask = status_page::Ask {
                    operator,
                    token_given: bad_token,
                    group_page: group.is_some(),
                };
                let html = StatusTemplate {
                    page: status_page::build(summary, config, &ask),
                    chrome: chrome.cached().at("status").operator(operator),
                }
                .render()?;
                Ok::<_, AppError>(Some(html))
            },
        )?
        .ok_or(AppError::NotFound("unknown group"))?;
    Ok(Html(page.serve(&req)).into_response())
}

pub(crate) async fn page(
    State(state): State<AppState>,
    viewer: Viewer,
    chrome: Chrome,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    status_response(&state, &viewer, &viewer.audience, None, chrome, &headers).await
}

/// The per-group status page (`/status/{group}`): the monitors of one display
/// group, nothing else - lightweight multi-tenancy for an operator hosting
/// several clients' services on one Hora. Anonymous viewers get the group's
/// public monitors; this group's own `server.group_tokens` entry reveals the
/// group's full view (private monitors, full failure detail) but never the
/// operator's streams (deploy markers) nor another group's private names; the
/// global viewer token is the operator. An unknown group - or a fully private
/// one viewed without a token - answers 404, exactly like a missing page.
pub(crate) async fn group_page(
    State(state): State<AppState>,
    Path(group): Path<String>,
    viewer: Viewer,
    chrome: Chrome,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let audience = viewer.audience_for_group(&group);
    status_response(&state, &viewer, &audience, Some(&group), chrome, &headers).await
}

/// One monitor's page (`/monitor/{id}`): its figures, its days, where it is
/// checked from, its incidents. Visibility follows the status pages: a
/// private monitor answers 404 unless the operator token, or its group's
/// token, is presented.
pub(crate) async fn monitor_page(
    State(state): State<AppState>,
    Path(id): Path<String>,
    viewer: Viewer,
    chrome: Chrome,
) -> Result<Html<String>, AppError> {
    let config = &viewer.config;
    let monitor = config
        .monitors
        .iter()
        .find(|monitor| monitor.id == id)
        .ok_or(AppError::NotFound("unknown monitor"))?;
    let audience = match &monitor.group {
        Some(group) => viewer.audience_for_group(group),
        None => viewer.audience.clone(),
    };
    let visibility = Visibility::new(config, &audience);
    if !visibility.can_see_monitor(monitor) {
        return Err(AppError::NotFound("unknown monitor"));
    }
    let summary = state.snapshot().await.summary(config, &audience);
    let view = summary
        .monitors
        .iter()
        .find(|view| view.id == id)
        .cloned()
        .ok_or(AppError::NotFound("unknown monitor"))?;

    let mut incidents = visible_incidents(&state.pool, &visibility, 200).await?;
    incidents.retain(|incident| incident.monitor_id == id);

    // The heatmap, inlined so the stylesheet themes it; cached like the image.
    let heatmap = if monitor.kind == Kind::Push {
        None
    } else {
        let key = format!("{id}#inline");
        let svg = state
            .heatmaps
            .get_or_build(&key, config, async {
                let now = Utc::now().timestamp();
                let since = (now / 86_400 - (crate::heatmap::HEATMAP_DAYS - 1)) * 86_400;
                let cells = db::latency_hourly(&state.pool, &id, since).await?;
                Ok::<_, sqlx::Error>(crate::heatmap::render(&cells, now, &monitor.name, false))
            })
            .await?;
        Some(svg.as_ref().clone())
    };

    let operator = viewer.is_operator();
    let group_view = matches!(audience, crate::visibility::Audience::Group(_));
    let page = crate::monitor_page::build(
        view,
        config,
        monitor,
        &incidents,
        &summary.events,
        heatmap,
        operator || visibility.detailed(&id),
        operator || group_view,
    );
    let html = crate::monitor_page::MonitorTemplate {
        chrome: chrome.at("status").operator(operator),
        page,
    }
    .render()?;
    Ok(Html(html))
}

/// The operator's view of the mesh (`/watchers`): this node's `/healthz`
/// view of its peers, the 30-day confirmations, each shared service from
/// each place. Anyone else gets a 404, like any page that is not theirs.
pub(crate) async fn watchers_page(
    State(state): State<AppState>,
    viewer: Viewer,
    chrome: Chrome,
) -> Result<Html<String>, AppError> {
    if !viewer.is_operator() {
        return Err(AppError::NotFound("unknown page"));
    }
    let config = &viewer.config;
    let health = hora_core::mesh::peer::report(&state.pool, config, &state.last_tick).await;
    let summary = state.snapshot().await.summary(config, &viewer.audience);
    let incidents = db::recent_incidents(&state.pool, 1000).await?;
    let html = crate::watchers::WatchersTemplate {
        chrome: chrome.at("watchers").operator(true),
        page: crate::watchers::build(config, &health, &summary, &incidents),
    }
    .render()?;
    Ok(Html(html))
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReportQuery {
    /// Restrict the report to one display group - the report twin of the
    /// `/status/{group}` page, and what an operator hands a client.
    #[serde(default)]
    group: Option<String>,
}

/// The printable monthly SLA report (`/report/2026-05`, optionally
/// `?group=X`). Anonymous viewers get the public monitors; the viewer token
/// includes the private ones, and on a `?group=` report the group's own
/// `server.group_tokens` entry does too - so a client can be handed *their*
/// report and nothing else. Server-rendered and print-first: "Save as PDF"
/// is the export.
pub(crate) async fn report_page(
    State(state): State<AppState>,
    Path(month): Path<String>,
    viewer: Viewer,
    chrome: Chrome,
    Query(query): Query<ReportQuery>,
) -> Result<Html<String>, AppError> {
    let config = &viewer.config;
    // Validate the month *before* building, so a malformed path is a clean
    // 400 and never reaches the database.
    let Some((_, end)) = hora_core::report::month_bounds(&month) else {
        return Err(AppError::BadRequest(
            "month must be YYYY-MM and not in the future",
        ));
    };
    // How far back a monthly report may reach: the daily aggregates it reads
    // live that long; an older month would only be an empty - and needlessly
    // expensive - report.
    if end <= Utc::now().timestamp() - db::AGGREGATE_RETENTION_DAYS * hora_core::SECONDS_PER_DAY {
        return Err(AppError::BadRequest(
            "month is older than the retained history (12 months)",
        ));
    }
    let audience = match query.group.as_deref() {
        Some(group) => viewer.audience_for_group(group),
        None => viewer.audience.clone(),
    };
    let visibility = Visibility::new(config, &audience);

    // The built report does not depend on the audience (rows are filtered
    // below), so one copy per month serves every viewer for a minute.
    let report = state
        .reports
        .get_or_build(
            &month,
            config,
            hora_core::report::build(&state.pool, config, &month),
        )
        .await?;
    let shown = |row: &hora_core::report::MonitorMonth| {
        visibility.can_see(&row.id)
            && query
                .group
                .as_deref()
                .is_none_or(|group| row.group.as_deref() == Some(group))
    };
    let groups = crate::report::group_rows(&report, &config.maintenance, shown);
    let kpis = crate::report::kpis(&report, shown);
    // A scoped report with nothing visible answers like the group page: 404,
    // revealing neither the group's existence nor its members.
    if groups.is_empty() && query.group.is_some() {
        return Err(AppError::NotFound("unknown group"));
    }
    let title = match &query.group {
        Some(group) => format!("{} · {group}", config.page.title),
        None => config.page.title.clone(),
    };
    let services = groups
        .iter()
        .map(crate::report::ReportGroup::len)
        .sum::<usize>();
    let lede = format!(
        "{}, times in UTC. {services} service{}.",
        crate::report::span(&month),
        if services == 1 { "" } else { "s" }
    );
    let (prev, next) = crate::report::neighbours(&month);
    let month_query = match (&query.group, chrome.req.q.is_empty()) {
        (Some(group), true) => format!("?group={}", hora_core::fmt::percent_encode(group)),
        (Some(group), false) => format!(
            "{}&group={}",
            chrome.req.q,
            hora_core::fmt::percent_encode(group)
        ),
        (None, _) => chrome.req.q.clone(),
    };
    let html = crate::report::ReportTemplate {
        chrome: chrome.at("report").operator(viewer.is_operator()),
        title,
        label: report.label.clone(),
        lede,
        generated: Utc::now().format("%-d %B %Y, %H:%M UTC").to_string(),
        kpis,
        groups,
        prev,
        next,
        month_query,
    }
    .render()?;
    Ok(Html(html))
}

/// The most rows a visibility-filtered list scans to fill its page: the
/// newest rows may all belong to private monitors, so the scan widens until
/// `limit` visible rows are found - bounded, since the endpoints are public.
const MAX_VISIBILITY_SCAN: i64 = 5_000;

/// Collect up to `limit` rows the audience may see, filtering *before*
/// truncating: `fetch(n)` returns the newest `n` rows, `filter` keeps (and
/// sanitizes) the visible ones. The window widens 4x per round until enough
/// rows are visible, the table is exhausted, or [`MAX_VISIBILITY_SCAN`] is hit.
async fn collect_visible<T, Fut>(
    limit: i64,
    mut fetch: impl FnMut(i64) -> Fut,
    filter: impl Fn(&mut Vec<T>),
) -> Result<Vec<T>, AppError>
where
    Fut: Future<Output = sqlx::Result<Vec<T>>>,
{
    let wanted = usize::try_from(limit).unwrap_or(0);
    let mut window = limit.max(1);
    loop {
        let mut rows = fetch(window).await?;
        let exhausted = usize::try_from(window).is_ok_and(|window| rows.len() < window);
        filter(&mut rows);
        if rows.len() >= wanted || exhausted || window >= MAX_VISIBILITY_SCAN {
            rows.truncate(wanted);
            return Ok(rows);
        }
        window = window.saturating_mul(4).min(MAX_VISIBILITY_SCAN);
    }
}

/// Recent incidents restricted to (and sanitized for) what the audience may
/// see - see [`Visibility::sanitize_incident`].
async fn visible_incidents(
    pool: &sqlx::SqlitePool,
    visibility: &Visibility<'_>,
    limit: i64,
) -> Result<Vec<db::Incident>, AppError> {
    collect_visible(
        limit,
        |window| db::recent_incidents(pool, window),
        |rows| visibility.filter_incidents(rows),
    )
    .await
}

/// Recent pushed alerts restricted to (and sanitized for) what the audience
/// may see - see [`Visibility::sanitize_alert`].
async fn visible_pushed_alerts(
    pool: &sqlx::SqlitePool,
    visibility: &Visibility<'_>,
    limit: i64,
) -> Result<Vec<db::PushedAlert>, AppError> {
    collect_visible(
        limit,
        |window| db::recent_pushed_alerts(pool, window),
        |rows| visibility.filter_alerts(rows),
    )
    .await
}

/// The auto-generated post-mortem page (`/incident/{id}`): everything the
/// incident already knows, assembled server-side, with the raw markdown ready
/// to copy into a ticket. Visibility mirrors the history page: a private
/// monitor's incident (or one whose monitor left the config) answers 404 to
/// anonymous viewers, and a public one is sanitized unless the monitor opted
/// into `public_error_detail`.
pub(crate) async fn incident_page(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    viewer: Viewer,
    chrome: Chrome,
) -> Result<Html<String>, AppError> {
    let config = &viewer.config;
    let visibility = Visibility::new(config, &viewer.audience);
    let mut incident = db::incident_by_id(&state.pool, id)
        .await?
        .ok_or(AppError::NotFound("unknown incident"))?;
    if !visibility.can_see(&incident.monitor_id) {
        return Err(AppError::NotFound("unknown incident"));
    }
    visibility.sanitize_incident(&mut incident);

    let monitor_name = config.monitor_name(&incident.monitor_id).to_owned();
    let markdown = hora_core::postmortem::render(&incident, &monitor_name);
    let operator = viewer.is_operator();
    let html = crate::history::IncidentTemplate {
        chrome: chrome.at("history").operator(operator),
        row: crate::history::incident_rows(std::slice::from_ref(&incident), &monitor_names(config))
            .remove(0),
        markdown,
        owl: crate::render::owl(
            if incident.ended_at.is_some() {
                "up"
            } else {
                "down"
            },
            80,
        ),
        operator,
    }
    .render()?;
    Ok(Html(html))
}

/// Map of monitor id to display name, for rendering incidents.
fn monitor_names(config: &Config) -> std::collections::HashMap<String, String> {
    config
        .monitors
        .iter()
        .map(|monitor| (monitor.id.clone(), monitor.name.clone()))
        .collect()
}

pub(crate) async fn history_page(
    State(state): State<AppState>,
    viewer: Viewer,
    chrome: Chrome,
) -> Result<Html<String>, AppError> {
    let config = &viewer.config;
    let visibility = Visibility::new(config, &viewer.audience);
    let incidents = visible_incidents(&state.pool, &visibility, 100).await?;
    let pushed_alerts = visible_pushed_alerts(&state.pool, &visibility, 100).await?;
    // Event markers are operator info (deploy titles).
    let events = if visibility.sees_operator_streams() {
        db::recent_events(&state.pool, 100).await?
    } else {
        Vec::new()
    };
    // The latency heatmaps live on each monitor's own page now.
    let names = monitor_names(config);
    let html = history::HistoryTemplate {
        chrome: chrome.at("history").operator(viewer.is_operator()),
        incidents: history::incident_rows(&incidents, &names),
        pushed_alerts: history::alert_rows(&pushed_alerts, &names),
        events: history::event_rows(&events),
    }
    .render()?;
    Ok(Html(html))
}

/// How far back the `/timeline` page reaches, and how many entries it shows.
const TIMELINE_DAYS: i64 = 7;
const TIMELINE_LIMIT: usize = 200;

/// The unified chronology (`/timeline`): downs/recoveries, operator events,
/// pushed alerts, announcements and silences merged newest-first. The
/// operator view gets everything; anyone else gets the same subset they may
/// see elsewhere - sanitized visible incidents and announcements - and never
/// the operator streams (events, silences, alert detail).
pub(crate) async fn timeline_page(
    State(state): State<AppState>,
    viewer: Viewer,
    chrome: Chrome,
) -> Result<Html<String>, AppError> {
    let config = &viewer.config;
    let visibility = Visibility::new(config, &viewer.audience);
    let since = Utc::now().timestamp() - TIMELINE_DAYS * hora_core::SECONDS_PER_DAY;

    let sources = if visibility.sees_operator_streams() {
        hora_core::timeline::fetch(&state.pool, since, 500).await?
    } else {
        let mut incidents = visible_incidents(&state.pool, &visibility, 500).await?;
        incidents.retain(|incident| {
            incident.started_at >= since || incident.ended_at.is_some_and(|ended| ended >= since)
        });
        hora_core::timeline::Sources {
            incidents,
            announcements: db::announcements_since(&state.pool, since).await?,
            ..Default::default()
        }
    };
    let entries =
        hora_core::timeline::merge(&sources, &monitor_names(config), since, TIMELINE_LIMIT);

    let html = history::TimelineTemplate {
        chrome: chrome.at("timeline").operator(viewer.is_operator()),
        entries: history::timeline_rows(&entries),
    }
    .render()?;
    Ok(Html(html))
}

pub(crate) async fn history_atom(
    State(state): State<AppState>,
    viewer: Viewer,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let config = &viewer.config;
    let visibility = Visibility::new(config, &viewer.audience);
    let incidents = visible_incidents(&state.pool, &visibility, 50).await?;
    // Absolute feed links: scheme from the proxy's x-forwarded-proto (plain
    // http when absent), host from the Host header.
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|proto| *proto == "https" || *proto == "http")
        .unwrap_or("http");
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("localhost");
    let base_url = format!("{proto}://{host}");
    let body = history::render_atom(
        &incidents,
        &monitor_names(config),
        &base_url,
        &config.page.title,
    );
    Ok((
        [(header::CONTENT_TYPE, "application/atom+xml; charset=utf-8")],
        body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn collect_visible_widens_until_enough_rows_are_visible() {
        // 100 rows, newest first; only every 10th is "visible".
        let rows: Vec<i64> = (0..100).collect();
        let mut fetched = Vec::new();
        let visible = collect_visible(
            5,
            |window| {
                fetched.push(window);
                let page: Vec<i64> = rows
                    .iter()
                    .copied()
                    .take(usize::try_from(window).unwrap())
                    .collect();
                async move { Ok(page) }
            },
            |page: &mut Vec<i64>| page.retain(|row| row % 10 == 0),
        )
        .await
        .ok()
        .expect("rows");
        // Filtering happened before the limit: five visible rows, not the
        // one or two a "take 5 then filter" would leave.
        assert_eq!(visible, vec![0, 10, 20, 30, 40]);
        assert_eq!(fetched, vec![5, 20, 80]);
    }

    #[tokio::test]
    async fn collect_visible_stops_when_the_table_is_exhausted() {
        let visible = collect_visible(
            50,
            |_window| async { Ok(vec![1_i64, 2, 3]) },
            |page: &mut Vec<i64>| page.retain(|row| *row != 2),
        )
        .await
        .ok()
        .expect("rows");
        assert_eq!(visible, vec![1, 3]);
    }
}
