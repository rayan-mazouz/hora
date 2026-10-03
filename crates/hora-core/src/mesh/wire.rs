//! The JSON shapes exchanged between Hora instances: the `/healthz` report a
//! witness reads, the confirmation probe of `/api/peer/probe`, and the
//! monitor listing of `/api/peer/monitors`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::config::Kind;

// --- /healthz report ------------------------------------------------------

/// The body of `/healthz`: this node's own health plus its view of every peer it
/// watches. `status` is `"ok"` only when the node is fully healthy, so an external
/// keyword monitor (e.g. `UptimeRobot` matching `"ok"`) detects trouble; the rest of
/// the document is ignored by such pollers but read by other Hora for quorum.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct HealthReport {
    /// `"ok"` when the scheduler and database are both healthy, else `"degraded"`.
    pub status: String,
    pub scheduler_ok: bool,
    pub db_ok: bool,
    /// Seconds since the most recent scheduler tick; `-1` if it has not ticked yet.
    pub last_tick_age: i64,
    /// This node's identity (`[health].id`), absent if no `[health]` section.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// This node's view of each watched peer, keyed by the peer's global id.
    pub peers: HashMap<String, PeerSeen>,
}

/// One node's view of a peer, as reported on `/healthz`.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PeerSeen {
    /// `"up"`, `"down"`, or `"unknown"` (never seen).
    pub state: String,
    /// Seconds since the peer's last heartbeat; `-1` if never seen.
    pub age: i64,
}

// --- Peer API (/api/peer/*) --------------------------------------------------

/// What one node asks another to probe. The responder matches `kind` +
/// `target` against its own monitors and refuses anything else.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ProbeRequest {
    /// The requesting node's `[health].id`; the responder authenticates it
    /// against that peer's `listen_token`.
    pub from: String,
    pub kind: Kind,
    pub target: String,
}

/// The vantage's verdict on one probe.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ProbeResponse {
    pub up: bool,
    /// The failure reason when not up, bounded by the responder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What a node discloses to an authenticated peer on `GET /api/peer/monitors`:
/// its probeable monitors with their live view - enough for `hora peers diff`
/// (are our configs aligned?) and the per-vantage latency display, and
/// nothing else (no names, notes or credentials).
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PeerMonitors {
    pub monitors: Vec<PeerMonitor>,
}

/// One probeable monitor as seen from a peer's vantage.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PeerMonitor {
    pub kind: Kind,
    pub target: String,
    /// `up` | `degraded` | `down` | `unknown`, from that node's view.
    pub status: String,
    /// That node's 24h median latency to the target, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p50_ms: Option<i64>,
}
