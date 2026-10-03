//! ICMP echo (ping) checks.

use super::FailureKind;
use socket2::Type;
use surge_ping::{
    Client as PingClient, Config as PingConfig, ICMP, PingIdentifier, PingSequence, SurgeError,
};

use super::family::Family;
use super::{Outcome, millis, over_threshold, resolve};
use crate::config::{IcmpSpec, Monitor};

/// ICMP echo (ping). Uses a per-probe unprivileged datagram socket (no
/// `CAP_NET_RAW`), so it works in rootless Docker; one socket per probe avoids
/// datagram identifier collisions between concurrent monitors. The address family
/// (IPv4/IPv6) follows the resolved address.
pub(super) async fn icmp(monitor: &Monitor, spec: &IcmpSpec) -> Outcome {
    icmp_family(monitor, spec, None).await
}

/// ICMP to the first resolved address - of one specific family when given.
pub(super) async fn icmp_family(
    monitor: &Monitor,
    spec: &IcmpSpec,
    family: Option<Family>,
) -> Outcome {
    let Some(addr) = resolve(&spec.target, family).await else {
        return match family {
            Some(family) => Outcome::down(
                FailureKind::Other,
                format!("no {} address for host", family.label()),
            ),
            None => Outcome::down(
                FailureKind::Unresolvable,
                "could not resolve host".to_owned(),
            ),
        };
    };

    let kind = if addr.is_ipv4() { ICMP::V4 } else { ICMP::V6 };
    let config = PingConfig::builder()
        .kind(kind)
        .sock_type_hint(Type::DGRAM)
        .build();
    let client = match PingClient::new(&config) {
        Ok(client) => client,
        // Usually a missing privilege: no unprivileged-ping permission and no
        // CAP_NET_RAW. Surface it clearly rather than as a generic failure.
        Err(err) => {
            return Outcome::down(
                FailureKind::Ping,
                format!(
                    "icmp socket unavailable ({err}); needs net.ipv4.ping_group_range or CAP_NET_RAW"
                ),
            );
        }
    };

    let mut pinger = client.pinger(addr, PingIdentifier(0)).await;
    pinger.timeout(monitor.timeout());
    match pinger.ping(PingSequence(0), &[0u8; 16]).await {
        Ok((_packet, rtt)) => {
            let latency = millis(rtt);
            Outcome::up(
                over_threshold(latency, monitor.degraded_over_ms),
                Some(latency),
                None,
            )
        }
        Err(SurgeError::Timeout { .. }) => {
            Outcome::down(FailureKind::Timeout, "request timed out".to_owned())
        }
        Err(err) => Outcome::down(FailureKind::Ping, format!("icmp error: {err}")),
    }
}
