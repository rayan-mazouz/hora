//! The pages: incidents, timeline, monthly report, group pages and tenants.

use super::*;

#[tokio::test]
async fn incident_page_sanitizes_for_anonymous_and_serves_markdown() {
    let (app, pool) = test_app_with_pool().await;
    let id = hora_core::db::insert_incident_start(
        &pool,
        "web",
        Some("HTTP 503: secret stack trace"),
        None,
        &[],
        Some("HTTP/2 503\n\nsecret body"),
        Some("deploy api v2.3, 3m before"),
    )
    .await
    .unwrap();
    hora_core::db::update_incident_vantage(&pool, id, "seen UP by Hora B")
        .await
        .unwrap();

    // Anonymous ("web" is public, no public_error_detail): the reason
    // collapses to its category; snapshot, change and vantage stay private.
    let anon = body_text(
        app.clone()
            .oneshot(get(&format!("/incident/{id}")))
            .await
            .unwrap(),
    )
    .await;
    assert!(anon.contains("HTTP 503"), "{anon}");
    for hidden in ["secret stack trace", "secret body", "deploy api", "Hora B"] {
        assert!(!anon.contains(hidden), "{hidden} leaked:\n{anon}");
    }

    // Authenticated: full detail plus the copyable markdown post-mortem.
    let full = body_text(
        app.clone()
            .oneshot(get(&format!("/incident/{id}?token=0123456789abcdef")))
            .await
            .unwrap(),
    )
    .await;
    assert!(full.contains("secret stack trace") && full.contains("secret body"));
    assert!(full.contains("deploy api v2.3, 3m before"));
    assert!(full.contains("seen UP by Hora B"));
    assert!(full.contains("# Post-mortem"), "{full}");

    let res = app.oneshot(get("/incident/99999")).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn private_monitor_incident_page_is_hidden_from_anonymous() {
    let (app, pool) = test_app_with_pool().await;
    let id = hora_core::db::insert_incident_start(&pool, "intra", None, None, &[], None, None)
        .await
        .unwrap();

    // A private monitor's post-mortem answers like a missing page.
    let res = app
        .clone()
        .oneshot(get(&format!("/incident/{id}")))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let res = app
        .oneshot(get(&format!("/incident/{id}?token=0123456789abcdef")))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn timeline_merges_and_keeps_operator_streams_private() {
    let (app, pool) = test_app_with_pool().await;
    hora_core::db::insert_event(&pool, "deploy api v2.3")
        .await
        .unwrap();
    hora_core::db::insert_silence(
        &pool,
        "web",
        chrono::Utc::now().timestamp() + 600,
        Some("deploying"),
    )
    .await
    .unwrap();
    let incident_id = hora_core::db::insert_incident_start(
        &pool,
        "web",
        Some("HTTP 503: secret detail"),
        None,
        &[],
        None,
        None,
    )
    .await
    .unwrap();
    hora_core::db::insert_announcement(&pool, "Fiber cut", "ETA 6pm", "warning", None)
        .await
        .unwrap();

    // Anonymous: the public incident (sanitized) and the announcement -
    // never the operator streams (events, silences) or the raw reason.
    let anon = body_text(app.clone().oneshot(get("/timeline")).await.unwrap()).await;
    assert!(anon.contains("Web down"), "{anon}");
    assert!(anon.contains("HTTP 503") && !anon.contains("secret detail"));
    assert!(anon.contains("Fiber cut"));
    for hidden in ["deploy api v2.3", "silenced", "deploying"] {
        assert!(!anon.contains(hidden), "{hidden} leaked:\n{anon}");
    }

    // Authenticated: everything, with the post-mortem link.
    let full = body_text(
        app.oneshot(get("/timeline?token=0123456789abcdef"))
            .await
            .unwrap(),
    )
    .await;
    for shown in [
        "deploy api v2.3",
        "silenced Web for 10m",
        "deploying",
        "secret detail",
        "Fiber cut",
    ] {
        assert!(full.contains(shown), "{shown} missing:\n{full}");
    }
    assert!(
        full.contains(&format!("/incident/{incident_id}?token=")),
        "{full}"
    );
}

/// This month, as the report route spells it.
fn this_month() -> String {
    chrono::Utc::now().format("%Y-%m").to_string()
}

#[tokio::test]
async fn report_renders_and_rejects_bad_months() {
    let month = this_month();
    let res = test_app()
        .await
        .oneshot(get(&format!("/report/{month}")))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_text(res).await;
    let label = chrono::Utc::now().format("%B %Y").to_string();
    assert!(
        body.contains("SLA report") && body.contains(&label),
        "{body}"
    );
    // Anonymous: the private monitor stays out of the report.
    assert!(!body.contains("Intra"), "{body}");

    // Malformed, future, or older than the retained history (a full
    // history scan for an always-empty report).
    for bad in [
        "/report/never",
        "/report/2999-01",
        "/report/0001-01",
        "/report/2021-01",
    ] {
        let res = test_app().await.oneshot(get(bad)).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "{bad}");
    }
}

#[tokio::test]
async fn group_report_scopes_and_honours_the_group_token() {
    // The group token reveals the group's private monitor on ITS report.
    let res = test_app()
        .await
        .oneshot(get(&format!(
            "/report/{}?group=App&token=appappappappapp1",
            this_month()
        )))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_text(res).await;
    assert!(body.contains("Intra") && body.contains("Web"), "{body}");
    // Scoped: the push monitor (ungrouped) is not in a group report.
    assert!(!body.contains("Beat"), "{body}");

    // An unknown group answers like a missing page.
    let res = test_app()
        .await
        .oneshot(get(&format!("/report/{}?group=Nope", this_month())))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn group_page_filters_and_404s_unknown() {
    let res = test_app().await.oneshot(get("/status/App")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_text(res).await;
    // Anonymous: the group's public monitor only - and no peers section.
    assert!(body.contains("Web"), "{body}");
    assert!(!body.contains("Intra"), "{body}");
    assert!(!body.contains("Peer X"), "{body}");

    let res = test_app().await.oneshot(get("/status/Nope")).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn group_page_negotiates_plain_text() {
    let res = test_app()
        .await
        .oneshot(
            Request::builder()
                .uri("/status/App")
                .header("user-agent", "curl/8.0")
                .extension(fake_peer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.headers().get("content-type").unwrap(),
        "text/plain; charset=utf-8"
    );
    assert!(body_text(res).await.contains("Web"));

    // Names are padded per group, so the figure columns line up.
    let res = test_app()
        .await
        .oneshot(
            Request::builder()
                .uri("/status/App?token=appappappappapp1")
                .header("user-agent", "curl/8.0")
                .extension(fake_peer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_text(res).await;
    let widths: Vec<usize> = body
        .lines()
        .filter(|line| line.contains("Web") || line.contains("Intra"))
        .map(|line| line.chars().count())
        .collect();
    assert_eq!(widths.len(), 2, "{body}");
    assert_eq!(widths[0], widths[1], "{body}");
}

/// Two tenants sharing a node, with a cross-tenant dependency: "Web"
/// (group App, public) depends on "Internal DB" (group Other, private).
const TENANTS: &str = r#"
        [page]
        title = "Hosting"
        [server]
        auth_token = "0123456789abcdef"
        [server.group_tokens]
        App = "appappappappapp1"
        [[monitors]]
        id = "web"
        name = "Web"
        target = "https://example.com"
        interval_secs = 60
        group = "App"
        depends_on = ["db"]
        [[monitors]]
        id = "intra"
        name = "Intra"
        target = "https://intra.example.com"
        interval_secs = 60
        group = "App"
        public = false
        [[monitors]]
        id = "db"
        name = "Internal DB"
        target = "https://db.internal"
        interval_secs = 60
        group = "Other"
        public = false
    "#;

#[tokio::test]
async fn group_token_gets_its_detail_but_no_operator_data() {
    let (app, pool) = app_with_pool(TENANTS).await;
    let now = chrono::Utc::now().timestamp();
    // Web and the DB are down (3 failures meet the threshold); Intra has
    // a latency series (so its sparkline can carry markers) and a
    // detailed failure.
    for id in ["web", "db"] {
        for offset in [60, 120, 180] {
            sqlx::query("INSERT INTO checks (time, monitor_id, status, error) VALUES (?, ?, 0, 'HTTP 503: boom')")
                    .bind(now - offset)
                    .bind(id)
                    .execute(&pool)
                    .await
                    .unwrap();
        }
    }
    for (offset, status) in [(7200, 1), (3600, 1), (60, 0)] {
        sqlx::query("INSERT INTO checks (time, monitor_id, status, latency_ms, error) VALUES (?, 'intra', ?, 40, 'HTTP 500: tenant secret')")
                .bind(now - offset)
                .bind(status)
                .execute(&pool)
                .await
                .unwrap();
    }
    sqlx::query("INSERT INTO events (title, created_at) VALUES ('deploy billing v9', ?)")
        .bind(now - 1800)
        .execute(&pool)
        .await
        .unwrap();

    // The operator sees everything: the cross-tenant cause and the
    // deploy marker.
    let operator = body_text(
        app.clone()
            .oneshot(get("/status/App?token=0123456789abcdef"))
            .await
            .unwrap(),
    )
    .await;
    assert!(operator.contains("Internal DB"), "{operator}");
    assert!(operator.contains("deploy billing v9"), "{operator}");

    // The group token: its private monitor with full detail...
    let group = body_text(
        app.clone()
            .oneshot(get("/status/App?token=appappappappapp1"))
            .await
            .unwrap(),
    )
    .await;
    assert!(group.contains("Intra"), "{group}");
    assert!(group.contains("tenant secret"), "{group}");
    // ...but neither another tenant's private monitor nor deploy titles.
    assert!(!group.contains("Internal DB"), "{group}");
    assert!(!group.contains("deploy billing v9"), "{group}");

    // Anonymous: public monitors only, sanitized.
    let public = body_text(app.oneshot(get("/status/App")).await.unwrap()).await;
    assert!(!public.contains("Intra") && !public.contains("tenant secret"));
    assert!(!public.contains("Internal DB") && !public.contains("deploy billing v9"));
}

#[tokio::test]
async fn anonymous_history_fills_its_page_past_private_incidents() {
    let (app, pool) = test_app_with_pool().await;
    // One old public incident, buried under more private ones than the
    // page shows.
    hora_core::db::insert_incident_start(&pool, "web", Some("HTTP 503"), None, &[], None, None)
        .await
        .unwrap();
    for _ in 0..120 {
        hora_core::db::insert_incident_start(&pool, "intra", None, None, &[], None, None)
            .await
            .unwrap();
    }
    let page = body_text(app.oneshot(get("/history")).await.unwrap()).await;
    assert!(
        page.contains("Web"),
        "the public incident must still be listed"
    );
}

#[tokio::test]
async fn group_token_reveals_its_group_and_nothing_else() {
    // The group token unlocks the group's private monitors...
    let res = test_app()
        .await
        .oneshot(get("/status/App?token=appappappappapp1"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(body_text(res).await.contains("Intra"));

    // ...but is NOT a global viewer token: the main summary stays public.
    let res = test_app()
        .await
        .oneshot(get("/api/summary?token=appappappappapp1"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(!body_text(res).await.contains("intra"));
}
