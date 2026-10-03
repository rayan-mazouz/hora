//! The response snapshot stored with a failure: status line, headers (secrets
//! masked) and the start of the body.

/// Maximum length (chars) of the response-body snippet kept on failure.
const MAX_BODY_SNIPPET: usize = 300;

/// Bounds of the failure snapshot ("what did the service actually answer?")
/// captured when an HTTP probe fails with a response: at most this many
/// headers, each line clipped, and this many chars of body. ~6 KiB worst case,
/// stored only on confirmed incidents.
pub(super) const MAX_SNAPSHOT_HEADERS: usize = 24;
pub(super) const MAX_SNAPSHOT_HEADER_CHARS: usize = 160;
pub(super) const MAX_SNAPSHOT_BODY_CHARS: usize = 2048;

/// The status line and (bounded) headers of a response, captured before the
/// body read consumes it.
pub(super) fn snapshot_head(response: &reqwest::Response) -> String {
    render_head(response.version(), response.status(), response.headers())
}

/// Format the status line and headers of the failure snapshot. Header values
/// are clipped per line and the count is capped, so a hostile response can't
/// bloat the incident record.
pub(super) fn render_head(
    version: reqwest::Version,
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
) -> String {
    let mut head = format!("{version:?} {status}");
    for (name, value) in headers.iter().take(MAX_SNAPSHOT_HEADERS) {
        let value = if is_sensitive_header(name) {
            "<redacted>"
        } else {
            value.to_str().unwrap_or("<binary>")
        };
        head.push('\n');
        head.extend(
            format!("{name}: {value}")
                .chars()
                .take(MAX_SNAPSHOT_HEADER_CHARS),
        );
    }
    let dropped = headers.len().saturating_sub(MAX_SNAPSHOT_HEADERS);
    if dropped > 0 {
        let _ = std::fmt::Write::write_fmt(&mut head, format_args!("\n({dropped} more headers)"));
    }
    head
}

/// Response headers whose value is a credential - a session cookie issued by
/// the monitored app, an auth challenge or token echoed back. The snapshot is
/// stored on the incident, shown on `/history` and pasted into post-mortems,
/// so these keep their name (the fact is useful) and lose their value.
fn is_sensitive_header(name: &reqwest::header::HeaderName) -> bool {
    const SENSITIVE: &[&str] = &[
        "set-cookie",
        "cookie",
        "authorization",
        "proxy-authorization",
        "www-authenticate",
        "proxy-authenticate",
        "x-amz-security-token",
        "x-api-key",
        "x-auth-token",
        "x-csrf-token",
        "x-xsrf-token",
    ];
    // `HeaderName` is always lowercase, so a plain comparison suffices.
    SENSITIVE.contains(&name.as_str())
}

/// Assemble the stored failure snapshot: status line and headers, a blank
/// line, then the start of the body (lossy UTF-8, bounded in chars).
pub(super) fn render_snapshot(head: &str, body: &[u8]) -> String {
    let text: String = String::from_utf8_lossy(body)
        .chars()
        .take(MAX_SNAPSHOT_BODY_CHARS)
        .collect();
    if text.trim().is_empty() {
        head.to_owned()
    } else {
        format!("{head}\n\n{text}")
    }
}

/// Collapse a byte body into a bounded, single-line snippet for failure detail.
pub(super) fn snippet(body: &[u8]) -> String {
    // Fold the whitespace-separated words straight into one string, so there is
    // no intermediate `Vec<&str>` just to `join` it.
    String::from_utf8_lossy(body)
        .split_whitespace()
        .fold(String::new(), |mut acc, word| {
            if !acc.is_empty() {
                acc.push(' ');
            }
            acc.push_str(word);
            acc
        })
        .chars()
        .take(MAX_BODY_SNIPPET)
        .collect()
}
