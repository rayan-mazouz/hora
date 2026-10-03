//! The view model the page template and the JSON API render.

use std::sync::Arc;

use askama::Template;
use serde::Serialize;
use utoipa::ToSchema;

// --- View model ----------------------------------------------------------

#[derive(Serialize, ToSchema)]
pub(crate) struct Summary {
    pub(crate) title: String,
    pub(crate) overall: &'static str,
    pub(crate) overall_label: &'static str,
    pub(super) generated_at: String,
    #[serde(skip)]
    pub(crate) updated_utc: String,
    pub(crate) incidents: Vec<IncidentView>,
    pub(crate) maintenances: Vec<MaintenanceView>,
    /// Shared with the groups' card lists (and the per-group pages derived
    /// from this summary), so a view - chart SVG included - is built once.
    #[serde(serialize_with = "serialize_views")]
    #[schema(value_type = Vec<MonitorView>)]
    pub(crate) monitors: Vec<Arc<MonitorView>>,
    /// Monitor groups: `(group_name, monitors)`. Ungrouped monitors appear under
    /// an empty-string key, always last.
    pub(crate) groups: Vec<GroupView>,
    pub(crate) peers: Vec<PeerView>,
    /// Notification-channel health (operator only; empty in the public view).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) channels: Vec<ChannelView>,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct GroupView {
    pub(crate) name: String,
    /// Member monitor ids, in display order. The full monitor objects live once in
    /// the top-level `monitors`; the API carries only ids here so the response is
    /// not doubled, and a caller maps ids back to the monitors to render sections.
    pub(super) ids: Vec<String>,
    /// The rendered cards, for the server-side template only; skipped from the API
    /// (it would duplicate every monitor object).
    #[serde(skip)]
    pub(crate) monitors: Vec<Arc<MonitorView>>,
}

/// Serialize shared views as plain objects (serde's `rc` feature is off).
fn serialize_views<S: serde::Serializer>(
    views: &[Arc<MonitorView>],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(views.iter().map(AsRef::as_ref))
}

#[derive(Clone, Serialize, ToSchema)]
pub(crate) struct IncidentView {
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) severity: &'static str,
    pub(super) at: Option<String>,
}

#[derive(Clone, Serialize, ToSchema)]
pub(crate) struct MaintenanceView {
    pub(crate) reason: String,
    pub(crate) monitors: String,
}

/// A watched peer's view for the status page: just its current status and when it
/// was last seen. Peers are deliberately rendered apart from monitors.
#[derive(Clone, Serialize, ToSchema)]
pub(crate) struct PeerView {
    pub(super) id: String,
    pub(crate) name: String,
    pub(crate) status: &'static str,
    pub(super) last_seen: Option<String>,
    /// Whether this node also heartbeats the peer (the OUT side is configured).
    pub(super) pings: bool,
}

/// Notification-channel health, for the operator view only (`top`, authenticated
/// `/api/summary`). The public status page never sees this: channel names and
/// delivery health are the operator's business, not a client's.
#[derive(Serialize, ToSchema)]
pub(crate) struct ChannelView {
    pub(super) name: String,
    pub(super) failing: bool,
    pub(super) consecutive_failures: u32,
    /// Seconds since the first failure in the current streak; `null` when the
    /// channel is healthy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) failing_for_secs: Option<u64>,
}

/// A card. Cloned only to adjust a few fields for another audience (see
/// `summary::derive`): the heavy parts - the chart SVGs and the daily bar -
/// are shared, not copied.
#[derive(Clone, Serialize, ToSchema)]
pub(crate) struct MonitorView {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) status: &'static str,
    pub(crate) last_latency_ms: Option<i64>,
    /// Failure reason of the most recent check, when it was not up: surfaces
    /// the *why* behind a degraded/down card without opening the database.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) last_error: Option<String>,
    pub(super) last_checked: Option<String>,
    #[serde(rename = "uptime_24h_permille")]
    pub(crate) uptime_permille: Option<i64>,
    #[serde(skip)]
    pub(super) uptime_label: Option<String>,
    #[serde(rename = "latency_p50_ms")]
    pub(crate) p50_ms: Option<i64>,
    #[serde(rename = "latency_p95_ms")]
    pub(crate) p95_ms: Option<i64>,
    #[serde(rename = "latency_p99_ms")]
    pub(crate) p99_ms: Option<i64>,
    #[serde(rename = "slo_latency_ms")]
    pub(super) slo_latency_ms: Option<i64>,
    #[serde(skip)]
    pub(super) slo_state: &'static str,
    /// Availability SLO target, percent (e.g. 99.9).
    #[serde(rename = "slo_uptime_pct", skip_serializing_if = "Option::is_none")]
    pub(super) slo_uptime_pct: Option<f64>,
    #[serde(
        rename = "error_budget_minutes_total",
        skip_serializing_if = "Option::is_none"
    )]
    pub(super) budget_minutes_total: Option<i64>,
    #[serde(
        rename = "error_budget_minutes_left",
        skip_serializing_if = "Option::is_none"
    )]
    pub(super) budget_minutes_left: Option<i64>,
    #[serde(skip)]
    pub(super) budget_label: Option<String>,
    #[serde(skip)]
    pub(super) budget_title: String,
    #[serde(skip)]
    pub(super) budget_state: &'static str,
    /// Active maintenance window title (alerts muted); `None` outside a window.
    pub(super) maintenance: Option<String>,
    #[serde(rename = "cert_expiry_days")]
    pub(crate) cert_days: Option<i64>,
    #[serde(skip)]
    pub(super) cert_label: Option<String>,
    #[serde(skip)]
    pub(super) cert_state: &'static str,
    #[serde(rename = "history")]
    #[schema(value_type = Vec<DayCell>)]
    pub(super) bar: DayBar,
    /// The sparkline as this audience sees it (event markers for the
    /// operator only).
    #[serde(skip)]
    pub(super) chart_svg: Arc<str>,
    /// The sparkline without event markers, what every other audience gets
    /// (the same allocation as `chart_svg` when no marker falls on it).
    #[serde(skip)]
    pub(super) chart_plain: Arc<str>,
    /// Display group this monitor belongs to.
    pub(super) group: Option<String>,
    /// Upstream monitor name causing this failure (topology annotation).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cause: Option<String>,
    /// Downstream monitor names impacted by this root-cause failure.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) impacted: Vec<String>,
    /// The peers' view of this target ("80 ms from EU"), when the mesh
    /// monitors it too. Empty without peers.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) vantages: Vec<VantageView>,
}

/// One peer's view of a monitor's target, for the card and the API.
#[derive(Clone, Serialize, ToSchema)]
pub(crate) struct VantageView {
    /// The peer's display name.
    pub(crate) peer: String,
    /// `up` | `degraded` | `down` | `unknown` from that vantage.
    pub(crate) status: String,
    /// That vantage's 24h median latency, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) p50_ms: Option<i64>,
    /// The pre-formatted card label ("Hora B: 220ms" / "Hora B: down").
    #[serde(skip)]
    pub(crate) label: String,
}

/// The daily uptime bar, shared between a card and its copies for other
/// audiences. Iterates and serializes like the plain list of cells.
#[derive(Clone)]
pub(crate) struct DayBar(Arc<[DayCell]>);

impl From<Vec<DayCell>> for DayBar {
    fn from(cells: Vec<DayCell>) -> Self {
        Self(cells.into())
    }
}

impl std::ops::Deref for DayBar {
    type Target = [DayCell];

    fn deref(&self) -> &[DayCell] {
        &self.0
    }
}

impl<'a> IntoIterator for &'a DayBar {
    type Item = &'a DayCell;
    type IntoIter = std::slice::Iter<'a, DayCell>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl Serialize for DayBar {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.iter())
    }
}

#[derive(Clone, Serialize, ToSchema)]
pub(crate) struct DayCell {
    pub(super) date: String,
    pub(super) state: &'static str,
    #[serde(skip)]
    pub(super) title: String,
}

#[derive(Template)]
#[template(path = "status.html")]
pub(crate) struct StatusTemplate<'a> {
    pub(crate) summary: &'a Summary,
}
