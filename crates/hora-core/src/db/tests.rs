use super::aggregates::iso_day;
use super::certs::cert_not_after;
use super::checks::{insert_heartbeat_miss_at, insert_push_at};
use super::retention::{
    ORPHAN_GRACE_SECS, delete_orphans, prune_announcements, prune_events, prune_incidents,
    prune_silences,
};
use super::*;
use crate::SECONDS_PER_DAY;
use crate::config::Config;

async fn memory_pool() -> SqlitePool {
    let options = SqliteConnectOptions::new()
        .filename(":memory:")
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("connect in-memory");
    migrator().run(&pool).await.expect("run migrations");
    pool
}

#[tokio::test]
async fn a_release_alerts_once_and_a_new_project_starts_afresh() {
    let pool = memory_pool().await;
    assert_eq!(release_watch(&pool, "chat").await.unwrap(), None);

    upsert_release_watch(&pool, "chat", "a/b", "v1.9.2", "https://x/v1.9.2", 100)
        .await
        .unwrap();
    mark_release_notified(&pool, "chat", "v1.9.2")
        .await
        .unwrap();

    // The same project looked up again: the alert already sent is remembered.
    upsert_release_watch(&pool, "chat", "a/b", "v1.9.2", "https://x/v1.9.2", 200)
        .await
        .unwrap();
    let stored = release_watch(&pool, "chat").await.unwrap().expect("stored");
    assert_eq!(stored.checked_at, 200);
    assert_eq!(stored.notified.as_deref(), Some("v1.9.2"));

    // Another project's tags mean nothing to this one.
    upsert_release_watch(&pool, "chat", "c/d", "v1.9.2", "https://y/v1.9.2", 300)
        .await
        .unwrap();
    let stored = release_watch(&pool, "chat").await.unwrap().expect("stored");
    assert_eq!(stored.project, "c/d");
    assert_eq!(stored.notified, None);
}

async fn insert(pool: &SqlitePool, id: &str, time: i64, status: i64, latency: Option<i64>) {
    sqlx::query(
        "INSERT INTO checks (time, monitor_id, status, latency_ms, status_code, error) \
             VALUES (?, ?, ?, ?, NULL, NULL)",
    )
    .bind(time)
    .bind(id)
    .bind(status)
    .bind(latency)
    .execute(pool)
    .await
    .expect("insert check");
}

#[tokio::test]
async fn availability_latest_and_series() {
    let pool = memory_pool().await;
    insert(&pool, "m", 100, 1, Some(10)).await;
    insert(&pool, "m", 200, 0, None).await;
    insert(&pool, "m", 300, 1, Some(20)).await;

    assert_eq!(availability(&pool, "m", 0).await.unwrap(), (2, 3));

    let recent = recent_checks(&pool, "m", 2).await.unwrap();
    assert_eq!(recent.len(), 2);
    assert_eq!(
        (recent[0].time, recent[0].status, recent[0].latency_ms),
        (300, 1, Some(20))
    );
    assert_eq!(
        recent.iter().map(|c| c.status).collect::<Vec<_>>(),
        vec![1, 0]
    );

    let series = latency_series(&pool, "m", 0, 1).await.unwrap();
    assert_eq!(series.len(), 2);
    assert_eq!(series[0].latency_ms, 10);
}

#[tokio::test]
async fn last_heartbeat_ignores_recorded_misses() {
    let pool = memory_pool().await;
    insert(&pool, "m", 100, 1, Some(5)).await; // positive heartbeat
    insert(&pool, "m", 200, 0, None).await; // recorded miss (fresher)

    // last_check_time sees the miss; last_heartbeat_time skips it, so staleness
    // is measured from the real heartbeat and an outage can't reset the clock.
    assert_eq!(last_check_time(&pool, "m").await.unwrap(), Some(200));
    assert_eq!(last_heartbeat_time(&pool, "m").await.unwrap(), Some(100));

    // A monitor with only misses has no positive heartbeat at all.
    insert(&pool, "only-miss", 50, 0, None).await;
    assert_eq!(last_heartbeat_time(&pool, "only-miss").await.unwrap(), None);
}

#[tokio::test]
async fn explicit_down_push_is_a_heartbeat_but_a_miss_is_not() {
    let pool = memory_pool().await;
    insert_push_at(&pool, "job", 1, None, None, 100)
        .await
        .unwrap();
    insert_push_at(&pool, "job", 0, None, Some("backup failed"), 200)
        .await
        .unwrap();
    insert_heartbeat_miss_at(&pool, "job", "missing heartbeat", 300)
        .await
        .unwrap();

    // The newest heartbeat is the explicit down push, message included;
    // the fresher recorded miss is skipped.
    let beat = last_heartbeat(&pool, "job").await.unwrap().unwrap();
    assert_eq!(beat.time, 200);
    assert_eq!(beat.status, 0);
    assert_eq!(beat.error.as_deref(), Some("backup failed"));
}

#[tokio::test]
async fn same_second_pushes_keep_the_last_write() {
    let pool = memory_pool().await;
    // `status=up` at the start of a job, `status=down` when it fails, both
    // within the same second: the failure must survive.
    insert_push_at(&pool, "job", 1, Some(5), None, 100)
        .await
        .unwrap();
    insert_push_at(&pool, "job", 0, None, Some("disk full"), 100)
        .await
        .unwrap();
    let beat = last_heartbeat(&pool, "job").await.unwrap().unwrap();
    assert_eq!((beat.time, beat.status), (100, 0));
    assert_eq!(beat.error.as_deref(), Some("disk full"));
    assert_eq!(beat.latency_ms, None);
    let rows = recent_checks(&pool, "job", 10).await.unwrap();
    assert_eq!(rows.len(), 1, "one row per second");
}

#[tokio::test]
async fn push_replaces_a_same_second_miss_and_never_the_reverse() {
    let pool = memory_pool().await;
    // The scheduler records a miss, then the late heartbeat lands in the
    // same second: the heartbeat wins, ending the miss streak.
    insert_heartbeat_miss_at(&pool, "job", "missing heartbeat", 100)
        .await
        .unwrap();
    insert_push_at(&pool, "job", 1, None, None, 100)
        .await
        .unwrap();
    let beat = last_heartbeat(&pool, "job").await.unwrap().unwrap();
    assert_eq!((beat.time, beat.status), (100, 1));

    // The other order: a miss racing a push it did not see must not erase it.
    insert_push_at(&pool, "job", 1, None, None, 200)
        .await
        .unwrap();
    insert_heartbeat_miss_at(&pool, "job", "missing heartbeat", 200)
        .await
        .unwrap();
    let beat = last_heartbeat(&pool, "job").await.unwrap().unwrap();
    assert_eq!((beat.time, beat.status), (200, 1));
    assert_eq!(recent_checks(&pool, "job", 10).await.unwrap()[0].status, 1);
}

#[tokio::test]
async fn latency_percentiles_match_nearest_rank() {
    let pool = memory_pool().await;
    // Latencies 10,20,..,100 (n=10) for "x"; a down check (no latency) is ignored.
    let latencies = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
    for (i, &latency) in latencies.iter().enumerate() {
        insert(
            &pool,
            "x",
            1000 + i64::try_from(i).unwrap(),
            1,
            Some(latency),
        )
        .await;
    }
    insert(&pool, "x", 2000, 0, None).await;
    insert(&pool, "y", 1000, 1, Some(42)).await; // single sample

    let pcts = latency_percentiles_all(&pool, 0).await.unwrap();
    // rank = ceil(p*n/100): p50 -> 5th = 50, p95 & p99 -> 10th = 100.
    assert_eq!(pcts["x"], (50, 100, 100));
    // n = 1: every percentile is the only value.
    assert_eq!(pcts["y"], (42, 42, 42));
}

#[tokio::test]
async fn latency_sparkline_buckets_and_averages() {
    let pool = memory_pool().await;
    insert(&pool, "m", 0, 1, Some(10)).await;
    insert(&pool, "m", 50, 1, Some(30)).await; // same 100s bucket as t=0 -> avg 20
    insert(&pool, "m", 150, 1, Some(80)).await; // next bucket
    insert(&pool, "m", 120, 0, None).await; // no latency -> ignored

    let spark = latency_sparkline_all(&pool, 0, 100).await.unwrap();
    let points = &spark["m"];
    assert_eq!(points.len(), 2);
    assert_eq!(points[0].latency_ms, 20); // (10 + 30) / 2
    assert_eq!(points[1].latency_ms, 80);

    // Buckets are anchored to the epoch, not to `since`: moving the window
    // start must not regroup the rows (t=90 and t=110 straddle an epoch
    // boundary, but would share a bucket anchored at t=50).
    insert(&pool, "n", 90, 1, Some(10)).await;
    insert(&pool, "n", 110, 1, Some(30)).await;
    let shifted = latency_sparkline_all(&pool, 50, 100).await.unwrap();
    let points: Vec<i64> = shifted["n"].iter().map(|p| p.latency_ms).collect();
    assert_eq!(points, vec![10, 30]);
}

#[tokio::test]
async fn iso_day_matches_sqlite_strftime() {
    let pool = memory_pool().await;
    for secs in [0_i64, 86_399, 86_400, 951_782_400, 1_789_695_050] {
        let sqlite: String = sqlx::query_scalar("SELECT strftime('%Y-%m-%d', ?, 'unixepoch')")
            .bind(secs)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(iso_day(secs / SECONDS_PER_DAY), sqlite, "{secs}");
    }
}

#[tokio::test]
async fn daily_aggregates_by_utc_day() {
    let pool = memory_pool().await;
    let day0 = 1_609_459_200; // 2021-01-01 00:00:00 UTC
    insert(&pool, "m", day0, 1, Some(5)).await;
    insert(&pool, "m", day0 + 10, 0, None).await;
    insert(&pool, "m", day0 + SECONDS_PER_DAY, 1, Some(7)).await;

    let rows = &daily_all(&pool, 0, day0 + 2 * SECONDS_PER_DAY)
        .await
        .unwrap()["m"];
    assert_eq!(rows.len(), 2);
    assert_eq!(
        (rows[0].day.as_str(), rows[0].up, rows[0].down),
        ("2021-01-01", 1, 1)
    );
    assert_eq!((rows[1].day.as_str(), rows[1].up), ("2021-01-02", 1));
}

#[tokio::test]
async fn latency_hourly_reads_buckets_below_the_frontier_and_raw_above() {
    let pool = memory_pool().await;
    let hour0 = 10 * 86_400;
    let hour1 = hour0 + 3600;

    // hour0 is rolled up (with a doctored average) and still has raw rows;
    // hour1 is raw only; an older hour is a bucket only.
    sqlx::query(
        "INSERT INTO checks_hourly \
                (monitor_id, hour, up_count, down_count, degraded_count, avg_latency_ms) \
             VALUES ('m', ?, 10, 0, 0, 999), ('m', ?, 5, 0, 0, 50)",
    )
    .bind(hour0)
    .bind(hour0 - 3600)
    .execute(&pool)
    .await
    .unwrap();
    insert(&pool, "m", hour0 + 10, 1, Some(100)).await;
    insert(&pool, "m", hour0 + 20, 1, Some(200)).await;
    insert(&pool, "m", hour1 + 10, 1, Some(300)).await;

    let cells = latency_hourly(&pool, "m", 0).await.unwrap();
    // Oldest first; a rolled-up hour answers from its bucket (the raw rows
    // below the frontier are never scanned), the hour above it from raw.
    assert_eq!(cells, vec![(hour0 - 3600, 50), (hour0, 999), (hour1, 300)]);

    // `since` trims the window.
    let recent = latency_hourly(&pool, "m", hour0).await.unwrap();
    assert_eq!(recent.len(), 2);
}

#[tokio::test]
async fn cert_upsert_roundtrip() {
    let pool = memory_pool().await;
    assert_eq!(cert_not_after(&pool, "m").await.unwrap(), None);
    upsert_cert(&pool, "m", 1000, 1).await.unwrap();
    upsert_cert(&pool, "m", 2000, 2).await.unwrap();
    assert_eq!(cert_not_after(&pool, "m").await.unwrap(), Some(2000));
}

#[tokio::test]
async fn batch_reads_group_by_monitor() {
    let pool = memory_pool().await;
    insert(&pool, "a", 100, 1, Some(10)).await;
    insert(&pool, "a", 200, 0, None).await; // down, no latency
    insert(&pool, "b", 150, 1, Some(20)).await;
    upsert_cert(&pool, "a", 5000, 1).await.unwrap();

    let availability = availability_all(&pool, 0).await.unwrap();
    assert_eq!(availability.get("a"), Some(&(1, 2))); // 1 up of 2
    assert_eq!(availability.get("b"), Some(&(1, 1)));

    let daily = daily_all(&pool, 0, 300).await.unwrap();
    assert!(daily.contains_key("a") && daily.contains_key("b"));

    let certs = cert_all(&pool).await.unwrap();
    assert_eq!(certs.get("a"), Some(&5000));
    assert!(!certs.contains_key("b"));
}

#[tokio::test]
async fn prune_removes_orphans_and_expired_rows() {
    let pool = memory_pool().await;
    let now = chrono::Utc::now().timestamp();

    insert(&pool, "keep", now - 10, 1, Some(5)).await; // recent, retained
    insert(&pool, "keep", now - 200 * SECONDS_PER_DAY, 1, Some(5)).await; // beyond retention
    insert(&pool, "gone", now - 10, 1, Some(5)).await; // monitor removed from config
    upsert_cert(&pool, "gone", now + 1000, now).await.unwrap();
    meta_set(&pool, "heartbeat_expected_since:gone", "5")
        .await
        .unwrap();
    meta_set(&pool, "heartbeat_expected_since:keep", "5")
        .await
        .unwrap();
    // A push monitor removed before it ever pinged: no rows, only state.
    meta_set(&pool, "heartbeat_expected_since:never", "5")
        .await
        .unwrap();

    let config: Config = toml::from_str(
        r#"
            [page]
            [server]
            [[monitors]]
            id = "keep"
            name = "Keep"
            target = "https://example.com"
            interval_secs = 60
        "#,
    )
    .unwrap();

    prune(&pool, &config).await.unwrap();

    // The retained monitor keeps only its recent row.
    assert_eq!(latency_series(&pool, "keep", 0, 1).await.unwrap().len(), 1);

    // The orphan is only scheduled for deletion: its data survives the
    // grace period, and the first sighting is recorded.
    assert_eq!(recent_checks(&pool, "gone", 10).await.unwrap().len(), 1);
    let mark = meta_get(&pool, "orphan_since:gone").await.unwrap();
    assert!(mark.is_some_and(|at| at.parse::<i64>().unwrap() >= now));
    let expected = |id: &str| {
        let pool = pool.clone();
        let key = format!("heartbeat_expected_since:{id}");
        async move { meta_get(&pool, &key).await.unwrap() }
    };
    assert!(expected("gone").await.is_some(), "kept through the grace");
    assert!(expected("keep").await.is_some());
    assert_eq!(expected("never").await, None, "nothing to wait for");

    // A week later the sweep deletes it entirely, mark included.
    delete_orphans(&pool, &config, now + ORPHAN_GRACE_SECS + 60)
        .await
        .unwrap();
    assert!(recent_checks(&pool, "gone", 10).await.unwrap().is_empty());
    assert_eq!(cert_not_after(&pool, "gone").await.unwrap(), None);
    assert_eq!(meta_get(&pool, "orphan_since:gone").await.unwrap(), None);
    assert_eq!(
        expected("gone").await,
        None,
        "per-id meta goes with the rows"
    );
    assert!(expected("keep").await.is_some());
    assert_eq!(recent_checks(&pool, "keep", 10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn an_orphan_that_comes_back_keeps_its_history() {
    let pool = memory_pool().await;
    let now = 1_000_000;
    insert(&pool, "api", now - 10, 1, Some(5)).await;
    let config_with = |ids: &[&str]| -> Config {
        let [id] = ids else { unreachable!() };
        toml::from_str(&format!(
            "[page]\n[server]\n[[monitors]]\nid = \"{id}\"\nname = \"{id}\"\n\
                 target = \"https://example.com\"\ninterval_secs = 60\n"
        ))
        .unwrap()
    };

    // Renamed away: marked, not deleted.
    delete_orphans(&pool, &config_with(&["other"]), now)
        .await
        .unwrap();
    assert!(meta_get(&pool, "orphan_since:api").await.unwrap().is_some());

    // Back before the grace ran out: the mark is cleared, the data kept.
    delete_orphans(&pool, &config_with(&["api"]), now + 3600)
        .await
        .unwrap();
    assert_eq!(meta_get(&pool, "orphan_since:api").await.unwrap(), None);

    // Removed again much later: a fresh grace period starts then, so the
    // old (cleared) sighting can never shortcut it.
    let later = now + 30 * SECONDS_PER_DAY;
    delete_orphans(&pool, &config_with(&["other"]), later)
        .await
        .unwrap();
    delete_orphans(&pool, &config_with(&["other"]), later + 3600)
        .await
        .unwrap();
    assert_eq!(recent_checks(&pool, "api", 10).await.unwrap().len(), 1);
    assert_eq!(
        meta_get(&pool, "orphan_since:api").await.unwrap(),
        Some(later.to_string())
    );
}

#[test]
fn upsert_sql_spells_out_every_column() {
    assert_eq!(
        upsert_sql!("certs", "monitor_id", "not_after", "checked_at"),
        "INSERT INTO certs (monitor_id, not_after, checked_at) VALUES (?, ?, ?) \
             ON CONFLICT(monitor_id) DO UPDATE SET \
             not_after = excluded.not_after, checked_at = excluded.checked_at"
    );
    assert_eq!(
        upsert_sql!("meta", "key", "value"),
        "INSERT INTO meta (key, value) VALUES (?, ?) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value"
    );
}

#[tokio::test]
async fn incidents_between_selects_every_overlap() {
    let pool = memory_pool().await;
    let span = |monitor: &str, started: i64, ended: Option<i64>| {
        sqlx::query(
            "INSERT INTO incidents (monitor_id, started_at, ended_at, created_at) \
                 VALUES (?, ?, ?, ?)",
        )
        .bind(monitor.to_owned())
        .bind(started)
        .bind(ended)
        .bind(started)
    };
    span("before", 10, Some(90)).execute(&pool).await.unwrap();
    span("straddles-start", 50, Some(150))
        .execute(&pool)
        .await
        .unwrap();
    span("inside", 120, Some(130)).execute(&pool).await.unwrap();
    span("open", 140, None).execute(&pool).await.unwrap();
    span("straddles-end", 190, Some(260))
        .execute(&pool)
        .await
        .unwrap();
    span("after", 200, Some(210)).execute(&pool).await.unwrap();

    let ids: Vec<String> = incidents_between(&pool, 100, 200)
        .await
        .unwrap()
        .into_iter()
        .map(|incident| incident.monitor_id)
        .collect();
    // Newest first; ended exactly at `since` or started at `until` is out.
    assert_eq!(ids, ["straddles-end", "open", "inside", "straddles-start"]);
}

#[tokio::test]
async fn incident_marks_give_the_last_end_and_the_open_start() {
    let pool = memory_pool().await;
    for (monitor, started, ended) in [
        ("calm", 10, Some(20)),
        ("calm", 30, Some(45)),
        ("down", 10, Some(20)),
        ("down", 50, None),
        ("first", 70, None),
    ] {
        sqlx::query(
            "INSERT INTO incidents (monitor_id, started_at, ended_at, created_at) \
                 VALUES (?, ?, ?, ?)",
        )
        .bind(monitor)
        .bind(started)
        .bind(ended)
        .bind(started)
        .execute(&pool)
        .await
        .unwrap();
    }
    let marks = super::incident_marks(&pool).await.unwrap();
    let mark = |last_end, open_since| super::IncidentMarks {
        last_end,
        open_since,
    };
    assert_eq!(marks["calm"], mark(Some(45), None));
    assert_eq!(marks["down"], mark(Some(20), Some(50)));
    assert_eq!(marks["first"], mark(None, Some(70)));
    assert!(!marks.contains_key("never"));

    // One monitor's own incidents, newest first, from `since`.
    let starts: Vec<i64> = super::monitor_incidents(&pool, "calm", 20, 10)
        .await
        .unwrap()
        .iter()
        .map(|incident| incident.started_at)
        .collect();
    assert_eq!(starts, [30]);
    assert_eq!(
        super::monitor_incidents(&pool, "down", 0, 1).await.unwrap()[0].started_at,
        50
    );
}

#[tokio::test]
async fn daily_all_ignores_data_past_until() {
    let pool = memory_pool().await;
    let day0 = 10 * SECONDS_PER_DAY;
    insert(&pool, "m", day0 + 10, 1, Some(5)).await;
    insert(&pool, "m", day0 + SECONDS_PER_DAY + 10, 0, None).await;
    downsample_hourly(&pool, day0 + 3 * SECONDS_PER_DAY)
        .await
        .unwrap();
    insert(&pool, "m", day0 + 3 * SECONDS_PER_DAY + 10, 0, None).await;

    // Only the first day: neither the later bucket nor the raw tail.
    let bars = daily_all(&pool, 0, day0 + SECONDS_PER_DAY - 1)
        .await
        .unwrap();
    assert_eq!(bars["m"].len(), 1);
    assert_eq!((bars["m"][0].up, bars["m"][0].down), (1, 0));
}

#[tokio::test]
async fn downsampling_is_write_once_and_only_buckets_complete_hours() {
    let pool = memory_pool().await;
    let hour0 = 10 * 86400; // aligned on both an hour and a day
    let hour1 = hour0 + 3600;

    insert(&pool, "m", hour0 + 10, 1, Some(100)).await;
    insert(&pool, "m", hour0 + 20, 1, Some(200)).await;
    insert(&pool, "m", hour0 + 30, 0, None).await;
    insert(&pool, "m", hour1 + 10, 1, Some(300)).await;

    // Cutoff inside hour1: only hour0 is entirely below it, so only hour0
    // is bucketed.
    downsample_hourly(&pool, hour1 + 1800).await.unwrap();
    let buckets = sqlx::query_as::<_, (i64, i64, i64, i64)>(
        "SELECT hour, up_count, down_count, avg_latency_ms FROM checks_hourly ORDER BY hour",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(buckets, vec![(hour0, 2, 1, 150)]);

    // Write-once: a late raw row never rewrites an existing bucket.
    insert(&pool, "m", hour0 + 40, 0, None).await;
    downsample_hourly(&pool, hour1 + 3600).await.unwrap();
    let buckets = sqlx::query_as::<_, (i64, i64, i64, i64)>(
        "SELECT hour, up_count, down_count, avg_latency_ms FROM checks_hourly ORDER BY hour",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(buckets, vec![(hour0, 2, 1, 150), (hour1, 1, 0, 300)]);

    // Daily roll-up weights the latency average by sample count:
    // (150 * 2 + 300 * 1) / 3 = 200.
    downsample_daily(&pool, hour0 + 86400).await.unwrap();
    let days = sqlx::query_as::<_, (i64, i64, i64, i64)>(
        "SELECT day, up_count, down_count, avg_latency_ms FROM checks_daily",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(days, vec![(hour0, 3, 1, 200)]);
}

#[tokio::test]
async fn downsampling_only_scans_above_the_newest_bucket() {
    let pool = memory_pool().await;
    let hour0 = 10 * 86400;
    let hour1 = hour0 + 3600;

    // A bucket already exists for hour1: the floor for the next run.
    insert(&pool, "m", hour1 + 10, 1, Some(100)).await;
    downsample_hourly(&pool, hour1 + 3600).await.unwrap();

    // Raw rows below the floor - a backdated write, which live recording
    // never produces - are outside the incremental scan and stay
    // unbucketed; a second monitor's rows above the floor still roll up
    // (the floor is global, monotonic time covers every monitor).
    insert(&pool, "m", hour0 + 10, 1, Some(999)).await;
    insert(&pool, "other", hour1 + 7200 + 10, 0, None).await;
    downsample_hourly(&pool, hour1 + 3 * 3600).await.unwrap();

    let buckets = sqlx::query_as::<_, (String, i64)>(
        "SELECT monitor_id, hour FROM checks_hourly ORDER BY hour",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        buckets,
        vec![("m".to_owned(), hour1), ("other".to_owned(), hour1 + 7200)]
    );
}

#[tokio::test]
async fn daily_all_reads_aggregates_once_raw_is_pruned() {
    let pool = memory_pool().await;
    let hour0 = 10 * 86400;

    insert(&pool, "m", hour0 + 10, 1, Some(100)).await;
    insert(&pool, "m", hour0 + 20, 0, None).await;
    downsample_hourly(&pool, hour0 + 3600).await.unwrap();

    // Raw still present: counts come from it (and agree with the bucket).
    let day_key = "1970-01-11";
    let bars = daily_all(&pool, 0, hour0 + 3600).await.unwrap();
    assert_eq!(bars["m"][0].day, day_key);
    assert_eq!((bars["m"][0].up, bars["m"][0].down), (1, 1));

    // Raw pruned: the hourly bucket transparently takes over.
    sqlx::query("DELETE FROM checks")
        .execute(&pool)
        .await
        .unwrap();
    let bars = daily_all(&pool, 0, hour0 + 3600).await.unwrap();
    assert_eq!(bars["m"][0].day, day_key);
    assert_eq!((bars["m"][0].up, bars["m"][0].down), (1, 1));
}

#[tokio::test]
async fn daily_all_adds_the_raw_tail_to_the_rolled_up_hours() {
    let pool = memory_pool().await;
    let day0 = 10 * 86400;

    // 01:00 is rolled up, 05:00 is still raw: the same day is split across
    // both sources, and the bar must count both halves.
    insert(&pool, "m", day0 + 3600 + 10, 1, Some(100)).await;
    downsample_hourly(&pool, day0 + 2 * 3600).await.unwrap();
    insert(&pool, "m", day0 + 5 * 3600 + 10, 0, None).await;
    let now = day0 + 6 * 3600;
    let bars = daily_all(&pool, 0, now).await.unwrap();
    assert_eq!((bars["m"][0].up, bars["m"][0].down), (1, 1));

    // Rolling the tail up moves it to the buckets without double counting.
    downsample_hourly(&pool, now).await.unwrap();
    let bars = daily_all(&pool, 0, now).await.unwrap();
    assert_eq!((bars["m"][0].up, bars["m"][0].down), (1, 1));
}

#[tokio::test]
async fn daily_all_reads_old_days_from_buckets_not_raw() {
    let pool = memory_pool().await;
    let now = 100 * 86400;
    let old = now - 30 * SECONDS_PER_DAY; // well past the raw window
    let recent = now - 3600; // inside the raw window

    // The old day exists as raw rows *and* a (complete) hourly bucket; the
    // bucket must be the source consulted - raw that old is never scanned.
    // The doctored bucket disagrees with raw at the *same* sample count, so
    // if raw were still read it would win the tie (first writer) and the
    // day would come out down instead.
    insert(&pool, "m", old, 0, None).await;
    downsample_hourly(&pool, old + 3600).await.unwrap();
    sqlx::query("UPDATE checks_hourly SET up_count = 1, down_count = 0")
        .execute(&pool)
        .await
        .unwrap();
    insert(&pool, "m", recent, 1, Some(10)).await;

    let bars = daily_all(&pool, 0, now).await.unwrap();
    assert_eq!(bars["m"].len(), 2);
    // Old day: the doctored bucket answers, proving raw was not read.
    assert_eq!((bars["m"][0].up, bars["m"][0].down), (1, 0));
    // Recent day: raw is still authoritative inside the window.
    assert_eq!((bars["m"][1].up, bars["m"][1].down), (1, 0));
}

#[tokio::test]
async fn recent_checks_carry_the_failure_reason() {
    let pool = memory_pool().await;
    insert(&pool, "m", 100, 1, Some(10)).await;
    sqlx::query(
        "INSERT INTO checks (time, monitor_id, status, latency_ms, status_code, error) \
             VALUES (200, 'm', 0, NULL, 522, 'HTTP 522: origin timeout')",
    )
    .execute(&pool)
    .await
    .unwrap();

    let recent = recent_checks(&pool, "m", 2).await.unwrap();
    assert_eq!(recent[0].error.as_deref(), Some("HTTP 522: origin timeout"));
    assert_eq!(recent[1].error, None);
}

#[tokio::test]
async fn incidents_open_close_and_prune() {
    let pool = memory_pool().await;

    let id = insert_incident_start(
        &pool,
        "m",
        Some("boom"),
        None,
        &["a".to_owned()],
        Some("HTTP/2 503\n\n<html>maintenance</html>"),
        Some("deploy api v2.3, 3m before"),
    )
    .await
    .unwrap();
    assert_eq!(find_open_incident(&pool, "m").await.unwrap(), Some(id));

    update_incident_end(&pool, id).await.unwrap();
    assert_eq!(find_open_incident(&pool, "m").await.unwrap(), None);

    let incidents = recent_incidents(&pool, 10).await.unwrap();
    assert_eq!(incidents.len(), 1);
    assert_eq!(incidents[0].error.as_deref(), Some("boom"));
    assert_eq!(
        incidents[0].snapshot.as_deref(),
        Some("HTTP/2 503\n\n<html>maintenance</html>")
    );
    assert_eq!(
        incidents[0].event.as_deref(),
        Some("deploy api v2.3, 3m before")
    );
    assert!(incidents[0].duration_s.is_some());

    // The vantage verdict lands after the peers answered; readable back.
    update_incident_vantage(&pool, id, "confirmed down from 2/2 vantage points")
        .await
        .unwrap();
    assert_eq!(
        incident_by_id(&pool, id)
            .await
            .unwrap()
            .expect("incident exists")
            .vantage
            .as_deref(),
        Some("confirmed down from 2/2 vantage points")
    );
    assert!(incident_by_id(&pool, 999).await.unwrap().is_none());

    // Closed incidents prune by age; open ones never do.
    let open = insert_incident_start(&pool, "m", None, None, &[], None, None)
        .await
        .unwrap();
    prune_incidents(&pool, chrono::Utc::now().timestamp() + 1000)
        .await
        .unwrap();
    let incidents = recent_incidents(&pool, 10).await.unwrap();
    assert_eq!(incidents.len(), 1);
    assert_eq!(incidents[0].id, open);
}

#[tokio::test]
async fn incident_notes_set_clear_and_resolve_last() {
    let pool = memory_pool().await;
    let first = insert_incident_start(&pool, "m", None, None, &[], None, None)
        .await
        .unwrap();
    let second = insert_incident_start(&pool, "m", None, None, &[], None, None)
        .await
        .unwrap();

    // `last` resolves to the most recently started incident.
    assert_eq!(latest_incident_id(&pool).await.unwrap(), Some(second));

    assert!(set_incident_note(&pool, first, "fiber cut").await.unwrap());
    let incidents = recent_incidents(&pool, 10).await.unwrap();
    let annotated = incidents.iter().find(|i| i.id == first).unwrap();
    assert_eq!(annotated.note.as_deref(), Some("fiber cut"));

    // An empty note clears; an unknown id reports "not found".
    assert!(set_incident_note(&pool, first, "").await.unwrap());
    let incidents = recent_incidents(&pool, 10).await.unwrap();
    assert_eq!(incidents.iter().find(|i| i.id == first).unwrap().note, None);
    assert!(!set_incident_note(&pool, 999, "nope").await.unwrap());
}

#[tokio::test]
async fn events_record_list_correlate_and_prune() {
    let pool = memory_pool().await;
    let now = chrono::Utc::now().timestamp();

    let first = insert_event(&pool, "deploy api v2.3").await.unwrap();
    let second = insert_event(&pool, "config rollout").await.unwrap();

    // Newest first for the list; oldest first for the chart overlay.
    let recent = recent_events(&pool, 10).await.unwrap();
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0].id, second);
    assert_eq!(recent[0].title, "config rollout");
    let overlay = events_since(&pool, now - 60).await.unwrap();
    assert_eq!(overlay[0].id, first);

    // Correlation picks the most recent event inside the window only.
    let correlated = latest_event_before(&pool, now, 3600).await.unwrap();
    assert_eq!(correlated.expect("in window").id, second);
    let outside = latest_event_before(&pool, now + 7200, 3600).await.unwrap();
    assert!(
        outside.is_none(),
        "events older than the window never match"
    );

    // Old markers age out with the pruner.
    prune_events(&pool, now + 1000).await.unwrap();
    assert!(recent_events(&pool, 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn announcements_pin_expire_and_clear() {
    let pool = memory_pool().await;
    let now = 1000;

    let id = insert_announcement(&pool, "Fiber cut", "ETA 6pm", "warning", Some(now + 600))
        .await
        .unwrap();
    insert_announcement(&pool, "Maintenance done", "", "resolved", None)
        .await
        .unwrap();

    // Newest first; both active.
    let pinned = active_announcements(&pool, now).await.unwrap();
    assert_eq!(pinned.len(), 2);
    assert_eq!(pinned[0].title, "Maintenance done");
    assert_eq!(pinned[1].id, id);
    assert_eq!(pinned[1].severity, "warning");

    // The bounded one expires on its own; the unbounded one stays.
    let later = active_announcements(&pool, now + 601).await.unwrap();
    assert_eq!(later.len(), 1);
    assert_eq!(later[0].title, "Maintenance done");

    // The pruner drops only the expired row.
    prune_announcements(&pool, now + 700).await.unwrap();
    assert_eq!(active_announcements(&pool, 0).await.unwrap().len(), 1);

    assert_eq!(clear_announcements(&pool, now).await.unwrap(), 1);
    assert!(active_announcements(&pool, now).await.unwrap().is_empty());
}

#[tokio::test]
async fn silences_cover_wildcard_expire_and_clear() {
    let pool = memory_pool().await;
    let now = 1000;

    insert_silence(&pool, "api", now + 600, Some("deploying"))
        .await
        .unwrap();
    assert!(is_silenced(&pool, "api", now).await.unwrap());
    assert!(!is_silenced(&pool, "web", now).await.unwrap());
    // Expiry is a hard edge: at `until` the silence is over.
    assert!(!is_silenced(&pool, "api", now + 600).await.unwrap());

    // The wildcard covers every monitor.
    insert_silence(&pool, "*", now + 300, None).await.unwrap();
    assert!(is_silenced(&pool, "web", now).await.unwrap());

    let active = active_silences(&pool, now).await.unwrap();
    assert_eq!(active.len(), 2);
    // Soonest to expire first.
    assert_eq!(active[0].monitor_id, "*");
    assert_eq!(active[1].reason.as_deref(), Some("deploying"));

    // The expired row is swept by the pruner; the active ones survive.
    prune_silences(&pool, now + 450).await.unwrap();
    assert_eq!(active_silences(&pool, 0).await.unwrap().len(), 1);

    assert_eq!(clear_silences(&pool, now).await.unwrap(), 1);
    assert!(!is_silenced(&pool, "api", now).await.unwrap());
}

#[tokio::test]
async fn backup_into_snapshots_and_refuses_overwrite() {
    let dir = std::env::temp_dir();
    let src = dir.join(format!("hora-test-backup-src-{}.db", std::process::id()));
    let dest = dir.join(format!("hora-test-backup-dest-{}.db", std::process::id()));
    for path in [&src, &dest] {
        let _ = std::fs::remove_file(path);
    }
    let src_s = src.to_str().unwrap();
    let dest_s = dest.to_str().unwrap();

    let pool = connect(src_s).await.unwrap();
    insert(&pool, "m", 100, 1, Some(10)).await;
    backup_into(src_s, dest_s).await.unwrap();

    // Owner-only, like the live database (created so, never chmod-ed).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&dest).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "{mode:o}");
    }

    // A failed copy leaves no empty placeholder behind.
    let missing = dir.join(format!("hora-test-backup-none-{}.db", std::process::id()));
    let failed = dir.join(format!("hora-test-backup-fail-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&failed);
    assert!(
        backup_into(missing.to_str().unwrap(), failed.to_str().unwrap())
            .await
            .is_err()
    );
    assert!(!failed.exists());

    // The snapshot is a self-sufficient database with the data.
    let copy = connect(dest_s).await.unwrap();
    assert_eq!(recent_checks(&copy, "m", 10).await.unwrap().len(), 1);
    copy.close().await;

    // An existing destination is never overwritten.
    let err = backup_into(src_s, dest_s).await.unwrap_err();
    assert!(err.to_string().contains("already exists"), "{err}");

    pool.close().await;
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{src_s}{suffix}"));
        let _ = std::fs::remove_file(format!("{dest_s}{suffix}"));
    }
}

// --- Rolling windows: roll-ups + raw edges must equal the raw-only reads ---

/// A deterministic spread of checks for `monitors` over `[start, end)`: a
/// sample every `step` seconds, mostly up with varied latency, some
/// degraded and down (down checks without latency), so every aggregate has
/// something to count.
async fn seed_checks(pool: &SqlitePool, monitors: &[&str], start: i64, end: i64, step: i64) {
    let mut state: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut tx = pool.begin().await.unwrap();
    let mut time = start;
    while time < end {
        for id in monitors {
            let roll = next() % 100;
            let (status, latency) = match roll {
                0..=2 => (0, None),
                3..=7 => (2, Some(1500 + i64::try_from(next() % 900).unwrap())),
                _ => (1, Some(20 + i64::try_from(next() % 400).unwrap())),
            };
            sqlx::query(
                "INSERT INTO checks (time, monitor_id, status, latency_ms) VALUES (?, ?, ?, ?)",
            )
            .bind(time + i64::try_from(next() % 5).unwrap())
            .bind(id)
            .bind(status)
            .bind(latency)
            .execute(&mut *tx)
            .await
            .unwrap();
        }
        time += step;
    }
    tx.commit().await.unwrap();
}

/// The window path must agree with the raw-only reads: availability exactly,
/// percentiles within the histogram's 1/64 bound.
async fn assert_window_matches_raw(pool: &SqlitePool, since: i64) {
    let stats = window_stats_all(pool, since).await.unwrap();
    let availability = availability_all(pool, since).await.unwrap();
    let percentiles = latency_percentiles_all(pool, since).await.unwrap();
    assert_eq!(stats.len(), availability.len());
    for (id, &(available, total)) in &availability {
        let window = &stats[id];
        assert_eq!((window.available, window.total), (available, total), "{id}");
        let (p50, p95, p99) = window.latency.p50_p95_p99().expect("samples");
        let (e50, e95, e99) = percentiles[id];
        for (got, want) in [(p50, e50), (p95, e95), (p99, e99)] {
            assert!((got - want).abs() * 64 <= want, "{id}: {got} vs {want}");
        }
    }
}

#[tokio::test]
async fn window_stats_equal_the_raw_reads_across_roll_ups() {
    let pool = memory_pool().await;
    let day0 = 100 * SECONDS_PER_DAY;
    let now = day0 + 30 * 3600 + 1234;
    seed_checks(&pool, &["a", "b", "c"], day0, now, 60).await;
    // A mid-hour window start, so the leading partial hour is read raw.
    let since = now - SECONDS_PER_DAY;

    // Nothing rolled up: everything raw.
    assert_window_matches_raw(&pool, since).await;
    // Rolled up to two hours ago: buckets plus raw edges.
    roll_up_recent(&pool, now - 7200).await.unwrap();
    assert_window_matches_raw(&pool, since).await;
    // Everything that can be: only the current hour stays raw.
    roll_up_recent(&pool, now).await.unwrap();
    assert_window_matches_raw(&pool, since).await;
    // Every bucket in the window carries a histogram now.
    let missing: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM checks_hourly WHERE hour >= ? AND latency_hist IS NULL",
    )
    .bind(since - 3600)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(missing, 0);

    // Buckets rolled up before histograms existed: the window reads their
    // latency raw, and the next roll-up backfills them.
    sqlx::query("UPDATE checks_hourly SET latency_hist = NULL WHERE hour % 7200 = 0")
        .execute(&pool)
        .await
        .unwrap();
    assert_window_matches_raw(&pool, since).await;
    fill_latency_histograms(&pool, since - 3600).await.unwrap();
    assert_window_matches_raw(&pool, since).await;
}

#[tokio::test]
async fn histograms_cover_every_bucket_even_without_latency() {
    let pool = memory_pool().await;
    let hour0 = 10 * 86_400;
    insert(&pool, "fast", hour0 + 10, 1, Some(40)).await;
    insert(&pool, "fast", hour0 + 20, 1, Some(40)).await;
    insert(&pool, "dead", hour0 + 10, 0, None).await;
    roll_up_recent(&pool, hour0 + 3600 + 300).await.unwrap();

    let rows = sqlx::query_as::<_, (String, Vec<u8>)>(
        "SELECT monitor_id, latency_hist FROM checks_hourly ORDER BY monitor_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let decoded: Vec<(String, u64)> = rows
        .into_iter()
        .map(|(id, bytes)| {
            let hist = crate::histogram::LatencyHistogram::decode(&bytes).unwrap();
            (id, hist.len())
        })
        .collect();
    assert_eq!(
        decoded,
        vec![("dead".to_owned(), 0), ("fast".to_owned(), 2)]
    );
}

#[tokio::test]
async fn sparkline_cache_equals_the_one_shot_read_as_time_moves() {
    let pool = memory_pool().await;
    let day0 = 100 * SECONDS_PER_DAY;
    let bucket = 720;
    let mut cache = SparklineCache::default();
    let mut now = day0 + 26 * 3600;
    seed_checks(&pool, &["a", "b"], day0, now, 45).await;
    for step in 0..12 {
        let since = now - SECONDS_PER_DAY;
        let cached = cache.refresh(&pool, since, bucket, now).await.unwrap();
        let direct = latency_sparkline_all(&pool, since, bucket).await.unwrap();
        assert_eq!(cached.len(), direct.len(), "step {step}");
        for (id, points) in &direct {
            let got: Vec<(i64, i64)> = cached[id].iter().map(|p| (p.t, p.latency_ms)).collect();
            let want: Vec<(i64, i64)> = points.iter().map(|p| (p.t, p.latency_ms)).collect();
            assert_eq!(got, want, "step {step}, {id}");
        }
        // Time moves by uneven strides (inside a bucket, across several),
        // and new checks keep arriving.
        let stride = [5, 400, 30, 2000, 719, 721][step % 6];
        seed_checks(&pool, &["a", "b"], now, now + stride, 45).await;
        now += stride;
    }
    // Another bucket width starts over.
    let since = now - SECONDS_PER_DAY;
    let cached = cache.refresh(&pool, since, 600, now).await.unwrap();
    let direct = latency_sparkline_all(&pool, since, 600).await.unwrap();
    assert_eq!(cached["a"].len(), direct["a"].len());
}

#[tokio::test]
async fn daily_cache_equals_daily_all_as_roll_ups_and_days_advance() {
    let pool = memory_pool().await;
    let day0 = 100 * SECONDS_PER_DAY;
    let mut now = day0 + 5 * SECONDS_PER_DAY + 7 * 3600;
    seed_checks(&pool, &["a", "b"], day0, now, 300).await;
    roll_up_recent(&pool, now).await.unwrap();
    // Older days live in daily buckets only, as after the 90-day roll-up.
    downsample_daily(&pool, day0 + 2 * SECONDS_PER_DAY)
        .await
        .unwrap();
    prune_hourly(&pool, day0 + SECONDS_PER_DAY).await.unwrap();

    let mut cache = DailyCache::default();
    for step in 0..10 {
        let since = now - 4 * SECONDS_PER_DAY - 1800;
        let cached = cache.refresh(&pool, since, now).await.unwrap();
        let direct = daily_all(&pool, since, now).await.unwrap();
        assert_eq!(cached.len(), direct.len(), "step {step}");
        for (id, rows) in &direct {
            let got: Vec<_> = cached[id]
                .iter()
                .map(|r| (r.day.clone(), r.up, r.down, r.degraded))
                .collect();
            let want: Vec<_> = rows
                .iter()
                .map(|r| (r.day.clone(), r.up, r.down, r.degraded))
                .collect();
            assert_eq!(got, want, "step {step}, {id}");
        }
        // Hours of new checks, rolled up on some steps only, and a step
        // that crosses midnight.
        let stride = [1800, 3 * 3600, 600, 20 * 3600][step % 4];
        seed_checks(&pool, &["a", "b"], now, now + stride, 300).await;
        now += stride;
        if step % 2 == 0 {
            roll_up_recent(&pool, now).await.unwrap();
        }
    }
}

#[tokio::test]
async fn window_cache_equals_the_one_shot_read_as_time_moves() {
    let pool = memory_pool().await;
    let day0 = 100 * SECONDS_PER_DAY;
    let mut now = day0 + 26 * 3600 + 1234;
    seed_checks(&pool, &["a", "b"], day0, now, 30).await;
    roll_up_recent(&pool, now - 3 * 3600).await.unwrap();
    // Some hours predate histograms.
    sqlx::query("UPDATE checks_hourly SET latency_hist = NULL WHERE hour % 10800 = 0")
        .execute(&pool)
        .await
        .unwrap();

    let mut cache = WindowCache::default();
    for step in 0..16 {
        let since = now - SECONDS_PER_DAY;
        let cached = cache.refresh(&pool, since, now).await.unwrap();
        let direct = window_stats_all(&pool, since).await.unwrap();
        assert_eq!(cached, direct, "step {step}");
        // Strides inside the hour, across it, and past the tail's lag; new
        // checks keep arriving and roll-ups land on some steps.
        let stride = [5, 61, 900, 3600, 2, 4000][step % 6];
        seed_checks(&pool, &["a", "b"], now, now + stride, 30).await;
        now += stride;
        if step % 3 == 1 {
            roll_up_recent(&pool, now).await.unwrap();
        }
    }
}
