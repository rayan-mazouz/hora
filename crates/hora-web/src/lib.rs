//! Web layer: the server-rendered status page and the JSON API.

mod auth;
mod cache;
mod error;
mod flood;
mod handlers;
mod heatmap;
mod history;
mod metrics;
mod render;
mod report;
mod routes;
mod summary;
mod text;
mod visibility;

use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use axum::http::{HeaderName, Request};
use hora_core::config::Config;
use hora_core::notifications::{self, Notifiers};
use sqlx::SqlitePool;
use tokio::sync::watch;
use tower_governor::errors::GovernorError;
use tower_governor::key_extractor::{KeyExtractor, PeerIpKeyExtractor};

use crate::cache::{Cache, Cached, Memo, ProbeClients};
use crate::summary::{Summary, build_summary};
use crate::visibility::{Audience, Visibility};

pub use routes::router;

pub(crate) const SECONDS_PER_HOUR: i64 = 3_600;
pub(crate) const MAX_LATENCY_HOURS: i64 = 24 * 30;
pub(crate) const SUMMARY_CACHE_TTL: Duration = Duration::from_secs(5);
/// How long a rendered heatmap or a built monthly report is reused: both scan
/// weeks of history and change slowly, and both are reachable anonymously.
pub(crate) const VIEW_CACHE_TTL: Duration = Duration::from_mins(1);
// The free-text caps shared with the CLI (push message, alert/event title,
// announcement, silence reason) live in hora-core (`hora_core::MAX_*_CHARS`).
/// Cap on a pushed alert's `dedup_key`.
pub(crate) const MAX_ALERT_DEDUP_CHARS: usize = 200;
/// Cap on how many `tags` a pushed alert may fold into its message, and on the
/// length of each key/value - a buggy producer can't bloat the row this way.
pub(crate) const MAX_ALERT_TAGS: usize = 20;
pub(crate) const MAX_ALERT_TAG_CHARS: usize = 100;
/// Cap on points returned by the latency endpoint (evenly downsampled beyond it).
pub(crate) const MAX_LATENCY_POINTS: usize = 2000;
/// Number of time buckets a 24h card sparkline is averaged into, so its size is
/// fixed no matter the check frequency (the chart is `CHART_W`px wide).
pub(crate) const SPARK_BUCKETS: i64 = 120;

pub(crate) const FAVICON_SVG: &str = include_str!("../assets/favicon.svg");
pub(crate) const FONT_WOFF2: &[u8] = include_bytes!("../assets/CalSans-SemiBold.woff2");

// The page is fully self-contained (inline styles, same-origin font/icon, no
// JS). No `data:` images: nothing embeds one.
pub(crate) const CSP: &str = "default-src 'self'; script-src 'none'; style-src 'self' 'unsafe-inline'; \
     img-src 'self'; font-src 'self'; base-uri 'none'; frame-ancestors 'none'";
/// Powerful browser features the pages never use, denied outright.
pub(crate) const PERMISSIONS_POLICY: &str = "accelerometer=(), camera=(), geolocation=(), \
     gyroscope=(), magnetometer=(), microphone=(), payment=(), usb=()";

/// Shared application state handed to every handler.
#[derive(Clone)]
pub struct AppState {
    pool: SqlitePool,
    config: watch::Receiver<Arc<Config>>,
    cache: Arc<Cache>,
    /// Rendered heatmaps by monitor id (the image does not depend on the
    /// audience; visibility is checked before the lookup).
    heatmaps: Arc<Memo<String, String>>,
    /// Built monthly reports by month (audience-independent: rows are
    /// filtered per request).
    reports: Arc<Memo<String, hora_core::report::MonthReport>>,
    /// Peer-probe clients by proxy, reused across `/api/peer/probe` calls.
    probe_clients: Arc<ProbeClients>,
    /// The scheduler's liveness beacon, written by the monitor loops and read by
    /// `/healthz` to report whether the scheduler is still ticking.
    last_tick: Arc<AtomicU64>,
    /// The hot-swappable notification channels, so the alert endpoint can fan a
    /// pushed alert out to a monitor's channels (shared with the daemon's
    /// supervisor, which rebuilds it on config reload).
    notifier: Notifiers,
    /// Per-process anti-flood state for pushed alerts carrying a `dedup_key`.
    flood: Arc<flood::Flood>,
    /// The peers' view of shared targets (per-vantage latency), written by the
    /// daemon's poller and read by the summary builder. Empty when the node
    /// has no askable peers.
    vantage: hora_core::vantage::VantageMap,
}

impl AppState {
    #[must_use]
    pub fn new(
        pool: SqlitePool,
        config: watch::Receiver<Arc<Config>>,
        last_tick: Arc<AtomicU64>,
        notifier: Notifiers,
    ) -> Self {
        Self {
            pool,
            config,
            cache: Arc::new(Cache::default()),
            heatmaps: Arc::new(Memo::new(VIEW_CACHE_TTL)),
            reports: Arc::new(Memo::new(VIEW_CACHE_TTL)),
            probe_clients: Arc::new(ProbeClients::default()),
            last_tick,
            notifier,
            flood: Arc::new(flood::Flood::default()),
            vantage: hora_core::vantage::new_map(),
        }
    }

    /// Attach the daemon's vantage snapshot (the poller's output). Without it
    /// the cards simply render no per-vantage line.
    #[must_use]
    pub fn with_vantage(mut self, vantage: hora_core::vantage::VantageMap) -> Self {
        self.vantage = vantage;
        self
    }
}

/// Rate-limit key extractor that trusts a configured header (e.g.
/// `cf-connecting-ip` behind Cloudflare) for the client IP, falling back to the
/// real TCP peer address when it is absent or unparseable.
///
/// The fallback is deliberately the peer socket - *not* `x-forwarded-for` or
/// other forwarded headers - because a direct client can set those freely: a
/// smart extractor would let an attacker mint a fresh rate-limit bucket per
/// request (and inflate the keyed-bucket map) simply by rotating the header.
/// Forwarded headers are honored only when an operator behind a trusted proxy
/// names one via `server.client_ip_header`.
#[derive(Clone)]
pub(crate) struct ConfiguredIp {
    header: Option<HeaderName>,
}

impl KeyExtractor for ConfiguredIp {
    type Key = IpAddr;

    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, GovernorError> {
        if let Some(header) = &self.header
            && let Some(ip) = req
                .headers()
                .get(header)
                .and_then(|value| value.to_str().ok())
                .and_then(|raw| raw.split(',').next())
                .and_then(|first| first.trim().parse::<IpAddr>().ok())
        {
            return Ok(ip);
        }
        PeerIpKeyExtractor.extract(req)
    }
}

impl ConfiguredIp {
    /// Parse the configured header name once; an invalid name is ignored (with a
    /// warning) and the extractor falls back to the peer address.
    pub(crate) fn from_config(name: Option<&str>) -> Self {
        let header = name.and_then(|name| {
            name.parse::<HeaderName>()
                .inspect_err(|_| {
                    tracing::warn!("invalid server.client_ip_header {name:?}, ignoring");
                })
                .ok()
        });
        Self { header }
    }
}

// --- Summary cache (lock-free read + single-flight build) ----------------

/// Return a fresh-enough cached summary for `audience`, or build exactly one
/// (single-flight) and cache it. The cache busts immediately when the config
/// is reloaded.
pub(crate) async fn summary_for(
    pool: &SqlitePool,
    config: &Arc<Config>,
    cache: &Cache,
    audience: &Audience,
    notifier: &Notifiers,
    vantage: &hora_core::vantage::VantageMap,
) -> Arc<Summary> {
    let cached = || match audience {
        Audience::Public => cache::fresh(&cache.public, config, SUMMARY_CACHE_TTL),
        Audience::Operator => cache::fresh(&cache.operator, config, SUMMARY_CACHE_TTL),
        Audience::Group(group) => cache.groups.get(group, config),
    };
    if let Some(fresh) = cached() {
        return fresh;
    }
    // Only one task builds at a time; the rest wait and reuse the result.
    let _build = cache.build.lock().await;
    if let Some(fresh) = cached() {
        return fresh;
    }
    // Channel health is read live from the dispatcher - only for the
    // operator's view, and only on a cache miss (every 5s at most).
    let health = if *audience == Audience::Operator {
        notifications::health_snapshot(notifier)
    } else {
        Vec::new()
    };
    // The poller's last snapshot: a lock-free read, never the network.
    let vantage = vantage.load_full();
    let visibility = Visibility::new(config, audience);
    let summary = Arc::new(build_summary(pool, config, &visibility, &health, &vantage).await);
    match audience {
        Audience::Public => cache
            .public
            .store(Some(Arc::new(Cached::new(config, Arc::clone(&summary))))),
        Audience::Operator => cache
            .operator
            .store(Some(Arc::new(Cached::new(config, Arc::clone(&summary))))),
        Audience::Group(group) => cache.groups.insert(group, config, Arc::clone(&summary)),
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configured_ip_prefers_header_then_falls_back_to_peer() {
        use axum::extract::ConnectInfo;
        use std::net::SocketAddr;

        let extractor = ConfiguredIp::from_config(Some("cf-connecting-ip"));
        let peer = SocketAddr::from(([198, 51, 100, 9], 4444));

        // Header present: it wins over the peer address.
        let with_header = Request::builder()
            .header("cf-connecting-ip", "203.0.113.7")
            .extension(ConnectInfo(peer))
            .body(())
            .expect("request");
        assert_eq!(
            extractor.extract(&with_header).unwrap(),
            "203.0.113.7".parse::<IpAddr>().unwrap()
        );

        // Header absent: fall back to the real peer address, NOT a spoofable
        // x-forwarded-for the client supplied.
        let without_header = Request::builder()
            .header("x-forwarded-for", "10.0.0.1")
            .extension(ConnectInfo(peer))
            .body(())
            .expect("request");
        assert_eq!(extractor.extract(&without_header).unwrap(), peer.ip());
    }
}
