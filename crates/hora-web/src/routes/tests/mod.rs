//! End-to-end tests of the router: one area per submodule, the shared
//! fixtures and the cross-cutting checks (headers, request ids, rate limits) here.

mod annotations;
mod pages;
mod peer;
mod push;
mod snapshot;

use super::*;

use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use axum::http::StatusCode;
use tokio::sync::watch;
use tower::ServiceExt as _;
async fn test_app() -> Router {
    test_app_with_pool().await.0
}

async fn test_app_with_pool() -> (Router, hora_core::db::Store) {
    test_app_with_vantage(hora_core::mesh::vantage::new_map()).await
}

/// The test app, with the peers' view of the shared targets already polled.
async fn test_app_with_vantage(
    vantage: hora_core::mesh::vantage::VantageMap,
) -> (Router, hora_core::db::Store) {
    let store = hora_core::db::Store::in_memory().await;
    let config = hora_core::config::parse(
        r#"
            [page]
            [server]
            auth_token = "0123456789abcdef"
            [server.group_tokens]
            App = "appappappappapp1"
            [health]
            id = "test-node"
            [[peers]]
            id = "peer-x"
            name = "Peer X"
            expect_every_secs = 60
            listen_token = "peertok"
            [[monitors]]
            id = "web"
            name = "Web"
            target = "https://example.com"
            interval_secs = 60
            group = "App"
            [[monitors]]
            id = "intra"
            name = "Intra"
            target = "https://intra.example.com"
            interval_secs = 60
            group = "App"
            public = false
            [[monitors]]
            id = "beat"
            name = "Beat"
            kind = "push"
            interval_secs = 60
            push_token = "s3cret"
            "#,
    )
    .expect("config");
    let config = Arc::new(config);
    let client = hora_core::http::client(None).expect("client");
    let notifier = hora_core::notifications::shared(&config, &client);
    let (_tx, rx) = watch::channel(config);
    let app = router(
        AppState::new(
            store.clone(),
            rx,
            Arc::new(AtomicU64::new(fresh_tick())),
            notifier,
        )
        .with_vantage(vantage),
    );
    (app, store)
}

/// A scheduler beacon that just ticked, so `/healthz` reports ok.
fn fresh_tick() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp()).expect("positive")
}

/// The rate limiter keys on the peer address; oneshot has no real
/// connection, so supply one.
fn fake_peer() -> axum::extract::ConnectInfo<std::net::SocketAddr> {
    axum::extract::ConnectInfo(std::net::SocketAddr::from(([127, 0, 0, 1], 12345)))
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .extension(fake_peer())
        .body(Body::empty())
        .expect("request")
}
fn push(uri: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .extension(fake_peer())
        .body(Body::empty())
        .expect("request")
}

/// The test app's `server.auth_token`.
const OPERATOR_TOKEN: &str = "0123456789abcdef";

/// `request`, carrying the operator token as `Authorization: Bearer`.
fn as_operator(mut request: Request<Body>) -> Request<Body> {
    request.headers_mut().insert(
        header::AUTHORIZATION,
        header::HeaderValue::from_str(&format!("Bearer {OPERATOR_TOKEN}")).expect("header"),
    );
    request
}

/// `request`, carrying `token` as `X-Push-Token`.
fn with_push_token(mut request: Request<Body>, token: &str) -> Request<Body> {
    request.headers_mut().insert(
        "x-push-token",
        header::HeaderValue::from_str(token).expect("header"),
    );
    request
}

/// Node B's config for the peer-probe tests: it knows the tcp target and
/// expects requests from peer `hora-a` with this token.
fn vantage_config(target_port: u16) -> String {
    format!(
        r#"
            [page]
            [server]
            [health]
            id = "hora-b"
            [[peers]]
            id = "hora-a"
            name = "A"
            expect_every_secs = 60
            listen_token = "tok-a-to-b-16char"
            [[monitors]]
            id = "svc"
            name = "Svc"
            kind = "tcp"
            target = "127.0.0.1:{target_port}"
            interval_secs = 60
            timeout_secs = 2
            "#
    )
}

#[tokio::test]
async fn healthz_is_ok() {
    let res = test_app().await.oneshot(get("/healthz")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn healthz_is_503_when_degraded_with_the_same_body() {
    // The scheduler never ticked although monitors are configured: a
    // degraded node must fail a plain HTTP health check.
    let res = app_from(&vantage_config(9))
        .await
        .oneshot(get("/healthz"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    // Still starting: the 503 is no failure for the access log.
    assert!(res.extensions().get::<crate::routes::Starting>().is_some());
    let body = body_text(res).await;
    assert!(body.contains(r#""status":"degraded""#), "{body}");
}

#[tokio::test]
async fn a_stalled_scheduler_is_a_logged_failure() {
    // It ticked once, an hour ago: a real stall, not a start-up.
    let res = app_ticked(&vantage_config(9), fresh_tick() - 3600)
        .await
        .oneshot(get("/healthz"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(res.extensions().get::<crate::routes::Starting>().is_none());
}

/// Like [`app_from`], with a live scheduler beacon and the pool returned.
async fn app_with_pool(toml: &str) -> (Router, hora_core::db::Store) {
    let (app, store, tx) = app_with_reload(toml).await;
    std::mem::forget(tx);
    (app, store)
}

/// Like [`app_with_pool`], plus the config sender, to reload the config.
async fn app_with_reload(
    toml: &str,
) -> (
    Router,
    hora_core::db::Store,
    watch::Sender<Arc<hora_core::config::Config>>,
) {
    let store = hora_core::db::Store::in_memory().await;
    let config = Arc::new(hora_core::config::parse(toml).expect("config"));
    let client = hora_core::http::client(None).expect("client");
    let notifier = hora_core::notifications::shared(&config, &client);
    let (tx, rx) = watch::channel(config);
    let app = router(AppState::new(
        store.clone(),
        rx,
        Arc::new(AtomicU64::new(fresh_tick())),
        notifier,
    ));
    (app, store, tx)
}

/// Build an app from an arbitrary config TOML (the shared `test_app` has a
/// fixed one; the peer-probe tests need targets bound to live local ports).
async fn app_from(toml: &str) -> Router {
    app_ticked(toml, 0).await
}

/// Like [`app_from`], with the scheduler's last tick at `tick` (0: never).
async fn app_ticked(toml: &str, tick: u64) -> Router {
    let store = hora_core::db::Store::in_memory().await;
    let config = Arc::new(hora_core::config::parse(toml).expect("config"));
    let client = hora_core::http::client(None).expect("client");
    let notifier = hora_core::notifications::shared(&config, &client);
    let (tx, rx) = watch::channel(config);
    // Keep the sender alive for the app's lifetime.
    std::mem::forget(tx);
    router(AppState::new(
        store,
        rx,
        Arc::new(AtomicU64::new(tick)),
        notifier,
    ))
}

/// Read a response body as text (the pages are small).
async fn body_text(res: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .expect("body");
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn heatmap_is_svg_and_unknown_is_404() {
    let res = test_app()
        .await
        .oneshot(get("/api/monitors/web/heatmap.svg"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get("content-type").unwrap(), "image/svg+xml");

    let res = test_app()
        .await
        .oneshot(get("/api/monitors/nope/heatmap.svg"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unknown_monitor_is_404() {
    let res = test_app()
        .await
        .oneshot(get("/api/monitors/nope/latency"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn known_monitor_latency_is_ok() {
    let res = test_app()
        .await
        .oneshot(get("/api/monitors/web/latency"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn summary_has_security_and_ratelimit_headers() {
    let res = test_app().await.oneshot(get("/api/summary")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let headers = res.headers();
    assert!(headers.contains_key("content-security-policy"));
    assert!(headers.contains_key("x-ratelimit-limit"));
    assert!(headers.contains_key("permissions-policy"));
    assert_eq!(
        headers.get("cross-origin-opener-policy").unwrap(),
        "same-origin"
    );
    let csp = headers["content-security-policy"].to_str().unwrap();
    assert!(!csp.contains("data:"), "{csp}");
    assert!(csp.contains("form-action 'self'"), "{csp}");
    // An anonymous answer may be cached like any public page.
    assert!(!headers.contains_key("cache-control"));
}

#[tokio::test]
async fn credentialed_answers_are_never_stored() {
    let app = test_app().await;
    let bearer = Request::builder()
        .uri("/api/summary")
        .header("authorization", "Bearer 0123456789abcdef")
        .extension(fake_peer())
        .body(Body::empty())
        .expect("request");
    for request in [
        bearer,
        get("/metrics?token=0123456789abcdef"),
        get("/?token=appappappappapp1"),
    ] {
        let uri = request.uri().clone();
        let res = app.clone().oneshot(request).await.unwrap();
        assert_eq!(res.headers()["cache-control"], "no-store", "{uri}");
    }
}

#[tokio::test]
async fn pages_have_their_own_more_generous_rate_limit() {
    let app = test_app().await;
    let res = app.clone().oneshot(get("/report/2999-01")).await.unwrap();
    let page_limit: u32 = res.headers()["x-ratelimit-limit"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    let res = app.clone().oneshot(get("/api/summary")).await.unwrap();
    let api_limit: u32 = res.headers()["x-ratelimit-limit"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(page_limit, api_limit * PAGE_LIMIT_FACTOR);
    // Static assets stay outside any limiter.
    let res = app.oneshot(get("/favicon.svg")).await.unwrap();
    assert!(!res.headers().contains_key("x-ratelimit-limit"));
}

#[tokio::test]
async fn metrics_skip_up_for_unknown_monitors() {
    let (app, store) = test_app_with_pool().await;
    sqlx::query(
        "INSERT INTO checks (time, monitor_id, status, latency_ms) VALUES (?, 'web', 1, 12)",
    )
    .bind(chrono::Utc::now().timestamp())
    .execute(store.fixture_pool())
    .await
    .unwrap();
    let body = body_text(
        app.oneshot(get("/metrics?token=0123456789abcdef"))
            .await
            .unwrap(),
    )
    .await;
    assert!(
        body.contains(r#"hora_monitor_up{id="web",name="Web"} 1"#),
        "{body}"
    );
    // Never checked: no `up` sample (a 0 would read as down)...
    assert!(!body.contains(r#"hora_monitor_up{id="intra""#), "{body}");
    // ...but its state is exported.
    assert!(
        body.contains(r#"hora_monitor_status{id="intra",name="Intra",status="unknown"} 1"#),
        "{body}"
    );
    assert!(
        !body.contains("# TYPE hora_monitor_latency_ms summary"),
        "{body}"
    );
}

#[tokio::test]
async fn response_carries_a_minted_request_id() {
    let res = test_app().await.oneshot(get("/healthz")).await.unwrap();
    let id = res
        .headers()
        .get(REQUEST_ID_HEADER)
        .expect("x-request-id present")
        .to_str()
        .expect("ascii");
    // Minted ids are 32 hex chars (16-char random prefix + 16-char counter).
    assert_eq!(id.len(), 32);
    assert!(id.bytes().all(|b| b.is_ascii_hexdigit()));
}

#[tokio::test]
async fn malformed_inbound_request_id_is_replaced() {
    for forged in ["has space", "x".repeat(65).as_str(), "quote\"d", ""] {
        let request = Request::builder()
            .uri("/healthz")
            .header(REQUEST_ID_HEADER, forged)
            .body(Body::empty())
            .expect("request");
        let res = test_app().await.oneshot(request).await.unwrap();
        let id = res
            .headers()
            .get(REQUEST_ID_HEADER)
            .unwrap()
            .to_str()
            .unwrap();
        assert_ne!(id, forged);
        assert_eq!(id.len(), 32, "a fresh id replaces {forged:?}");
    }
}

#[tokio::test]
async fn inbound_request_id_is_preserved() {
    let request = Request::builder()
        .uri("/healthz")
        .header(REQUEST_ID_HEADER, "trace-from-proxy")
        .body(Body::empty())
        .expect("request");
    let res = test_app().await.oneshot(request).await.unwrap();
    assert_eq!(
        res.headers().get(REQUEST_ID_HEADER).unwrap(),
        "trace-from-proxy"
    );
}

#[tokio::test]
async fn openapi_and_page_render() {
    let res = test_app()
        .await
        .oneshot(get("/api/openapi.json"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    // Writes document no ?token= parameter: they take a header only.
    let doc: serde_json::Value = serde_json::from_str(&body_text(res).await).unwrap();
    for (path, method) in [("/api/push/{id}", "post"), ("/api/silence", "post")] {
        let params = doc["paths"][path][method]["parameters"].as_array().cloned();
        assert!(
            !params
                .unwrap_or_default()
                .iter()
                .any(|p| p["name"] == "token"),
            "{path}"
        );
    }
    assert_eq!(
        test_app().await.oneshot(get("/")).await.unwrap().status(),
        StatusCode::OK
    );
}
#[tokio::test]
async fn status_badge_is_svg() {
    let app = test_app().await;
    let res = app
        .clone()
        .oneshot(get("/api/badge/web/status"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get("content-type").unwrap(), "image/svg+xml");
    assert!(body_text(res).await.contains(r#"id="s""#));

    let res = app
        .clone()
        .oneshot(get("/api/badge/web/status?style=flat-square"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(!body_text(res).await.contains(r#"id="s""#));

    let res = app
        .oneshot(get("/api/badge/web/status?style=for-the-badge"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(body_text(res).await.contains(r#"height="28""#));
}

#[tokio::test]
async fn unknown_badge_is_404() {
    let res = test_app()
        .await
        .oneshot(get("/api/badge/nope/uptime"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn only_the_operator_summary_carries_the_database_size() {
    let app = test_app().await;
    let operator = app
        .clone()
        .oneshot(get("/api/summary?token=0123456789abcdef"))
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_str(&body_text(operator).await).unwrap();
    let storage = &json["storage"];
    assert!(
        storage["db_bytes"].as_u64().is_some_and(|bytes| bytes > 0),
        "{json}"
    );
    assert!(storage["reclaimable_bytes"].as_u64().is_some(), "{json}");

    let public = app.oneshot(get("/api/summary")).await.unwrap();
    let json: serde_json::Value = serde_json::from_str(&body_text(public).await).unwrap();
    assert!(json.get("storage").is_none(), "{json}");
}
