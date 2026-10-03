//! Web layer: the server-rendered status page and the JSON API.

mod auth;
mod cache;
mod error;
mod flood;
mod handlers;
mod heatmap;
mod history;
mod layout;
mod metrics;
mod monitor_page;
mod render;
mod report;
mod routes;
mod snapshot;
mod status_page;
mod summary;
mod text;
mod visibility;
mod watchers;

use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use axum::http::{HeaderName, Request};
use hora_core::config::Config;
use hora_core::db::Store;
use hora_core::notifications::Notifiers;
use tokio::sync::watch;
use tower_governor::errors::GovernorError;
use tower_governor::key_extractor::{KeyExtractor, PeerIpKeyExtractor};

use crate::cache::{Memo, ProbeClients};
use crate::snapshot::{Refresher, Snapshot};

pub use routes::router;

pub(crate) const SECONDS_PER_HOUR: i64 = 3_600;
pub(crate) const MAX_LATENCY_HOURS: i64 = 24 * 30;
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

// The pages are self-contained: one same-origin stylesheet (no inline style,
// not even an attribute), same-origin fonts and icons, no JS. No `data:`
// images: nothing embeds one.
pub(crate) const CSP: &str = "default-src 'self'; script-src 'none'; style-src 'self'; \
     img-src 'self'; font-src 'self'; base-uri 'none'; frame-ancestors 'none'";
/// Powerful browser features the pages never use, denied outright.
pub(crate) const PERMISSIONS_POLICY: &str = "accelerometer=(), camera=(), geolocation=(), \
     gyroscope=(), magnetometer=(), microphone=(), payment=(), usb=()";

/// Shared application state handed to every handler.
#[derive(Clone)]
pub struct AppState {
    store: Store,
    config: watch::Receiver<Arc<Config>>,
    /// The status summary, rebuilt in the background (see [`snapshot`]).
    refresher: Arc<Refresher>,
    /// Rendered heatmaps by monitor id (the image does not depend on the
    /// audience; visibility is checked before the lookup).
    heatmaps: Arc<Memo<String, String>>,
    /// Built monthly reports by month (audience-independent: rows are
    /// filtered per request).
    reports: Arc<Memo<String, hora_core::report::MonthReport>>,
    /// Rendered report pages, per month, audience and `?group=`: each with
    /// the built report it was rendered from, so it lives exactly as long
    /// as that report's memo entry (see `handlers::pages::report_page`).
    report_pages: Arc<std::sync::Mutex<handlers::pages::ReportPages>>,
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
    vantage: hora_core::mesh::vantage::VantageMap,
}

impl AppState {
    #[must_use]
    pub fn new(
        store: Store,
        config: watch::Receiver<Arc<Config>>,
        last_tick: Arc<AtomicU64>,
        notifier: Notifiers,
    ) -> Self {
        Self {
            store,
            config,
            refresher: Arc::new(Refresher::default()),
            heatmaps: Arc::new(Memo::new(VIEW_CACHE_TTL)),
            reports: Arc::new(Memo::new(VIEW_CACHE_TTL)),
            report_pages: Arc::default(),
            probe_clients: Arc::new(ProbeClients::default()),
            last_tick,
            notifier,
            flood: Arc::new(flood::Flood::default()),
            vantage: hora_core::mesh::vantage::new_map(),
        }
    }

    /// Attach the daemon's vantage snapshot (the poller's output). Without it
    /// the cards simply render no per-vantage line.
    #[must_use]
    pub fn with_vantage(mut self, vantage: hora_core::mesh::vantage::VantageMap) -> Self {
        self.vantage = vantage;
        self
    }

    /// Start building the status summary in the background now, so the
    /// first page view finds it ready (otherwise the first request starts
    /// it, and waits for it).
    pub fn start_refresher(&self) {
        self.refresher.start(self);
    }

    /// The latest status snapshot (see [`snapshot`]).
    pub(crate) async fn snapshot(&self) -> Arc<Snapshot> {
        self.refresher.snapshot(self).await
    }

    /// Rebuild the snapshot and wait for it: an operator write that must
    /// show on the next page view.
    pub(crate) async fn refresh_now(&self) {
        self.refresher.refresh_now(self).await;
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
///
/// Behind a proxy that nobody named, every client shares the proxy's address,
/// hence one rate-limit bucket: a busy page then answers 429 to everyone. The
/// first request that arrives with a forwarding header while none is
/// configured logs a single warning saying so.
#[derive(Clone)]
pub(crate) struct ConfiguredIp {
    header: Option<HeaderName>,
    /// Set once the "behind an unnamed proxy" warning was logged.
    proxy_hint: Arc<std::sync::atomic::AtomicBool>,
}

/// The headers a reverse proxy adds; their presence without a configured
/// `client_ip_header` means the peer address is the proxy's.
const FORWARDING_HEADERS: [&str; 3] = ["x-forwarded-for", "x-real-ip", "forwarded"];

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
        if self.header.is_none() {
            self.note_unnamed_proxy(req);
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
        Self {
            header,
            proxy_hint: Arc::default(),
        }
    }

    /// Log, once per process, that requests come through a proxy that no
    /// `client_ip_header` names. Returns whether this call logged it.
    fn note_unnamed_proxy<T>(&self, req: &Request<T>) -> bool {
        use std::sync::atomic::Ordering;
        let forwarded = FORWARDING_HEADERS
            .iter()
            .find(|name| req.headers().contains_key(**name));
        let Some(name) = forwarded else {
            return false;
        };
        if self.proxy_hint.swap(true, Ordering::Relaxed) {
            return false;
        }
        tracing::warn!(
            "requests carry a {name} header but server.client_ip_header is unset: the \
             rate limits key on the TCP peer, so behind a reverse proxy every client \
             shares the proxy's bucket (429s for everyone under load). Name the header \
             your proxy sets (e.g. client_ip_header = \"x-forwarded-for\") - only if \
             the proxy overwrites it and the origin is not reachable directly"
        );
        true
    }
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

    #[test]
    fn an_unnamed_proxy_is_reported_once() {
        let request = |header: &str| {
            Request::builder()
                .header(header, "10.0.0.1")
                .body(())
                .expect("request")
        };
        let unset = ConfiguredIp::from_config(None);
        assert!(!unset.note_unnamed_proxy(&request("accept")));
        assert!(unset.note_unnamed_proxy(&request("x-real-ip")));
        // Once: the clones the router holds share the flag.
        assert!(!unset.clone().note_unnamed_proxy(&request("forwarded")));
        assert!(!unset.note_unnamed_proxy(&request("x-forwarded-for")));
    }
}
