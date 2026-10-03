//! Reads over the check history and its hourly/daily rollups: availability,
//! daily bars and latency series, mostly batched for every monitor at once.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;
use sqlx::SqlitePool;

use crate::SECONDS_PER_DAY;

/// Per-day aggregate of check statuses.
#[derive(Debug, sqlx::FromRow)]
pub struct DayRow {
    pub day: String,
    pub up: i64,
    pub down: i64,
    pub degraded: i64,
}

/// A single latency sample, also serialized directly by the latency API.
#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct Point {
    pub t: i64,
    pub latency_ms: i64,
}

/// `(available, total)` check counts since `since`. Available = up or degraded.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn availability(
    pool: &SqlitePool,
    monitor_id: &str,
    since: i64,
) -> sqlx::Result<(i64, i64)> {
    sqlx::query_as::<_, (i64, i64)>(concat!(
        "SELECT ",
        available_total!(),
        " FROM checks WHERE monitor_id = ? AND time >= ?"
    ))
    .bind(monitor_id)
    .bind(since)
    .fetch_one(pool)
    .await
}

/// Latency samples since `since`, averaged into time buckets of `bucket_secs`
/// (must be `>= 1`), oldest first (rows without latency are skipped). Aggregating
/// in SQL caps the rows materialized at `window / bucket_secs` however dense the
/// checks are, so a wide window on a high-frequency monitor can't pull hundreds
/// of thousands of rows into memory. Buckets are anchored to the epoch, not to
/// `since`, so two consecutive requests group the same rows identically and an
/// auto-refreshing chart doesn't jitter. Ordering by the group key (`MIN(time)`
/// is monotone in it) lets `SQLite` reuse the GROUP BY sort instead of building
/// a second temp b-tree.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn latency_series(
    pool: &SqlitePool,
    monitor_id: &str,
    since: i64,
    bucket_secs: i64,
) -> sqlx::Result<Vec<Point>> {
    sqlx::query_as::<_, Point>(
        "SELECT MIN(time) AS t, CAST(AVG(latency_ms) AS INTEGER) AS latency_ms FROM checks \
         WHERE monitor_id = ?1 AND time >= ?2 AND latency_ms IS NOT NULL \
         GROUP BY time / ?3 ORDER BY time / ?3",
    )
    .bind(monitor_id)
    .bind(since)
    .bind(bucket_secs)
    .fetch_all(pool)
    .await
}

// --- Batch reads: one query for every monitor, used to build the summary ----
// The status page batches the 24h/90d aggregates here (keyed by `monitor_id`)
// instead of running them per monitor; the covering index serves every one.
// `recent_checks` stays per-monitor: an indexed `LIMIT N` is already minimal and,
// unlike a windowed batch, is correct for monitors checked less than once a day.

/// `(available, total)` per monitor since `since`. Available = up or degraded.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn availability_all(
    pool: &SqlitePool,
    since: i64,
) -> sqlx::Result<HashMap<String, (i64, i64)>> {
    let rows = sqlx::query_as::<_, (String, i64, i64)>(concat!(
        "SELECT monitor_id, ",
        available_total!(),
        " FROM checks WHERE time >= ? GROUP BY monitor_id"
    ))
    .bind(since)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, available, total)| (id, (available, total)))
        .collect())
}

/// Daily up/down/degraded aggregates per monitor over `[since, until]`, oldest
/// first. The page passes its current timestamp as `until`; the monthly report
/// passes the end of the month (or now, for the running month), so later data
/// is never scanned. Buckets are included when they *start* inside the range.
///
/// Every ended hour is rolled up into `checks_hourly` (see
/// `retention::HOURLY_ROLLUP_LAG_SECS`), so the hourly buckets cover everything
/// below the newest bucket and the raw checks are only read above it: a few
/// hours (one `maintenance::PRUNE_INTERVAL` at most) instead of re-aggregating
/// days of raw rows on every status-page rebuild. Both reads are bounded by the same frontier,
/// so they stay disjoint (and add up) even if a roll-up commits in between.
/// `checks_daily` takes over once the hourly buckets age out; for each
/// `(monitor, day)` the source with the most samples wins, which resolves the
/// boundary day whose hours were partly pruned after their daily roll-up.
///
/// # Errors
///
/// Returns an error if a query fails.
pub async fn daily_all(
    pool: &SqlitePool,
    since: i64,
    until: i64,
) -> sqlx::Result<HashMap<String, Vec<DayRow>>> {
    let newest_hour: Option<i64> = sqlx::query_scalar("SELECT MAX(hour) FROM checks_hourly")
        .fetch_one(pool)
        .await?;
    // Past `until` nothing is read, so the frontier never needs to exceed it.
    let frontier = newest_hour.map_or(since, |hour| since.max(hour + 3600).min(until + 1));
    // Days are grouped as integer UTC day numbers (`time / 86400`) and only
    // formatted once per bucket below: a per-row `strftime` string, and the
    // string-keyed GROUP BY it forces, was half of this scan's cost.
    let raw = sqlx::query_as::<_, (String, i64, i64, i64, i64)>(
        "SELECT monitor_id, time / 86400 AS day, \
            SUM(status = 1), SUM(status = 0), SUM(status = 2) \
         FROM checks WHERE time >= ? AND time <= ? GROUP BY monitor_id, day",
    )
    .bind(frontier)
    .bind(until)
    .fetch_all(pool)
    .await?;
    let hourly = sqlx::query_as::<_, (String, i64, i64, i64, i64)>(
        "SELECT monitor_id, hour / 86400 AS day, \
            SUM(up_count), SUM(down_count), SUM(degraded_count) \
         FROM checks_hourly WHERE hour >= ? AND hour < ? GROUP BY monitor_id, day",
    )
    .bind(since)
    .bind(frontier)
    .fetch_all(pool)
    .await?;
    let daily = sqlx::query_as::<_, (String, i64, i64, i64, i64)>(
        "SELECT monitor_id, day / 86400 AS day, \
            up_count, down_count, degraded_count \
         FROM checks_daily WHERE day >= ? AND day <= ?",
    )
    .bind(since)
    .bind(until)
    .fetch_all(pool)
    .await?;

    // Hourly buckets and raw rows cover disjoint ranges of the same day: sum.
    let mut best: HashMap<String, BTreeMap<i64, (i64, i64, i64)>> = HashMap::new();
    for (id, day, up, down, degraded) in raw.into_iter().chain(hourly) {
        let slot = best.entry(id).or_default().entry(day).or_insert((0, 0, 0));
        *slot = (slot.0 + up, slot.1 + down, slot.2 + degraded);
    }
    for (id, day, up, down, degraded) in daily {
        let slot = best.entry(id).or_default().entry(day).or_insert((0, 0, 0));
        if up + down + degraded > slot.0 + slot.1 + slot.2 {
            *slot = (up, down, degraded);
        }
    }
    // BTreeMap keys are day numbers, so iteration order is oldest-first already.
    Ok(best
        .into_iter()
        .map(|(id, days)| {
            let rows = days
                .into_iter()
                .map(|(day, (up, down, degraded))| DayRow {
                    day: iso_day(day),
                    up,
                    down,
                    degraded,
                })
                .collect();
            (id, rows)
        })
        .collect())
}

/// A UTC day number (`unix_secs / 86400`) as `YYYY-MM-DD`, the format SQLite's
/// `strftime('%Y-%m-%d', …, 'unixepoch')` produced.
pub(super) fn iso_day(day: i64) -> String {
    crate::fmt::date(day * SECONDS_PER_DAY)
}

/// 24h latency percentiles (p50/p95/p99) per monitor, computed in SQL so the raw
/// samples never have to be pulled into memory. The nearest-rank rule mirrors the
/// former in-Rust computation: `rank = ceil(p * n / 100)`, clamped into `[1, n]`,
/// and the value at that rank (oldest-to-largest order) is returned.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn latency_percentiles_all(
    pool: &SqlitePool,
    since: i64,
) -> sqlx::Result<HashMap<String, (i64, i64, i64)>> {
    let rows = sqlx::query_as::<_, (String, i64, i64, i64)>(
        "WITH ranked AS ( \
             SELECT monitor_id, latency_ms, \
                    ROW_NUMBER() OVER (PARTITION BY monitor_id ORDER BY latency_ms) AS rn, \
                    COUNT(*)     OVER (PARTITION BY monitor_id)                     AS n \
             FROM checks \
             WHERE time >= ?1 AND latency_ms IS NOT NULL \
         ) \
         SELECT monitor_id, \
                MAX(CASE WHEN rn = MAX(MIN((50 * n + 99) / 100, n), 1) THEN latency_ms END), \
                MAX(CASE WHEN rn = MAX(MIN((95 * n + 99) / 100, n), 1) THEN latency_ms END), \
                MAX(CASE WHEN rn = MAX(MIN((99 * n + 99) / 100, n), 1) THEN latency_ms END) \
         FROM ranked \
         GROUP BY monitor_id",
    )
    .bind(since)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(id, p50, p95, p99)| (id, (p50, p95, p99)))
        .collect())
}

/// Latency samples for the per-monitor sparkline, averaged into time buckets of
/// `bucket_secs` (must be `>= 1`) so the series stays small however dense the
/// checks are: one query, at most `window / bucket_secs` points per monitor,
/// oldest first. This caps both the memory held and the size of the rendered SVG.
/// Buckets are anchored to the epoch, like [`latency_series`], so consecutive
/// rebuilds group the same rows and the sparkline doesn't jitter.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn latency_sparkline_all(
    pool: &SqlitePool,
    since: i64,
    bucket_secs: i64,
) -> sqlx::Result<HashMap<String, Vec<Point>>> {
    let rows = sqlx::query_as::<_, (String, i64, i64)>(
        "SELECT monitor_id, MIN(time) AS t, CAST(AVG(latency_ms) AS INTEGER) AS latency_ms \
         FROM checks \
         WHERE time >= ?1 AND latency_ms IS NOT NULL \
         GROUP BY monitor_id, time / ?2 \
         ORDER BY monitor_id, t ASC",
    )
    .bind(since)
    .bind(bucket_secs)
    .fetch_all(pool)
    .await?;

    let mut map: HashMap<String, Vec<Point>> = HashMap::new();
    for (id, t, latency_ms) in rows {
        map.entry(id).or_default().push(Point { t, latency_ms });
    }
    Ok(map)
}

/// Hourly average latency for one monitor since `since`: `(hour, avg_ms)`
/// pairs, hour-aligned, oldest first. Reads the raw checks *and* the
/// downsampled `checks_hourly` buckets so the series extends beyond the raw
/// retention window; where both cover an hour the raw average wins (it is
/// authoritative while complete). Feeds the latency heatmap.
///
/// # Errors
///
/// Returns an error if a query fails.
pub async fn latency_hourly(
    pool: &SqlitePool,
    monitor_id: &str,
    since: i64,
) -> sqlx::Result<Vec<(i64, i64)>> {
    let buckets = sqlx::query_as::<_, (i64, i64)>(
        "SELECT hour, avg_latency_ms FROM checks_hourly \
         WHERE monitor_id = ?1 AND hour >= ?2 AND avg_latency_ms IS NOT NULL",
    )
    .bind(monitor_id)
    .bind(since)
    .fetch_all(pool)
    .await?;
    let raw = sqlx::query_as::<_, (i64, i64)>(
        "SELECT (time / 3600) * 3600 AS hour, CAST(AVG(latency_ms) AS INTEGER) FROM checks \
         WHERE monitor_id = ?1 AND time >= ?2 AND latency_ms IS NOT NULL GROUP BY hour",
    )
    .bind(monitor_id)
    .bind(since)
    .fetch_all(pool)
    .await?;

    // Buckets first, raw second: on overlap the raw average overwrites.
    let merged: BTreeMap<i64, i64> = buckets.into_iter().chain(raw).collect();
    Ok(merged.into_iter().collect())
}
