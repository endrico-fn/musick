use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::sync::Mutex;

use tokio::sync::watch;

struct InflightGuard<'a, K, V>
where
    K: Eq + Hash,
{
    map: &'a Mutex<HashMap<K, watch::Receiver<Option<V>>>>,
    key: K,
}

impl<K, V> Drop for InflightGuard<'_, K, V>
where
    K: Eq + Hash,
{
    fn drop(&mut self) {
        if let Ok(mut guard) = self.map.lock() {
            guard.remove(&self.key);
        }
    }
}

/// deduplicates concurrent async work by key with safe leader-cancel recovery
pub struct SingleFlight<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    inflight: Mutex<HashMap<K, watch::Receiver<Option<V>>>>,
}

impl<K, V> Default for SingleFlight<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V> SingleFlight<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    pub fn new() -> Self {
        Self {
            inflight: Mutex::new(HashMap::new()),
        }
    }

    pub fn is_inflight(&self, key: &K) -> bool {
        self.inflight
            .lock()
            .map(|m| m.contains_key(key))
            .unwrap_or(false)
    }

    pub async fn run<F, Fut>(&self, key: K, work: F) -> V
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = V>,
    {
        let leader_tx = loop {
            enum Slot<V> {
                Leader(watch::Sender<Option<V>>),
                Waiter(watch::Receiver<Option<V>>),
            }

            let slot = {
                let Ok(mut map) = self.inflight.lock() else {
                    return work().await;
                };
                if let Some(existing) = map.get(&key) {
                    Slot::Waiter(existing.clone())
                } else {
                    let (tx, rx) = watch::channel(None);
                    map.insert(key.clone(), rx);
                    Slot::Leader(tx)
                }
            };

            match slot {
                Slot::Leader(tx) => break tx,
                Slot::Waiter(mut rx) => {
                    if let Some(done) = rx.borrow().clone() {
                        return done;
                    }
                    if rx.changed().await.is_ok()
                        && let Some(done) = rx.borrow().clone()
                    {
                        return done;
                    }
                    // leader dropped before publishing; loop to retry as leader
                }
            }
        };

        let _guard = InflightGuard {
            map: &self.inflight,
            key,
        };
        let out = work().await;
        let _ = leader_tx.send(Some(out.clone()));
        out
    }
}
