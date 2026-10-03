//! Short-lived memoisation of expensive views: the status summary (one slot
//! per audience), and the heatmap and report bodies. Every entry is tied to
//! the config snapshot it was built from, so a reload busts it at once.

use std::any::Any;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use arc_swap::{ArcSwap, ArcSwapOption};
use hora_core::config::Config;

use crate::summary::Summary;

/// A value built from one config snapshot at one instant.
pub(crate) struct Cached<V> {
    at: Instant,
    config: Arc<Config>,
    value: Arc<V>,
}

impl<V> Cached<V> {
    pub(crate) fn new(config: &Arc<Config>, value: Arc<V>) -> Self {
        Self {
            at: Instant::now(),
            config: Arc::clone(config),
            value,
        }
    }

    fn fresh(&self, config: &Arc<Config>, ttl: Duration) -> bool {
        Arc::ptr_eq(&self.config, config) && self.at.elapsed() < ttl
    }
}

/// Read a slot if it was built from `config` within `ttl`.
pub(crate) fn fresh<V>(
    slot: &ArcSwapOption<Cached<V>>,
    config: &Arc<Config>,
    ttl: Duration,
) -> Option<Arc<V>> {
    let cached = slot.load_full()?;
    cached.fresh(config, ttl).then(|| Arc::clone(&cached.value))
}

/// A keyed memo with lock-free reads: the map is swapped whole on insert
/// (copy-on-write, dropping stale entries on the way), so it only ever holds
/// live keys - bounded by what callers validate as a key (configured monitor
/// ids, retained months, configured groups).
pub(crate) struct Memo<K, V> {
    ttl: Duration,
    entries: ArcSwap<HashMap<K, Arc<Cached<V>>>>,
}

impl<K: Eq + Hash + Clone, V> Memo<K, V> {
    pub(crate) fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: ArcSwap::from_pointee(HashMap::new()),
        }
    }

    pub(crate) fn get(&self, key: &K, config: &Arc<Config>) -> Option<Arc<V>> {
        let entries = self.entries.load();
        let cached = entries.get(key)?;
        cached
            .fresh(config, self.ttl)
            .then(|| Arc::clone(&cached.value))
    }

    pub(crate) fn insert(&self, key: &K, config: &Arc<Config>, value: Arc<V>) {
        let entry = Arc::new(Cached::new(config, value));
        self.entries.rcu(|entries| {
            let mut next: HashMap<K, Arc<Cached<V>>> = entries
                .iter()
                .filter(|(_, cached)| cached.fresh(config, self.ttl))
                .map(|(key, cached)| (key.clone(), Arc::clone(cached)))
                .collect();
            next.insert(key.clone(), Arc::clone(&entry));
            next
        });
    }

    pub(crate) fn clear(&self) {
        self.entries.store(Arc::new(HashMap::new()));
    }
}

/// The status-summary cache: lock-free reads (one slot per audience) plus a
/// single-flight build gate. The `public` slot caches the summary filtered to
/// public monitors, `operator` the unfiltered view served to the operator
/// (admin page views, Prometheus scrapes), and `groups` each group-token
/// holder's view - all bust on config reload.
pub(crate) struct Cache {
    pub(crate) public: ArcSwapOption<Cached<Summary>>,
    pub(crate) operator: ArcSwapOption<Cached<Summary>>,
    pub(crate) groups: Memo<String, Summary>,
    pub(crate) build: tokio::sync::Mutex<()>,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            public: ArcSwapOption::empty(),
            operator: ArcSwapOption::empty(),
            groups: Memo::new(crate::SUMMARY_CACHE_TTL),
            build: tokio::sync::Mutex::new(()),
        }
    }
}

impl Cache {
    /// Drop every cached view so the next request rebuilds it - used when a
    /// write (pinning or clearing an announcement) must show up immediately
    /// instead of after the TTL.
    pub(crate) fn invalidate(&self) {
        self.public.store(None);
        self.operator.store(None);
        self.groups.clear();
    }
}

/// Probe clients for `/api/peer/probe`, one per proxy setting, so a mesh
/// confirming flaps reuses connection pools and TLS setup instead of building
/// a client per call. Rebuilt (emptied) when the config changes, since a
/// monitor's proxy may have.
///
/// Type-erased: hora-web does not depend on reqwest directly, so the client
/// type is only known to the generic accessor (inferred from the builder).
#[derive(Default)]
pub(crate) struct ProbeClients {
    inner: Mutex<Option<ProbeClientSet>>,
}

struct ProbeClientSet {
    config: Arc<Config>,
    clients: HashMap<Option<String>, Box<dyn Any + Send + Sync>>,
}

impl ProbeClients {
    /// The cached client for `proxy` under `config`, or a fresh one from
    /// `build` (cached on success).
    pub(crate) fn get_or_build<C, E>(
        &self,
        config: &Arc<Config>,
        proxy: Option<&str>,
        build: impl FnOnce(Option<&str>) -> Result<C, E>,
    ) -> Result<C, E>
    where
        C: Clone + Send + Sync + 'static,
    {
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let set = match guard.take() {
            Some(set) if Arc::ptr_eq(&set.config, config) => set,
            _ => ProbeClientSet {
                config: Arc::clone(config),
                clients: HashMap::new(),
            },
        };
        let set = guard.insert(set);
        let key = proxy.map(str::to_owned);
        if let Some(client) = set
            .clients
            .get(&key)
            .and_then(|client| client.downcast_ref::<C>())
        {
            return Ok(client.clone());
        }
        let client = build(proxy)?;
        set.clients.insert(key, Box::new(client.clone()));
        Ok(client)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Arc<Config> {
        Arc::new(hora_core::config::parse("[page]\n[server]\n").expect("config"))
    }

    #[test]
    fn memo_serves_fresh_entries_and_busts_on_reload() {
        let memo: Memo<&str, u32> = Memo::new(Duration::from_mins(1));
        let config = config();
        assert!(memo.get(&"a", &config).is_none());
        memo.insert(&"a", &config, Arc::new(1));
        assert_eq!(memo.get(&"a", &config).as_deref(), Some(&1));
        // Another config snapshot (a reload) never reads the old entry...
        let reloaded = self::config();
        assert!(memo.get(&"a", &reloaded).is_none());
        // ...and inserting under it drops the stale one.
        memo.insert(&"b", &reloaded, Arc::new(2));
        assert_eq!(memo.entries.load().len(), 1);
    }

    #[test]
    fn memo_entries_expire() {
        let memo: Memo<&str, u32> = Memo::new(Duration::ZERO);
        let config = config();
        memo.insert(&"a", &config, Arc::new(1));
        assert!(memo.get(&"a", &config).is_none());
    }

    #[test]
    fn probe_clients_are_reused_per_proxy_and_rebuilt_on_reload() {
        let clients = ProbeClients::default();
        let config = config();
        let mut builds = 0;
        let mut build = |proxy: Option<&str>| -> Result<String, ()> {
            builds += 1;
            Ok(format!("client:{proxy:?}"))
        };
        assert_eq!(
            clients.get_or_build(&config, None, &mut build),
            Ok("client:None".to_owned())
        );
        assert_eq!(
            clients.get_or_build(&config, None, &mut build),
            Ok("client:None".to_owned())
        );
        clients
            .get_or_build(&config, Some("socks5://proxy:1080"), &mut build)
            .unwrap();
        let reloaded = self::config();
        clients.get_or_build(&reloaded, None, &mut build).unwrap();
        assert_eq!(builds, 3, "one build per proxy, plus one after the reload");
    }
}
