//! Probing logic: turn a monitor into a single [`Outcome`].
//!
//! One submodule per probe kind, plus the dual-stack (IPv4 + IPv6) fan-out,
//! the failure snapshot rendering and the public-visibility policy for
//! failure reasons.

mod dns;
mod family;
mod http;
mod icmp;
mod reason;
mod snapshot;
mod tcp;
#[cfg(test)]
mod tests;

pub use reason::public_reason;

use std::net::IpAddr;
use std::time::Duration;

use reqwest::Client;

use crate::config::{Kind, Monitor};
use crate::status::CheckStatus;

use dns::dns;
use family::{Family, dual_stack};
use http::http;
use icmp::icmp;
use tcp::tcp;

/// Result of a single probe.
#[derive(Debug)]
pub struct Outcome {
    pub status: CheckStatus,
    pub latency_ms: Option<i64>,
    pub status_code: Option<i64>,
    pub error: Option<String>,
    /// The failing HTTP response (status line, headers, start of the body),
    /// bounded; only set when the probe got a response back. Stored on the
    /// incident when the down is confirmed.
    pub snapshot: Option<String>,
}

impl Outcome {
    /// Whether the check passed (up or degraded).
    #[must_use]
    pub fn is_up(&self) -> bool {
        self.status.is_available()
    }

    /// Whether the check passed too slowly (or with a warning).
    #[must_use]
    pub fn is_degraded(&self) -> bool {
        self.status == CheckStatus::Degraded
    }

    pub(crate) fn down(error: String) -> Self {
        Self {
            status: CheckStatus::Down,
            latency_ms: None,
            status_code: None,
            error: Some(error),
            snapshot: None,
        }
    }
}

/// Pause before a retry, long enough for a micro-blip (packet loss, a
/// connection reset mid-deploy) to pass, short next to any real interval.
const RETRY_DELAY: Duration = Duration::from_secs(1);

/// Probe a monitor according to its kind, re-trying failures up to the
/// monitor's `probe_retries` (default 1). Only the final attempt is reported:
/// a blip that passes on retry never reaches the history, the page or the
/// error budget. Retries are logged so a flaky path stays visible in the logs
/// even when the recorded check ends up green.
#[must_use]
pub async fn run(client: &Client, monitor: &Monitor) -> Outcome {
    let mut outcome = probe_once(client, monitor).await;
    for attempt in 1..=monitor.probe_retries() {
        if outcome.is_up() {
            break;
        }
        tracing::info!(
            monitor = %monitor.id,
            attempt,
            error = outcome.error.as_deref().unwrap_or("unknown"),
            "probe failed, retrying"
        );
        tokio::time::sleep(RETRY_DELAY).await;
        outcome = probe_once(client, monitor).await;
    }
    outcome
}

async fn probe_once(client: &Client, monitor: &Monitor) -> Outcome {
    if monitor.dual_stack() {
        return dual_stack(monitor).await;
    }
    match monitor.kind {
        Kind::Http => http(client, monitor).await,
        Kind::Tcp => tcp(monitor).await,
        Kind::Icmp => icmp(monitor).await,
        Kind::Dns => dns(monitor).await,
        // Push monitors are evaluated from stored heartbeats, exec monitors by
        // the exec module - both routed by the scheduler before reaching here;
        // these arms are defensive only.
        Kind::Push => Outcome::down("push monitor has no active probe".to_owned()),
        Kind::Exec => Outcome::down("exec monitor is not a network probe".to_owned()),
    }
}

/// Resolve an ICMP target to a single IP: an IP literal is used directly,
/// otherwise DNS is consulted and the first address is taken (so an IPv4-only or
/// IPv6-only host resolves to the family it actually has). With a `family`, only
/// addresses of that family qualify.
async fn resolve(target: &str, family: Option<Family>) -> Option<IpAddr> {
    let wanted = |ip: IpAddr| family.is_none_or(|family| family.matches(ip));
    if let Ok(ip) = target.parse::<IpAddr>() {
        return wanted(ip).then_some(ip);
    }
    tokio::net::lookup_host((target, 0u16))
        .await
        .ok()?
        .map(|addr| addr.ip())
        .find(|ip| wanted(*ip))
}

fn over_threshold(latency_ms: i64, threshold: Option<i64>) -> bool {
    threshold.is_some_and(|limit| latency_ms > limit)
}

fn millis(elapsed: Duration) -> i64 {
    i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
}
