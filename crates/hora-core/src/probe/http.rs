//! HTTP(S) checks: status, redirects, latency and body assertions.

use crate::status::CheckStatus;
use std::collections::HashMap;
use std::time::Instant;

use reqwest::{Client, RequestBuilder};

use super::snapshot::{MAX_SNAPSHOT_BODY_CHARS, render_snapshot, snapshot_head, snippet};
use super::{Outcome, millis, over_threshold};
use crate::config::{Monitor, Parsed, Secret};

pub(super) async fn http(client: &Client, monitor: &Monitor) -> Outcome {
    let start = Instant::now();
    // One deadline for the whole request - every redirect hop and the body read -
    // so a chain of slow redirects can't outlive the monitor's timeout. Latency is
    // taken when the final response's headers arrive, before the body read, to
    // match the single-request timing this replaced.
    let attempt = async {
        let mut response = send_following_redirects(client, monitor).await?;
        let latency = millis(start.elapsed());
        let code = response.status().as_u16();
        let status_ok = match monitor.expected_status {
            Some(expected) => code == expected,
            None => response.status().is_success(),
        };
        // Read the body only when we need it: to detail a failure, or to run
        // a keyword/JSON assertion. Assertions get a larger budget. The head
        // (status line + headers) is captured first - reading the body
        // consumes the response - in case this turns into a failure snapshot.
        let assertions = monitor.keyword.is_some()
            || monitor.json_query.is_some()
            || monitor.number_regex.is_some();
        let (head, body) = if !status_ok || assertions {
            let head = snapshot_head(&response);
            let cap = if assertions {
                monitor.assertion_body_cap()
            } else {
                MAX_SNAPSHOT_BODY_CHARS
            };
            // A body cut short by a transport error is reported as such:
            // running the assertions on the fragment would blame the content
            // ("keyword missing") for what is a connection problem.
            // Past the cap the rest is dropped unread: a snapshot or an
            // assertion only ever looks at the first `cap` bytes.
            let body = crate::http::read_capped(&mut response, cap)
                .await
                .map_err(HttpError::Request)?;
            (head, body.bytes)
        } else {
            (String::new(), Vec::new())
        };
        Ok::<_, HttpError>((code, status_ok, head, body, latency))
    };

    match tokio::time::timeout(monitor.timeout(), attempt).await {
        Ok(Ok((code, status_ok, head, body, latency))) => {
            let (up, error) = if !status_ok {
                let snippet = snippet(&body);
                let detail = if snippet.is_empty() {
                    format!("HTTP {code}")
                } else {
                    format!("HTTP {code}: {snippet}")
                };
                (false, Some(detail))
            } else if let Some(failure) = check_assertions(monitor, &body) {
                (false, Some(failure))
            } else {
                (true, None)
            };

            let status = if up {
                CheckStatus::up_unless(over_threshold(latency, monitor.degraded_over_ms))
            } else {
                CheckStatus::Down
            };
            Outcome {
                status,
                latency_ms: Some(latency),
                status_code: Some(i64::from(code)),
                error,
                // The service answered something and the check failed: keep
                // what it answered for the incident record.
                snapshot: (!up).then(|| render_snapshot(&head, &body)),
            }
        }
        Ok(Err(HttpError::TooManyRedirects)) => Outcome::down("too many redirects".to_owned()),
        Ok(Err(HttpError::Request(err))) => Outcome::down(describe(&err).to_owned()),
        Err(_elapsed) => Outcome::down("request timed out".to_owned()),
    }
}

/// reqwest's default redirect ceiling; we follow them ourselves (below) and keep
/// the same bound.
const MAX_REDIRECTS: usize = 10;

/// Either a transport error or our own redirect-budget exhaustion. The probe
/// client never auto-follows, so reqwest can't produce a "too many redirects"
/// error of its own.
enum HttpError {
    Request(reqwest::Error),
    TooManyRedirects,
}

/// Send the monitor's GET, following redirects manually so the configured
/// headers - which may carry credentials (an API key, a bearer token) - are
/// re-attached only while the redirect stays on the original origin. reqwest
/// strips its own well-known sensitive headers across hosts but not arbitrary
/// custom ones, so without this a malicious or compromised target could 30x us
/// to an attacker host and harvest the header. Cross-origin hops are still
/// followed, just without the headers.
async fn send_following_redirects(
    client: &Client,
    monitor: &Monitor,
) -> Result<reqwest::Response, HttpError> {
    let Ok(target) = reqwest::Url::parse(&monitor.target) else {
        // Config validation rejects non-URL http targets; degrade gracefully by
        // letting reqwest surface the error on send.
        return client
            .get(&monitor.target)
            .timeout(monitor.timeout())
            .send()
            .await
            .map_err(HttpError::Request);
    };
    // A monitor expecting a 3xx asserts the redirect itself (HTTP -> HTTPS,
    // apex -> www): following it would judge the destination instead, and the
    // probe could never pass.
    let follow = !monitor
        .expected_status
        .is_some_and(|status| (300..400).contains(&status));
    let mut url = target.clone();
    for _ in 0..=MAX_REDIRECTS {
        // The per-request timeout overrides the client's 15s notifier backstop,
        // which may be *shorter* than the monitor's own; the caller's outer
        // deadline still bounds the whole chain.
        let mut request = client.get(url.clone()).timeout(monitor.timeout());
        if same_origin(&target, &url) {
            request = with_headers(request, &monitor.headers);
        }
        let response = request.send().await.map_err(HttpError::Request)?;
        if !follow || !response.status().is_redirection() {
            return Ok(response);
        }
        // A 3xx without a usable Location (or 304 Not Modified) is the final
        // response; hand it back rather than chase nothing.
        let next = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|location| url.join(location).ok());
        match next {
            Some(next) => url = next,
            None => return Ok(response),
        }
    }
    Err(HttpError::TooManyRedirects)
}

/// Whether `url` shares an origin (scheme + host + port) with the original
/// `target` - the rule that decides whether the monitor's (possibly
/// credential-bearing) headers are re-attached across a redirect.
pub(super) fn same_origin(target: &reqwest::Url, url: &reqwest::Url) -> bool {
    target.origin() == url.origin()
}

/// Run the configured keyword/JSON assertions against the body; the first that
/// fails returns its reason. `None` means every assertion passed.
pub(super) fn check_assertions(monitor: &Monitor, body: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(body);
    if let Some(keyword) = &monitor.keyword {
        let found = text.contains(keyword.as_str());
        if found == monitor.keyword_invert {
            return Some(if monitor.keyword_invert {
                format!("keyword present: {keyword}")
            } else {
                format!("keyword missing: {keyword}")
            });
        }
    }
    if let Some(query) = &monitor.json_query
        && let Some(failure) = check_json(query, monitor.json_expected.as_deref(), &text)
    {
        return Some(failure);
    }
    if let Some(pattern) = &monitor.number_regex {
        return check_number(pattern, monitor.number_min, monitor.number_max, &text);
    }
    None
}

/// Extract a number from the body via the configured regex (first capture
/// group, or the whole match for a group-less pattern) and check it against
/// the optional inclusive bounds. Returns a failure reason or `None`.
#[expect(
    clippy::cast_precision_loss,
    reason = "config bounds are human-scale thresholds, far below 2^52"
)]
pub(super) fn check_number(
    pattern: &Parsed<regex::Regex>,
    min: Option<i64>,
    max: Option<i64>,
    body: &str,
) -> Option<String> {
    // Compiled at config load, where validation rejects a bad pattern.
    let Some(regex) = pattern.get() else {
        return Some(format!("invalid number_regex: {}", pattern.raw()));
    };
    let pattern = pattern.raw();
    let Some(captures) = regex.captures(body) else {
        return Some(format!("number_regex matched nothing: {pattern}"));
    };
    let matched = captures
        .get(1)
        .or_else(|| captures.get(0))
        .map_or("", |capture| capture.as_str());
    let Ok(value) = matched.trim().parse::<f64>() else {
        return Some(format!("number_regex match is not a number: {matched}"));
    };
    if let Some(min) = min
        && value < min as f64
    {
        return Some(format!("number {value} below min {min}"));
    }
    if let Some(max) = max
        && value > max as f64
    {
        return Some(format!("number {value} above max {max}"));
    }
    None
}

/// Evaluate a `JSONPath` against the body. Returns a failure reason or `None`.
pub(super) fn check_json(
    query: &Parsed<serde_json_path::JsonPath>,
    expected: Option<&str>,
    body: &str,
) -> Option<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Some("response is not valid JSON".to_owned());
    };
    // Parsed at config load, where validation rejects a bad query.
    let Some(path) = query.get() else {
        return Some(format!("invalid JSON query: {}", query.raw()));
    };
    let query = query.raw();
    let nodes = path.query(&value).all();
    match expected {
        None => nodes
            .is_empty()
            .then(|| format!("JSON query matched nothing: {query}")),
        Some(expected) => {
            let matched = nodes.iter().any(|node| json_value_eq(node, expected));
            (!matched).then(|| format!("JSON query {query} != {expected}"))
        }
    }
}

/// Compare a queried JSON node to an expected string: strings match their inner
/// value, everything else matches its compact JSON text (`true`, `42`, …).
fn json_value_eq(value: &serde_json::Value, expected: &str) -> bool {
    match value {
        serde_json::Value::String(text) => text == expected,
        other => {
            let rendered = other.to_string();
            rendered == expected
        }
    }
}

/// Apply every configured header to the request. reqwest *appends* headers, so
/// each distinct header is kept - none overwrites another.
pub(super) fn with_headers(
    mut request: RequestBuilder,
    headers: &HashMap<String, Secret>,
) -> RequestBuilder {
    for (name, value) in headers {
        request = request.header(name, value.as_ref());
    }
    request
}

/// A concise, URL-free description of a request error. The raw error embeds the
/// target URL (which may carry credentials), so we categorize instead.
fn describe(err: &reqwest::Error) -> &'static str {
    if err.is_timeout() {
        "request timed out"
    } else if err.is_connect() {
        "connection failed"
    } else if err.is_redirect() {
        "too many redirects"
    } else if err.is_body() || err.is_decode() {
        "invalid response body"
    } else {
        "request error"
    }
}
