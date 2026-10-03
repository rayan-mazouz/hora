//! DNS checks: resolve a record and optionally compare the answer.

use super::FailureKind;
use crate::status::CheckStatus;
use std::net::SocketAddr;
use std::sync::OnceLock;
use std::time::Instant;

use hickory_resolver::TokioResolver;
use hickory_resolver::config::{ConnectionConfig, NameServerConfig, ResolverConfig, ResolverOpts};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::proto::rr::RecordType;

use super::snapshot::{MAX_SNAPSHOT_BODY_CHARS, snippet};
use super::{Outcome, millis, over_threshold};
use crate::config::{DnsRecord, Monitor, Parsed};

/// DNS resolution: resolve a name and, when `dns_expected` is set, pin the
/// answer (hijack detection). Without it any non-empty answer counts as up -
/// answers rotate freely behind CDNs and round-robin records, so alerting on
/// mere change would flap.
pub(super) async fn dns(monitor: &Monitor) -> Outcome {
    let record_type = record_type(
        monitor
            .dns_record
            .as_ref()
            .and_then(Parsed::get)
            .copied()
            .unwrap_or(DnsRecord::A),
    );

    let custom = monitor.dns_resolver.as_ref().and_then(Parsed::get).copied();
    let resolver = match resolver_for(custom) {
        Ok(resolver) => resolver,
        Err(err) => {
            return Outcome::down(
                FailureKind::DnsFailed,
                format!("resolver setup failed: {err}"),
            );
        }
    };

    let start = Instant::now();
    let result = tokio::time::timeout(
        monitor.timeout(),
        resolver.lookup(monitor.target.as_str(), record_type),
    )
    .await;
    let latency = millis(start.elapsed());

    match result {
        Ok(Ok(lookup)) => {
            // The answer section may also carry the CNAME chain; keep only the
            // requested type so assertions compare like with like.
            let mut answers: Vec<String> = lookup
                .answers()
                .iter()
                .filter(|record| record.record_type() == record_type)
                .map(|record| record.data.to_string().trim_end_matches('.').to_owned())
                .collect();
            if answers.is_empty() {
                return Outcome::down(
                    FailureKind::NoRecords,
                    format!("no {record_type} records found"),
                );
            }
            answers.sort();
            let answer = answers.join(",");

            if let Some(expected) = &monitor.dns_expected {
                // Both sides sorted: the assertion is order-insensitive, so
                // rotation within a pinned record set never flaps.
                let mut wanted: Vec<&str> = expected.split(',').map(str::trim).collect();
                wanted.sort_unstable();
                let wanted = wanted.join(",");
                if answer != wanted {
                    // The full (but still bounded) answer goes into the failure
                    // snapshot - "what did it actually answer?" applies to DNS
                    // pins too, and TXT answers rarely fit the inline reason.
                    let snapshot: String = format!("DNS answer: {answer}")
                        .chars()
                        .take(MAX_SNAPSHOT_BODY_CHARS)
                        .collect();
                    // The answer is remote-controlled (TXT records run to tens of
                    // KB and may span lines); snippet() bounds and single-lines it
                    // exactly like an HTTP failure body.
                    let answer = snippet(answer.as_bytes());
                    return Outcome {
                        status: CheckStatus::Down,
                        latency_ms: Some(latency),
                        status_code: None,
                        error: Some(format!("expected {wanted}, got {answer}")),
                        reason: Some(FailureKind::DnsMismatch),
                        snapshot: Some(snapshot),
                    };
                }
            }

            Outcome::up(
                over_threshold(latency, monitor.degraded_over_ms),
                Some(latency),
                None,
            )
        }
        Ok(Err(err)) => Outcome::down(FailureKind::DnsFailed, format!("DNS lookup failed: {err}")),
        Err(_elapsed) => Outcome::down(FailureKind::DnsTimeout, "DNS lookup timed out".to_owned()),
    }
}

pub(super) fn record_type(record: DnsRecord) -> RecordType {
    match record {
        DnsRecord::A => RecordType::A,
        DnsRecord::Aaaa => RecordType::AAAA,
        DnsRecord::Cname => RecordType::CNAME,
        DnsRecord::Mx => RecordType::MX,
        DnsRecord::Ns => RecordType::NS,
        DnsRecord::Txt => RecordType::TXT,
        DnsRecord::Srv => RecordType::SRV,
        DnsRecord::Soa => RecordType::SOA,
        DnsRecord::Ptr => RecordType::PTR,
    }
}

/// The system resolver, or a custom UDP resolver when configured (its address
/// parsed at config load). A fresh resolver per probe on purpose: a shared one
/// would answer from its cache, and a DNS monitor must ask the server every
/// time - so only the system configuration is shared (see [`system_conf`]).
pub(super) fn resolver_for(custom: Option<SocketAddr>) -> anyhow::Result<TokioResolver> {
    let provider = TokioRuntimeProvider::default();
    let Some(addr) = custom else {
        let (config, options) = system_conf()?;
        let mut builder = TokioResolver::builder_with_config(config, provider);
        *builder.options_mut() = options;
        return Ok(builder.build()?);
    };
    let mut connection = ConnectionConfig::udp();
    connection.port = addr.port();
    let nameserver = NameServerConfig::new(addr.ip(), true, vec![connection]);
    let config = ResolverConfig::from_parts(None, vec![], vec![nameserver]);
    Ok(TokioResolver::builder_with_config(config, provider).build()?)
}

/// The system resolver configuration (`/etc/resolv.conf` on Unix), read once:
/// reading it is blocking file I/O that has no business running on the async
/// runtime at every DNS probe. A failed read is not cached, so it is retried
/// at the next probe; a change to the file needs a restart to be seen.
fn system_conf() -> anyhow::Result<(ResolverConfig, ResolverOpts)> {
    static SYSTEM: OnceLock<(ResolverConfig, ResolverOpts)> = OnceLock::new();
    if let Some(conf) = SYSTEM.get() {
        return Ok(conf.clone());
    }
    let conf = hickory_resolver::system_conf::read_system_conf()?;
    Ok(SYSTEM.get_or_init(|| conf).clone())
}
