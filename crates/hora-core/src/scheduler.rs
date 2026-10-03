//! The scheduler: one independent probing loop per monitor, plus alert state.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use crate::db::Store;
use hora_notify::Event;
use reqwest::Client;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::coalesce::{AlertMsg, DownAlert};
use crate::config::{Config, Kind, Monitor};
use crate::heartbeat::{HeartbeatWatch, heartbeat_outcome_for};
use crate::notifications::Notifiers;
use crate::probe::Outcome;
use crate::status::MonitorState;
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
    pub store: Store,
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
        store,
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
    let mut open_incident: Option<i64> = db::find_open_incident(&store, &monitor.id)
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
        HeartbeatWatch::for_monitor(&store, &monitor).await
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
            &store,
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
        if !outcome.is_up() && state.level != AlertLevel::Down {
            warn!(
                monitor = %monitor.id,
                error = outcome.error.as_deref().unwrap_or("unknown"),
                "check failed"
            );
        }

        let (muted, threshold, alert_on_degraded) = alert_settings(&config, &monitor.id);
        // Ad-hoc silences (`hora silence`, POST /api/silence) mute exactly like
        // a maintenance window, read fresh each tick so they apply immediately.
        let muted = muted || silenced(&store, &monitor.id).await;
        // An incident is bound to confirmed-down alerts; any up tick (healthy
        // or merely degraded) ends it, whatever the alert state machine does -
        // including an incident inherited from a previous run, and even during
        // maintenance (the record should reflect the real outage span).
        if outcome.is_up() {
            close_open_incident(&store, &monitor.id, &mut open_incident).await;
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
            && (!outcome.is_up() || state.burn.any())
        {
            evaluate_burn(&store, &notifier, &monitor, slo_bp, &mut state.burn).await;
            alert_state.set(state);
        }

        if !outcome.is_up() {
            // Down resets degraded tracking; alert once `threshold` consecutive
            // failures confirm it (escalating from healthy or degraded).
            if state.observe_down(threshold) {
                error!(monitor = %monitor.id, failures = state.consecutive_down, "confirmed down");
                let snapshot = config.borrow().clone();
                confirm_down(
                    &snapshot,
                    &store,
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
        } else if outcome.is_degraded() && alert_on_degraded {
            // Up but slow: same anti-flap threshold as down, separate state.
            if state.observe_degraded(threshold) {
                alert_degraded(&notifier, &monitor, &outcome).await;
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
    store: &Store,
    confirm_client: &Client,
    alerts: &mpsc::UnboundedSender<AlertMsg>,
    monitor: &Monitor,
    outcome: &Outcome,
    threshold: u32,
    open_incident: &mut Option<i64>,
) {
    let (cause, impacted_names) = down_context(config, store, monitor, threshold).await;

    // "What changed?": the most recent event marker (`hora event`) within the
    // lookback, phrased relative to now ("deploy api v2.3, 3m before"). Read
    // errors just drop the annotation - it must never delay the alert.
    let event = correlated_event(store, chrono::Utc::now().timestamp()).await;

    // Unless one is already open (resumed from a previous run mid-outage).
    // Recorded *before* the peers are consulted, so the incident history
    // never waits on the network.
    if open_incident.is_none() {
        *open_incident = open_incident_record(
            store,
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
    let vantage = crate::mesh::confirm::confirm_with_peers(confirm_client, config, monitor).await;
    if let Some(verdict) = &vantage {
        info!(monitor = %monitor.id, %verdict, "multi-vantage verdict");
        // Recorded on the incident too (best effort), so the post-mortem can
        // replay what the mesh saw, not just what this node saw.
        if let Some(incident_id) = *open_incident
            && let Err(err) = db::update_incident_vantage(store, incident_id, verdict).await
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
async fn correlated_event(store: &Store, now: i64) -> Option<String> {
    match db::latest_event_before(store, now, EVENT_LOOKBACK_SECS).await {
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
async fn alert_degraded(notifier: &Notifiers, monitor: &Monitor, outcome: &Outcome) {
    let latency_ms = outcome.latency_ms;
    warn!(monitor = %monitor.id, ?latency_ms, "degraded");
    notifier
        .load_full()
        .dispatch(
            Event::Degraded {
                monitor: &monitor.name,
                latency_ms,
                // Only a push carries one: the job's own `msg`.
                detail: outcome.error.as_deref(),
            },
            monitor.notify.as_deref(),
        )
        .await;
}

/// Whether an ad-hoc silence covers this monitor right now. A read error fails
/// open (logged, not silenced): a database hiccup must never mute an alert.
pub(crate) async fn silenced(store: &Store, monitor_id: &str) -> bool {
    match db::is_silenced(store, monitor_id, chrono::Utc::now().timestamp()).await {
        Ok(silenced) => silenced,
        Err(err) => {
            error!(monitor = %monitor_id, "failed to read silences: {err:#}");
            false
        }
    }
}

/// Close the open incident, if any. Failures are logged, never fatal.
async fn close_open_incident(store: &Store, monitor_id: &str, open_incident: &mut Option<i64>) {
    if let Some(incident_id) = open_incident.take()
        && let Err(err) = db::update_incident_end(store, incident_id).await
    {
        error!(monitor = %monitor_id, "failed to close incident: {err:#}");
    }
}

/// One tick's outcome: probe active monitors (recording the check), run exec
/// checks, evaluate stored heartbeats for push monitors. `None` means nothing
/// to react to yet.
async fn tick_outcome(
    client: &Client,
    store: &Store,
    monitor: &Monitor,
    exec_dir: Option<&std::path::Path>,
    heartbeat: Option<&HeartbeatWatch>,
) -> Option<Outcome> {
    if monitor.kind == Kind::Push {
        // `None` only for an unusable cron schedule (logged at startup).
        let watch = heartbeat?;
        return heartbeat_outcome_for(
            store,
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
            None => crate::probe::Outcome::down(
                crate::probe::FailureKind::Plugin,
                "HORA_EXEC_DIR is not set".to_owned(),
            ),
        }
    } else {
        probe::run(client, monitor).await
    };
    if let Err(err) = db::insert_check(store, &monitor.id, &outcome).await {
        error!(monitor = %monitor.id, "failed to record check: {err:#}");
    }
    Some(outcome)
}

/// Open an incident record for a confirmed-down monitor; `None` (logged) when
/// the insert fails, so a database hiccup never blocks the alert itself.
async fn open_incident_record(
    store: &Store,
    monitor: &Monitor,
    outcome: &Outcome,
    cause: Option<&str>,
    impacted: &[String],
    event: Option<&str>,
) -> Option<i64> {
    match db::insert_incident_start(
        store,
        &monitor.id,
        outcome.error.as_deref(),
        outcome.reason,
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
    store: &Store,
    notifier: &Notifiers,
    monitor: &Monitor,
    slo_bp: u32,
    state: &mut BurnAlerts,
) {
    let window_days = monitor.slo_window_days();
    let now = chrono::Utc::now().timestamp();

    let burn_1h = burn_window(store, &monitor.id, now - 3600, slo_bp).await;
    let fast_threshold = slo::fast_burn_threshold_x10(window_days);
    let fast_now = burn_1h >= fast_threshold
        && burn_window(store, &monitor.id, now - 300, slo_bp).await >= fast_threshold;
    if fast_now {
        if !state.fast {
            state.fast = true;
            state.slow = true;
            fire_burn_alert(store, notifier, monitor, slo_bp, burn_1h, "1h").await;
        }
        return;
    }
    if burn_1h < fast_threshold {
        state.fast = false;
    }

    let burn_6h = burn_window(store, &monitor.id, now - 6 * 3600, slo_bp).await;
    let slow_threshold = slo::slow_burn_threshold_x10(window_days);
    let slow_now = burn_6h >= slow_threshold
        && burn_window(store, &monitor.id, now - 1800, slo_bp).await >= slow_threshold;
    if slow_now && !state.slow {
        state.slow = true;
        fire_burn_alert(store, notifier, monitor, slo_bp, burn_6h, "6h").await;
    } else if burn_6h < slow_threshold {
        state.slow = false;
    }
}

/// The burn rate over one lookback window, in tenths. A read error counts as
/// zero: never alert (or re-arm) off unreadable data.
async fn burn_window(store: &Store, id: &str, since: i64, slo_bp: u32) -> i64 {
    match db::availability(store, id, since).await {
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
    store: &Store,
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
    let exhausted_in_secs = match db::availability(store, &monitor.id, since).await {
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
    store: &Store,
    monitor: &Monitor,
    threshold: u32,
) -> (Option<(String, String)>, Vec<String>) {
    let threshold_i64 = i64::from(threshold.max(1));

    let upstreams = topology::transitive_upstreams(&config.monitors, &monitor.id);
    for up_id in &upstreams {
        let Ok(recent) = db::recent_checks(store, up_id, threshold_i64).await else {
            continue;
        };
        if db::derive_status(&recent, threshold_i64) == MonitorState::Down
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::CheckStatus;

    use std::time::Duration;

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
        store: &Store,
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
                store: store.clone(),
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
        let store = Store::in_memory().await;
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
        db::insert_push(
            &store,
            "job",
            CheckStatus::Down,
            None,
            Some("backup failed"),
        )
        .await
        .unwrap();
        let task = push_task(&store, &config_rx, &alerts_tx, &cell, &shutdown_rx);
        assert_eq!(
            next_alert(&mut alerts_rx, Duration::from_secs(5)).await,
            Some(("down", "job".to_owned()))
        );

        // A config edit restarts the task while still down: no second alert.
        task.abort();
        let task = push_task(&store, &config_rx, &alerts_tx, &cell, &shutdown_rx);
        assert_eq!(
            next_alert(&mut alerts_rx, Duration::from_secs(3)).await,
            None
        );

        // A daemon restart (no state carried over) seeds it from the open
        // incident instead: still no second alert.
        task.abort();
        let fresh = AlertCell::default();
        let task = push_task(&store, &config_rx, &alerts_tx, &fresh, &shutdown_rx);
        assert_eq!(
            next_alert(&mut alerts_rx, Duration::from_secs(3)).await,
            None
        );

        // The job recovers: the recovery is announced, exactly once.
        db::insert_push(&store, "job", CheckStatus::Up, None, None)
            .await
            .unwrap();
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
