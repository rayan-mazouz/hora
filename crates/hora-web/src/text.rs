//! Plain-text status rendering for curl and other text-based clients.

use std::fmt::Write as _;

use crate::summary::Summary;

pub(crate) fn render(summary: &Summary) -> String {
    let mut out = String::with_capacity(2048);
    let _ = writeln!(out, "{}", summary.title);
    // Bytes would over-shoot on accented titles; chars is close enough.
    let _ = writeln!(out, "{}", "=".repeat(summary.title.chars().count()));
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Overall: {} ({})",
        summary.overall, summary.overall_label
    );
    let _ = writeln!(out, "Updated: {}", summary.updated_utc);
    let _ = writeln!(out);

    if !summary.incidents.is_empty() {
        let _ = writeln!(out, "Incidents:");
        for incident in &summary.incidents {
            let _ = writeln!(
                out,
                "  [{}] {}",
                incident.severity.to_uppercase(),
                incident.title
            );
            if !incident.body.is_empty() {
                let _ = writeln!(out, "    {}", incident.body);
            }
        }
        let _ = writeln!(out);
    }

    if !summary.maintenances.is_empty() {
        let _ = writeln!(out, "Maintenance:");
        for m in &summary.maintenances {
            let _ = writeln!(out, "  {} ({})", m.reason, m.monitors);
        }
        let _ = writeln!(out);
    }

    if summary.monitors.is_empty() {
        let _ = writeln!(out, "No monitors configured yet.");
    } else {
        for group in &summary.groups {
            if !group.name.is_empty() {
                let _ = writeln!(out, "{}", group.name);
                let _ = writeln!(out, "{}", "-".repeat(group.name.chars().count()));
            }

            // Pad names to the group's longest so the figures line up
            // (chars, not bytes: names may be accented).
            let width = group
                .monitors
                .iter()
                .map(|monitor| monitor.name.chars().count())
                .max()
                .unwrap_or(0);
            for monitor in &group.monitors {
                let uptime = monitor
                    .uptime_permille
                    .map_or_else(|| "-".to_owned(), hora_core::fmt::permille);
                let latency = monitor
                    .last_latency_ms
                    .map_or_else(|| "-".to_owned(), |ms| format!("{ms}ms"));

                let _ = writeln!(
                    out,
                    "  {} {:<width$}  {:>8}  {:>8}",
                    status_symbol(monitor.status),
                    monitor.name,
                    uptime,
                    latency
                );

                if let Some(cause) = &monitor.cause {
                    let _ = writeln!(out, "      caused by {cause}");
                }
                if !monitor.impacted.is_empty() {
                    let _ = writeln!(out, "      impacts: {}", monitor.impacted.join(", "));
                }
            }
            let _ = writeln!(out);
        }
    }

    if !summary.peers.is_empty() {
        let _ = writeln!(out, "Peers");
        let _ = writeln!(out, "-----");
        for peer in &summary.peers {
            let _ = writeln!(out, "  {} {}", status_symbol(peer.status), peer.name);
        }
    }

    out
}

/// The one-glyph status marker of a monitor or peer line.
fn status_symbol(status: &str) -> &'static str {
    match status {
        "up" => "●",
        "degraded" => "◐",
        "down" => "○",
        _ => "?",
    }
}
