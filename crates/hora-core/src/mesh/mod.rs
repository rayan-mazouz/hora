//! The mesh: Hora instances watching and helping each other.
//!
//! - [`peer`]: mutual surveillance (outbound dead-man heartbeats, peer watches
//!   with a witness quorum, and the `/healthz` report).
//! - [`confirm`]: multi-vantage confirmation of a local down before alerting.
//! - [`vantage`]: per-vantage latency, polled from each peer's monitor listing.
//! - [`wire`]: the JSON shapes those exchanges carry.

pub mod confirm;
pub mod peer;
pub mod vantage;
pub mod wire;

use crate::config::{Monitor, redact_url_secrets};

/// A monitor's target as the mesh exchanges it: credentials in the URL
/// (`user:pass@`, query values) masked. Peers name and match shared monitors
/// by this form, so neither side's credentials cross the wire.
#[must_use]
pub fn shared_target(monitor: &Monitor) -> String {
    redact_url_secrets(monitor.target()).into_owned()
}
