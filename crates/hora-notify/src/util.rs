//! Shared helpers for the built-in notifiers.

use reqwest::{Client, RequestBuilder};
use serde::Serialize;
use tracing::warn;

/// Escape the characters special to HTML / Slack mrkdwn (`& < >`).
pub(crate) fn escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// `"2d 3h"`, `"6h"`, `"45m"`, `"30s"` - coarse on purpose: it phrases a burn
/// estimate and how long a channel has been failing, not a measurement.
pub(crate) fn human_duration(secs: u64) -> String {
    if secs >= 2 * 86_400 {
        format!("{}d {}h", secs / 86_400, (secs % 86_400) / 3600)
    } else if secs >= 3600 {
        format!("{}h", secs / 3600)
    } else if secs >= 60 {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

/// What a channel's length limit counts: characters (most APIs) or UTF-8
/// bytes (ntfy, which turns a longer body into an attachment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unit {
    Chars,
    Bytes,
}

/// The length of `text` in `unit`.
pub(crate) fn measure(text: &str, unit: Unit) -> usize {
    match unit {
        Unit::Chars => text.chars().count(),
        Unit::Bytes => text.len(),
    }
}

const ELLIPSIS: char = '…';

/// `text` cut to at most `max` (in `unit`) on a char boundary, ending with an
/// ellipsis when anything was cut so the reader knows there was more. Never
/// splits a multi-byte character; a budget too small for the ellipsis itself
/// yields a bare cut (or nothing at all for `max == 0`).
pub(crate) fn truncate(text: &str, max: usize, unit: Unit) -> String {
    if measure(text, unit) <= max {
        return text.to_owned();
    }
    let ellipsis = match unit {
        Unit::Chars => 1,
        Unit::Bytes => ELLIPSIS.len_utf8(),
    };
    let (room, mark) = if max >= ellipsis {
        (max - ellipsis, true)
    } else {
        (max, false)
    };
    let mut used = 0;
    let mut out = String::with_capacity(room.min(text.len()) + 3);
    for ch in text.chars() {
        let cost = match unit {
            Unit::Chars => 1,
            Unit::Bytes => ch.len_utf8(),
        };
        if used + cost > room {
            break;
        }
        used += cost;
        out.push(ch);
    }
    if mark {
        out.push(ELLIPSIS);
    }
    out
}

/// Marker error for a notification the API refused as too large (HTTP 413).
/// The channel itself answered, so the dispatcher does not count it toward the
/// channel's failure streak - one oversized alert must not make the watchdog
/// report a healthy channel as broken. Still logged and returned as a delivery
/// failure. A 400 is deliberately *not* treated this way: messages are already
/// cut to each API's limits, and the 400s that remain are misconfigurations
/// (Telegram "chat not found", a Pushover user key) the watchdog must report.
#[derive(Debug)]
pub(crate) struct PayloadRejected(pub String);

impl std::fmt::Display for PayloadRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PayloadRejected {}

/// Delivery attempts: the initial send plus two retries. The caller marks the
/// alert as sent regardless of the outcome, so a transient blip here would
/// otherwise silently drop the notification.
const MAX_ATTEMPTS: u32 = 3;

/// Send a request and log (never panic) on failure. `build` produces a fresh
/// request on each attempt, so it can be retried; every `secrets` entry is
/// stripped from any logged text so a token in the URL, a header or a query
/// never reaches the logs. On rejection a bounded snippet of the response body
/// is logged (it usually says *why*, e.g. "chat not found").
///
/// Transient failures (a network error, an HTTP 5xx, or 429) are retried with a
/// short backoff. Client errors (4xx other than 429) are permanent, so they are
/// reported immediately without retrying. Every notifier goes through here, so
/// they all share one retry and redaction policy.
/// The failure is both logged here (one policy for the daemon, which fires and
/// forgets) and returned, so a caller that *does* care - `hora test-alert`'s
/// exit code - can tell that delivery failed without re-deriving the detail.
pub(crate) async fn send_retrying<F>(
    build: F,
    channel: &str,
    secrets: &[&str],
) -> anyhow::Result<()>
where
    F: Fn() -> RequestBuilder,
{
    for attempt in 1..=MAX_ATTEMPTS {
        match build().send().await {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(response) => {
                let status = response.status();
                let transient =
                    status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS;
                if transient && attempt < MAX_ATTEMPTS {
                    backoff(attempt).await;
                    continue;
                }
                let body = response.text().await.unwrap_or_default();
                let detail = redact(&snippet(&body), secrets);
                warn!("{channel} rejected the notification (HTTP {status}): {detail}");
                let message = format!("rejected (HTTP {status}): {detail}");
                if payload_rejected(status) {
                    return Err(PayloadRejected(message).into());
                }
                anyhow::bail!(message);
            }
            Err(err) => {
                if attempt < MAX_ATTEMPTS {
                    backoff(attempt).await;
                    continue;
                }
                let detail = redact(&err.to_string(), secrets);
                warn!("{channel} request failed after {attempt} attempts: {detail}");
                anyhow::bail!("request failed after {attempt} attempts: {detail}");
            }
        }
    }
    unreachable!("every iteration either returns, bails or continues within MAX_ATTEMPTS")
}

/// Whether `status` means "this request's content was too large" (see
/// [`PayloadRejected`]) rather than "the channel is broken" (400/401/403/404:
/// bad chat id, revoked token, deleted webhook - those must trip the watchdog).
fn payload_rejected(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::PAYLOAD_TOO_LARGE
}

/// POST `payload` as JSON, retrying transient failures (see [`send_retrying`]).
pub(crate) async fn post_json<T: Serialize>(
    client: &Client,
    url: &str,
    payload: &T,
    channel: &str,
    secrets: &[&str],
) -> anyhow::Result<()> {
    send_retrying(|| client.post(url).json(payload), channel, secrets).await
}

/// Exponential backoff between delivery attempts: 200ms, then 400ms. Kept short
/// so a genuine outage alert is still delivered promptly.
async fn backoff(attempt: u32) {
    let ms = 200_u64 * 2_u64.pow(attempt - 1);
    tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
}

/// Strip every secret from `text` - raw, and in the two percent-encoded forms
/// a URL serializer may have written it in (strict RFC 3986 `%XX`, and
/// form-urlencoding where space is `+`), since an error that echoes a URL
/// echoes the *encoded* secret, not the one the operator typed. Empty secrets
/// are skipped (a `str::replace("", …)` would otherwise splice the replacement
/// between every character).
fn redact(text: &str, secrets: &[&str]) -> String {
    let mut out = text.to_owned();
    for secret in secrets {
        if secret.is_empty() {
            continue;
        }
        out = out.replace(secret, "<redacted>");
        for encoded in [percent_encode(secret, false), percent_encode(secret, true)] {
            if encoded != *secret {
                out = out.replace(&encoded, "<redacted>");
            }
        }
    }
    out
}

/// Percent-encode `value` byte-wise: every byte outside the unreserved set
/// becomes `%XX` (uppercase hex, as URL serializers emit). `form` selects
/// `application/x-www-form-urlencoded` (reqwest's query/form serializer):
/// space is `+` and `*` stays bare; the strict RFC 3986 variant keeps `~` bare
/// instead.
fn percent_encode(value: &str, form: bool) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(value.len());
    for &byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' => out.push(byte as char),
            b'~' if !form => out.push('~'),
            b'*' if form => out.push('*'),
            b' ' if form => out.push('+'),
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

/// A bounded, single-line snippet of a response body for log output.
fn snippet(body: &str) -> String {
    // Fold straight into one string: no intermediate `Vec<&str>` just to `join`.
    body.split_whitespace()
        .fold(String::new(), |mut acc, word| {
            if !acc.is_empty() {
                acc.push(' ');
            }
            acc.push_str(word);
            acc
        })
        .chars()
        .take(200)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_metacharacters() {
        assert_eq!(escape("a<b>&c"), "a&lt;b&gt;&amp;c");
    }

    #[test]
    fn human_duration_formats() {
        assert_eq!(human_duration(30), "30s");
        assert_eq!(human_duration(90), "1m");
        assert_eq!(human_duration(3600), "1h");
        assert_eq!(human_duration(9000), "2h");
        assert_eq!(human_duration(200_000), "2d 7h");
    }

    #[test]
    fn truncate_counts_chars_and_marks_the_cut() {
        assert_eq!(truncate("hello", 5, Unit::Chars), "hello");
        assert_eq!(truncate("hello", 4, Unit::Chars), "hel…");
        // Multi-byte characters count once and are never split.
        assert_eq!(truncate("héllo wörld", 4, Unit::Chars), "hél…");
        assert_eq!(truncate("hello", 1, Unit::Chars), "…");
        assert_eq!(truncate("hello", 0, Unit::Chars), "");
    }

    #[test]
    fn truncate_by_bytes_stays_on_a_char_boundary() {
        // "€" is 3 bytes; the ellipsis is 3 bytes too.
        let cut = truncate("€€€€", 8, Unit::Bytes);
        assert_eq!(cut, "€…");
        assert!(cut.len() <= 8);
        let cut = truncate("€€€€", 7, Unit::Bytes);
        assert_eq!(cut, "€…");
        // Too small for the ellipsis: a bare cut, still on a boundary.
        assert_eq!(truncate("ab€", 2, Unit::Bytes), "ab");
        assert_eq!(truncate("€€", 2, Unit::Bytes), "");
    }

    #[test]
    fn payload_statuses_are_classified() {
        assert!(payload_rejected(reqwest::StatusCode::PAYLOAD_TOO_LARGE));
        assert!(!payload_rejected(reqwest::StatusCode::BAD_REQUEST));
        // A revoked token or deleted webhook is a broken channel, not a bad payload.
        assert!(!payload_rejected(reqwest::StatusCode::UNAUTHORIZED));
        assert!(!payload_rejected(reqwest::StatusCode::FORBIDDEN));
        assert!(!payload_rejected(reqwest::StatusCode::NOT_FOUND));
    }

    #[test]
    fn redact_strips_secrets_and_handles_empty() {
        assert_eq!(
            redact("token abc123 failed", &["abc123"]),
            "token <redacted> failed"
        );
        // An empty secret must not splice <redacted> between every character.
        assert_eq!(redact("hello", &[""]), "hello");
        // Every secret in the set is stripped, not just the first.
        assert_eq!(
            redact("url https://h/t1 token t0k3n", &["https://h/t1", "t0k3n"]),
            "url <redacted> token <redacted>"
        );
    }

    #[test]
    fn redact_strips_encoded_forms() {
        // A secret echoed back inside a URL appears percent-encoded.
        assert_eq!(
            redact("GET /send?pass=p%40ss%3Aword failed", &["p@ss:word"]),
            "GET /send?pass=<redacted> failed"
        );
        // Form-urlencoding writes a space as `+`.
        assert_eq!(
            redact("pass=my+pass rejected", &["my pass"]),
            "pass=<redacted> rejected"
        );
        // And as %20 in strict RFC 3986 contexts (URL paths).
        assert_eq!(redact("at /my%20pass/x", &["my pass"]), "at /<redacted>/x");
    }
}
