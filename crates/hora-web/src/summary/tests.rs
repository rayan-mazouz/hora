use hora_core::db::DayRow;

use super::monitor::{format_minutes, format_slo_pct, topology_context};
use super::status::{build_bar, cert_state_for, day_cell, slo_state};
use super::*;

use crate::visibility::Audience;
fn check(status: i64) -> Latest {
    Latest {
        time: 0,
        latency_ms: None,
        status,
        error: None,
    }
}
#[test]
fn worse_picks_higher_severity() {
    assert_eq!(worse("up", "degraded"), "degraded");
    assert_eq!(worse("down", "degraded"), "down");
    assert_eq!(worse("up", "unknown"), "unknown");
    assert_eq!(worse("degraded", "up"), "degraded");
}

#[test]
fn derive_status_confirms_down_only_after_threshold() {
    assert_eq!(
        db::derive_status(&[check(0), check(0), check(0)], 3),
        "down"
    );
    assert_eq!(db::derive_status(&[check(0)], 3), "degraded");
    assert_eq!(db::derive_status(&[], 3), "unknown");
    assert_eq!(db::derive_status(&[check(1)], 3), "up");
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
    // Everything is down (3 failed checks meets the threshold).
    let recent: HashMap<String, Vec<Latest>> = ["db", "edge", "worker"]
        .into_iter()
        .map(|id| (id.to_owned(), vec![check(0), check(0), check(0)]))
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
    let (cause, _) = topology_context(edge, 3, &recent, &config.monitors, &operator);
    assert_eq!(cause.as_deref(), Some("Internal DB"));
    // Public or another group: a private monitor's name never leaves
    // through a cause.
    for vis in [&public, &group] {
        let (cause, _) = topology_context(edge, 3, &recent, &config.monitors, vis);
        assert_eq!(cause, None);
    }

    // Impacted lists drop private dependents outside the operator view.
    let db = &config.monitors[0];
    let (_, impacted) = topology_context(db, 3, &recent, &config.monitors, &operator);
    assert_eq!(impacted.len(), 2, "{impacted:?}");
    for vis in [&public, &group] {
        let (_, impacted) = topology_context(db, 3, &recent, &config.monitors, vis);
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
