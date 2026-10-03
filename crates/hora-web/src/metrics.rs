//! Prometheus `/metrics` endpoint: renders the summary as Prometheus text
//! exposition format (version 0.0.4).

use std::fmt::Write as _;

use crate::summary::Summary;

pub(crate) fn render(summary: &Summary) -> String {
    let mut out = String::with_capacity(4096);
    render_status(&mut out, summary);

    push_header(
        &mut out,
        "hora_monitor_uptime_ratio",
        "Uptime over the last 24 hours (0 to 1)",
        "gauge",
    );
    for monitor in &summary.monitors {
        if let Some(permille) = monitor.uptime_permille {
            // Permille is 0..=1000 by construction; clamp rather than cast.
            let ratio = f64::from(u16::try_from(permille).unwrap_or(1000)) / 1000.0;
            let _ = writeln!(
                out,
                "hora_monitor_uptime_ratio{} {ratio:.3}",
                labels(monitor)
            );
        }
    }

    push_header(
        &mut out,
        "hora_monitor_last_latency_ms",
        "Latency of the most recent check in milliseconds",
        "gauge",
    );
    for monitor in &summary.monitors {
        if let Some(ms) = monitor.last_latency_ms {
            let _ = writeln!(out, "hora_monitor_last_latency_ms{} {ms}", labels(monitor));
        }
    }

    push_header(
        &mut out,
        "hora_monitor_latency_ms",
        "Latency quantiles over the last 24 hours in milliseconds",
        // Not a Prometheus `summary` (no `_sum`/`_count`): plain gauges that
        // carry a `quantile` label.
        "gauge",
    );
    for monitor in &summary.monitors {
        for (quantile, value) in [
            ("0.5", monitor.p50_ms),
            ("0.95", monitor.p95_ms),
            ("0.99", monitor.p99_ms),
        ] {
            if let Some(ms) = value {
                let _ = writeln!(
                    out,
                    "hora_monitor_latency_ms{{id=\"{}\",name=\"{}\",quantile=\"{quantile}\"}} {ms}",
                    escape_label(&monitor.id),
                    escape_label(&monitor.name),
                );
            }
        }
    }

    push_header(
        &mut out,
        "hora_cert_expiry_days",
        "Days until the TLS certificate expires",
        "gauge",
    );
    for monitor in &summary.monitors {
        if let Some(days) = monitor.cert_days {
            let _ = writeln!(out, "hora_cert_expiry_days{} {days}", labels(monitor));
        }
    }

    out
}

/// The status families: `hora_monitor_status` for every monitor, then
/// `hora_monitor_up` / `hora_monitor_degraded` for those with a verdict.
fn render_status(out: &mut String, summary: &Summary) {
    // A monitor without a verdict yet (just added, or just restarted) has no
    // up/degraded sample at all: a 0 would be indistinguishable from down and
    // fire `hora_monitor_up == 0` alerts on every deploy. Its state is still
    // visible through `hora_monitor_status`.
    let known = || {
        summary
            .monitors
            .iter()
            .filter(|monitor| monitor.status != "unknown")
    };

    push_header(
        out,
        "hora_monitor_status",
        "Current status of the monitor (1 for its status: up, degraded, down or unknown)",
        "gauge",
    );
    for monitor in &summary.monitors {
        let _ = writeln!(
            out,
            "hora_monitor_status{{id=\"{}\",name=\"{}\",status=\"{}\"}} 1",
            escape_label(&monitor.id),
            escape_label(&monitor.name),
            monitor.status,
        );
    }

    push_header(
        out,
        "hora_monitor_up",
        "Whether the monitor is up (degraded still counts as up); absent while unknown",
        "gauge",
    );
    for monitor in known() {
        let up = u8::from(monitor.status == "up" || monitor.status == "degraded");
        let _ = writeln!(out, "hora_monitor_up{} {up}", labels(monitor));
    }

    push_header(
        out,
        "hora_monitor_degraded",
        "Whether the monitor is up but slower than its degraded threshold",
        "gauge",
    );
    for monitor in known() {
        let degraded = u8::from(monitor.status == "degraded");
        let _ = writeln!(out, "hora_monitor_degraded{} {degraded}", labels(monitor));
    }
}

fn push_header(out: &mut String, name: &str, help: &str, kind: &str) {
    let _ = writeln!(out, "# HELP {name} {help}");
    let _ = writeln!(out, "# TYPE {name} {kind}");
}

fn labels(monitor: &crate::summary::MonitorView) -> String {
    format!(
        "{{id=\"{}\",name=\"{}\"}}",
        escape_label(&monitor.id),
        escape_label(&monitor.name)
    )
}

/// Escape a label value per the Prometheus exposition format: backslash,
/// double quote and newline.
fn escape_label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_label_values() {
        assert_eq!(escape_label(r#"a"b"#), r#"a\"b"#);
        assert_eq!(escape_label("a\\b"), "a\\\\b");
        assert_eq!(escape_label("a\nb"), "a\\nb");
        assert_eq!(escape_label("plain"), "plain");
    }
}
