use std::collections::HashMap;
use std::net::SocketAddr;

use super::dns::{record_type, resolver_for};
use super::family::combine;
use super::http::{check_assertions, check_json, check_number, same_origin, with_headers};
use super::snapshot::{
    MAX_SNAPSHOT_BODY_CHARS, MAX_SNAPSHOT_HEADER_CHARS, MAX_SNAPSHOT_HEADERS, render_head,
    render_snapshot,
};
use super::*;
use crate::config::{DnsRecord, Parsed, Secret};

#[test]
fn public_reason_collapses_detail() {
    // Remote-controlled or config-revealing detail is dropped.
    assert_eq!(public_reason("HTTP 500: secret stack trace"), "HTTP 500");
    assert_eq!(
        public_reason("expected 1.2.3.4, got 6.6.6.6"),
        "unexpected DNS answer"
    );
    assert_eq!(
        public_reason("keyword missing: internal-marker"),
        "content check failed"
    );
    assert_eq!(
        public_reason("JSON query $.status != ok"),
        "content check failed"
    );
    assert_eq!(
        public_reason("number 0 below min 1"),
        "content check failed"
    );
    assert_eq!(
        public_reason("number_regex matched nothing: ships (\\d+)"),
        "content check failed"
    );
    assert_eq!(
        public_reason("missed scheduled heartbeat (was due 03:00 UTC + 30m grace)"),
        "missed scheduled heartbeat"
    );
    assert_eq!(
        public_reason("DNS lookup failed: proto error: io error"),
        "DNS lookup failed"
    );
    // Unknown text (raw TCP errors, push messages) falls back to generic.
    assert_eq!(
        public_reason("Connection refused (os error 61)"),
        "check failed"
    );
    // Plugin output (device names, container ids) collapses to a category.
    assert_eq!(
        public_reason("plugin timed out after 30s"),
        "plugin check failed"
    );
    assert_eq!(
        public_reason("plugin reported critical (exit 2)"),
        "plugin check failed"
    );
    // Safe statics pass through verbatim.
    assert_eq!(public_reason("HTTP 503"), "HTTP 503");
    assert_eq!(public_reason("request timed out"), "request timed out");
    assert_eq!(public_reason("no A records found"), "no A records found");
    // A push msg crafted to look like a safe shape must not pass through:
    // the HTTP branch is anchored to a numeric status, the DNS one to the
    // record types the probe queries.
    assert_eq!(
        public_reason("HTTP looks legit but is a push msg"),
        "check failed"
    );
    assert_eq!(public_reason("HTTP 99999: nope"), "check failed");
    assert_eq!(
        public_reason("no big deal, just injected records found"),
        "check failed"
    );
}

#[test]
fn same_origin_governs_header_forwarding() {
    let parse = |u: &str| reqwest::Url::parse(u).unwrap();
    let target = parse("https://api.example.com/v1");

    // Same host, scheme and port (path differs): headers stay attached.
    assert!(same_origin(
        &target,
        &parse("https://api.example.com/login")
    ));
    // Different host, scheme or port: headers are dropped.
    assert!(!same_origin(&target, &parse("https://evil.example.com/v1")));
    assert!(!same_origin(&target, &parse("http://api.example.com/v1")));
    assert!(!same_origin(
        &target,
        &parse("https://api.example.com:8443/v1")
    ));
}

#[test]
fn threshold_detects_slow() {
    assert!(over_threshold(900, Some(800)));
    assert!(!over_threshold(700, Some(800)));
    assert!(!over_threshold(900, None));
}

#[test]
fn millis_saturates() {
    assert_eq!(millis(Duration::from_millis(5)), 5);
}

#[test]
fn a_down_outcome_is_stored_as_down() {
    let down = Outcome::down("x".to_owned());
    assert_eq!(down.status, CheckStatus::Down);
    assert!(!down.is_up() && !down.is_degraded());
}

fn number(pattern: &str, min: Option<i64>, max: Option<i64>, body: &str) -> Option<String> {
    check_number(&Parsed::new(pattern), min, max, body)
}

fn json(query: &str, expected: Option<&str>, body: &str) -> Option<String> {
    check_json(&Parsed::new(query), expected, body)
}

#[test]
fn snapshot_redacts_credential_headers() {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("set-cookie", "session=s3cr3t; HttpOnly".parse().unwrap());
    headers.insert("www-authenticate", "Bearer realm=x".parse().unwrap());
    headers.insert("content-type", "text/html".parse().unwrap());
    let head = render_head(
        reqwest::Version::HTTP_11,
        reqwest::StatusCode::UNAUTHORIZED,
        &headers,
    );
    assert!(!head.contains("s3cr3t"), "{head}");
    assert!(head.contains("set-cookie: <redacted>"), "{head}");
    assert!(head.contains("www-authenticate: <redacted>"), "{head}");
    // Harmless headers keep their value.
    assert!(head.contains("content-type: text/html"), "{head}");
}

#[test]
fn dns_record_types_map_and_stay_public() {
    // Every configurable type maps onto the hickory type of the same
    // name, so the "no X records found" reason stays recognized.
    for record in DnsRecord::ALL {
        assert_eq!(record_type(record).to_string(), record.as_str());
        let reason = format!("no {} records found", record_type(record));
        assert_eq!(public_reason(&reason), reason);
    }
}

#[tokio::test]
async fn custom_resolver_takes_a_parsed_address() {
    // IPv6 resolvers work now that the address is parsed (brackets and
    // all) at load rather than split on the last colon here.
    for raw in ["9.9.9.9:53", "[2620:fe::fe]:53"] {
        let addr: SocketAddr = Parsed::new(raw).get().copied().expect(raw);
        assert!(resolver_for(Some(addr)).is_ok(), "{raw}");
    }
}

/// A one-shot HTTP server answering every connection with `response`;
/// returns its base URL and the number of requests served so far.
async fn serve(response: &'static str) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = hits.clone();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut buf = [0u8; 4096];
            let _ = socket.read(&mut buf).await;
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        }
    });
    (format!("http://{addr}"), hits)
}

#[tokio::test]
async fn expected_redirect_is_not_followed() {
    let (url, hits) = serve(
            "HTTP/1.1 301 Moved Permanently\r\nLocation: /elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;
    let client = crate::http::probe_client(None).unwrap();
    let mut monitor = http_monitor();
    monitor.target = url;
    monitor.expected_status = Some(301);
    let outcome = http(&client, &monitor).await;
    assert!(outcome.is_up(), "{:?}", outcome.error);
    assert_eq!(outcome.status_code, Some(301));
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);

    // Without a 3xx expectation the redirect is chased (here into a loop
    // back to the same server, until the hop budget runs out).
    monitor.expected_status = None;
    let outcome = http(&client, &monitor).await;
    assert_eq!(outcome.error.as_deref(), Some("too many redirects"));
}

#[tokio::test]
async fn truncated_body_is_a_body_error_not_an_assertion_failure() {
    // Announces 100 bytes, sends 7, closes: a transport failure mid-body.
    let (url, _hits) =
        serve("HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\npartial").await;
    let client = crate::http::probe_client(None).unwrap();
    let mut monitor = http_monitor();
    monitor.target = url;
    monitor.keyword = Some("complete".to_owned());
    let outcome = http(&client, &monitor).await;
    assert!(!outcome.is_up());
    assert_eq!(outcome.error.as_deref(), Some("invalid response body"));
}

fn http_monitor() -> Monitor {
    Monitor {
        id: "m".to_owned(),
        name: "M".to_owned(),
        target: "https://example.com".to_owned(),
        ..Monitor::default()
    }
}

#[test]
fn keyword_assertion() {
    let mut monitor = http_monitor();
    monitor.keyword = Some("OK".to_owned());
    assert!(check_assertions(&monitor, b"all OK here").is_none());
    assert!(check_assertions(&monitor, b"failure").is_some());

    monitor.keyword_invert = true;
    assert!(check_assertions(&monitor, b"failure").is_none());
    assert!(check_assertions(&monitor, b"all OK").is_some());
}

#[test]
fn number_assertion() {
    let body = r#"<div class="stationRealtimeShipUnique">11</div>"#;
    let pattern = r#"stationRealtimeShipUnique">(\d+)<"#;
    // In bounds, at the bound, and out of bounds on both sides.
    assert!(number(pattern, Some(1), None, body).is_none());
    assert!(number(pattern, Some(11), Some(11), body).is_none());
    assert_eq!(
        number(pattern, Some(12), None, body).as_deref(),
        Some("number 11 below min 12")
    );
    assert_eq!(
        number(pattern, None, Some(10), body).as_deref(),
        Some("number 11 above max 10")
    );
    // No bounds: the extraction itself is the assertion.
    assert!(number(pattern, None, None, body).is_none());
    assert!(number(pattern, None, None, "<html>maintenance</html>").is_some());
    // A group-less pattern falls back to the whole match; decimals parse.
    assert!(number(r"[0-9.]+", Some(99), None, "uptime 99.5 %").is_none());
    // A match that is not a number fails rather than passes vacuously.
    assert!(number("<(div)>", Some(1), None, "<div>7</div>").is_some());
}

#[test]
fn json_query_assertion() {
    // Expected value, string and non-string.
    assert!(json("$.status", Some("ok"), r#"{"status":"ok"}"#).is_none());
    assert!(json("$.status", Some("ok"), r#"{"status":"bad"}"#).is_some());
    assert!(json("$.healthy", Some("true"), r#"{"healthy":true}"#).is_none());
    // No expected value: the query just has to match something.
    assert!(json("$.data", None, r#"{"data":[1,2]}"#).is_none());
    assert!(json("$.missing", None, r#"{"data":1}"#).is_some());
    // Malformed JSON fails the assertion.
    assert!(json("$.x", Some("1"), "not json").is_some());
}

#[tokio::test]
async fn applies_every_configured_header() {
    let client = Client::new();
    let mut headers = HashMap::new();
    headers.insert("Accept".to_owned(), Secret("text/html".to_owned()));
    headers.insert("X-Token".to_owned(), Secret("abc".to_owned()));

    let request = with_headers(client.get("https://example.com"), &headers);
    let built = request.build().expect("request builds");

    // Both headers survive: appending never overwrites the previous one.
    assert_eq!(built.headers().len(), 2);
    assert_eq!(
        built.headers().get("accept").unwrap().to_str().unwrap(),
        "text/html"
    );
    assert_eq!(
        built.headers().get("x-token").unwrap().to_str().unwrap(),
        "abc"
    );
}

#[tokio::test]
async fn resolve_parses_ip_literals() {
    // IP literals short-circuit before any DNS lookup (no network in tests).
    assert_eq!(
        resolve("127.0.0.1", None).await,
        Some("127.0.0.1".parse().unwrap())
    );
    assert_eq!(
        resolve("2606:4700:4700::1111", None).await,
        Some("2606:4700:4700::1111".parse().unwrap())
    );
    // A family filter rejects a literal of the other family.
    assert_eq!(
        resolve("127.0.0.1", Some(Family::V4)).await,
        Some("127.0.0.1".parse().unwrap())
    );
    assert_eq!(resolve("127.0.0.1", Some(Family::V6)).await, None);
}

fn outcome(up: bool, latency_ms: Option<i64>, error: Option<&str>) -> Outcome {
    Outcome {
        status: if up {
            CheckStatus::Up
        } else {
            CheckStatus::Down
        },
        latency_ms,
        status_code: None,
        error: error.map(str::to_owned),
        snapshot: None,
    }
}

#[test]
fn combine_requires_both_families() {
    // Both up: up, with the worst latency.
    let both = combine(
        &outcome(true, Some(20), None),
        &outcome(true, Some(35), None),
    );
    assert!(both.is_up());
    assert_eq!(both.latency_ms, Some(35));
    assert_eq!(both.error, None);

    // One family broken: down, the reason names it, the latency is the
    // surviving path's.
    let v6_dead = combine(
        &outcome(true, Some(20), None),
        &outcome(false, None, Some("connection timed out")),
    );
    assert!(!v6_dead.is_up());
    assert_eq!(v6_dead.latency_ms, Some(20));
    assert_eq!(
        v6_dead.error.as_deref(),
        Some("IPv6 failing: connection timed out (IPv4 ok)")
    );

    let v4_dead = combine(
        &outcome(false, None, Some("connection refused")),
        &outcome(true, Some(12), None),
    );
    assert!(!v4_dead.is_up());
    assert_eq!(
        v4_dead.error.as_deref(),
        Some("IPv4 failing: connection refused (IPv6 ok)")
    );

    // Both down: both reasons, no latency.
    let dark = combine(
        &outcome(false, None, Some("timeout")),
        &outcome(false, None, Some("refused")),
    );
    assert!(!dark.is_up());
    assert_eq!(dark.latency_ms, None);
    assert_eq!(
        dark.error.as_deref(),
        Some("IPv4 and IPv6 failing: timeout; refused")
    );
}

#[test]
fn combine_keeps_either_degraded() {
    let mut slow_v6 = outcome(true, Some(900), None);
    slow_v6.status = CheckStatus::Degraded;
    let both = combine(&outcome(true, Some(20), None), &slow_v6);
    assert!(both.is_up());
    assert!(both.is_degraded());
}

#[test]
fn snapshot_renders_status_headers_and_body_bounded() {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("content-type", "text/html".parse().unwrap());
    headers.insert("retry-after", "120".parse().unwrap());

    let head = render_head(
        reqwest::Version::HTTP_2,
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
        &headers,
    );
    assert_eq!(
        head,
        "HTTP/2.0 503 Service Unavailable\ncontent-type: text/html\nretry-after: 120"
    );

    // Body appended after a blank line; an empty body leaves the head alone.
    let full = render_snapshot(&head, b"<html>maintenance</html>");
    assert_eq!(full, format!("{head}\n\n<html>maintenance</html>"));
    assert_eq!(render_snapshot(&head, b"  \n "), head);

    // A hostile response can't bloat the record: the body is clipped to
    // the cap, oversized header values per line, excess headers counted.
    let huge = vec![b'x'; 100_000];
    let clipped = render_snapshot(&head, &huge);
    assert_eq!(
        clipped.len(),
        head.len() + 2 + MAX_SNAPSHOT_BODY_CHARS,
        "body bounded"
    );
    let mut many = reqwest::header::HeaderMap::new();
    for i in 0..30 {
        many.append(
            "x-filler",
            format!("{i}-{}", "v".repeat(500)).parse().unwrap(),
        );
    }
    let head = render_head(
        reqwest::Version::HTTP_11,
        reqwest::StatusCode::BAD_GATEWAY,
        &many,
    );
    let lines: Vec<&str> = head.lines().collect();
    assert_eq!(lines.len(), 1 + MAX_SNAPSHOT_HEADERS + 1);
    assert_eq!(lines[lines.len() - 1], "(6 more headers)");
    assert!(lines[1].len() <= MAX_SNAPSHOT_HEADER_CHARS);
}

#[test]
fn public_reason_collapses_dual_stack_detail() {
    assert_eq!(
        public_reason("IPv6 failing: Connection refused (os error 61) (IPv4 ok)"),
        "IPv6 failing (IPv4 ok)"
    );
    assert_eq!(
        public_reason("IPv4 failing: no IPv4 address for host (IPv6 ok)"),
        "IPv4 failing (IPv6 ok)"
    );
    assert_eq!(
        public_reason("IPv4 and IPv6 failing: timeout; refused"),
        "IPv4 and IPv6 failing"
    );
}
