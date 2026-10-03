//! The scheduler: one independent probing loop per monitor, plus alert state.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use hora_notify::Event;
use reqwest::Client;
use sqlx::SqlitePool;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::coalesce::{AlertMsg, DownAlert};
use crate::config::{Config, Kind, Monitor};
use crate::notifications::Notifiers;
use crate::probe::Outcome;
use crate::topology;
use crate::{db, probe, slo};

/// The level a monitor (or watched peer) was most recently alerted at, so we
/// never re-alert the same state and can detect transitions (escalation,
/// recovery).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AlertLevel {
    #[default]
    Healthy,
    Degraded,
    Down,
    /// Peer watches only: down locally but a witness still sees the peer, a
    /// partition reported once as a link-degraded event, not an outage.
    Partition,
    /// Peer watches only: down locally and no witness reachable, so probably
    /// *this* node is isolated - nothing was announced.
    Isolated,
}

/// Edge-triggered burn-rate alert state: each severity fires once when its
/// window pair first exceeds the threshold and re-arms when the long window
/// cools back down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct BurnAlerts {
    fast: bool,
    slow: bool,
}

impl BurnAlerts {
    fn any(self) -> bool {
        self.fast || self.slow
    }
}

/// The anti-flap alert state machine, shared by monitor loops and peer
/// watches: consecutive-failure counters against the threshold, plus the level
/// last announced. Pure (no I/O), so the transitions are unit-testable; the
/// callers perform the side effect and then record the new [`AlertLevel`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AlertState {
    pub(crate) level: AlertLevel,
    consecutive_down: u32,
    consecutive_degraded: u32,
    burn: BurnAlerts,
}

impl AlertState {
    /// The state of a monitor found mid-outage at startup (an incident is
    /// still open): its down was already announced by the previous run, so a
    /// still-down monitor must not alert again, and its recovery must be.
    fn resumed_down(threshold: u32) -> Self {
        Self {
            level: AlertLevel::Down,
            consecutive_down: threshold,
            ..Self::default()
        }
    }

    /// Count a down tick. `true` when this tick confirms the outage: the
    /// threshold is reached and the down has not been announced yet.
    pub(crate) fn observe_down(&mut self, threshold: u32) -> bool {
        self.consecutive_down = self.consecutive_down.saturating_add(1);
        self.consecutive_degraded = 0;
        self.consecutive_down >= threshold && self.level != AlertLevel::Down
    }

    /// Count an up-but-slow tick (only when degraded alerts are on). `true`
    /// when this tick confirms the degradation, same threshold as down.
    fn observe_degraded(&mut self, threshold: u32) -> bool {
        self.consecutive_degraded = self.consecutive_degraded.saturating_add(1);
        self.consecutive_down = 0;
        self.consecutive_degraded >= threshold && self.level != AlertLevel::Degraded
    }

    /// Count a healthy tick. `true` when something was alerted before (the
    /// caller decides what a recovery from [`Self::level`] announces, then
    /// resets it to [`AlertLevel::Healthy`]).
    pub(crate) fn observe_up(&mut self) -> bool {
        self.consecutive_down = 0;
        self.consecutive_degraded = 0;
        self.level != AlertLevel::Healthy
    }
}

/// One monitor's (or peer's) alert state, owned by the supervisor and handed to
/// each task it spawns for that id. A task restarted by a config edit or after
/// a crash picks up where the previous one stopped: a still-down monitor is not
/// announced twice, and its recovery is not lost. Empty until the first task
/// seeds it (from the open incident, see [`AlertState::resumed_down`]).
///
/// The task writes it back synchronously right after each transition's side
/// effect, so an abort (always at an `.await`) can at worst repeat an alert
/// whose delivery it interrupted, never lose one.
#[derive(Clone, Debug, Default)]
pub struct AlertCell(Arc<Mutex<Option<AlertState>>>);

impl AlertCell {
    pub(crate) fn get(&self) -> Option<AlertState> {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn set(&self, state: AlertState) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(state);
    }
}

/// Everything a monitor loop borrows from the application: storage, its HTTP
/// client, the notifier set (degraded/burn alerts), the coalescer inbox
/// (down/recovered alerts), the liveness beacon, and its alert state.
pub struct MonitorDeps {
    pub pool: SqlitePool,
    pub client: Client,
    /// The shared plain client for multi-vantage confirmation requests to the
    /// peers. Distinct from `client` on purpose: that one may be bound to the
    /// monitor's proxy, and a peer request must never ride a monitor's proxy.
    pub confirm_client: Client,
    pub notifier: Notifiers,
    pub alerts: mpsc::UnboundedSender<AlertMsg>,
    pub last_tick: Arc<AtomicU64>,
    /// Survives this task: see [`AlertCell`].
    pub alert_state: AlertCell,
}

/// Spawn the probing loop for a single monitor. Aborting the returned handle
/// stops the loop (used by the supervisor when a monitor is removed or changed);
/// a shutdown signal lets it finish the current tick and exit cleanly.
#[must_use]
pub fn spawn_monitor(
    monitor: Monitor,
    config: watch::Receiver<Arc<Config>>,
    deps: MonitorDeps,
    shutdown: watch::Receiver<bool>,
) -> JoinHandle<()> {
    tokio::spawn(run(monitor, config, deps, shutdown))
}

/// The phase shift for a monitor's first tick: a stable hash of its id spread
/// over the interval, capped at one minute. FNV-1a rather than std's
/// `DefaultHasher`, whose algorithm is unspecified and may change with the
/// toolchain: a monitor keeps its phase across restarts, reloads and upgrades.
fn stagger_offset(id: &str, interval: std::time::Duration) -> std::time::Duration {
    let span = interval.min(std::time::Duration::from_mins(1));
    span.mul_f64(f64::from(u32::try_from(fnv1a(id.as_bytes()) % 1000).unwrap_or(0)) / 1000.0)
}

/// 64-bit FNV-1a: tiny, fixed forever, good enough to spread ids over phases.
fn fnv1a(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    bytes.iter().fold(OFFSET_BASIS, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(PRIME)
    })
}

/// Send an alert to the coalescer. It only fails once the coalescer is gone
/// (it panicked, or shutdown is under way), and then the alert is lost: say so
/// loudly rather than dropping it in silence.
fn send_alert(alerts: &mpsc::UnboundedSender<AlertMsg>, monitor_id: &str, message: AlertMsg) {
    if alerts.send(message).is_err() {
        error!(monitor = %monitor_id, "alert dropped: the alert coalescer is not running");
    }
}

#[allow(clippy::too_many_lines)]
async fn run(
    monitor: Monitor,
    config: watch::Receiver<Arc<Config>>,
    deps: MonitorDeps,
    mut shutdown: watch::Receiver<bool>,
) {
    let MonitorDeps {
        pool,
        client,
        confirm_client,
        notifier,
        alerts,
        last_tick,
        alert_state,
    } = deps;
    // Fixed cadence: the tick interval does not drift by the probe duration.
    // The first tick is phase-shifted per monitor so a fleet sharing the same
    // interval doesn't probe - and insert - in lockstep at boot and then on
    // every aligned tick forever. The offset is a stable hash of the id (same
    // phase across restarts), capped so a long-interval monitor still gets its
    // first check within a minute of starting.
    let offset = stagger_offset(&monitor.id, monitor.interval());
    let mut ticker =
        tokio::time::interval_at(tokio::time::Instant::now() + offset, monitor.interval());
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The exec directory comes from the environment (immutable for the process
    // lifetime), so one read outlives every config reload.
    let exec_dir = config.borrow().exec_dir.clone();
    // Re-attach to an incident left open by a previous run (restart mid-outage,
    // monitor edited live): it gets closed on the first healthy tick instead of
    // staying open forever, and a still-down monitor keeps its original start.
    let mut open_incident: Option<i64> = db::find_open_incident(&pool, &monitor.id)
        .await
        .ok()
        .flatten();
    // The alert state of the previous task for this id, if the supervisor
    // restarted it; otherwise (daemon start) an open incident says the down
    // was already announced.
    let mut state = alert_state.get().unwrap_or_else(|| {
        if open_incident.is_some() {
            AlertState::resumed_down(alert_settings(&config, &monitor.id).1)
        } else {
            AlertState::default()
        }
    });
    alert_state.set(state);
    // Push monitors are judged from stored heartbeats against their cadence,
    // counted from the first heartbeat expected (persisted, so a monitor that
    // never pinged still alerts across restarts).
    let heartbeat = if monitor.kind == Kind::Push {
        HeartbeatWatch::for_monitor(&pool, &monitor).await
    } else {
        None
    };

    loop {
        tokio::select! {
            _ = ticker.tick() => {}
            _ = shutdown.changed() => break,
        }
        // Liveness beacon: record that this scheduler loop iterated, independent of
        // the probe outcome (a hung probe never reaches here, so the timestamp goes
        // stale - exactly what the dead-man heartbeat and /healthz want to detect).
        // `fetch_max` across all monitors tracks the most recent tick of any of them.
        last_tick.fetch_max(
            u64::try_from(chrono::Utc::now().timestamp()).unwrap_or(0),
            Ordering::Relaxed,
        );

        // No outcome means a push monitor still inside its first expected
        // window (or a read error): status stays unknown, nothing to react to.
        let Some(outcome) = tick_outcome(
            &client,
            &pool,
            &monitor,
            exec_dir.as_deref(),
            heartbeat.as_ref(),
        )
        .await
        else {
            continue;
        };

        // Every failure that survived its retries is worth a log line (the
        // page only shows "degraded" until the threshold confirms, so this is
        // where a blip's reason is visible). Quiet once the outage is
        // confirmed - "confirmed down" already said it.
        if !outcome.up && state.level != AlertLevel::Down {
            warn!(
                monitor = %monitor.id,
                error = outcome.error.as_deref().unwrap_or("unknown"),
                "check failed"
            );
        }

        let (muted, threshold, alert_on_degraded) = alert_settings(&config, &monitor.id);
        // Ad-hoc silences (`hora silence`, POST /api/silence) mute exactly like
        // a maintenance window, read fresh each tick so they apply immediately.
        let muted = muted || silenced(&pool, &monitor.id).await;
        // An incident is bound to confirmed-down alerts; any up tick (healthy
        // or merely degraded) ends it, whatever the alert state machine does -
        // including an incident inherited from a previous run, and even during
        // maintenance (the record should reflect the real outage span).
        if outcome.up {
            close_open_incident(&pool, &monitor.id, &mut open_incident).await;
        }

        if muted {
            // Checks are still recorded above; only alert transitions are skipped.
            continue;
        }

        // Burn-rate alerting is orthogonal to the up/down state machine: a
        // monitor flapping every few minutes never confirms down, yet burns
        // its budget - exactly what this catches. Healthy, armed monitors
        // cost nothing; once tripped, evaluation continues on up ticks so the
        // alert can re-arm when the windows cool.
        if let Some(slo_bp) = monitor.slo_uptime
            && (!outcome.up || state.burn.any())
        {
            evaluate_burn(&pool, &notifier, &monitor, slo_bp, &mut state.burn).await;
            alert_state.set(state);
        }

        if !outcome.up {
            // Down resets degraded tracking; alert once `threshold` consecutive
            // failures confirm it (escalating from healthy or degraded).
            if state.observe_down(threshold) {
                error!(monitor = %monitor.id, failures = state.consecutive_down, "confirmed down");
                let snapshot = config.borrow().clone();
                confirm_down(
                    &snapshot,
                    &pool,
                    &confirm_client,
                    &alerts,
                    &monitor,
                    &outcome,
                    threshold,
                    &mut open_incident,
                )
                .await;
                state.level = AlertLevel::Down;
            }
        } else if outcome.degraded && alert_on_degraded {
            // Up but slow: same anti-flap threshold as down, separate state.
            if state.observe_degraded(threshold) {
                alert_degraded(&notifier, &monitor, outcome.latency_ms).await;
                state.level = AlertLevel::Degraded;
            }
        } else if state.observe_up() {
            // Fully healthy (or degraded with the option off, treated as up)
            // after an alert.
            info!(monitor = %monitor.id, "recovered");
            // Through the coalescer too: the recovery of a folded down
            // alert stays silent (nothing was announced going down).
            send_alert(
                &alerts,
                &monitor.id,
                AlertMsg::Recovered {
                    id: monitor.id.clone(),
                    name: monitor.name.clone(),
                    notify: monitor.notify.clone(),
                },
            );
            state.level = AlertLevel::Healthy;
        }
        // Written back right after the side effect, with no `.await` in
        // between: see [`AlertCell`].
        alert_state.set(state);
    }
}

/// A monitor just confirmed down: resolve the topology context, open (or
/// resume) the incident record, ask the peers for a multi-vantage verdict,
/// and hand the alert to the coalescer, which may fold it into its root
/// cause's single notification.
#[allow(clippy::too_many_arguments)]
async fn confirm_down(
    config: &Config,
    pool: &SqlitePool,
    confirm_client: &Client,
    alerts: &mpsc::UnboundedSender<AlertMsg>,
    monitor: &Monitor,
    outcome: &Outcome,
    threshold: u32,
    open_incident: &mut Option<i64>,
) {
    let (cause, impacted_names) = down_context(config, pool, monitor, threshold).await;

    // "What changed?": the most recent event marker (`hora event`) within the
    // lookback, phrased relative to now ("deploy api v2.3, 3m before"). Read
    // errors just drop the annotation - it must never delay the alert.
    let event = correlated_event(pool, chrono::Utc::now().timestamp()).await;

    // Unless one is already open (resumed from a previous run mid-outage).
    // Recorded *before* the peers are consulted, so the incident history
    // never waits on the network.
    if open_incident.is_none() {
        *open_incident = open_incident_record(
            pool,
            monitor,
            outcome,
            cause.as_ref().map(|(_, name)| name.as_str()),
            &impacted_names,
            event.as_deref(),
        )
        .await;
    }

    // Multi-vantage confirmation: bounded (one concurrent round, hard
    // deadline) and strictly fail-open - `None` means the alert reads exactly
    // as it would without the feature.
    let vantage = crate::confirm::confirm_with_peers(confirm_client, config, monitor).await;
    if let Some(verdict) = &vantage {
        info!(monitor = %monitor.id, %verdict, "multi-vantage verdict");
        // Recorded on the incident too (best effort), so the post-mortem can
        // replay what the mesh saw, not just what this node saw.
        if let Some(incident_id) = *open_incident
            && let Err(err) = db::update_incident_vantage(pool, incident_id, verdict).await
        {
            error!(monitor = %monitor.id, "failed to record vantage verdict: {err:#}");
        }
    }

    // The coalescer groups on the *configured* upstreams: in a cascade this
    // monitor often confirms a tick before its upstream is derivably down,
    // so the derived cause alone would lose the race.
    send_alert(
        alerts,
        &monitor.id,
        AlertMsg::Down(DownAlert {
            id: monitor.id.clone(),
            name: monitor.name.clone(),
            error: outcome.error.clone(),
            upstreams: topology::transitive_upstreams(&config.monitors, &monitor.id)
                .into_iter()
                .map(str::to_owned)
                .collect(),
            cause_name: cause.map(|(_, name)| name),
            impacted: impacted_names,
            notify: monitor.notify.clone(),
            vantage,
            event,
        }),
    );
}

/// How far back a recorded event still counts as "what changed" for a down
/// that confirms now. An hour: long enough for a slow rollout to bite, short
/// enough that yesterday's deploy is not blamed for today's outage.
const EVENT_LOOKBACK_SECS: i64 = 3600;

/// The correlated-event phrase for a down confirming at `now` ("deploy api
/// v2.3, 3m before"), or `None` when no event was recorded within the
/// lookback. A read error drops the annotation (logged), never the alert.
async fn correlated_event(pool: &SqlitePool, now: i64) -> Option<String> {
    match db::latest_event_before(pool, now, EVENT_LOOKBACK_SECS).await {
        Ok(found) => found.map(|event| event_phrase(&event.title, now - event.created_at)),
        Err(err) => {
            error!("failed to read event markers: {err:#}");
            None
        }
    }
}

/// `"deploy api v2.3, 3m 10s before"` - the phrase stored on the incident and
/// appended to the alert (each channel prefixes its own "recent change:").
fn event_phrase(title: &str, age_secs: i64) -> String {
    format!("{title}, {} before", crate::fmt::duration(age_secs))
}

/// Live alert settings for this tick, read fresh so a maintenance window or a
/// reloaded threshold applies immediately: (muted, fail threshold, degraded
/// alerts enabled).
fn alert_settings(config: &watch::Receiver<Arc<Config>>, monitor_id: &str) -> (bool, u32, bool) {
    let snapshot = config.borrow();
    (
        snapshot.in_maintenance(monitor_id, chrono::Utc::now()),
        snapshot.alerts.fail_threshold.max(1),
        snapshot.alerts.alert_on_degraded,
    )
}

/// Announce a confirmed-degraded monitor (up, but over its latency budget).
async fn alert_degraded(notifier: &Notifiers, monitor: &Monitor, latency_ms: Option<i64>) {
    warn!(monitor = %monitor.id, ?latency_ms, "degraded");
    notifier
        .load_full()
        .dispatch(
            Event::Degraded {
                monitor: &monitor.name,
                latency_ms,
            },
            monitor.notify.as_deref(),
        )
        .await;
}

/// Whether an ad-hoc silence covers this monitor right now. A read error fails
/// open (logged, not silenced): a database hiccup must never mute an alert.
pub(crate) async fn silenced(pool: &SqlitePool, monitor_id: &str) -> bool {
    match db::is_silenced(pool, monitor_id, chrono::Utc::now().timestamp()).await {
        Ok(silenced) => silenced,
        Err(err) => {
            error!(monitor = %monitor_id, "failed to read silences: {err:#}");
            false
        }
    }
}

/// Close the open incident, if any. Failures are logged, never fatal.
async fn close_open_incident(pool: &SqlitePool, monitor_id: &str, open_incident: &mut Option<i64>) {
    if let Some(incident_id) = open_incident.take()
        && let Err(err) = db::update_incident_end(pool, incident_id).await
    {
        error!(monitor = %monitor_id, "failed to close incident: {err:#}");
    }
}

/// One tick's outcome: probe active monitors (recording the check), run exec
/// checks, evaluate stored heartbeats for push monitors. `None` means nothing
/// to react to yet.
async fn tick_outcome(
    client: &Client,
    pool: &SqlitePool,
    monitor: &Monitor,
    exec_dir: Option<&std::path::Path>,
    heartbeat: Option<&HeartbeatWatch>,
) -> Option<Outcome> {
    if monitor.kind == Kind::Push {
        // `None` only for an unusable cron schedule (logged at startup).
        let watch = heartbeat?;
        return heartbeat_outcome_for(
            pool,
            &monitor.id,
            &watch.cadence,
            watch.expected_since,
            chrono::Utc::now().timestamp(),
        )
        .await;
    }
    let outcome = if monitor.kind == Kind::Exec {
        match exec_dir {
            Some(dir) => crate::exec::run(dir, monitor).await,
            // Config validation guarantees the directory; defensive only.
            None => crate::probe::Outcome::down("HORA_EXEC_DIR is not set".to_owned()),
        }
    } else {
        probe::run(client, monitor).await
    };
    if let Err(err) = db::insert_check(pool, &monitor.id, outcome.status_value(), &outcome).await {
        error!(monitor = %monitor.id, "failed to record check: {err:#}");
    }
    Some(outcome)
}

/// Open an incident record for a confirmed-down monitor; `None` (logged) when
/// the insert fails, so a database hiccup never blocks the alert itself.
async fn open_incident_record(
    pool: &SqlitePool,
    monitor: &Monitor,
    outcome: &Outcome,
    cause: Option<&str>,
    impacted: &[String],
    event: Option<&str>,
) -> Option<i64> {
    match db::insert_incident_start(
        pool,
        &monitor.id,
        outcome.error.as_deref(),
        cause,
        impacted,
        outcome.snapshot.as_deref(),
        event,
    )
    .await
    {
        Ok(incident_id) => Some(incident_id),
        Err(err) => {
            error!(monitor = %monitor.id, "failed to record incident: {err:#}");
            None
        }
    }
}

/// Multi-window burn-rate evaluation (Google SRE): page when ~2% of the error
/// budget burns within an hour (confirmed by the last 5 minutes, so a stale
/// spike never pages), warn when ~5% burns within six hours (confirmed by 30
/// minutes). A fast alert subsumes the slow one for the same episode.
async fn evaluate_burn(
    pool: &SqlitePool,
    notifier: &Notifiers,
    monitor: &Monitor,
    slo_bp: u32,
    state: &mut BurnAlerts,
) {
    let window_days = monitor.slo_window_days();
    let now = chrono::Utc::now().timestamp();

    let burn_1h = burn_window(pool, &monitor.id, now - 3600, slo_bp).await;
    let fast_threshold = slo::fast_burn_threshold_x10(window_days);
    let fast_now = burn_1h >= fast_threshold
        && burn_window(pool, &monitor.id, now - 300, slo_bp).await >= fast_threshold;
    if fast_now {
        if !state.fast {
            state.fast = true;
            state.slow = true;
            fire_burn_alert(pool, notifier, monitor, slo_bp, burn_1h, "1h").await;
        }
        return;
    }
    if burn_1h < fast_threshold {
        state.fast = false;
    }

    let burn_6h = burn_window(pool, &monitor.id, now - 6 * 3600, slo_bp).await;
    let slow_threshold = slo::slow_burn_threshold_x10(window_days);
    let slow_now = burn_6h >= slow_threshold
        && burn_window(pool, &monitor.id, now - 1800, slo_bp).await >= slow_threshold;
    if slow_now && !state.slow {
        state.slow = true;
        fire_burn_alert(pool, notifier, monitor, slo_bp, burn_6h, "6h").await;
    } else if burn_6h < slow_threshold {
        state.slow = false;
    }
}

/// The burn rate over one lookback window, in tenths. A read error counts as
/// zero: never alert (or re-arm) off unreadable data.
async fn burn_window(pool: &SqlitePool, id: &str, since: i64, slo_bp: u32) -> i64 {
    match db::availability(pool, id, since).await {
        Ok((available, total)) => slo::burn_rate_x10(available, total, slo_bp),
        Err(err) => {
            error!(monitor = %id, "failed to read availability: {err:#}");
            0
        }
    }
}

/// Dispatch a [`Event::BudgetBurn`], with the exhaustion estimate computed
/// from the full SLO window. The window is assumed fully covered: for a
/// monitor younger than the window this overstates consumption, which only
/// makes the estimate conservative.
async fn fire_burn_alert(
    pool: &SqlitePool,
    notifier: &Notifiers,
    monitor: &Monitor,
    slo_bp: u32,
    burn_x10: i64,
    window: &'static str,
) {
    let window_days = monitor.slo_window_days();
    let now = chrono::Utc::now().timestamp();
    let since = now - i64::from(window_days) * crate::SECONDS_PER_DAY;
    // No estimate beats a wrong one: unreadable history drops the ETA only.
    let exhausted_in_secs = match db::availability(pool, &monitor.id, since).await {
        Ok((available, total)) => {
            let remaining = slo::remaining_minutes(window_days, slo_bp, available, total);
            slo::exhausted_in_secs(remaining, burn_x10, slo_bp)
        }
        Err(_) => None,
    };
    warn!(
        monitor = %monitor.id,
        burn_x10, window, "error budget burning"
    );
    notifier
        .load_full()
        .dispatch(
            Event::BudgetBurn {
                monitor: &monitor.name,
                burn_rate_x10: burn_x10,
                window,
                exhausted_in_secs,
            },
            monitor.notify.as_deref(),
        )
        .await;
}

/// Compute topology annotation for a down alert: the nearest down upstream
/// (`cause`, as config id + display name) if any, or the list of impacted
/// dependents (`impacted`) if this monitor is a root cause. Returns
/// `(None, vec![])` when the monitor has no topology configured.
async fn down_context(
    config: &Config,
    pool: &SqlitePool,
    monitor: &Monitor,
    threshold: u32,
) -> (Option<(String, String)>, Vec<String>) {
    let threshold_i64 = i64::from(threshold.max(1));

    let upstreams = topology::transitive_upstreams(&config.monitors, &monitor.id);
    for up_id in &upstreams {
        let Ok(recent) = db::recent_checks(pool, up_id, threshold_i64).await else {
            continue;
        };
        if db::derive_status(&recent, threshold_i64) == "down"
            && let Some(name) = topology::monitor_name(&config.monitors, up_id)
        {
            return (Some(((*up_id).to_owned(), name.to_owned())), Vec::new());
        }
    }

    let dependents = topology::transitive_dependents(&config.monitors, &monitor.id);
    let impacted: Vec<String> = dependents
        .iter()
        .filter_map(|dep_id| topology::monitor_name(&config.monitors, dep_id).map(String::from))
        .collect();

    (None, impacted)
}

/// How often a heartbeat (a push monitor's, or a watched peer's) is expected.
pub(crate) enum Cadence {
    /// At least one heartbeat every this many seconds.
    Every(u64),
    /// One per scheduled run of a cron expression, late by at most `grace_secs`.
    Cron {
        cron: Box<croner::Cron>,
        grace_secs: u64,
    },
}

impl Cadence {
    /// Why a heartbeat last seen at `since` is overdue at `now`, or `None`
    /// while it is on time. `seen` says whether `since` is a real heartbeat
    /// (or only when one was first expected): only a real one may count for
    /// the run it slightly precedes (see [`cron_due`]).
    fn overdue(&self, since: i64, seen: bool, now: i64) -> Option<String> {
        match self {
            Self::Every(interval_secs) => {
                let max_gap = i64::try_from(*interval_secs).unwrap_or(i64::MAX);
                (now - since > max_gap).then(|| "missing heartbeat".to_owned())
            }
            Self::Cron { cron, grace_secs } => {
                let early_ok = seen.then_some(*grace_secs);
                let due = cron_missed(cron, since, *grace_secs, early_ok, now)?;
                let due_label = chrono::DateTime::from_timestamp(due, 0)
                    .map_or_else(|| due.to_string(), |dt| dt.format("%H:%M UTC").to_string());
                Some(format!(
                    "missed scheduled heartbeat (was due {due_label} + {}m grace)",
                    grace_secs / 60
                ))
            }
        }
    }
}

/// A push monitor's heartbeat expectations, resolved once per task.
struct HeartbeatWatch {
    cadence: Cadence,
    /// When a heartbeat was first expected (persisted, see
    /// [`heartbeat_expected_since`]).
    expected_since: i64,
}

impl HeartbeatWatch {
    /// `None` (logged) for a schedule that does not parse - validated at
    /// config load, so defensive only: the monitor then stays unknown.
    async fn for_monitor(pool: &SqlitePool, monitor: &Monitor) -> Option<Self> {
        let cadence = match &monitor.schedule {
            None => Cadence::Every(monitor.interval_secs),
            Some(schedule) => {
                let Ok(cron) = crate::config::parse_cron(schedule) else {
                    error!(monitor = %monitor.id, "invalid cron schedule {schedule:?}");
                    return None;
                };
                Cadence::Cron {
                    cron: Box::new(cron),
                    grace_secs: monitor.push_grace_secs(),
                }
            }
        };
        let now = chrono::Utc::now().timestamp();
        Some(Self {
            cadence,
            expected_since: heartbeat_expected_since(pool, &monitor.id, now).await,
        })
    }
}

/// When a heartbeat was first expected for `id`: the first time a task ever
/// watched it, persisted in `meta` so a restart does not push the deadline
/// back. A job that never pings (broken from day one, a typo in its push URL)
/// is then judged against this instead of staying unknown forever. A database
/// error falls back to `now`, unpersisted: the watch still works, just from
/// this start.
pub(crate) async fn heartbeat_expected_since(pool: &SqlitePool, id: &str, now: i64) -> i64 {
    let key = format!("{}{id}", db::HEARTBEAT_EXPECTED_META_PREFIX);
    match db::meta_get(pool, &key).await {
        Ok(Some(stored)) => {
            if let Ok(since) = stored.parse::<i64>() {
                return since;
            }
        }
        Ok(None) => {}
        Err(err) => {
            error!(monitor = %id, "failed to read the heartbeat start: {err:#}");
            return now;
        }
    }
    if let Err(err) = db::meta_set(pool, &key, &now.to_string()).await {
        error!(monitor = %id, "failed to record the heartbeat start: {err:#}");
    }
    now
}

/// With a cron schedule, the heartbeat is overdue once `now` passes the
/// scheduled run it should have answered (see [`cron_due`]) plus the grace.
/// Returns that due time when missed (for the alert message), `None` while on
/// time. A schedule with no computable next occurrence never alerts rather
/// than alerting forever.
fn cron_missed(
    cron: &croner::Cron,
    last: i64,
    grace_secs: u64,
    early_secs: Option<u64>,
    now: i64,
) -> Option<i64> {
    let due = cron_due(cron, last, early_secs)?;
    let deadline = due.saturating_add(i64::try_from(grace_secs).unwrap_or(i64::MAX));
    (now > deadline).then_some(due)
}

/// The scheduled run the next heartbeat after `last` must answer: normally the
/// first run after `last`. A heartbeat arriving slightly *before* a run (clock
/// skew between the job's host and this one, a job that pings as it starts)
/// belongs to that run, so the one after it is due instead - otherwise a ping
/// at 02:59:59 for a `0 3 * * *` job reads as "missed 03:00". "Slightly" is
/// the grace (`early_secs`), capped at half the gap between runs so a frequent
/// schedule can never skip a real run this way.
fn cron_due(cron: &croner::Cron, last: i64, early_secs: Option<u64>) -> Option<i64> {
    let last_at = chrono::DateTime::from_timestamp(last, 0)?;
    let next = cron.find_next_occurrence(&last_at, false).ok()?;
    let Some(early_secs) = early_secs else {
        return Some(next.timestamp());
    };
    let Ok(after) = cron.find_next_occurrence(&next, false) else {
        return Some(next.timestamp());
    };
    let (next, after) = (next.timestamp(), after.timestamp());
    let early = i64::try_from(early_secs)
        .unwrap_or(i64::MAX)
        .min((after - next) / 2);
    Some(if next - last <= early { after } else { next })
}

/// What the stored heartbeats say about a push monitor or peer right now.
#[derive(Debug)]
enum HeartbeatVerdict {
    /// Never pinged, and the first expected heartbeat is not late yet.
    Unknown,
    /// The heartbeat is overdue, for this reason.
    Overdue(String),
    /// On time: the outcome of the latest heartbeat, as pushed.
    OnTime(Outcome),
}

/// Judge the latest heartbeat (`None`: never pinged) against the cadence at
/// `now`. A monitor that never pinged is judged from when a heartbeat was
/// first expected. An on-time heartbeat yields what the job pushed: an
/// explicit `status=down` is down with the job's own message, `degraded` is
/// degraded - the job's verdict, not just its liveness, drives the alert.
fn judge_heartbeat(
    cadence: &Cadence,
    last: Option<&db::Heartbeat>,
    expected_since: i64,
    now: i64,
) -> HeartbeatVerdict {
    let since = last.map_or(expected_since, |beat| beat.time);
    if let Some(reason) = cadence.overdue(since, last.is_some(), now) {
        return HeartbeatVerdict::Overdue(if last.is_some() {
            reason
        } else {
            "no heartbeat received yet".to_owned()
        });
    }
    let Some(beat) = last else {
        return HeartbeatVerdict::Unknown;
    };
    HeartbeatVerdict::OnTime(match beat.status {
        0 => Outcome::down(
            beat.error
                .clone()
                .unwrap_or_else(|| "push reported down".to_owned()),
        ),
        status => Outcome {
            up: true,
            degraded: status == 2,
            latency_ms: beat.latency_ms,
            status_code: None,
            error: None,
            snapshot: None,
        },
    })
}

/// Evaluate a heartbeat-driven monitor (push monitor or peer watch) from the
/// stored pings for `id`: down (and recorded as a miss) when overdue, else the
/// latest heartbeat's own status. `None` means nothing to react to yet (never
/// pinged and not yet late, or a read error).
///
/// Staleness is measured from the last *heartbeat*, not the last check: the
/// misses recorded here carry a fresh timestamp, so measuring from them would
/// reset the clock each tick and the monitor would flap instead of confirming
/// down (see [`db::last_heartbeat`]).
pub(crate) async fn heartbeat_outcome_for(
    pool: &SqlitePool,
    id: &str,
    cadence: &Cadence,
    expected_since: i64,
    now: i64,
) -> Option<Outcome> {
    let last = match db::last_heartbeat(pool, id).await {
        Ok(last) => last,
        Err(err) => {
            error!(monitor = %id, "failed to read last heartbeat: {err:#}");
            return None;
        }
    };
    match judge_heartbeat(cadence, last.as_ref(), expected_since, now) {
        HeartbeatVerdict::Unknown => None,
        HeartbeatVerdict::OnTime(outcome) => Some(outcome),
        HeartbeatVerdict::Overdue(reason) => {
            // Recorded so the page and history show it; marked as a miss so it
            // never reads as a heartbeat (or as the job's own down).
            if let Err(err) = db::insert_heartbeat_miss(pool, id, &reason).await {
                error!(monitor = %id, "failed to record heartbeat miss: {err:#}");
            }
            Some(Outcome::down(reason))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::Duration;

    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    async fn memory_pool() -> SqlitePool {
        let options = SqliteConnectOptions::new()
            .filename(":memory:")
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .expect("connect in-memory");
        db::migrator().run(&pool).await.expect("run migrations");
        pool
    }

    fn beat(time: i64, status: i64, error: Option<&str>) -> db::Heartbeat {
        db::Heartbeat {
            time,
            status,
            latency_ms: None,
            error: error.map(str::to_owned),
        }
    }

    fn nightly() -> Cadence {
        Cadence::Cron {
            cron: Box::new("0 3 * * *".parse().expect("valid cron")),
            grace_secs: 1800,
        }
    }

    #[test]
    fn stagger_offset_is_stable_and_bounded() {
        let minute = std::time::Duration::from_mins(1);
        // Same id, same phase - across restarts and config reloads.
        assert_eq!(stagger_offset("api", minute), stagger_offset("api", minute));
        // Different ids should generally land on different phases.
        assert_ne!(stagger_offset("api", minute), stagger_offset("db", minute));
        // Never past the interval, and capped at a minute for slow cadences.
        let five_secs = std::time::Duration::from_secs(5);
        assert!(stagger_offset("api", five_secs) < five_secs);
        let daily = std::time::Duration::from_hours(24);
        assert!(stagger_offset("api", daily) < minute);
    }

    #[test]
    fn stagger_hash_is_fixed_across_toolchains() {
        // The published FNV-1a 64 test vectors: phases never move on upgrade.
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn event_phrase_formats_the_age() {
        assert_eq!(
            event_phrase("deploy api v2.3", 45),
            "deploy api v2.3, 45s before"
        );
        assert_eq!(
            event_phrase("deploy api v2.3", 180),
            "deploy api v2.3, 3m 0s before"
        );
        assert_eq!(
            event_phrase("deploy api v2.3", 4380),
            "deploy api v2.3, 1h 13m before"
        );
        // A clock skew can't produce a negative age.
        assert_eq!(event_phrase("x", -5), "x, 0s before");
    }

    #[test]
    fn cron_missed_only_after_due_plus_grace() {
        // Nightly at 03:00 UTC; epoch day boundaries make the math readable.
        let cron: croner::Cron = "0 3 * * *".parse().expect("valid cron");
        let day = 86_400;
        let last = day + 3 * 3600 + 120; // pinged 03:02, day 2
        let due = 2 * day + 3 * 3600; // next run: 03:00, day 3
        let grace: u64 = 1800;
        let deadline = due + 1800; // due + grace, as a timestamp

        for early in [None, Some(grace)] {
            // Before the next run, and within the grace window: on time.
            assert_eq!(cron_missed(&cron, last, grace, early, due - 3600), None);
            assert_eq!(cron_missed(&cron, last, grace, early, deadline), None);
            // Past due + grace: missed, reporting the due time.
            assert_eq!(
                cron_missed(&cron, last, grace, early, deadline + 1),
                Some(due)
            );
            // A heartbeat long dead stays missed until a fresh ping moves `last`.
            assert_eq!(
                cron_missed(&cron, last, grace, early, due + 30 * day),
                Some(due)
            );
        }
    }

    #[test]
    fn early_cron_heartbeat_counts_for_the_run_it_precedes() {
        let cron: croner::Cron = "0 3 * * *".parse().expect("valid cron");
        let day = 86_400;
        let run = day + 3 * 3600; // 03:00, day 2
        let grace: u64 = 1800;
        // Pinged at 02:59:59 (clock skew): that is the 03:00 run, so 03:30
        // today is *not* a miss - tomorrow's run is the one due.
        let last = run - 1;
        assert_eq!(
            cron_missed(&cron, last, grace, Some(grace), run + 1801),
            None
        );
        assert_eq!(
            cron_missed(&cron, last, grace, Some(grace), run + day + 1801),
            Some(run + day)
        );
        // Without the early allowance (only a "first expected" time, not a
        // real ping) the 03:00 run is still due.
        assert_eq!(cron_missed(&cron, last, grace, None, run + 1801), Some(run));
        // Far ahead of the run (more than the grace) is not "slightly early".
        let last = run - 3600;
        assert_eq!(
            cron_missed(&cron, last, grace, Some(grace), run + 1801),
            Some(run)
        );
    }

    #[test]
    fn early_allowance_never_skips_a_run_of_a_frequent_schedule() {
        // Every 15 minutes with a 30-minute grace: a ping right after 03:00
        // must not be taken as the 03:15 run (half the gap caps the allowance).
        let cron: croner::Cron = "*/15 * * * *".parse().expect("valid cron");
        let run = 86_400 + 3 * 3600; // 03:00
        assert_eq!(cron_due(&cron, run + 5, Some(1800)), Some(run + 900));
        // 02:59:00 is within half the gap of 03:00: it answers 03:00.
        assert_eq!(cron_due(&cron, run - 60, Some(1800)), Some(run + 900));
    }

    #[test]
    fn explicit_down_and_degraded_pushes_drive_the_outcome() {
        let every = Cadence::Every(60);
        let now = 1_000;

        // A fresh `status=down&msg=backup failed`: down, with the job's words.
        let HeartbeatVerdict::OnTime(outcome) =
            judge_heartbeat(&every, Some(&beat(990, 0, Some("backup failed"))), 0, now)
        else {
            panic!("on time");
        };
        assert!(!outcome.up);
        assert_eq!(outcome.error.as_deref(), Some("backup failed"));

        // Without a message it still reads as the job's own verdict.
        let HeartbeatVerdict::OnTime(outcome) =
            judge_heartbeat(&every, Some(&beat(990, 0, None)), 0, now)
        else {
            panic!("on time");
        };
        assert_eq!(outcome.error.as_deref(), Some("push reported down"));

        // Degraded is up-but-degraded, so `alert_on_degraded` applies.
        let HeartbeatVerdict::OnTime(outcome) =
            judge_heartbeat(&every, Some(&beat(990, 2, None)), 0, now)
        else {
            panic!("on time");
        };
        assert!(outcome.up && outcome.degraded);

        // Up is up.
        let HeartbeatVerdict::OnTime(outcome) =
            judge_heartbeat(&every, Some(&beat(990, 1, None)), 0, now)
        else {
            panic!("on time");
        };
        assert!(outcome.up && !outcome.degraded);

        // Any of them, once stale, is a missing heartbeat.
        assert!(matches!(
            judge_heartbeat(&every, Some(&beat(900, 0, Some("x"))), 0, now),
            HeartbeatVerdict::Overdue(reason) if reason == "missing heartbeat"
        ));
    }

    #[test]
    fn a_monitor_that_never_pinged_alerts_after_its_first_window() {
        let every = Cadence::Every(60);
        // Expected since t=1000: unknown within the interval...
        assert!(matches!(
            judge_heartbeat(&every, None, 1000, 1060),
            HeartbeatVerdict::Unknown
        ));
        // ...then overdue, saying why.
        assert!(matches!(
            judge_heartbeat(&every, None, 1000, 1061),
            HeartbeatVerdict::Overdue(reason) if reason == "no heartbeat received yet"
        ));

        // Scheduled: the first run after it was expected, plus the grace.
        let day = 86_400;
        let since = day + 3600; // 01:00, day 2
        let run = day + 3 * 3600; // 03:00, day 2
        assert!(matches!(
            judge_heartbeat(&nightly(), None, since, run + 1800),
            HeartbeatVerdict::Unknown
        ));
        assert!(matches!(
            judge_heartbeat(&nightly(), None, since, run + 1801),
            HeartbeatVerdict::Overdue(_)
        ));
    }

    #[tokio::test]
    async fn heartbeat_expectation_survives_restarts() {
        let pool = memory_pool().await;
        assert_eq!(heartbeat_expected_since(&pool, "job", 1000).await, 1000);
        // A later task start (daemon restart) keeps the original deadline.
        assert_eq!(heartbeat_expected_since(&pool, "job", 5000).await, 1000);
        // Per id.
        assert_eq!(heartbeat_expected_since(&pool, "other", 5000).await, 5000);
    }

    #[tokio::test]
    async fn explicit_down_push_is_evaluated_and_never_recorded_as_a_miss() {
        let pool = memory_pool().await;
        let now = chrono::Utc::now().timestamp();
        db::insert_push(&pool, "job", 0, None, Some("backup failed"))
            .await
            .unwrap();
        let outcome = heartbeat_outcome_for(&pool, "job", &Cadence::Every(60), now, now)
            .await
            .expect("an outcome");
        assert!(!outcome.up);
        assert_eq!(outcome.error.as_deref(), Some("backup failed"));
        // Only the push itself is stored: an on-time down records no miss.
        assert_eq!(db::recent_checks(&pool, "job", 10).await.unwrap().len(), 1);

        // Never pinged and past its first window: a recorded miss.
        let outcome = heartbeat_outcome_for(&pool, "silent", &Cadence::Every(60), now - 120, now)
            .await
            .expect("an outcome");
        assert_eq!(outcome.error.as_deref(), Some("no heartbeat received yet"));
        let rows = db::recent_checks(&pool, "silent", 10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, 0);
        // And that miss is not a heartbeat: still never pinged.
        assert_eq!(db::last_heartbeat(&pool, "silent").await.unwrap(), None);
    }

    #[test]
    fn alert_state_confirms_once_and_recovers_once() {
        let mut state = AlertState::default();
        assert!(!state.observe_down(2));
        assert!(state.observe_down(2), "threshold reached");
        state.level = AlertLevel::Down;
        assert!(!state.observe_down(2), "already announced");

        // A restarted task resumes the same state: no second down...
        let mut resumed = state;
        assert!(!resumed.observe_down(2));
        // ...and the recovery is still owed.
        assert!(resumed.observe_up());
        resumed.level = AlertLevel::Healthy;
        assert!(!resumed.observe_up());

        // Found mid-outage at startup: the same, from the open incident.
        let mut seeded = AlertState::resumed_down(2);
        assert!(!seeded.observe_down(2));
        assert!(seeded.observe_up());
    }

    /// A push monitor ticking every two seconds with a threshold of 1, wired to a
    /// test coalescer inbox.
    fn push_task(
        pool: &SqlitePool,
        config: &watch::Receiver<Arc<Config>>,
        alerts: &mpsc::UnboundedSender<AlertMsg>,
        cell: &AlertCell,
        shutdown: &watch::Receiver<bool>,
    ) -> JoinHandle<()> {
        let client = crate::http::client(None).expect("client");
        let monitor = config.borrow().monitors[0].clone();
        spawn_monitor(
            monitor,
            config.clone(),
            MonitorDeps {
                pool: pool.clone(),
                client: client.clone(),
                confirm_client: client.clone(),
                notifier: crate::notifications::shared(&config.borrow(), &client),
                alerts: alerts.clone(),
                last_tick: Arc::new(AtomicU64::new(0)),
                alert_state: cell.clone(),
            },
            shutdown.clone(),
        )
    }

    /// The next alert within `wait`, as `("down" | "recovered", id)`.
    async fn next_alert(
        rx: &mut mpsc::UnboundedReceiver<AlertMsg>,
        wait: Duration,
    ) -> Option<(&'static str, String)> {
        match tokio::time::timeout(wait, rx.recv()).await {
            Ok(Some(AlertMsg::Down(alert))) => Some(("down", alert.id)),
            Ok(Some(AlertMsg::Recovered { id, .. })) => Some(("recovered", id)),
            _ => None,
        }
    }

    #[tokio::test]
    async fn restarted_monitor_task_keeps_its_alert_state() {
        let pool = memory_pool().await;
        let config = crate::config::parse(
            r#"
            [page]
            [server]
            [alerts]
            fail_threshold = 1
            [[monitors]]
            id = "job"
            name = "Job"
            kind = "push"
            interval_secs = 2
            "#,
        )
        .expect("config");
        let (_config_tx, config_rx) = watch::channel(Arc::new(config));
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let (alerts_tx, mut alerts_rx) = mpsc::unbounded_channel();
        let cell = AlertCell::default();

        // The job reports a failure: confirmed down, announced once.
        db::insert_push(&pool, "job", 0, None, Some("backup failed"))
            .await
            .unwrap();
        let task = push_task(&pool, &config_rx, &alerts_tx, &cell, &shutdown_rx);
        assert_eq!(
            next_alert(&mut alerts_rx, Duration::from_secs(5)).await,
            Some(("down", "job".to_owned()))
        );

        // A config edit restarts the task while still down: no second alert.
        task.abort();
        let task = push_task(&pool, &config_rx, &alerts_tx, &cell, &shutdown_rx);
        assert_eq!(
            next_alert(&mut alerts_rx, Duration::from_secs(3)).await,
            None
        );

        // A daemon restart (no state carried over) seeds it from the open
        // incident instead: still no second alert.
        task.abort();
        let fresh = AlertCell::default();
        let task = push_task(&pool, &config_rx, &alerts_tx, &fresh, &shutdown_rx);
        assert_eq!(
            next_alert(&mut alerts_rx, Duration::from_secs(3)).await,
            None
        );

        // The job recovers: the recovery is announced, exactly once.
        db::insert_push(&pool, "job", 1, None, None).await.unwrap();
        assert_eq!(
            next_alert(&mut alerts_rx, Duration::from_secs(5)).await,
            Some(("recovered", "job".to_owned()))
        );
        assert_eq!(
            next_alert(&mut alerts_rx, Duration::from_millis(1500)).await,
            None
        );
        task.abort();
    }
}
