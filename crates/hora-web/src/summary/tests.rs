use hora_core::db::DayRow;
use hora_core::status::{CheckStatus, MonitorState};

use super::monitor::{format_minutes, format_slo_pct, topology_context};
use super::status::{build_bar, cert_state_for, day_cell, slo_state};
use super::*;

use crate::visibility::Audience;
fn check(status: CheckStatus) -> Latest {
    Latest {
        time: 0,
        latency_ms: None,
        status,
        error: None,
    }
}
#[test]
fn worse_picks_higher_severity() {
    use MonitorState::{Degraded, Down, Unknown, Up};
    assert_eq!(worse(Up, Degraded), Degraded);
    assert_eq!(worse(Down, Degraded), Down);
    assert_eq!(worse(Up, Unknown), Unknown);
    assert_eq!(worse(Degraded, Up), Degraded);
}

#[test]
fn derive_status_confirms_down_only_after_threshold() {
    let down = CheckStatus::Down;
    assert_eq!(
        db::derive_status(&[check(down), check(down), check(down)], 3),
        MonitorState::Down
    );
    assert_eq!(db::derive_status(&[check(down)], 3), MonitorState::Degraded);
    assert_eq!(db::derive_status(&[], 3), MonitorState::Unknown);
    assert_eq!(
        db::derive_status(&[check(CheckStatus::Up)], 3),
        MonitorState::Up
    );
}

#[test]
fn cert_state_thresholds() {
    assert_eq!(cert_state_for(None, 14), "none");
    assert_eq!(cert_state_for(Some(-1), 14), "expired");
    assert_eq!(cert_state_for(Some(10), 14), "warn");
    assert_eq!(cert_state_for(Some(40), 14), "ok");
}

#[test]
fn build_bar_zero_fills_to_requested_days() {
    let now = DateTime::from_timestamp(1_609_459_200, 0).unwrap();
    let rows = vec![DayRow {
        day: "2021-01-01".to_owned(),
        up: 10,
        down: 0,
        degraded: 0,
    }];
    let bar = build_bar(&rows, now, 7);
    assert_eq!(bar.len(), 7);
    assert_eq!(bar.last().unwrap().state, "up");
    assert_eq!(bar[0].state, "empty");
}

#[test]
fn day_cell_reds_only_real_outages() {
    let row = |up, down| DayRow {
        day: "2021-01-01".to_owned(),
        up,
        down,
        degraded: 0,
    };
    assert_eq!(day_cell("d".to_owned(), &row(100, 0)).state, "up"); // 100%
    assert_eq!(day_cell("d".to_owned(), &row(1439, 1)).state, "degraded"); // ~99.9% blip
    assert_eq!(day_cell("d".to_owned(), &row(1400, 40)).state, "degraded"); // ~97% - mostly up
    assert_eq!(day_cell("d".to_owned(), &row(800, 640)).state, "down"); // ~56% - real outage
}
#[test]
fn slo_state_compares_p95() {
    assert_eq!(slo_state(Some(200), Some(150)), "met");
    assert_eq!(slo_state(Some(200), Some(250)), "breached");
    assert_eq!(slo_state(None, Some(150)), "none");
    assert_eq!(slo_state(Some(200), None), "none");
}

#[test]
fn topology_context_hides_private_names_from_public_view() {
    let config = hora_core::config::parse(
        r#"
            [page]
            [server]
            auth_token = "seekrit-long-token"
            [[monitors]]
            id = "db"
            name = "Internal DB"
            target = "https://db.internal"
            interval_secs = 60
            public = false
            [[monitors]]
            id = "edge"
            name = "Edge"
            target = "https://example.com"
            interval_secs = 60
            depends_on = ["db"]
            [[monitors]]
            id = "worker"
            name = "Worker"
            target = "https://worker.internal"
            interval_secs = 60
            public = false
            depends_on = ["db"]
        "#,
    )
    .expect("config");
    // Everything is down.
    let statuses: HashMap<String, MonitorState> = ["db", "edge", "worker"]
        .into_iter()
        .map(|id| (id.to_owned(), MonitorState::Down))
        .collect();

    let operator = Audience::Operator;
    let operator = Visibility::new(&config, &operator);
    let public = Audience::Public;
    let public = Visibility::new(&config, &public);
    // A group token holder is no operator: the private DB and worker
    // belong to no group of theirs.
    let group = Audience::Group("App".to_owned());
    let group = Visibility::new(&config, &group);

    // Operator: the private upstream is named as the cause.
    let edge = &config.monitors[1];
    let (cause, _) = topology_context(edge, &statuses, &config.monitors, &operator);
    assert_eq!(cause.as_deref(), Some("Internal DB"));
    // Public or another group: a private monitor's name never leaves
    // through a cause.
    for vis in [&public, &group] {
        let (cause, _) = topology_context(edge, &statuses, &config.monitors, vis);
        assert_eq!(cause, None);
    }

    // Impacted lists drop private dependents outside the operator view.
    let db = &config.monitors[0];
    let (_, impacted) = topology_context(db, &statuses, &config.monitors, &operator);
    assert_eq!(impacted.len(), 2, "{impacted:?}");
    for vis in [&public, &group] {
        let (_, impacted) = topology_context(db, &statuses, &config.monitors, vis);
        assert_eq!(impacted, vec!["Edge".to_owned()]);
    }
}

#[test]
fn budget_durations_and_pct_format() {
    assert_eq!(format_minutes(43), "43m");
    assert_eq!(format_minutes(187), "3h07m");
    assert_eq!(format_minutes(3000), "2d2h");
    assert_eq!(format_slo_pct(9990), "99.9");
    assert_eq!(format_slo_pct(9995), "99.95");
    assert_eq!(format_slo_pct(9900), "99");
}

/// Three monitors - `ok` (up), `bad` (degraded, a detailed reason) and
/// `hidden` (private) - with one check each, and their config.
async fn three_monitors() -> (hora_core::db::Store, hora_core::config::Config, i64) {
    let store = hora_core::db::Store::in_memory().await;
    let config = hora_core::config::parse(
        r#"
            [page]
            [server]
            auth_token = "seekrit-long-token"
            [[monitors]]
            id = "ok"
            name = "Fine"
            target = "https://ok.example.com"
            interval_secs = 60
            [[monitors]]
            id = "bad"
            name = "Failing"
            target = "https://bad.example.com"
            interval_secs = 60
            [[monitors]]
            id = "hidden"
            name = "Hidden"
            target = "https://hidden.example.com"
            interval_secs = 60
            public = false
        "#,
    )
    .unwrap();
    let now = Utc::now().timestamp();
    for (id, status, error) in [
        ("ok", 1, None),
        ("bad", 2, Some("HTTP 500: stack trace")),
        ("hidden", 1, None),
    ] {
        sqlx::query(
            "INSERT INTO checks (time, monitor_id, status, latency_ms, error) VALUES (?, ?, ?, 40, ?)",
        )
        .bind(now - 60)
        .bind(id)
        .bind(status)
        .bind(error)
        .execute(store.fixture_pool())
        .await
        .unwrap();
    }
    (store, config, now)
}

#[tokio::test]
async fn derived_views_share_unchanged_cards_and_redact_the_rest() {
    let (store, config, _) = three_monitors().await;
    let built = build_summary(
        &store,
        &config,
        &mut BuildState::default(),
        &[],
        &HashMap::new(),
    )
    .await;
    assert_eq!(built.summary.monitors.len(), 3);

    let audience = Audience::Public;
    let public = derive(&built, &config, &Visibility::new(&config, &audience));
    let ids: Vec<&str> = public.monitors.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["ok", "bad"]);
    // The healthy card is the operator's own allocation...
    assert!(Arc::ptr_eq(&public.monitors[0], &built.summary.monitors[0]));
    // ...the failing one a copy with the safe reason, sharing the heavy parts.
    let (theirs, ours) = (&public.monitors[1], &built.summary.monitors[1]);
    assert_eq!(theirs.last_error.as_deref(), Some("HTTP 500"));
    assert_eq!(ours.last_error.as_deref(), Some("HTTP 500: stack trace"));
    assert!(Arc::ptr_eq(&theirs.spark, &ours.spark));
    assert!(std::ptr::eq(theirs.bar.as_ptr(), ours.bar.as_ptr()));
    assert_eq!(public.groups.len(), 1);
    assert_eq!(public.groups[0].ids, ["ok", "bad"]);
    assert_eq!(public.overall, MonitorState::Degraded);
}

#[tokio::test]
async fn each_audience_lists_the_incidents_it_may_see() {
    let (store, config, now) = three_monitors().await;
    // Two finished incidents: the hidden monitor's, a day ago, and the
    // public one's, three days ago (with a detailed reason).
    for (id, ended, error) in [
        ("ok", now - 3 * 86_400, "HTTP 503: upstream db-7 refused"),
        ("hidden", now - 86_400, "timeout"),
    ] {
        sqlx::query(
            "INSERT INTO incidents (monitor_id, started_at, ended_at, duration_s, error, created_at) \
             VALUES (?, ?, ?, 600, ?, ?)",
        )
        .bind(id)
        .bind(ended - 600)
        .bind(ended)
        .bind(error)
        .bind(ended - 600)
        .execute(store.fixture_pool())
        .await
        .unwrap();
    }
    let built = build_summary(
        &store,
        &config,
        &mut BuildState::default(),
        &[],
        &HashMap::new(),
    )
    .await;
    // The operator sees both incidents, newest first, and the last day.
    let titles: Vec<&str> = built
        .summary
        .recent
        .iter()
        .map(|i| i.title.as_str())
        .collect();
    assert_eq!(
        titles,
        ["Hidden was down for 10 min", "Fine was down for 10 min"]
    );
    assert_eq!(built.summary.quiet_days, Some(1));

    let audience = Audience::Public;
    let public = derive(&built, &config, &Visibility::new(&config, &audience));
    // The public list leaves the private monitor's incident out, the reason
    // reduced to its safe category; its quiet days count from its own.
    assert_eq!(public.recent.len(), 1);
    assert_eq!(public.recent[0].title, "Fine was down for 10 min");
    assert_eq!(
        public.recent[0].note.as_deref(),
        Some("Last answer: HTTP 503.")
    );
    assert_eq!(public.quiet_days, Some(3));
    // No event marker reaches a derived view.
    assert!(public.events.is_empty());
}
