//! Release watches: the last upstream version seen and whether it was notified.

use super::Store;

/// A monitor's release watch as stored: the latest upstream release seen, when
/// it was looked up, and the release an alert was last sent for.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct StoredRelease {
    pub project: String,
    pub latest: String,
    pub url: String,
    pub checked_at: i64,
    pub notified: Option<String>,
}

/// Store (or refresh) the latest upstream release of a monitor's project. The
/// `notified` mark is kept while the project stays the same, and dropped with
/// it: another project's tags mean nothing to this one.
///
/// # Errors
///
/// Returns an error if the upsert fails.
pub async fn upsert_release_watch(
    store: &Store,
    monitor_id: &str,
    project: &str,
    latest: &str,
    url: &str,
    checked_at: i64,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO release_watch (monitor_id, project, latest, url, checked_at) \
         VALUES (?, ?, ?, ?, ?) \
         ON CONFLICT(monitor_id) DO UPDATE SET \
            notified = CASE WHEN project = excluded.project THEN notified END, \
            project = excluded.project, latest = excluded.latest, \
            url = excluded.url, checked_at = excluded.checked_at",
    )
    .bind(monitor_id)
    .bind(project)
    .bind(latest)
    .bind(url)
    .bind(checked_at)
    .execute(store.sqlx())
    .await?;
    Ok(())
}

/// The stored release watch of a monitor, if any.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn release_watch(store: &Store, monitor_id: &str) -> sqlx::Result<Option<StoredRelease>> {
    sqlx::query_as::<_, StoredRelease>(
        "SELECT project, latest, url, checked_at, notified FROM release_watch WHERE monitor_id = ?",
    )
    .bind(monitor_id)
    .fetch_optional(store.sqlx())
    .await
}

/// Record that an alert went out for `tag`, so the release alerts once - and
/// not again after a restart.
///
/// # Errors
///
/// Returns an error if the update fails.
pub async fn mark_release_notified(store: &Store, monitor_id: &str, tag: &str) -> sqlx::Result<()> {
    sqlx::query("UPDATE release_watch SET notified = ? WHERE monitor_id = ?")
        .bind(tag)
        .bind(monitor_id)
        .execute(store.sqlx())
        .await?;
    Ok(())
}
