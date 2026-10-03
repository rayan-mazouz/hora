//! The public-visibility policy for failure reasons.

use crate::config::DnsRecord;

/// The public form of a stored failure reason. Stored reasons keep full
/// operator detail - response-body snippets, DNS answers, asserted keywords,
/// raw socket errors - which anonymous viewers of a public monitor must not
/// see: snippets and DNS answers are remote-controlled, keywords and JSON
/// queries reveal operator config. Known-safe reasons pass through verbatim;
/// detailed ones collapse to their category; anything unrecognized (raw TCP
/// errors, free-form push messages, legacy rows) falls back to "check failed".
///
/// Must track the reasons produced by the probes (the sibling modules) and the
/// scheduler's heartbeat misses; an unmatched new reason degrades safely to the
/// fallback.
#[must_use]
pub fn public_reason(reason: &str) -> &str {
    // Static reasons carry no detail; they are their own category.
    const VERBATIM: &[&str] = &[
        "request timed out",
        "connection failed",
        "connection timed out",
        "too many redirects",
        "invalid response body",
        "request error",
        "could not resolve host",
        "DNS lookup timed out",
        "missing heartbeat",
    ];
    // "HTTP 500: <body snippet>" keeps the status, drops the snippet. Anchored
    // to a numeric status: `checks.error` also stores free-form push messages,
    // so a bare "HTTP <anything>" prefix must not become a passthrough that
    // lets a push-token holder place arbitrary text on the public page.
    if let Some(rest) = reason.strip_prefix("HTTP ") {
        let code = rest.find(':').map_or(rest, |colon| &rest[..colon]);
        if code.parse::<u16>().is_ok() {
            return &reason[..5 + code.len()];
        }
    }
    if VERBATIM.contains(&reason) {
        return reason;
    }
    // Same anchoring: only the record types the DNS probe actually queries.
    if let Some(record) = reason
        .strip_prefix("no ")
        .and_then(|rest| rest.strip_suffix(" records found"))
        && DnsRecord::ALL.iter().any(|known| known.as_str() == record)
    {
        return reason;
    }
    // "missed scheduled heartbeat (was due 03:00 UTC + 30m grace)" reveals the
    // job's schedule; keep the category only.
    if reason.starts_with("missed scheduled heartbeat") {
        return "missed scheduled heartbeat";
    }
    // Keyword, JSON and number assertions embed the configured keyword/query/
    // regex - and the number reasons additionally carry the extracted value.
    if reason.starts_with("keyword ")
        || reason.starts_with("JSON query")
        || reason.starts_with("invalid JSON query")
        || reason.starts_with("number")
        || reason.starts_with("invalid number_regex")
        || reason == "response is not valid JSON"
    {
        return "content check failed";
    }
    // The DNS pin mismatch embeds the remote-controlled answer.
    if reason.starts_with("expected ") {
        return "unexpected DNS answer";
    }
    if reason.starts_with("DNS lookup failed") || reason.starts_with("resolver setup failed") {
        return "DNS lookup failed";
    }
    if reason.starts_with("icmp") {
        return "ping failed";
    }
    // Exec plugins: their output is operator infrastructure detail (paths,
    // device names, container ids); anonymous viewers get the category only.
    if reason.starts_with("plugin ") || reason.starts_with("exec ") {
        return "plugin check failed";
    }
    // Dual-stack failures embed the failing family's detail (which may be a raw
    // socket error); keep only which family broke. Checked longest-prefix first.
    if reason.starts_with("IPv4 and IPv6 failing") {
        return "IPv4 and IPv6 failing";
    }
    if reason.starts_with("IPv4 failing") {
        return "IPv4 failing (IPv6 ok)";
    }
    if reason.starts_with("IPv6 failing") {
        return "IPv6 failing (IPv4 ok)";
    }
    "check failed"
}
