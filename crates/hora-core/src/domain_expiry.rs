//! Domain-expiry monitoring: the daily RDAP lookups (see [`crate::rdap`]) for
//! monitors with a `domain_expiry`, and the alert when one nears expiration.
//! Run by the certificate watcher's tick ([`crate::cert::spawn_watcher`]).

use std::collections::HashMap;

use hora_notify::Event;
use sqlx::SqlitePool;
use tracing::{info, warn};

use crate::SECONDS_PER_DAY;
use crate::config::Config;
use crate::db;
use crate::notifications::Notifiers;

/// RDAP lookups happen at most this often per monitor (just under a day, so
/// the 12h ticks land on a daily cadence): registries rate-limit, the answer
/// moves yearly, and the gate is the stored `checked_at`, so a restart never
/// re-queries early.
const DOMAIN_CHECK_SECS: i64 = 20 * 3600;

/// One pass of the RDAP domain-expiry checks: for each monitor with a
/// `domain_expiry`, refresh the registry's expiration date at most daily
/// (gated on the stored `checked_at`, so restarts never re-query early) and
/// alert once when it enters the warning window - the same edge-triggered,
/// maintenance-muted policy as the certificate expiry in [`crate::cert`].
pub(crate) async fn check_domains(
    pool: &SqlitePool,
    snapshot: &Config,
    notifier: &Notifiers,
    client: &reqwest::Client,
    domain_warned: &mut HashMap<String, bool>,
    now: i64,
) {
    let threshold_days = i64::from(snapshot.alerts.domain_expiry_days);
    for monitor in &snapshot.monitors {
        let Some(domain) = &monitor.domain_expiry else {
            continue;
        };
        let stored = match db::domain_expiry(pool, &monitor.id).await {
            Ok(stored) => stored,
            Err(err) => {
                warn!(monitor = %monitor.id, "failed to read domain expiry: {err:#}");
                continue;
            }
        };
        // A changed `domain_expiry` in the config re-queries immediately.
        let expires_at = match stored {
            Some((ref stored_domain, expires_at, checked_at))
                if stored_domain == domain && now - checked_at < DOMAIN_CHECK_SECS =>
            {
                expires_at
            }
            _ => match crate::rdap::domain_expiration(client, domain).await {
                Ok(expires_at) => {
                    if let Err(err) =
                        db::upsert_domain_expiry(pool, &monitor.id, domain, expires_at, now).await
                    {
                        warn!(monitor = %monitor.id, "failed to store domain expiry: {err:#}");
                    }
                    let days_left = (expires_at - now) / SECONDS_PER_DAY;
                    info!(monitor = %monitor.id, domain, days_left, "checked domain expiry (RDAP)");
                    expires_at
                }
                Err(err) => {
                    warn!(monitor = %monitor.id, domain, "RDAP domain check failed: {err:#}");
                    continue;
                }
            },
        };

        let days_left = (expires_at - now) / SECONDS_PER_DAY;
        let expiring = days_left <= threshold_days;
        let already_warned = domain_warned.get(&monitor.id).copied().unwrap_or(false);
        // Mute (without recording the warned state) during maintenance, so
        // the alert still fires once the window ends.
        if snapshot.in_maintenance(&monitor.id, chrono::Utc::now()) {
            continue;
        }
        if expiring && !already_warned {
            notifier
                .load_full()
                .dispatch(
                    Event::DomainExpiring {
                        monitor: &monitor.name,
                        domain,
                        days_left,
                    },
                    monitor.notify.as_deref(),
                )
                .await;
        }
        domain_warned.insert(monitor.id.clone(), expiring);
    }
}
