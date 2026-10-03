//! Retention: downsampling the raw checks into hourly and daily buckets, aging
//! every table out, and sweeping the rows of monitors removed from the config.

use std::collections::{BTreeMap, HashMap};

use sqlx::SqlitePool;

use crate::SECONDS_PER_DAY;
use crate::config::{Config, Peer};
use crate::histogram::LatencyHistogram;

use super::annotations::{meta_delete, meta_set};
use super::{CERT_PIN_AGAINST_META_PREFIX, HEARTBEAT_EXPECTED_META_PREFIX};

/// Raw checks roll up into hourly buckets once the hour ended this long ago.
/// Checks are stamped with the time they are recorded, so an ended hour only
/// needs a margin for an insert whose timestamp was taken just before the
/// boundary and committed just after it.
const HOURLY_ROLLUP_LAG_SECS: i64 = 300;
/// Hourly buckets roll up into daily ones (and are pruned) once older than this.
const DOWNSAMPLE_DAILY_AFTER_DAYS: i64 = 90;
/// Daily buckets and closed incidents are kept this long.
pub const AGGREGATE_RETENTION_DAYS: i64 = 365;

/// How long a monitor id missing from the config keeps its history before the
/// pruner deletes it. A rename, a monitor commented out for an afternoon or a
/// bad merge must not cost a year of aggregates and incidents on the next
/// prune tick; a week is long enough to notice and restore it.
pub(super) const ORPHAN_GRACE_SECS: i64 = 7 * SECONDS_PER_DAY;
/// `meta` key prefix recording when an id was first seen missing from the
/// config (`orphan_since:<id>` = unix seconds).
const ORPHAN_META_PREFIX: &str = "orphan_since:";
/// Every `meta` key prefix that holds per-id state, swept with the id's rows
/// (see [`delete_orphans`]).
const PER_ID_META_PREFIXES: &[&str] =
    &[HEARTBEAT_EXPECTED_META_PREFIX, CERT_PIN_AGAINST_META_PREFIX];

/// Drop announcements whose expiry passed before `cutoff`.
pub(super) async fn prune_announcements(pool: &SqlitePool, cutoff: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM announcements WHERE until IS NOT NULL AND until < ?")
        .bind(cutoff)
        .execute(pool)
        .await?;
    Ok(())
}

/// Drop event markers older than `cutoff` (they age out with the closed
/// incidents they may have been correlated into).
pub(super) async fn prune_events(pool: &SqlitePool, cutoff: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM events WHERE created_at < ?")
        .bind(cutoff)
        .execute(pool)
        .await?;
    Ok(())
}

/// Drop pushed alerts older than `cutoff` (by `created_at`).
async fn prune_pushed_alerts(pool: &SqlitePool, cutoff: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM pushed_alerts WHERE created_at < ?")
        .bind(cutoff)
        .execute(pool)
        .await?;
    Ok(())
}

/// Drop silences that have expired before `cutoff`.
pub(super) async fn prune_silences(pool: &SqlitePool, cutoff: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM silences WHERE until < ?")
        .bind(cutoff)
        .execute(pool)
        .await?;
    Ok(())
}

/// Aggregate raw checks into hourly buckets for long-term storage.
///
/// Only hours that lie *entirely* below `cutoff` are aggregated, and a bucket
/// is written once (`INSERT OR IGNORE`): a complete hour aggregated from
/// still-complete raw data is final. Recomputing it later (`OR REPLACE`) could
/// silently shrink it once retention starts eating the raw rows it came from.
///
/// Incremental: because buckets are write-once, everything at or below the
/// newest bucket is already rolled up, so only raw rows above it are scanned
/// (an indexed range of roughly one prune interval). The old full scan held
/// the write lock for seconds on a months-old database - long enough for the
/// scheduler's inserts to time out ("database is locked"). Checks are always
/// recorded at the current time, so no raw row can appear below a bucket that
/// was complete when written.
///
/// # Errors
///
/// Returns an error if the aggregation fails.
pub async fn downsample_hourly(pool: &SqlitePool, cutoff: i64) -> sqlx::Result<()> {
    let newest: Option<i64> = sqlx::query_scalar("SELECT MAX(hour) FROM checks_hourly")
        .fetch_one(pool)
        .await?;
    let floor = newest.map_or(i64::MIN, |hour| hour + 3600);
    // Only whole hours below the cutoff: with none ended since the newest
    // bucket (most ticks), there is nothing to scan at all.
    let end = cutoff.div_euclid(3600) * 3600;
    if end <= floor {
        return Ok(());
    }
    sqlx::query(
        "INSERT OR IGNORE INTO checks_hourly \
            (monitor_id, hour, up_count, down_count, degraded_count, avg_latency_ms) \
         SELECT \
           monitor_id, \
           (time / 3600) * 3600 AS hour, \
           SUM(CASE WHEN status = 1 THEN 1 ELSE 0 END), \
           SUM(CASE WHEN status = 0 THEN 1 ELSE 0 END), \
           SUM(CASE WHEN status = 2 THEN 1 ELSE 0 END), \
           CAST(AVG(latency_ms) AS INTEGER) \
         FROM checks \
         WHERE time >= ? AND time < ? \
         GROUP BY monitor_id, hour",
    )
    .bind(floor)
    .bind(end)
    .execute(pool)
    .await?;
    Ok(())
}

/// Hours whose latency histograms one [`fill_latency_histograms`] call
/// computes at most: a day and a bit, what the 24h window reads.
const MAX_HISTOGRAM_HOURS_PER_FILL: usize = 26;

/// Compute the latency histogram (see [`crate::histogram`]) of every hourly
/// bucket at or after `since` that lacks one: the hours just rolled up, and
/// after an upgrade the last day's buckets written before histograms existed.
/// One hour of raw checks is read per bucket hour (outside any write lock);
/// the hour's histograms are then written in one short transaction. Monitors
/// without a latency sample that hour get the empty histogram, so "not
/// computed" (NULL) never lingers.
///
/// # Errors
///
/// Returns an error if a query fails.
pub async fn fill_latency_histograms(pool: &SqlitePool, since: i64) -> sqlx::Result<()> {
    let hours: Vec<i64> = sqlx::query_scalar(
        "SELECT DISTINCT hour FROM checks_hourly \
         WHERE hour >= ? AND latency_hist IS NULL ORDER BY hour DESC LIMIT ?",
    )
    .bind(since)
    .bind(i64::try_from(MAX_HISTOGRAM_HOURS_PER_FILL).unwrap_or(i64::MAX))
    .fetch_all(pool)
    .await?;
    for hour in hours {
        let rows = sqlx::query_as::<_, (String, i64, i64)>(
            "SELECT monitor_id, latency_ms, COUNT(*) FROM checks \
             WHERE time >= ? AND time < ? AND latency_ms IS NOT NULL \
             GROUP BY monitor_id, latency_ms",
        )
        .bind(hour)
        .bind(hour + 3600)
        .fetch_all(pool)
        .await?;
        let mut histograms: HashMap<String, LatencyHistogram> = HashMap::new();
        for (id, latency_ms, count) in rows {
            histograms
                .entry(id)
                .or_default()
                .record(latency_ms, u64::try_from(count).unwrap_or(0));
        }
        let mut tx = pool.begin().await?;
        for (id, histogram) in &histograms {
            sqlx::query(
                "UPDATE checks_hourly SET latency_hist = ? \
                 WHERE monitor_id = ? AND hour = ? AND latency_hist IS NULL",
            )
            .bind(histogram.encode())
            .bind(id)
            .bind(hour)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "UPDATE checks_hourly SET latency_hist = ? WHERE hour = ? AND latency_hist IS NULL",
        )
        .bind(LatencyHistogram::default().encode())
        .bind(hour)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
    }
    Ok(())
}

/// Roll every ended hour up (counts, then latency histograms). Cheap when
/// no hour ended since the last call, so the maintenance task runs it every
/// few minutes: the status page reads ended hours from the buckets and only
/// the raw rows above them, so a fresh frontier keeps that raw tail short.
///
/// # Errors
///
/// Returns an error if a query fails.
pub async fn roll_up_recent(pool: &SqlitePool, now: i64) -> sqlx::Result<()> {
    downsample_hourly(pool, now - HOURLY_ROLLUP_LAG_SECS).await?;
    // The 24h window's hours, plus the one it starts in.
    fill_latency_histograms(pool, now - SECONDS_PER_DAY - 3600).await
}

/// Aggregate hourly buckets into daily ones for even longer-term storage.
///
/// Same write-once rule (and the same incremental floor) as
/// [`downsample_hourly`]: only days entirely below `cutoff`, inserted once,
/// scanning only the hourly buckets above the newest daily one. The latency
/// average is weighted by each hour's sample count, so a quiet hour does not
/// skew the day.
///
/// # Errors
///
/// Returns an error if the aggregation fails.
pub async fn downsample_daily(pool: &SqlitePool, cutoff: i64) -> sqlx::Result<()> {
    let newest: Option<i64> = sqlx::query_scalar("SELECT MAX(day) FROM checks_daily")
        .fetch_one(pool)
        .await?;
    let floor = newest.map_or(i64::MIN, |day| day + 86400);
    sqlx::query(
        "INSERT OR IGNORE INTO checks_daily \
            (monitor_id, day, up_count, down_count, degraded_count, avg_latency_ms) \
         SELECT \
           monitor_id, \
           (hour / 86400) * 86400 AS day, \
           SUM(up_count), \
           SUM(down_count), \
           SUM(degraded_count), \
           CAST(SUM(avg_latency_ms * (up_count + degraded_count)) \
                / NULLIF(SUM(CASE WHEN avg_latency_ms IS NOT NULL \
                              THEN up_count + degraded_count END), 0) AS INTEGER) \
         FROM checks_hourly \
         WHERE hour >= ? AND hour < ? \
         GROUP BY monitor_id, day \
         HAVING day + 86400 <= ?",
    )
    .bind(floor)
    .bind(cutoff)
    .bind(cutoff)
    .execute(pool)
    .await?;
    Ok(())
}

/// Prune old hourly aggregates beyond the retention period.
///
/// # Errors
///
/// Returns an error if the deletion fails.
pub async fn prune_hourly(pool: &SqlitePool, cutoff: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM checks_hourly WHERE hour < ?")
        .bind(cutoff)
        .execute(pool)
        .await?;
    Ok(())
}

/// Prune old daily aggregates beyond the retention period.
///
/// # Errors
///
/// Returns an error if the deletion fails.
pub async fn prune_daily(pool: &SqlitePool, cutoff: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM checks_daily WHERE day < ?")
        .bind(cutoff)
        .execute(pool)
        .await?;
    Ok(())
}

/// Downsample old history and age the aggregates out. Failures are logged and
/// non-fatal: the retention pruning in [`prune`] still runs.
async fn roll_up_history(pool: &SqlitePool, now: i64) {
    // Downsample before any deletion: every ended hour rolls up into an hourly
    // bucket, hourly buckets older than 90 days into daily ones. Each bucket is
    // written exactly once (see `downsample_hourly`), so the aggregates survive
    // after retention prunes the raw rows they came from.
    if let Err(err) = roll_up_recent(pool, now).await {
        tracing::warn!("hourly downsampling failed: {err}");
    }
    let daily_cutoff = now - DOWNSAMPLE_DAILY_AFTER_DAYS * SECONDS_PER_DAY;
    if let Err(err) = downsample_daily(pool, daily_cutoff).await {
        tracing::warn!("daily downsampling failed: {err}");
    }

    // Prune aggregates: hourly beyond 90 days (rounded down to a whole day, so
    // only hours already rolled up into a *complete* daily bucket are dropped),
    // daily beyond a year.
    let hourly_prune_cutoff = (daily_cutoff / 86400) * 86400;
    if let Err(err) = prune_hourly(pool, hourly_prune_cutoff).await {
        tracing::warn!("hourly prune failed: {err}");
    }
    let yearly_cutoff = now - AGGREGATE_RETENTION_DAYS * SECONDS_PER_DAY;
    if let Err(err) = prune_daily(pool, yearly_cutoff).await {
        tracing::warn!("daily prune failed: {err}");
    }
    // Closed incidents age out with the daily aggregates; open ones are kept
    // (they are still being displayed, and close on the next healthy tick).
    if let Err(err) = prune_incidents(pool, yearly_cutoff).await {
        tracing::warn!("incident prune failed: {err}");
    }
    // Expired silences are dead weight the moment they lapse.
    if let Err(err) = prune_silences(pool, now).await {
        tracing::warn!("silence prune failed: {err}");
    }
    // Same for expired announcements (the active query already hides them).
    if let Err(err) = prune_announcements(pool, now).await {
        tracing::warn!("announcement prune failed: {err}");
    }
    // Pushed alerts age out with the closed incidents (a year of timeline).
    if let Err(err) = prune_pushed_alerts(pool, yearly_cutoff).await {
        tracing::warn!("pushed-alert prune failed: {err}");
    }
    // Event markers too: a year covers any incident they could correlate with.
    if let Err(err) = prune_events(pool, yearly_cutoff).await {
        tracing::warn!("event prune failed: {err}");
    }
}

pub(crate) async fn prune(pool: &SqlitePool, config: &Config) -> anyhow::Result<()> {
    let now = chrono::Utc::now().timestamp();

    roll_up_history(pool, now).await;

    // Trim each monitor's history to its retention window. Monitors are grouped
    // by cutoff (most share the default), so this is one DELETE per distinct
    // retention rather than one per monitor.
    let mut ids_by_cutoff: HashMap<i64, Vec<&str>> = HashMap::new();
    for monitor in &config.monitors {
        let retention = i64::from(monitor.retention_days(config.alerts.default_retention_days));
        let cutoff = now - retention * SECONDS_PER_DAY;
        ids_by_cutoff
            .entry(cutoff)
            .or_default()
            .push(monitor.id.as_str());
    }
    // Watched peers store their heartbeats under `listen_id`; trim them to the
    // default retention like any other check series.
    let peer_cutoff = now - i64::from(config.alerts.default_retention_days) * SECONDS_PER_DAY;
    for peer in config.peers.iter().filter(|peer| peer.is_watched()) {
        ids_by_cutoff
            .entry(peer_cutoff)
            .or_default()
            .push(peer.listen_id());
    }
    for (cutoff, ids) in ids_by_cutoff {
        let ids = serde_json::to_string(&ids)?;
        sqlx::query(
            "DELETE FROM checks WHERE time < ? \
             AND monitor_id IN (SELECT value FROM json_each(?))",
        )
        .bind(cutoff)
        .bind(&ids)
        .execute(pool)
        .await?;
    }

    delete_orphans(pool, config, now).await?;

    // Keep the planner statistics current as the tables grow and the prunes
    // reshape them - same rationale as the call in [`connect`]; cheap unless
    // the data actually drifted.
    sqlx::query("PRAGMA optimize").execute(pool).await?;
    Ok(())
}

/// The tables swept for rows left behind by removed monitors, all keyed by a
/// `monitor_id` column.
const ORPHAN_TABLES: [&str; 9] = [
    "checks",
    "certs",
    "cert_pins",
    "domain_expiry",
    "release_watch",
    "incidents",
    "pushed_alerts",
    "checks_hourly",
    "checks_daily",
];

/// Drop everything left behind by monitors removed from the config - once they
/// have been gone for [`ORPHAN_GRACE_SECS`]. Both monitor ids and watched
/// peers' listen ids are kept, so the sweep never deletes a peer's heartbeat
/// history.
///
/// The first sweep that finds an id missing records when (`orphan_since:<id>`
/// in `meta`) and logs a warning naming the ids and the deletion date; an id
/// that comes back before then simply loses its mark. Only a mark older than
/// the grace period deletes anything, so a rename or a monitor commented out
/// for a while keeps its year of history. The id's per-id `meta` state
/// ([`PER_ID_META_PREFIXES`]) goes with its rows.
///
/// The sweep first *reads* which ids each table actually holds and only then
/// deletes the orphaned ones, one targeted `DELETE` per id. The previous
/// `NOT EXISTS` anti-join was one statement per table but a full table scan
/// inside a write transaction - on `checks` that held the write lock for
/// seconds every prune tick, timing out the scheduler's inserts, all to
/// usually delete nothing. Reads don't block writers under WAL, and the
/// targeted deletes only run once a removed monitor's grace has run out.
pub(super) async fn delete_orphans(
    pool: &SqlitePool,
    config: &Config,
    now: i64,
) -> anyhow::Result<()> {
    let keep: std::collections::HashSet<&str> = config
        .monitors
        .iter()
        .map(|m| m.id.as_str())
        .chain(
            config
                .peers
                .iter()
                .filter(|peer| peer.is_watched())
                .map(Peer::listen_id),
        )
        .collect();

    // Orphaned id -> the tables still holding its rows.
    let mut orphans: BTreeMap<String, Vec<&'static str>> = BTreeMap::new();
    for table in ORPHAN_TABLES {
        // A plain DISTINCT read: on `checks` it walks the covering index
        // (fractions of a second even at millions of rows) and, being a read,
        // never holds the write lock. (A recursive-CTE "skip scan" would seek
        // instead of scanning, but its correlated MIN subquery loops forever
        // under some query plans - the UNIQUE autoindex triggers it - so the
        // boring query wins.)
        // The interpolated name comes from the ORPHAN_TABLES const, not input.
        let present: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT DISTINCT monitor_id FROM {table}"
        )))
        .fetch_all(pool)
        .await?;
        for id in present {
            if !keep.contains(id.as_str()) {
                orphans.entry(id).or_default().push(table);
            }
        }
    }

    // When each orphan was first seen missing. Marks of ids that came back
    // (or whose rows are gone anyway) are dropped, so a later removal starts
    // a fresh grace period.
    let marks: Vec<(String, String)> =
        sqlx::query_as("SELECT key, value FROM meta WHERE substr(key, 1, length(?1)) = ?1")
            .bind(ORPHAN_META_PREFIX)
            .fetch_all(pool)
            .await?;
    let mut missing_since: HashMap<String, i64> = HashMap::new();
    for (key, value) in marks {
        let id = &key[ORPHAN_META_PREFIX.len()..];
        match value.parse::<i64>() {
            Ok(since) if orphans.contains_key(id) => {
                missing_since.insert(id.to_owned(), since);
            }
            _ => meta_delete(pool, &key).await?,
        }
    }

    let mut scheduled = Vec::new();
    for (id, tables) in &orphans {
        let key = format!("{ORPHAN_META_PREFIX}{id}");
        match missing_since.get(id) {
            None => {
                meta_set(pool, &key, &now.to_string()).await?;
                scheduled.push(id.as_str());
            }
            Some(&since) if now - since >= ORPHAN_GRACE_SECS => {
                for table in tables {
                    sqlx::query(sqlx::AssertSqlSafe(format!(
                        "DELETE FROM {table} WHERE monitor_id = ?"
                    )))
                    .bind(id)
                    .execute(pool)
                    .await?;
                }
                meta_delete(pool, &key).await?;
                for prefix in PER_ID_META_PREFIXES {
                    meta_delete(pool, &format!("{prefix}{id}")).await?;
                }
                tracing::warn!(
                    monitor = %id,
                    "deleted the history of a monitor missing from the config since {}",
                    crate::fmt::utc(since)
                );
            }
            Some(_) => {}
        }
    }
    // Per-id state of an id that is gone and has no rows to wait for (a push
    // monitor removed before its first heartbeat): nothing to restore, so no
    // grace - otherwise re-adding it months later would start "overdue".
    for prefix in PER_ID_META_PREFIXES {
        let keys: Vec<String> =
            sqlx::query_scalar("SELECT key FROM meta WHERE substr(key, 1, length(?1)) = ?1")
                .bind(prefix)
                .fetch_all(pool)
                .await?;
        for key in keys {
            let id = &key[prefix.len()..];
            if !keep.contains(id) && !orphans.contains_key(id) {
                meta_delete(pool, &key).await?;
            }
        }
    }
    if !scheduled.is_empty() {
        tracing::warn!(
            "monitors no longer in the config: {}; their history will be deleted after {} \
             unless they are restored",
            scheduled.join(", "),
            crate::fmt::utc(now + ORPHAN_GRACE_SECS)
        );
    }
    Ok(())
}

/// Drop closed incidents older than `cutoff` (by start time). Open incidents
/// are never pruned here.
pub(super) async fn prune_incidents(pool: &SqlitePool, cutoff: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM incidents WHERE ended_at IS NOT NULL AND started_at < ?")
        .bind(cutoff)
        .execute(pool)
        .await?;
    Ok(())
}
