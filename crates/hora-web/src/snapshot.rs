//! The status summary, kept fresh in the background and served from memory.
//!
//! Building the summary reads the database (milliseconds to seconds,
//! depending on its size); serving it must not. A refresher task rebuilds
//! the operator's view continuously and swaps it in whole; a request only
//! ever loads the latest snapshot (stale-while-revalidate) - the very first
//! one after start waits for the first build, nothing waits after that. Every
//! audience's view, and every rendered body (page, text, JSON, metrics), is
//! derived from that one build in memory, at most once per snapshot.
//!
//! Each build starts at most every [`MIN_REFRESH`], and never sooner than its
//! own duration after the previous one ended: on a database slow enough that
//! a build takes longer than that, the refresher idles at least half the
//! time instead of keeping a connection and a core busy forever. A config
//! reload, or an operator write that must show at once (an announcement, an
//! event marker), starts a build immediately.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use arc_swap::ArcSwapOption;
use axum::body::Bytes;
use hora_core::config::Config;
use hora_core::db::Store;
use hora_core::mesh::vantage::VantageMap;
use hora_core::notifications::{self, Notifiers};
use tokio::sync::{Notify, watch};

use crate::AppState;
use crate::layout::Spliced;
use crate::summary::{BuildState, Built, Summary, build_summary, derive};
use crate::visibility::{Audience, Visibility};

/// The shortest pause between two builds' starts.
pub(crate) const MIN_REFRESH: Duration = Duration::from_secs(5);
/// How long an operator write waits for the build that shows it.
const WRITE_VISIBLE_TIMEOUT: Duration = Duration::from_mins(1);

/// A rendered body: which view, in which format.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Body {
    /// The status page (whole, or one group's) as HTML, with its per-request
    /// parts left as holes (see `crate::layout::Spliced`). The flag is a
    /// `?token=` that opened nothing (the private door says so): whether one
    /// came, never which.
    Page(Audience, Option<String>, bool),
    /// The same, as aligned plain text.
    Text(Audience, Option<String>),
    /// `/api/summary`.
    Json(Audience),
    /// `/metrics`.
    Metrics(Audience),
}

/// One build and everything derived from it.
pub(crate) struct Snapshot {
    built: Built,
    /// Derived views and rendered bodies, for the config they were derived
    /// against (a reload invalidates them before the next build lands).
    derived: Mutex<Derived>,
}

#[derive(Default)]
struct Derived {
    config: Option<Arc<Config>>,
    summaries: HashMap<Audience, Arc<Summary>>,
    bodies: HashMap<Body, Bytes>,
    pages: HashMap<Body, Arc<Spliced>>,
}

impl Derived {
    /// The memo, emptied first if it was derived against another config.
    fn for_config(&mut self, config: &Arc<Config>) -> &mut Self {
        if !self
            .config
            .as_ref()
            .is_some_and(|derived| Arc::ptr_eq(derived, config))
        {
            *self = Self {
                config: Some(Arc::clone(config)),
                ..Self::default()
            };
        }
        self
    }
}

impl Snapshot {
    fn new(built: Built) -> Self {
        Self {
            built,
            derived: Mutex::new(Derived::default()),
        }
    }

    fn derived(&self) -> std::sync::MutexGuard<'_, Derived> {
        self.derived.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The summary as `audience` sees it under `config`.
    pub(crate) fn summary(&self, config: &Arc<Config>, audience: &Audience) -> Arc<Summary> {
        if *audience == Audience::Operator {
            return Arc::clone(&self.built.summary);
        }
        if let Some(summary) = self.derived().for_config(config).summaries.get(audience) {
            return Arc::clone(summary);
        }
        // Derived outside the lock: a few hundred `Arc` clones, but no reason
        // to serialize concurrent first requests on it.
        let visibility = Visibility::new(config, audience);
        let summary = Arc::new(derive(&self.built, config, &visibility));
        self.derived()
            .for_config(config)
            .summaries
            .insert(audience.clone(), Arc::clone(&summary));
        summary
    }

    /// A rendered body, rendered once per snapshot. `render` returning `None`
    /// (a group nobody here may see) is not cached.
    pub(crate) fn body<E>(
        &self,
        config: &Arc<Config>,
        key: Body,
        render: impl FnOnce() -> Result<Option<Bytes>, E>,
    ) -> Result<Option<Bytes>, E> {
        if let Some(body) = self.derived().for_config(config).bodies.get(&key) {
            return Ok(Some(body.clone()));
        }
        let Some(body) = render()? else {
            return Ok(None);
        };
        self.derived()
            .for_config(config)
            .bodies
            .insert(key, body.clone());
        Ok(Some(body))
    }
}

impl Snapshot {
    /// A page with holes, rendered and cut once per snapshot (see
    /// [`Snapshot::body`]); each request splices its own parts in.
    pub(crate) fn page<E>(
        &self,
        config: &Arc<Config>,
        key: Body,
        render: impl FnOnce() -> Result<Option<String>, E>,
    ) -> Result<Option<Arc<Spliced>>, E> {
        if let Some(page) = self.derived().for_config(config).pages.get(&key) {
            return Ok(Some(Arc::clone(page)));
        }
        let Some(rendered) = render()? else {
            return Ok(None);
        };
        let page = Arc::new(Spliced::new(rendered));
        self.derived()
            .for_config(config)
            .pages
            .insert(key, Arc::clone(&page));
        Ok(Some(page))
    }
}

/// The latest snapshot and the background task keeping it fresh.
pub(crate) struct Refresher {
    current: ArcSwapOption<Snapshot>,
    /// The generation of the latest published snapshot.
    published: watch::Sender<u64>,
    /// The generation of the latest build started.
    started: AtomicU64,
    wake: Arc<Notify>,
    spawned: AtomicBool,
}

impl Default for Refresher {
    fn default() -> Self {
        Self {
            current: ArcSwapOption::empty(),
            published: watch::Sender::new(0),
            started: AtomicU64::new(0),
            wake: Arc::new(Notify::new()),
            spawned: AtomicBool::new(false),
        }
    }
}

impl Refresher {
    /// Start the background task (once; later calls are no-ops). The daemon
    /// calls it at startup so the first build overlaps the boot; a request
    /// arriving first starts it too.
    pub(crate) fn start(self: &Arc<Self>, state: &AppState) {
        if self.spawned.swap(true, Ordering::AcqRel) {
            return;
        }
        let refresher = Arc::downgrade(self);
        let wake = Arc::clone(&self.wake);
        let store = state.store.clone();
        let config = state.config.clone();
        let notifier = state.notifier.clone();
        let vantage = Arc::clone(&state.vantage);
        tokio::spawn(run(refresher, wake, store, config, notifier, vantage));
    }

    /// The latest snapshot. Never waits on a build, except for the very
    /// first one after start.
    pub(crate) async fn snapshot(self: &Arc<Self>, state: &AppState) -> Arc<Snapshot> {
        if let Some(snapshot) = self.current.load_full() {
            return snapshot;
        }
        self.start(state);
        let mut published = self.published.subscribe();
        // The sender lives in `self`, so the wait cannot fail.
        let _ = published.wait_for(|&generation| generation >= 1).await;
        self.current
            .load_full()
            .expect("a published generation has a snapshot")
    }

    /// Rebuild now and wait (bounded) until a snapshot that started after
    /// this call is published - for an operator write that should be visible
    /// on the very next page view.
    pub(crate) async fn refresh_now(self: &Arc<Self>, state: &AppState) {
        self.start(state);
        let target = self.started.load(Ordering::Acquire) + 1;
        let mut published = self.published.subscribe();
        self.wake.notify_one();
        let _ = tokio::time::timeout(
            WRITE_VISIBLE_TIMEOUT,
            published.wait_for(|&generation| generation >= target),
        )
        .await;
    }
}

/// The refresher loop. Holds the refresher weakly, so it ends with the app
/// state. A closed config channel only means no more reloads.
async fn run(
    refresher: Weak<Refresher>,
    wake: Arc<Notify>,
    store: Store,
    mut config: watch::Receiver<Arc<Config>>,
    notifier: Notifiers,
    vantage: VantageMap,
) {
    let mut state = BuildState::default();
    let mut built_for: Option<Arc<Config>> = None;
    let mut reloads = true;
    loop {
        let Some(this) = refresher.upgrade() else {
            return;
        };
        let current = config.borrow_and_update().clone();
        // The caches are keyed by monitor id and follow the windows on their
        // own; only an id coming back (its history may have been swept while
        // it was gone) must not find what they kept of it.
        if built_for
            .as_ref()
            .is_some_and(|previous| adds_monitors(previous, &current))
        {
            state = BuildState::default();
        }
        built_for = Some(Arc::clone(&current));
        let generation = this.started.fetch_add(1, Ordering::AcqRel) + 1;
        let began = Instant::now();
        // Channel health is read live from the dispatcher; the vantage map is
        // the poller's last snapshot - a lock-free read, never the network.
        let health = notifications::health_snapshot(&notifier);
        let peers = vantage.load_full();
        // Built in a task of its own: a panic loses this build's caches, not
        // the refresher (the page would freeze on its last snapshot).
        let task = tokio::spawn({
            let store = store.clone();
            let current = Arc::clone(&current);
            let mut state = std::mem::take(&mut state);
            async move {
                let built = build_summary(&store, &current, &mut state, &health, &peers).await;
                (state, built)
            }
        });
        let took = match task.await {
            Ok((kept, built)) => {
                state = kept;
                this.current.store(Some(Arc::new(Snapshot::new(built))));
                this.published.send_replace(generation);
                let took = began.elapsed();
                if generation == 1 {
                    tracing::info!("status page ready (first build {took:.1?})");
                } else {
                    tracing::debug!("status summary rebuilt in {took:.1?}");
                }
                took
            }
            Err(err) => {
                tracing::error!("status summary build failed: {err}");
                began.elapsed()
            }
        };
        drop(this);

        let pause = MIN_REFRESH.saturating_sub(took).max(took);
        tokio::select! {
            () = tokio::time::sleep(pause) => {}
            () = wake.notified() => {}
            changed = config.changed(), if reloads => {
                reloads = changed.is_ok();
            }
        }
    }
}

/// Whether `next` configures a monitor id `previous` did not.
fn adds_monitors(previous: &Config, next: &Config) -> bool {
    let known: std::collections::HashSet<&str> =
        previous.monitors.iter().map(|m| m.id.as_str()).collect();
    next.monitors
        .iter()
        .any(|monitor| !known.contains(monitor.id.as_str()))
}
