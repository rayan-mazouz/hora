//! The background-refreshed status snapshot: what requests wait for (only
//! the first build), what reloads and operator writes change at once.

use super::*;

const TWO: &str = r#"
    [page]
    [server]
    auth_token = "0123456789abcdef"
    [[monitors]]
    id = "web"
    name = "Web"
    target = "https://example.com"
    interval_secs = 60
    [[monitors]]
    id = "api"
    name = "Api"
    target = "https://api.example.com"
    interval_secs = 60
"#;

async fn insert_up(pool: &sqlx::SqlitePool, id: &str, latency: i64) {
    sqlx::query("INSERT INTO checks (time, monitor_id, status, latency_ms) VALUES (?, ?, 1, ?)")
        .bind(chrono::Utc::now().timestamp())
        .bind(id)
        .bind(latency)
        .execute(pool)
        .await
        .unwrap();
}

async fn summary_json(app: &Router, uri: &str) -> serde_json::Value {
    let res = app.clone().oneshot(get(uri)).await.unwrap();
    serde_json::from_str(&body_text(res).await).unwrap()
}

#[tokio::test]
async fn requests_are_served_from_the_snapshot_never_a_rebuild() {
    let (app, pool, _tx) = app_with_reload(TWO).await;
    insert_up(&pool, "web", 42).await;
    // The first request waits for the first build...
    let first = summary_json(&app, "/api/summary").await;
    assert_eq!(first["monitors"][0]["last_latency_ms"], 42);

    // ...later ones answer from it, even when the data moved meanwhile: the
    // refresher catches up in the background, no request rebuilds.
    sqlx::query("DELETE FROM checks")
        .execute(&pool)
        .await
        .unwrap();
    let again = summary_json(&app, "/api/summary").await;
    assert_eq!(again["generated_at"], first["generated_at"]);
    assert_eq!(again["monitors"][0]["last_latency_ms"], 42);
}

#[tokio::test]
async fn a_reload_rebuilds_at_once_and_hides_new_private_monitors_before() {
    let (app, _pool, tx) = app_with_reload(TWO).await;
    let page = body_text(app.clone().oneshot(get("/")).await.unwrap()).await;
    assert!(page.contains("Api"), "{page}");
    let built = summary_json(&app, "/api/summary").await["generated_at"].clone();

    // `api` turns private: the very next anonymous view drops it, before any
    // rebuild (views are derived against the live config)...
    let private = TWO.replace(
        "target = \"https://api.example.com\"",
        "target = \"https://api.example.com\"\npublic = false",
    );
    tx.send(Arc::new(hora_core::config::parse(&private).unwrap()))
        .unwrap();
    let page = body_text(app.clone().oneshot(get("/")).await.unwrap()).await;
    assert!(!page.contains("Api"), "{page}");
    let operator = body_text(
        app.clone()
            .oneshot(get("/?token=0123456789abcdef"))
            .await
            .unwrap(),
    )
    .await;
    assert!(operator.contains("Api"), "{operator}");

    // ...and the reload starts a build right away, not a refresh period later.
    let rebuilt = tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let now = summary_json(&app, "/api/summary").await["generated_at"].clone();
            if now != built {
                return now;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(rebuilt.is_ok(), "no rebuild after the reload");
}

#[tokio::test]
async fn bodies_are_rendered_once_per_snapshot_and_audience() {
    let (app, pool, _tx) = app_with_reload(TWO).await;
    insert_up(&pool, "web", 42).await;
    let public = body_text(app.clone().oneshot(get("/")).await.unwrap()).await;
    let again = body_text(app.clone().oneshot(get("/")).await.unwrap()).await;
    assert_eq!(public, again);
    // Each format and audience has its own body.
    let text = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .header("user-agent", "curl/8")
                .extension(fake_peer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        text.headers()[axum::http::header::CONTENT_TYPE],
        "text/plain; charset=utf-8"
    );
    assert!(body_text(text).await.contains("Web"));
    let json = app.clone().oneshot(get("/api/summary")).await.unwrap();
    assert_eq!(
        json.headers()[axum::http::header::CONTENT_TYPE],
        "application/json"
    );
    let metrics = body_text(
        app.oneshot(get("/metrics?token=0123456789abcdef"))
            .await
            .unwrap(),
    )
    .await;
    assert!(
        metrics.contains("hora_monitor_last_latency_ms"),
        "{metrics}"
    );
}
