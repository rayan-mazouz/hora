//! A `[[peers]]` entry: another Hora instance to watch or answer.

use std::time::Duration;

use serde::Deserialize;

use super::secret::Secret;

/// A peer in a mutual-surveillance mesh. The two halves are independent: `ping_url`
/// is the OUT side (I heartbeat the peer when healthy); `expect_every_secs` enables
/// the IN side (I mark the peer down if it stops heartbeating me). Either or both
/// may be set; each half can terminate at another Hora or at an external service.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Peer {
    /// The peer's global identity (its `[health].id`). Witnesses key their view of
    /// the peer by this id, so it must match across the mesh.
    pub id: String,
    pub name: String,
    /// OUT: where I `POST` my heartbeat when healthy - the peer's
    /// `/api/push/{my-id}`, or an external receiver. Absent = I don't ping it.
    #[serde(default)]
    pub ping_url: Option<Secret>,
    /// Token sent as the `X-Push-Token` header on the outbound ping (kept out of
    /// the peer's access logs, unlike a `?token=` query).
    #[serde(default)]
    pub ping_token: Option<Secret>,
    /// IN: the local push id the peer heartbeats (`/api/push/{listen_id}`).
    /// Defaults to `id`. Only meaningful when `expect_every_secs` is set.
    #[serde(default)]
    pub listen_id: Option<String>,
    /// Token required on the inbound `/api/push/{listen_id}` (the peer's
    /// `ping_token` for me).
    #[serde(default)]
    pub listen_token: Option<Secret>,
    /// Watch the peer without a `listen_token`: its heartbeats are accepted on
    /// the id alone. For isolated networks; warned at every load.
    #[serde(default)]
    pub allow_unauthenticated_push: bool,
    /// Watch the peer: mark it down if no inbound heartbeat arrives within this
    /// window. Setting it enables the IN side; leaving it unset makes this an
    /// OUT-only peer (a plain dead-man to an external receiver).
    #[serde(default)]
    pub expect_every_secs: Option<u64>,
    /// Where other peers poll this peer's `/healthz` for quorum. Defaults to the
    /// origin (`scheme://host[:port]`) of `ping_url` plus `/healthz`. A
    /// [`Secret`] like `ping_url`: it may carry a token in its query string.
    #[serde(default)]
    pub witness_url: Option<Secret>,
    /// Restrict this peer's alerts to these channel names. Unset = every channel.
    #[serde(default)]
    pub notify: Option<Vec<String>>,
}

impl Peer {
    /// The local push id this peer heartbeats (falls back to the peer's `id`).
    #[must_use]
    pub fn listen_id(&self) -> &str {
        self.listen_id.as_deref().unwrap_or(&self.id)
    }

    /// Whether the IN side is active (I watch this peer for missed heartbeats).
    #[must_use]
    pub fn is_watched(&self) -> bool {
        self.expect_every_secs.is_some()
    }

    /// The inbound staleness window, defaulting to the global health interval.
    #[must_use]
    pub fn expect_every(&self) -> Option<Duration> {
        self.expect_every_secs.map(Duration::from_secs)
    }

    /// The peer's `/healthz` URL for quorum: the explicit `witness_url`, else the
    /// origin (`scheme://host[:port]`) of `ping_url` with `/healthz` appended.
    #[must_use]
    pub fn effective_witness_url(&self) -> Option<String> {
        if let Some(url) = &self.witness_url {
            return Some(url.as_ref().to_owned());
        }
        self.api_origin().map(|origin| format!("{origin}/healthz"))
    }

    /// The peer's `/api/peer/monitors` URL - the shared-monitor disclosure
    /// behind `hora peers diff` and the per-vantage latency display - derived
    /// like [`probe_url`](Self::probe_url).
    #[must_use]
    pub fn monitors_url(&self) -> Option<String> {
        self.api_origin()
            .map(|origin| format!("{origin}/api/peer/monitors"))
    }

    /// The peer's `/api/peer/probe` URL for multi-vantage confirmation, derived
    /// from the origin of `ping_url`. `None` when there is no `ping_url` (an
    /// IN-only or external peer cannot be asked to probe).
    #[must_use]
    pub fn probe_url(&self) -> Option<String> {
        self.api_origin()
            .map(|origin| format!("{origin}/api/peer/probe"))
    }

    /// The `scheme://host[:port]` origin of the peer's API, from its `ping_url`.
    fn api_origin(&self) -> Option<String> {
        let ping = self.ping_url.as_ref()?;
        let url = reqwest::Url::parse(ping.as_ref()).ok()?;
        let origin = url.origin();
        origin.is_tuple().then(|| origin.ascii_serialization())
    }
}
