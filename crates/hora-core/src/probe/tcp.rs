//! TCP connect checks.

use crate::status::CheckStatus;
use std::time::Instant;

use tokio::net::TcpStream;

use super::family::Family;
use super::{Outcome, millis, over_threshold};
use crate::config::Monitor;

pub(super) async fn tcp(monitor: &Monitor) -> Outcome {
    tcp_connect(monitor, monitor.target.as_str()).await
}

/// TCP for one address family: resolve the `host:port` target ourselves and
/// connect to the first address of that family.
pub(super) async fn tcp_family(monitor: &Monitor, family: Family) -> Outcome {
    let Ok(addrs) = tokio::net::lookup_host(&monitor.target).await else {
        return Outcome::down("could not resolve host".to_owned());
    };
    match addrs.into_iter().find(|addr| family.matches(addr.ip())) {
        Some(addr) => tcp_connect(monitor, addr).await,
        None => Outcome::down(format!("no {} address for host", family.label())),
    }
}

async fn tcp_connect<A: tokio::net::ToSocketAddrs>(monitor: &Monitor, addr: A) -> Outcome {
    let start = Instant::now();
    match tokio::time::timeout(monitor.timeout(), TcpStream::connect(addr)).await {
        Ok(Ok(_stream)) => {
            let latency = millis(start.elapsed());
            Outcome {
                status: CheckStatus::up_unless(over_threshold(latency, monitor.degraded_over_ms)),
                latency_ms: Some(latency),
                status_code: None,
                error: None,
                snapshot: None,
            }
        }
        Ok(Err(err)) => Outcome::down(err.to_string()),
        Err(_elapsed) => Outcome::down("connection timed out".to_owned()),
    }
}
