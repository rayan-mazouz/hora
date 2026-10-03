//! Incidents: one row per outage, opened and closed by the scheduler.

use sqlx::SqlitePool;

/// An automatically recorded incident from a down/up transition.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Incident {
    pub id: i64,
    pub monitor_id: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub duration_s: Option<i64>,
    pub cause: Option<String>,
    pub impacted: Option<String>,
    pub error: Option<String>,
    /// Operator-written annotation ("fiber cut"), set via `hora annotate`.
    pub note: Option<String>,
    /// What the service actually answered: the failing response's status line,
    /// headers and body start, captured (bounded) when the down was confirmed.
    pub snapshot: Option<String>,
    /// The correlated event phrase ("deploy api v2.3, 3m before"), resolved
    /// when the down was confirmed - the "what changed?" answer.
    pub event: Option<String>,
    /// The multi-vantage verdict recorded once the peers answered; `None` when
    /// no peers were asked.
    pub vantage: Option<String>,
    pub created_at: i64,
}

/// Record the start of an incident (monitor going down).
///
/// # Errors
///
/// Returns an error if the insert fails.
pub async fn insert_incident_start(
    pool: &SqlitePool,
    monitor_id: &str,
    error: Option<&str>,
    cause: Option<&str>,
    impacted: &[String],
    snapshot: Option<&str>,
    event: Option<&str>,
) -> sqlx::Result<i64> {
    let now = chrono::Utc::now().timestamp();
    let impacted_json = serde_json::to_string(impacted).unwrap_or_else(|_| "[]".to_owned());
    let result = sqlx::query(
        "INSERT INTO incidents \
            (monitor_id, started_at, error, cause, impacted, snapshot, event, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(monitor_id)
    .bind(now)
    .bind(error)
    .bind(cause)
    .bind(&impacted_json)
    .bind(snapshot)
    .bind(event)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
}

/// Record the multi-vantage verdict on an incident, once the peers answered
/// (the incident row is written *before* the peers are consulted, so the
/// history never waits on the network).
///
/// # Errors
///
/// Returns an error if the update fails.
pub async fn update_incident_vantage(
    pool: &SqlitePool,
    incident_id: i64,
    vantage: &str,
) -> sqlx::Result<()> {
    sqlx::query("UPDATE incidents SET vantage = ? WHERE id = ?")
        .bind(vantage)
        .bind(incident_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Record the end of an incident (monitor recovering).
///
/// # Errors
///
/// Returns an error if the update fails.
pub async fn update_incident_end(pool: &SqlitePool, incident_id: i64) -> sqlx::Result<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query("UPDATE incidents SET ended_at = ?, duration_s = (? - started_at) WHERE id = ?")
        .bind(now)
        .bind(now)
        .bind(incident_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Find the most recent open incident for a monitor (one without `ended_at`).
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn find_open_incident(pool: &SqlitePool, monitor_id: &str) -> sqlx::Result<Option<i64>> {
    sqlx::query_scalar::<_, i64>(
        "SELECT id FROM incidents WHERE monitor_id = ? AND ended_at IS NULL \
         ORDER BY started_at DESC LIMIT 1",
    )
    .bind(monitor_id)
    .fetch_optional(pool)
    .await
}

/// Fetch recent incidents for the history page/Atom feed.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn recent_incidents(pool: &SqlitePool, limit: i64) -> sqlx::Result<Vec<Incident>> {
    // The id tie-break keeps the order deterministic when incidents share a
    // start second (a cascade), and matches what [`latest_incident_id`] calls
    // "last" - so `hora annotate last` annotates the incident listed first.
    sqlx::query_as::<_, Incident>(concat!(
        "SELECT ",
        incident_columns!(),
        " FROM incidents ORDER BY started_at DESC, id DESC LIMIT ?"
    ))
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// One monitor's incidents started since `since`, newest first, at most
/// `limit` (its own page: an indexed read of its rows only).
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn monitor_incidents(
    pool: &SqlitePool,
    monitor_id: &str,
    since: i64,
    limit: i64,
) -> sqlx::Result<Vec<Incident>> {
    sqlx::query_as::<_, Incident>(concat!(
        "SELECT ",
        incident_columns!(),
        " FROM incidents WHERE monitor_id = ? AND started_at >= ? \
         ORDER BY started_at DESC, id DESC LIMIT ?"
    ))
    .bind(monitor_id)
    .bind(since)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Where a monitor stands in its incident log: when its last finished
/// incident ended, and since when it is down if an incident is open.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IncidentMarks {
    pub last_end: Option<i64>,
    pub open_since: Option<i64>,
}

/// Every monitor's [`IncidentMarks`], in one grouped pass over the incident
/// log (one row per outage: small next to the checks), for the status page's
/// "down since" and "no incident in N days".
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn incident_marks(
    pool: &SqlitePool,
) -> sqlx::Result<std::collections::HashMap<String, IncidentMarks>> {
    let rows = sqlx::query_as::<_, (String, Option<i64>, Option<i64>)>(
        "SELECT monitor_id, MAX(ended_at), \
            MIN(CASE WHEN ended_at IS NULL THEN started_at END) \
         FROM incidents GROUP BY monitor_id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, last_end, open_since)| {
            (
                id,
                IncidentMarks {
                    last_end,
                    open_since,
                },
            )
        })
        .collect())
}

/// Every incident overlapping `[since, until)`: started before `until` and
/// still open or ended after `since`. Newest first, unbounded - the monthly
/// report and the digest must count them all, not the latest N.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn incidents_between(
    pool: &SqlitePool,
    since: i64,
    until: i64,
) -> sqlx::Result<Vec<Incident>> {
    sqlx::query_as::<_, Incident>(concat!(
        "SELECT ",
        incident_columns!(),
        " FROM incidents WHERE started_at < ? AND (ended_at IS NULL OR ended_at > ?) \
         ORDER BY started_at DESC, id DESC"
    ))
    .bind(until)
    .bind(since)
    .fetch_all(pool)
    .await
}

/// Fetch one incident by id, for the post-mortem page and CLI.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn incident_by_id(pool: &SqlitePool, id: i64) -> sqlx::Result<Option<Incident>> {
    sqlx::query_as::<_, Incident>(concat!(
        "SELECT ",
        incident_columns!(),
        " FROM incidents WHERE id = ?"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// Set (or, with an empty string, clear) the operator note on an incident.
/// Returns whether the incident exists.
///
/// # Errors
///
/// Returns an error if the update fails.
pub async fn set_incident_note(pool: &SqlitePool, id: i64, note: &str) -> sqlx::Result<bool> {
    let note = (!note.is_empty()).then_some(note);
    let result = sqlx::query("UPDATE incidents SET note = ? WHERE id = ?")
        .bind(note)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// The id of the most recently started incident, if any (`hora annotate last`).
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn latest_incident_id(pool: &SqlitePool) -> sqlx::Result<Option<i64>> {
    sqlx::query_scalar::<_, i64>(
        "SELECT id FROM incidents ORDER BY started_at DESC, id DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
}
