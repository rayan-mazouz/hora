//! Background maintenance: the periodic task that keeps the database within its
//! retention (see [`crate::db`] for the pruning itself).

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use crate::config::Config;
use crate::db::{SqlitePool, prune};

const PRUNE_INTERVAL: Duration = Duration::from_hours(6);
/// How long after startup the first prune tick runs. Boot is already the
/// busiest write window (every monitor's first probe, the cert sweep, possibly
/// a schema migration); maintenance can wait until the burst has settled.
const PRUNE_STARTUP_DELAY: Duration = Duration::from_mins(5);

/// Background task: periodically prune each monitor's history to its retention,
/// and drop any data left behind by monitors removed from the config (after a
/// grace period, see `db::retention::delete_orphans`). A shutdown
/// signal lets it stop between ticks instead of being aborted.
#[must_use]
pub fn spawn_pruner(
    pool: &SqlitePool,
    config: watch::Receiver<Arc<Config>>,
    mut shutdown: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    let pool = pool.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval_at(
            tokio::time::Instant::now() + PRUNE_STARTUP_DELAY,
            PRUNE_INTERVAL,
        );
        loop {
            tokio::select! {
                _ = ticker.tick() => {}
                _ = shutdown.changed() => break,
            }
            let config = config.borrow().clone();
            if let Err(err) = prune(&pool, &config).await {
                tracing::warn!("pruning failed: {err}");
            }
        }
    })
}
