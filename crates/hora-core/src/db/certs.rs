//! TLS certificate expiry, certificate pins and domain (RDAP) expiry.

use std::collections::HashMap;

use sqlx::SqlitePool;

/// Store (or refresh) the latest known certificate expiry for a monitor.
///
/// # Errors
///
/// Returns an error if the upsert fails.
pub async fn upsert_cert(
    pool: &SqlitePool,
    monitor_id: &str,
    not_after: i64,
    checked_at: i64,
) -> sqlx::Result<()> {
    sqlx::query(upsert_sql!(
        "certs",
        "monitor_id",
        "not_after",
        "checked_at"
    ))
    .bind(monitor_id)
    .bind(not_after)
    .bind(checked_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// The stored certificate `not_after` timestamp for a monitor, if known. Only
/// the tests read one monitor's expiry; the page batches them ([`cert_all`]).
#[cfg(test)]
pub(super) async fn cert_not_after(
    pool: &SqlitePool,
    monitor_id: &str,
) -> sqlx::Result<Option<i64>> {
    sqlx::query_scalar::<_, i64>("SELECT not_after FROM certs WHERE monitor_id = ?")
        .bind(monitor_id)
        .fetch_optional(pool)
        .await
}

/// The stored certificate `not_after` for every monitor that has one.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn cert_all(pool: &SqlitePool) -> sqlx::Result<HashMap<String, i64>> {
    let rows = sqlx::query_as::<_, (String, i64)>("SELECT monitor_id, not_after FROM certs")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().collect())
}

/// Store (or refresh) a monitor's registered-domain expiration (RDAP).
///
/// # Errors
///
/// Returns an error if the upsert fails.
pub async fn upsert_domain_expiry(
    pool: &SqlitePool,
    monitor_id: &str,
    domain: &str,
    expires_at: i64,
    checked_at: i64,
) -> sqlx::Result<()> {
    sqlx::query(upsert_sql!(
        "domain_expiry",
        "monitor_id",
        "domain",
        "expires_at",
        "checked_at"
    ))
    .bind(monitor_id)
    .bind(domain)
    .bind(expires_at)
    .bind(checked_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// The stored `(domain, expires_at, checked_at)` for a monitor, if any. The
/// watcher uses `checked_at` to poll RDAP at most once a day per monitor, and
/// `domain` to re-query immediately when the configured domain changed.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn domain_expiry(
    pool: &SqlitePool,
    monitor_id: &str,
) -> sqlx::Result<Option<(String, i64, i64)>> {
    sqlx::query_as::<_, (String, i64, i64)>(
        "SELECT domain, expires_at, checked_at FROM domain_expiry WHERE monitor_id = ?",
    )
    .bind(monitor_id)
    .fetch_optional(pool)
    .await
}

/// Store (or refresh) the certificate pin (SHA-256 fingerprint of the leaf public key).
///
/// # Errors
///
/// Returns an error if the upsert fails.
pub async fn upsert_cert_pin(
    pool: &SqlitePool,
    monitor_id: &str,
    fingerprint: &str,
    checked_at: i64,
) -> sqlx::Result<()> {
    sqlx::query(upsert_sql!(
        "cert_pins",
        "monitor_id",
        "fingerprint",
        "checked_at"
    ))
    .bind(monitor_id)
    .bind(fingerprint)
    .bind(checked_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// The stored certificate pin fingerprint for a monitor, if known.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn cert_pin_fingerprint(
    pool: &SqlitePool,
    monitor_id: &str,
) -> sqlx::Result<Option<String>> {
    sqlx::query_scalar::<_, String>("SELECT fingerprint FROM cert_pins WHERE monitor_id = ?")
        .bind(monitor_id)
        .fetch_optional(pool)
        .await
}
