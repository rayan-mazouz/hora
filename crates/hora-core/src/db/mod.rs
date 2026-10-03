//! `SQLite` persistence layer (sqlx). All timestamps are unix epoch seconds (UTC).
//!
//! Connection and backup live here; the queries are grouped by table family in
//! the submodules and re-exported, so callers keep using `db::insert_check`,
//! `db::recent_incidents`, ...

#[macro_use]
mod sql;
mod aggregates;
mod annotations;
mod certs;
mod checks;
mod incidents;
mod release;
mod retention;
#[cfg(test)]
mod tests;
mod window;

pub use aggregates::{
    DayRow, Point, availability, availability_all, daily_all, latency_hourly,
    latency_percentiles_all, latency_series, latency_sparkline_all,
};
pub use annotations::{
    Announcement, EventMarker, PushedAlert, Silence, active_announcements, active_silences,
    announcements_since, clear_announcements, clear_silences, events_since, insert_announcement,
    insert_event, insert_pushed_alert, insert_silence, insert_silences, is_silenced,
    latest_event_before, meta_get, meta_set, recent_events, recent_pushed_alerts, silences_since,
};
pub use certs::{
    cert_all, cert_pin_fingerprint, domain_expiry, upsert_cert, upsert_cert_pin,
    upsert_domain_expiry,
};
pub use checks::{
    CheckSample, Heartbeat, Latest, check_samples, derive_status, insert_check,
    insert_heartbeat_miss, insert_push, last_check_time, last_heartbeat, last_heartbeat_time,
    recent_checks,
};
pub use incidents::{
    Incident, find_open_incident, incident_by_id, incidents_between, insert_incident_start,
    latest_incident_id, recent_incidents, set_incident_note, update_incident_end,
    update_incident_vantage,
};
pub use release::{StoredRelease, mark_release_notified, release_watch, upsert_release_watch};
pub(crate) use retention::prune;
pub use retention::{
    AGGREGATE_RETENTION_DAYS, downsample_daily, downsample_hourly, fill_latency_histograms,
    prune_daily, prune_hourly, roll_up_recent,
};
pub use window::{DailyCache, SparklineCache, WindowCache, WindowStats, window_stats_all};

use std::time::Duration;

// Re-exported so the CLI can hold a pool without depending on sqlx directly.
pub use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

/// `meta` key prefix recording since when a push monitor or watched peer has
/// been expecting its first heartbeat (`heartbeat_expected_since:<id>`).
pub(crate) const HEARTBEAT_EXPECTED_META_PREFIX: &str = "heartbeat_expected_since:";
/// `meta` key prefix recording the `cert_pin` the stored `cert_pins` key was
/// last checked against (`cert_pin_against:<id>` = the configured pin), so a
/// reported mismatch is not taken as reported against a *different* pin.
pub(crate) const CERT_PIN_AGAINST_META_PREFIX: &str = "cert_pin_against:";

/// The embedded database migrations.
#[must_use]
pub fn migrator() -> sqlx::migrate::Migrator {
    sqlx::migrate!("./migrations")
}

/// Open the pool (creating the file if needed, WAL mode) and run migrations.
///
/// # Errors
///
/// Returns an error if the database cannot be opened or migrations fail.
pub async fn connect(database_path: &str) -> anyhow::Result<SqlitePool> {
    // The database holds failure snippets, push payloads and incident detail, so
    // create it private (0600) rather than at the process umask. SQLite then
    // mirrors that mode onto the -wal/-shm sidecars it spawns.
    let path = database_path.to_owned();
    tokio::task::spawn_blocking(move || precreate_private(&path)).await??;
    let options = SqliteConnectOptions::new()
        .filename(database_path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        // NORMAL is the recommended durability level under WAL: writes fsync only
        // at checkpoint, not on every probe insert. Safe for this time series -
        // at worst a power loss drops the last few checks.
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5))
        // Keep the hot index pages resident across the 5s summary rebuilds
        // (negative = KiB, so this is 16 MiB rather than the 2 MiB default).
        .pragma("cache_size", "-16000");

    // WAL lets readers run while one writer (the scheduler) inserts, so a roomy
    // pool keeps the parallel summary queries from queueing.
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        // Fail fast under contention instead of the 30s default; a slow monitor
        // query then degrades just that card, not the whole page.
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options)
        .await?;

    migrator().run(&pool).await?;
    // Give the planner statistics before the first page builds. Without
    // `sqlite_stat1` it favors a full scan of the monitor-led index for the
    // 24h aggregates (seconds on a seasoned database); with stats it skip-scans
    // (`ANY(monitor_id) AND time>?`), milliseconds. `PRAGMA optimize` only
    // re-runs ANALYZE when the shape of the data has drifted, so on most boots
    // this is a no-op; the first one pays a sub-second full ANALYZE.
    sqlx::query("PRAGMA optimize").execute(&pool).await?;
    Ok(pool)
}

/// Create the database file with owner-only (0600) permissions before sqlx opens
/// it, so secrets in the time series aren't world-readable. A no-op for an
/// in-memory database, an existing file, or a non-Unix platform (where file
/// modes don't apply).
#[cfg(unix)]
fn precreate_private(database_path: &str) -> anyhow::Result<()> {
    use anyhow::Context as _;

    // `file:` URIs are skipped wholesale: parsing them (query params, mode=memory)
    // isn't worth it for a spelling Hora never documents. An operator who points a
    // `file:` URI at a real on-disk database owns its permissions.
    if database_path == ":memory:" || database_path.starts_with("file:") {
        return Ok(());
    }
    match create_private(database_path) {
        Ok(()) => Ok(()),
        // Already there (or another opener won the race): keep its mode.
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(err) => Err(err).with_context(|| format!("creating database file {database_path}")),
    }
}

#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
fn precreate_private(_database_path: &str) -> anyhow::Result<()> {
    Ok(())
}

/// Create `path` as a new, empty file - owner-only (0600) on Unix. Fails with
/// `AlreadyExists` when anything is already there, atomically (no
/// check-then-create window).
fn create_private(path: &str) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path).map(drop)
}

/// Copy the database into `dest` with `VACUUM INTO`: a consistent, compacted
/// snapshot taken through `SQLite` itself, safe while the daemon is writing
/// (a WAL reader does not block the writer). The source is opened read-only,
/// so a backup never creates or migrates a database.
///
/// # Errors
///
/// Returns an error if `dest` already exists, the source cannot be opened, or
/// the copy fails.
pub async fn backup_into(database_path: &str, dest: &str) -> anyhow::Result<()> {
    use anyhow::Context as _;

    // Claim the destination first, empty and owner-only (0600) like the live
    // database: VACUUM INTO accepts an empty file and keeps its mode, so the
    // snapshot is never readable at the process umask, not even briefly.
    // `create_new` also makes the "never overwrite a backup" check atomic.
    let target = dest.to_owned();
    match tokio::task::spawn_blocking(move || create_private(&target)).await? {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            anyhow::bail!("destination {dest} already exists; refusing to overwrite a backup")
        }
        Err(err) => return Err(err).with_context(|| format!("creating {dest}")),
    }

    let copied = vacuum_into(database_path, dest).await;
    if copied.is_err() {
        // Don't leave the empty placeholder behind: it would block a retry.
        let target = dest.to_owned();
        let _ = tokio::task::spawn_blocking(move || std::fs::remove_file(target)).await;
    }
    copied
}

async fn vacuum_into(database_path: &str, dest: &str) -> anyhow::Result<()> {
    use anyhow::Context as _;

    let options = SqliteConnectOptions::new()
        .filename(database_path)
        .read_only(true)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .with_context(|| format!("opening {database_path} read-only"))?;
    let copied = sqlx::query("VACUUM INTO ?")
        .bind(dest)
        .execute(&pool)
        .await
        .with_context(|| format!("copying into {dest}"));
    pool.close().await;
    copied.map(|_| ())
}
