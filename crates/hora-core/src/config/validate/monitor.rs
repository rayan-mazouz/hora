//! Per-monitor validation: the fields each probe kind accepts.

use crate::config::{Kind, Monitor, Parsed, ReleaseWatch, parse_cron, split_host_port};

/// Exec monitors: a `command` argv instead of a `target`, with `command[0]`
/// a bare file name - resolution happens strictly inside `HORA_EXEC_DIR`, so
/// a path here is either a mistake or an escape attempt.
fn validate_exec_io(monitor: &Monitor) -> anyhow::Result<()> {
    anyhow::ensure!(
        monitor.target.is_empty(),
        "monitor {}: exec monitors take a `command`, not a `target`",
        monitor.id
    );
    let Some(name) = monitor.command.first() else {
        anyhow::bail!(
            "monitor {}: exec monitors need a non-empty `command` array",
            monitor.id
        );
    };
    anyhow::ensure!(
        !name.is_empty()
            && !name.contains('/')
            && !name.contains('\\')
            && name != "."
            && name != "..",
        "monitor {}: command[0] must be a bare file name inside HORA_EXEC_DIR",
        monitor.id
    );
    Ok(())
}

/// Validate a monitor's target, latency thresholds and headers (split out of
/// [`validate`](super::validate) to keep it small).
pub(super) fn validate_monitor_io(monitor: &Monitor) -> anyhow::Result<()> {
    // Parse the target now, so a typo fails at load instead of at probe time.
    match monitor.kind {
        Kind::Http => anyhow::ensure!(
            reqwest::Url::parse(&monitor.target)
                .is_ok_and(|url| matches!(url.scheme(), "http" | "https")),
            "monitor {}: target must be an http(s) URL",
            monitor.id
        ),
        Kind::Tcp => anyhow::ensure!(
            split_host_port(&monitor.target).is_some(),
            "monitor {}: tcp target must be host:port (an IPv6 address in brackets: [::1]:80)",
            monitor.id
        ),
        Kind::Icmp => {
            // A stray `:port` is a common mistake; an IPv6 literal parses as an IP
            // and is exempt, so its colons are fine.
            let has_port = monitor.target.parse::<std::net::IpAddr>().is_err()
                && split_host_port(&monitor.target).is_some();
            anyhow::ensure!(
                !monitor.target.contains("://")
                    && !monitor.target.contains('/')
                    && !monitor.target.chars().any(char::is_whitespace)
                    && !has_port,
                "monitor {}: icmp target must be a bare host or IP (no scheme, path, or port)",
                monitor.id
            );
        }
        Kind::Push => {}
        Kind::Exec => validate_exec_io(monitor)?,
        Kind::Dns => validate_dns_io(monitor)?,
    }
    anyhow::ensure!(
        monitor.kind == Kind::Exec || monitor.command.is_empty(),
        "monitor {}: `command` requires kind = \"exec\"",
        monitor.id
    );
    // A negative latency threshold would mark every check degraded/breached.
    for (label, value) in [
        ("degraded_over_ms", monitor.degraded_over_ms),
        ("slo_latency_ms", monitor.slo_latency_ms),
    ] {
        if let Some(ms) = value {
            anyhow::ensure!(ms > 0, "monitor {}: {label} must be > 0", monitor.id);
        }
    }
    // Each retry can cost a full timeout; an unbounded count would let one
    // tick outlive several intervals.
    anyhow::ensure!(
        monitor.probe_retries.is_none_or(|retries| retries <= 5),
        "monitor {}: probe_retries must be at most 5",
        monitor.id
    );
    anyhow::ensure!(
        monitor.max_body_kb != Some(0),
        "monitor {}: max_body_kb must be > 0",
        monitor.id
    );
    // No server answers outside 100-599, so such a monitor could never be up.
    if let Some(status) = monitor.expected_status {
        anyhow::ensure!(
            (100..=599).contains(&status),
            "monitor {}: expected_status {status} is not an HTTP status (100-599)",
            monitor.id
        );
    }
    // Catch header typos / CR-LF injection at load rather than at send time.
    for (name, value) in &monitor.headers {
        anyhow::ensure!(
            reqwest::header::HeaderName::from_bytes(name.as_bytes()).is_ok(),
            "monitor {}: invalid header name {name:?}",
            monitor.id
        );
        anyhow::ensure!(
            reqwest::header::HeaderValue::from_str(value.as_ref()).is_ok(),
            "monitor {}: invalid value for header {name:?}",
            monitor.id
        );
    }
    anyhow::ensure!(
        monitor.kind == Kind::Http
            || (monitor.keyword.is_none()
                && monitor.json_query.is_none()
                && monitor.number_regex.is_none()
                && monitor.proxy.is_none()),
        "monitor {}: keyword/json_query/number_regex/proxy require an http monitor",
        monitor.id
    );
    validate_body_assertions(monitor)?;
    if let Some(proxy) = &monitor.proxy {
        reqwest::Proxy::all(proxy)
            .map_err(|err| anyhow::anyhow!("monitor {}: invalid proxy: {err}", monitor.id))?;
    }
    anyhow::ensure!(
        monitor.kind == Kind::Dns
            || (monitor.dns_record.is_none()
                && monitor.dns_expected.is_none()
                && monitor.dns_resolver.is_none()),
        "monitor {}: dns_record/dns_expected/dns_resolver require a dns monitor",
        monitor.id
    );
    validate_starttls(monitor)?;
    // A malformed pin (wrong length, non-hex) can never match the observed
    // fingerprint, which silently disables pinning after one spurious alert.
    validate_pins(monitor)?;
    if monitor.dual_stack() {
        validate_dual_stack(monitor)?;
    }
    validate_schedule_and_slo(monitor)?;
    Ok(())
}

/// Validate the body assertions: the `JSONPath` and the number regex must
/// compile, and the number bounds only make sense with a regex to extract the
/// number and in min <= max order.
fn validate_body_assertions(monitor: &Monitor) -> anyhow::Result<()> {
    if let Some(err) = monitor.json_query.as_ref().and_then(Parsed::error) {
        anyhow::bail!("monitor {}: invalid json_query: {err}", monitor.id);
    }
    if let Some(pattern) = &monitor.number_regex {
        if let Some(err) = pattern.error() {
            anyhow::bail!("monitor {}: invalid number_regex: {err}", monitor.id);
        }
    } else {
        anyhow::ensure!(
            monitor.number_min.is_none() && monitor.number_max.is_none(),
            "monitor {}: number_min/number_max require number_regex",
            monitor.id
        );
    }
    if let (Some(min), Some(max)) = (monitor.number_min, monitor.number_max) {
        anyhow::ensure!(
            min <= max,
            "monitor {}: number_min {min} exceeds number_max {max}",
            monitor.id
        );
    }
    Ok(())
}

/// STARTTLS is the certificate watcher's business: it only makes sense on a
/// tcp monitor (the protocols it speaks live on host:port targets), and only
/// for the protocols the negotiation implements.
fn validate_starttls(monitor: &Monitor) -> anyhow::Result<()> {
    if let Some(name) = &monitor.ehlo_name {
        anyhow::ensure!(
            monitor.starttls.as_deref() == Some("smtp"),
            "monitor {}: ehlo_name requires starttls = \"smtp\"",
            monitor.id
        );
        anyhow::ensure!(
            valid_ehlo_name(name),
            "monitor {}: ehlo_name must be a fully qualified domain or an address literal like [192.0.2.1]",
            monitor.id
        );
    }
    let Some(mode) = &monitor.starttls else {
        return Ok(());
    };
    anyhow::ensure!(
        monitor.kind == Kind::Tcp,
        "monitor {}: starttls requires a tcp monitor",
        monitor.id
    );
    anyhow::ensure!(
        matches!(mode.as_str(), "smtp" | "imap"),
        "monitor {}: starttls must be \"smtp\" or \"imap\"",
        monitor.id
    );
    Ok(())
}

/// RFC 5321 `Domain` (at least two LDH labels) or `address-literal`
/// (`[IPv4]` / `[IPv6:…]`). A bare single label is what MX servers reject.
fn valid_ehlo_name(name: &str) -> bool {
    if let Some(literal) = name.strip_prefix('[').and_then(|n| n.strip_suffix(']')) {
        return match literal.strip_prefix("IPv6:") {
            Some(v6) => v6.parse::<std::net::Ipv6Addr>().is_ok(),
            None => literal.parse::<std::net::Ipv4Addr>().is_ok(),
        };
    }
    let labels: Vec<&str> = name.trim_end_matches('.').split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// Validate the identity assertions: the certificate pin and the RDAP domain.
fn validate_pins(monitor: &Monitor) -> anyhow::Result<()> {
    if let Some(pin) = &monitor.cert_pin {
        anyhow::ensure!(
            pin.len() == 64 && pin.bytes().all(|b| b.is_ascii_hexdigit()),
            "monitor {}: cert_pin must be 64 hex chars (SHA-256 of the leaf public key)",
            monitor.id
        );
    }
    // A bare registrable name ("example.com"): an URL or a path would query
    // RDAP for garbage, and a name without a dot is a TLD, not a domain.
    if let Some(domain) = &monitor.domain_expiry {
        anyhow::ensure!(
            domain.contains('.')
                && !domain.contains('/')
                && !domain.contains(':')
                && !domain.contains(char::is_whitespace),
            "monitor {}: domain_expiry must be a bare domain name like \"example.com\"",
            monitor.id
        );
    }
    if let Some(release) = &monitor.release {
        validate_release(&monitor.id, release)?;
    }
    Ok(())
}

/// A release watch names a GitHub project as `owner/repo`, and says where the
/// running version comes from in exactly one way.
fn validate_release(id: &str, release: &ReleaseWatch) -> anyhow::Result<()> {
    let name = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    anyhow::ensure!(
        release
            .github
            .split_once('/')
            .is_some_and(|(owner, repo)| name(owner) && name(repo)),
        "monitor {id}: release.github must be \"owner/repo\""
    );
    anyhow::ensure!(
        release.current.is_some() != release.current_url.is_some(),
        "monitor {id}: release needs exactly one of `current` and `current_url`"
    );
    if let Some(current) = &release.current {
        anyhow::ensure!(
            !current.trim().is_empty(),
            "monitor {id}: release.current is empty"
        );
    }
    if let Some(url) = &release.current_url {
        anyhow::ensure!(
            url.starts_with("http://") || url.starts_with("https://"),
            "monitor {id}: release.current_url must be an http(s) URL"
        );
    }
    if let Some(query) = &release.current_query {
        anyhow::ensure!(
            release.current_url.is_some(),
            "monitor {id}: release.current_query needs release.current_url"
        );
        if let Some(err) = query.error() {
            anyhow::bail!("monitor {id}: invalid release.current_query: {err}");
        }
    }
    Ok(())
}

/// `dual_stack` needs a probe that can be steered per address family: an
/// active kind, no proxy in the way, and a hostname target (an IP literal has
/// a single family by construction). Runs after the per-kind target checks,
/// so the target is already known to be well-shaped.
fn validate_dual_stack(monitor: &Monitor) -> anyhow::Result<()> {
    anyhow::ensure!(
        matches!(monitor.kind, Kind::Http | Kind::Tcp | Kind::Icmp),
        "monitor {}: dual_stack requires an http, tcp or icmp monitor",
        monitor.id
    );
    anyhow::ensure!(
        monitor.proxy.is_none(),
        "monitor {}: dual_stack cannot go through a proxy (only the proxy hop's family would be tested)",
        monitor.id
    );
    let host = match monitor.kind {
        Kind::Http => reqwest::Url::parse(&monitor.target)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned)),
        Kind::Tcp => split_host_port(&monitor.target).map(|(host, _port)| host.to_owned()),
        _ => Some(monitor.target.clone()),
    };
    // URL hosts carry an IPv6 literal in brackets (split_host_port strips them).
    let host = host.unwrap_or_default();
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    anyhow::ensure!(
        bare.parse::<std::net::IpAddr>().is_err(),
        "monitor {}: dual_stack requires a hostname target (an IP literal has a single address family)",
        monitor.id
    );
    Ok(())
}

/// Validate the push `schedule`/`grace_secs` pair and the availability SLO
/// fields. The cron expression is parsed here so a typo fails at load, not at
/// the first missed heartbeat.
fn validate_schedule_and_slo(monitor: &Monitor) -> anyhow::Result<()> {
    if let Some(schedule) = &monitor.schedule {
        anyhow::ensure!(
            monitor.kind == Kind::Push,
            "monitor {}: schedule requires a push monitor",
            monitor.id
        );
        parse_cron(schedule).map_err(|err| {
            anyhow::anyhow!(
                "monitor {}: invalid schedule {schedule:?}: {err}",
                monitor.id
            )
        })?;
    }
    anyhow::ensure!(
        monitor.schedule.is_some() || monitor.grace_secs.is_none(),
        "monitor {}: grace_secs requires a schedule",
        monitor.id
    );
    if let Some(window) = monitor.slo_window_days {
        anyhow::ensure!(
            window > 0,
            "monitor {}: slo_window_days must be > 0",
            monitor.id
        );
        anyhow::ensure!(
            monitor.slo_uptime.is_some(),
            "monitor {}: slo_window_days requires slo_uptime",
            monitor.id
        );
    }
    Ok(())
}

/// The DNS-specific half of [`validate_monitor_io`]: target shape, record
/// type, resolver address.
fn validate_dns_io(monitor: &Monitor) -> anyhow::Result<()> {
    anyhow::ensure!(
        !monitor.target.is_empty()
            && !monitor.target.contains("://")
            && !monitor.target.chars().any(char::is_whitespace),
        "monitor {}: dns target must be a hostname (no scheme or whitespace)",
        monitor.id
    );
    if let Some(err) = monitor.dns_record.as_ref().and_then(Parsed::error) {
        anyhow::bail!("monitor {}: {err}", monitor.id);
    }
    if let Some(err) = monitor.dns_resolver.as_ref().and_then(Parsed::error) {
        anyhow::bail!("monitor {}: dns_resolver {err}", monitor.id);
    }
    Ok(())
}
