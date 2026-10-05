//! Producer writes: push heartbeats and pushed alerts.

use super::*;

#[tokio::test]
async fn push_records_heartbeat_with_token() {
    let res = test_app()
        .await
        .oneshot(with_push_token(push("/api/push/beat?status=up"), "s3cret"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn push_rejects_unknown_status_and_negative_ping() {
    let (app, store) = test_app_with_pool().await;
    for bad in [
        "/api/push/beat?status=dwon",
        "/api/push/beat?status=0",
        "/api/push/beat?ping=-5",
    ] {
        let res = app
            .clone()
            .oneshot(with_push_token(push(bad), "s3cret"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "{bad}");
    }
    // Nothing was recorded by the rejected pushes.
    let recorded = hora_core::db::recent_checks(&store, "beat", 10)
        .await
        .unwrap();
    assert!(recorded.is_empty());

    for good in [
        "/api/push/beat",
        "/api/push/beat?status=down&ping=0",
        "/api/push/beat?status=degraded",
    ] {
        let res = app
            .clone()
            .oneshot(with_push_token(push(good), "s3cret"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "{good}");
    }
}

#[tokio::test]
async fn push_rejects_wrong_token() {
    let res = test_app()
        .await
        .oneshot(with_push_token(push("/api/push/beat"), "wrong"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn push_to_an_unknown_id_answers_like_a_wrong_token() {
    // "web" exists but is an HTTP monitor, not a push target; "nope" does
    // not exist at all. Both read like a wrong token: no id oracle.
    let app = test_app().await;
    for (uri, token) in [
        ("/api/push/web", Some("x")),
        ("/api/push/nope", None),
        ("/api/push/beat", Some("wrong")),
    ] {
        let request = match token {
            Some(token) => with_push_token(push(uri), token),
            None => push(uri),
        };
        let res = app.clone().oneshot(request).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "{uri}");
        assert_eq!(body_text(res).await, "invalid push token", "{uri}");
    }
}

#[tokio::test]
async fn push_records_peer_heartbeat() {
    // A watched peer's listen id accepts heartbeats, like a push monitor.
    let res = test_app()
        .await
        .oneshot(with_push_token(push("/api/push/peer-x"), "peertok"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn push_rejects_wrong_peer_token() {
    let res = test_app()
        .await
        .oneshot(with_push_token(push("/api/push/peer-x"), "nope"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

/// A JSON POST to the alert endpoint, with an optional `X-Push-Token` (the
/// item token) and/or `Authorization: Bearer` (the global token).
fn alert(uri: &str, body: &str, push_token: Option<&str>, bearer: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .extension(fake_peer());
    if let Some(token) = push_token {
        builder = builder.header("x-push-token", token);
    }
    if let Some(token) = bearer {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    builder.body(Body::from(body.to_owned())).expect("request")
}

#[tokio::test]
async fn writes_refuse_a_query_token() {
    // A write takes its credential from a header only: the right token in
    // `?token=` is refused like a missing one.
    for request in [
        push("/api/push/beat?token=s3cret"),
        push("/api/push/peer-x?token=peertok"),
        push("/api/event?title=deploy&token=0123456789abcdef"),
        push("/api/silence?monitors=web&duration=10m&token=0123456789abcdef"),
        push("/api/announce?title=x&token=0123456789abcdef"),
        alert(
            "/api/monitors/web/alert?token=0123456789abcdef",
            r#"{"title":"deploy started"}"#,
            None,
            None,
        ),
    ] {
        let uri = request.uri().to_string();
        let res = test_app().await.oneshot(request).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "{uri}");
    }

    // Read-only views keep ?token=.
    let res = test_app()
        .await
        .oneshot(get("/api/summary?token=0123456789abcdef"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn alert_dispatches_with_item_token_and_records_it() {
    let (app, store) = test_app_with_pool().await;
    let res = app
            .oneshot(alert(
                "/api/monitors/beat/alert",
                r#"{"severity":"error","title":"Patch failed","message":"3/12 ops","tags":{"task_id":"t1"}}"#,
                Some("s3cret"),
                None,
            ))
            .await
            .unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);

    let alerts = hora_core::db::recent_pushed_alerts(&store, 10)
        .await
        .unwrap();
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].monitor_id, "beat");
    assert_eq!(alerts[0].severity, "error");
    assert_eq!(alerts[0].title, "Patch failed");
    // The tag is folded into the stored message alongside the producer text.
    assert!(
        alerts[0].message.contains("3/12 ops") && alerts[0].message.contains("task_id=t1"),
        "{}",
        alerts[0].message
    );
}

#[tokio::test]
async fn alert_accepts_global_auth_token_for_a_non_push_monitor() {
    // "web" is an HTTP monitor with no push_token: the global token
    // authorizes, as a Bearer header.
    let res = test_app()
        .await
        .oneshot(alert(
            "/api/monitors/web/alert",
            r#"{"title":"deploy started"}"#,
            None,
            Some("0123456789abcdef"),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn alert_rejects_wrong_or_missing_token() {
    for (push_token, bearer) in [(Some("wrong"), None), (None, None), (None, Some("nope"))] {
        let res = test_app()
            .await
            .oneshot(alert(
                "/api/monitors/beat/alert",
                r#"{"title":"x"}"#,
                push_token,
                bearer,
            ))
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::UNAUTHORIZED,
            "{push_token:?} {bearer:?}"
        );
    }
}

#[tokio::test]
async fn alert_does_not_reveal_which_ids_exist() {
    // Without the operator token, an unknown id, a private monitor and a
    // public one all answer the same 401 - no existence oracle.
    for uri in [
        "/api/monitors/nope/alert",
        "/api/monitors/intra/alert",
        "/api/monitors/web/alert",
    ] {
        let res = test_app()
            .await
            .oneshot(alert(uri, r#"{"title":"x"}"#, Some("guess"), None))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "{uri}");
        assert_eq!(
            body_text(res).await,
            "alerting requires the monitor's push_token (X-Push-Token) or server.auth_token",
            "{uri}"
        );
    }
}

#[tokio::test]
async fn alert_unknown_monitor_is_404() {
    let res = test_app()
        .await
        .oneshot(alert(
            "/api/monitors/nope/alert",
            r#"{"title":"x"}"#,
            None,
            Some("0123456789abcdef"),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn alert_rejects_empty_title_and_bad_severity() {
    for body in [r#"{"title":"   "}"#, r#"{"severity":"boom","title":"x"}"#] {
        let res = test_app()
            .await
            .oneshot(alert(
                "/api/monitors/beat/alert",
                body,
                Some("s3cret"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "{body}");
    }
}

#[tokio::test]
async fn alert_coalesces_a_repeated_dedup_key() {
    let (app, store) = test_app_with_pool().await;
    let body = r#"{"severity":"error","title":"flap","dedup_key":"ekb:mat"}"#;

    let first = app
        .clone()
        .oneshot(alert(
            "/api/monitors/beat/alert",
            body,
            Some("s3cret"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::ACCEPTED);
    assert!(body_text(first).await.contains("dispatched"));

    // Same key inside the window: coalesced, not dispatched or recorded again.
    let second = app
        .clone()
        .oneshot(alert(
            "/api/monitors/beat/alert",
            body,
            Some("s3cret"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::ACCEPTED);
    assert!(body_text(second).await.contains("coalesced"));

    let alerts = hora_core::db::recent_pushed_alerts(&store, 10)
        .await
        .unwrap();
    assert_eq!(alerts.len(), 1, "the repeat must not add a second row");
}

#[tokio::test]
async fn alert_history_respects_public_and_private_visibility() {
    let (app, _pool) = test_app_with_pool().await;
    // A public monitor's alert ("web" has no push_token: global token).
    let res = app
        .clone()
        .oneshot(alert(
            "/api/monitors/web/alert",
            r#"{"severity":"warning","title":"web alert","message":"web detail"}"#,
            None,
            Some("0123456789abcdef"),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    // A private monitor's alert ("intra" is public = false).
    let res = app
        .clone()
        .oneshot(alert(
            "/api/monitors/intra/alert",
            r#"{"severity":"error","title":"intra alert","message":"intra detail"}"#,
            None,
            Some("0123456789abcdef"),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);

    // Anonymous: the public monitor's title shows, its message is collapsed,
    // and the private monitor's alert is hidden outright.
    let anon = body_text(app.clone().oneshot(get("/history")).await.unwrap()).await;
    assert!(anon.contains("web alert"), "{anon}");
    assert!(!anon.contains("web detail"), "{anon}");
    assert!(!anon.contains("intra alert"), "{anon}");

    // Authenticated: full detail, private monitor included.
    let full = body_text(
        app.oneshot(get("/history?token=0123456789abcdef"))
            .await
            .unwrap(),
    )
    .await;
    assert!(
        full.contains("web detail")
            && full.contains("intra alert")
            && full.contains("intra detail"),
        "{full}"
    );
}
