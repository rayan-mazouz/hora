//! Raw check rows: probe results, push heartbeats and missed heartbeats.

use sqlx::SqlitePool;

use crate::probe::Outcome;

/// Latest stored check for a monitor.
#[derive(Debug, sqlx::FromRow)]
pub struct Latest {
    pub time: i64,
    pub latency_ms: Option<i64>,
    pub status: i64,
    /// Failure reason (or response snippet) when the check was not up.
    pub error: Option<String>,
}

/// Where a check row came from (the `checks.source` column).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckSource {
    /// The scheduler probed (or ran) the monitor.
    Probe,
    /// An explicit heartbeat on `/api/push/{id}`.
    Push,
    /// The scheduler recorded an overdue heartbeat as down.
    Miss,
}

impl CheckSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Probe => "probe",
            Self::Push => "push",
            Self::Miss => "miss",
        }
    }
}

/// One row of `checks`, as written by the scheduler or the push endpoint.
struct CheckRow<'a> {
    time: i64,
    monitor_id: &'a str,
    status: i64,
    latency_ms: Option<i64>,
    status_code: Option<i64>,
    error: Option<&'a str>,
    source: CheckSource,
}

/// The single `checks` insert. `UNIQUE(monitor_id, time)` makes same-second
/// writes collide, and who wins depends on the source: an explicit push
/// overwrites whatever holds its second (a `status=up` then `status=down`
/// within one second keeps the failure; a push landing in the second of a
/// recorded miss replaces the miss, so the heartbeat that ends the streak is
/// not lost). Probes and misses never overwrite: a same-second duplicate probe
/// is a no-op, and a miss must not erase the push it raced with.
async fn insert_check_row(pool: &SqlitePool, row: CheckRow<'_>) -> sqlx::Result<()> {
    let sql = if row.source == CheckSource::Push {
        "INSERT INTO checks (time, monitor_id, status, latency_ms, status_code, error, source) \
         VALUES (?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(monitor_id, time) DO UPDATE SET \
            status = excluded.status, latency_ms = excluded.latency_ms, \
            status_code = excluded.status_code, error = excluded.error, \
            source = excluded.source"
    } else {
        "INSERT OR IGNORE INTO checks \
            (time, monitor_id, status, latency_ms, status_code, error, source) \
         VALUES (?, ?, ?, ?, ?, ?, ?)"
    };
    sqlx::query(sql)
        .bind(row.time)
        .bind(row.monitor_id)
        .bind(row.status)
        .bind(row.latency_ms)
        .bind(row.status_code)
        .bind(row.error)
        .bind(row.source.as_str())
        .execute(pool)
        .await?;
    Ok(())
}

/// Insert one probe result.
///
/// # Errors
///
/// Returns an error if the insert fails.
pub async fn insert_check(
    pool: &SqlitePool,
    monitor_id: &str,
    status: i64,
    outcome: &Outcome,
) -> sqlx::Result<()> {
    insert_check_row(
        pool,
        CheckRow {
            time: chrono::Utc::now().timestamp(),
            monitor_id,
            status,
            latency_ms: outcome.latency_ms,
            status_code: outcome.status_code,
            error: outcome.error.as_deref(),
            source: CheckSource::Probe,
        },
    )
    .await
}

/// Record an overdue heartbeat (push monitor or watched peer) as a down check,
/// marked as a miss so it is never mistaken for an explicit `status=down` push
/// (see [`last_heartbeat`]). A push already holding this second wins.
///
/// # Errors
///
/// Returns an error if the insert fails.
pub async fn insert_heartbeat_miss(
    pool: &SqlitePool,
    monitor_id: &str,
    reason: &str,
) -> sqlx::Result<()> {
    insert_heartbeat_miss_at(pool, monitor_id, reason, chrono::Utc::now().timestamp()).await
}

pub(super) async fn insert_heartbeat_miss_at(
    pool: &SqlitePool,
    monitor_id: &str,
    reason: &str,
    time: i64,
) -> sqlx::Result<()> {
    insert_check_row(
        pool,
        CheckRow {
            time,
            monitor_id,
            status: 0,
            latency_ms: None,
            status_code: None,
            error: Some(reason),
            source: CheckSource::Miss,
        },
    )
    .await
}

/// Record a heartbeat pushed via the API (status: 0 down, 1 up, 2 degraded).
/// The last push within a second wins (see [`insert_check_row`]).
///
/// # Errors
///
/// Returns an error if the insert fails.
pub async fn insert_push(
    pool: &SqlitePool,
    monitor_id: &str,
    status: i64,
    latency_ms: Option<i64>,
    message: Option<&str>,
) -> sqlx::Result<()> {
    insert_push_at(
        pool,
        monitor_id,
        status,
        latency_ms,
        message,
        chrono::Utc::now().timestamp(),
    )
    .await
}

pub(super) async fn insert_push_at(
    pool: &SqlitePool,
    monitor_id: &str,
    status: i64,
    latency_ms: Option<i64>,
    message: Option<&str>,
    time: i64,
) -> sqlx::Result<()> {
    insert_check_row(
        pool,
        CheckRow {
            time,
            monitor_id,
            status,
            latency_ms,
            status_code: None,
            error: message,
            source: CheckSource::Push,
        },
    )
    .await
}

/// The last `limit` checks for a monitor, newest first. One query serves both the
/// current status (from the statuses) and the latest sample (the first row).
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn recent_checks(
    pool: &SqlitePool,
    monitor_id: &str,
    limit: i64,
) -> sqlx::Result<Vec<Latest>> {
    sqlx::query_as::<_, Latest>(
        "SELECT time, latency_ms, status, error FROM checks \
         WHERE monitor_id = ? ORDER BY time DESC LIMIT ?",
    )
    .bind(monitor_id)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// A stored check reduced to what `hora tune` replays: when it ran, its status
/// (0 down, 1 up, 2 degraded) and the latency sample, if any.
#[derive(Debug, sqlx::FromRow)]
pub struct CheckSample {
    pub time: i64,
    pub status: i64,
    pub latency_ms: Option<i64>,
}

/// Every stored check for a monitor since `since`, oldest first - the raw
/// material [`crate::tune`] replays against alternative thresholds. Raw rows
/// survive the monitor's full retention window (default 90 days): the 7-day
/// downsampling only *copies* them into the hourly aggregate, so this reaches
/// as far back as retention even though [`latency_series`](super::latency_series) reads the buckets.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn check_samples(
    pool: &SqlitePool,
    monitor_id: &str,
    since: i64,
) -> sqlx::Result<Vec<CheckSample>> {
    sqlx::query_as::<_, CheckSample>(
        "SELECT time, status, latency_ms FROM checks \
         WHERE monitor_id = ? AND time >= ? ORDER BY time ASC",
    )
    .bind(monitor_id)
    .bind(since)
    .fetch_all(pool)
    .await
}

/// The timestamp of a monitor's most recent check, if any (push staleness).
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn last_check_time(pool: &SqlitePool, monitor_id: &str) -> sqlx::Result<Option<i64>> {
    sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(time) FROM checks WHERE monitor_id = ?")
        .bind(monitor_id)
        .fetch_one(pool)
        .await
}

/// The most recent heartbeat of a push monitor or watched peer: an explicit
/// push of any status (an `up`, a `degraded`, or a `down` carrying the job's
/// own message), never a recorded miss.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Heartbeat {
    pub time: i64,
    /// 0 down, 1 up, 2 degraded, as pushed.
    pub status: i64,
    pub latency_ms: Option<i64>,
    /// The pushed `msg`, if any.
    pub error: Option<String>,
}

/// A monitor's most recent heartbeat, ignoring recorded misses. Heartbeat
/// staleness must be measured from this, not from [`last_check_time`]: a
/// recorded miss has a fresh timestamp, so using the latter would reset the
/// staleness clock every tick and mask a continuing outage (the monitor would
/// flap down/up and never confirm down).
///
/// An explicit `status=down` push *is* a heartbeat - the job ran and said it
/// failed - so it keeps the clock fresh and its status and message drive the
/// alert. Rows written before the `source` column existed are read as they
/// always were: positive statuses are heartbeats, downs are misses.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn last_heartbeat(
    pool: &SqlitePool,
    monitor_id: &str,
) -> sqlx::Result<Option<Heartbeat>> {
    sqlx::query_as::<_, Heartbeat>(
        "SELECT time, status, latency_ms, error FROM checks \
         WHERE monitor_id = ? AND (source = 'push' OR (source = 'probe' AND status != 0)) \
         ORDER BY time DESC LIMIT 1",
    )
    .bind(monitor_id)
    .fetch_optional(pool)
    .await
}

/// The timestamp of a monitor's most recent heartbeat (see [`last_heartbeat`]).
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn last_heartbeat_time(pool: &SqlitePool, monitor_id: &str) -> sqlx::Result<Option<i64>> {
    Ok(last_heartbeat(pool, monitor_id)
        .await?
        .map(|beat| beat.time))
}

/// Current status from the recent checks (newest first): a single failure only
/// counts as `degraded` until `threshold` consecutive failures confirm `down`.
#[must_use]
pub fn derive_status(recent: &[Latest], threshold: i64) -> &'static str {
    let Some(latest) = recent.first() else {
        return "unknown";
    };
    match latest.status {
        1 => "up",
        2 => "degraded",
        _ => {
            let needed = usize::try_from(threshold).unwrap_or(usize::MAX);
            if recent.len() >= needed && recent.iter().all(|check| check.status == 0) {
                "down"
            } else {
                "degraded"
            }
        }
    }
}
