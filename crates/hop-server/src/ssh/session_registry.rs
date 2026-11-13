use std::{
    collections::{HashMap, HashSet},
    net::IpAddr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex as StdMutex,
    },
    time::Duration,
};

use hop_core::Asset;
use tokio::sync::{mpsc, watch, Mutex, Notify};

pub(crate) const TERMINATED_BY_MANAGEMENT: &str = "terminated by management request";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerminateSessionResult {
    Signaled,
    NotFound,
}

#[derive(Debug, Clone)]
pub struct ResolvedTuiTarget {
    pub asset: Asset,
    pub parent_closed: watch::Receiver<bool>,
}

#[derive(Debug)]
struct SelectedTuiTarget {
    token: u64,
    asset: Asset,
    closed_tx: watch::Sender<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TuiAffinityKey {
    key_id: String,
    peer_ip: Option<IpAddr>,
}

impl TuiAffinityKey {
    fn new(key_id: &str, peer_ip: Option<IpAddr>) -> Self {
        Self {
            key_id: key_id.to_string(),
            peer_ip: peer_ip.map(|ip| ip.to_canonical()),
        }
    }
}

#[derive(Debug, Default)]
struct TuiAffinityEntry {
    open_selectors: HashSet<u64>,
    selected_targets: Vec<SelectedTuiTarget>,
}

impl TuiAffinityEntry {
    fn is_empty(&self) -> bool {
        self.open_selectors.is_empty() && self.selected_targets.is_empty()
    }
}

#[derive(Debug, Default)]
struct TuiAffinityStore {
    next_token: u64,
    entries: HashMap<TuiAffinityKey, TuiAffinityEntry>,
}

impl TuiAffinityStore {
    fn alloc_token(&mut self) -> u64 {
        self.next_token = self.next_token.wrapping_add(1);
        self.next_token
    }
}

#[derive(Debug)]
pub struct TuiSelectorGuard {
    store: Arc<StdMutex<TuiAffinityStore>>,
    notify: Arc<Notify>,
    key: TuiAffinityKey,
    token: Option<u64>,
}

impl Drop for TuiSelectorGuard {
    fn drop(&mut self) {
        let Some(token) = self.token.take() else {
            return;
        };
        if let Ok(mut store) = self.store.lock() {
            let mut remove_entry = false;
            if let Some(entry) = store.entries.get_mut(&self.key) {
                entry.open_selectors.remove(&token);
                remove_entry = entry.is_empty();
            }
            if remove_entry {
                store.entries.remove(&self.key);
            }
        }
        self.notify.notify_waiters();
    }
}

#[derive(Debug)]
pub struct TuiTargetGuard {
    store: Arc<StdMutex<TuiAffinityStore>>,
    notify: Arc<Notify>,
    key: TuiAffinityKey,
    token: Option<u64>,
}

impl TuiTargetGuard {
    pub fn release(&mut self) {
        let Some(token) = self.token.take() else {
            return;
        };
        let mut closed_senders = Vec::new();
        if let Ok(mut store) = self.store.lock() {
            let mut remove_entry = false;
            if let Some(entry) = store.entries.get_mut(&self.key) {
                let mut idx = 0;
                while idx < entry.selected_targets.len() {
                    if entry.selected_targets[idx].token == token {
                        let removed = entry.selected_targets.remove(idx);
                        closed_senders.push(removed.closed_tx);
                    } else {
                        idx += 1;
                    }
                }
                remove_entry = entry.is_empty();
            }
            if remove_entry {
                store.entries.remove(&self.key);
            }
        }
        for closed_tx in closed_senders {
            let _ = closed_tx.send(true);
        }
        self.notify.notify_waiters();
    }
}

impl Drop for TuiTargetGuard {
    fn drop(&mut self) {
        self.release();
    }
}

#[derive(Debug, Clone)]
pub struct ActiveSessionRegistry {
    inner: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<()>>>>,
    shutting_down: Arc<AtomicBool>,
    affinity: Arc<StdMutex<TuiAffinityStore>>,
    affinity_notify: Arc<Notify>,
}

impl Default for ActiveSessionRegistry {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            shutting_down: Arc::new(AtomicBool::new(false)),
            affinity: Arc::new(StdMutex::new(TuiAffinityStore::default())),
            affinity_notify: Arc::new(Notify::new()),
        }
    }
}

impl ActiveSessionRegistry {
    pub(crate) async fn register(
        &self,
        session_id: impl Into<String>,
        terminate: mpsc::UnboundedSender<()>,
    ) {
        self.inner
            .lock()
            .await
            .insert(session_id.into(), terminate.clone());
        if self.shutting_down.load(Ordering::Acquire) {
            let _ = terminate.send(());
        }
    }

    pub(crate) async fn unregister(&self, session_id: &str) {
        self.inner.lock().await.remove(session_id);
    }

    pub(crate) async fn terminate(&self, session_id: &str) -> TerminateSessionResult {
        let terminate = {
            let sessions = self.inner.lock().await;
            sessions.get(session_id).cloned()
        };
        let Some(terminate) = terminate else {
            return TerminateSessionResult::NotFound;
        };
        if terminate.send(()).is_ok() {
            TerminateSessionResult::Signaled
        } else {
            self.unregister(session_id).await;
            TerminateSessionResult::NotFound
        }
    }

    /// Ask every currently registered transport to close. The senders are
    /// cloned before signaling so a session can unregister itself without
    /// contending with this broadcast.
    pub(crate) async fn terminate_all(&self) -> usize {
        self.shutting_down.store(true, Ordering::Release);
        self.affinity_notify.notify_waiters();
        let sessions = self
            .inner
            .lock()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        sessions
            .into_iter()
            .filter(|terminate| terminate.send(()).is_ok())
            .count()
    }

    pub(crate) fn register_tui_selector(
        &self,
        key_id: &str,
        peer_ip: Option<IpAddr>,
    ) -> TuiSelectorGuard {
        let key = TuiAffinityKey::new(key_id, peer_ip);
        let token = {
            let mut store = self.affinity.lock().expect("tui affinity mutex poisoned");
            let token = store.alloc_token();
            store
                .entries
                .entry(key.clone())
                .or_default()
                .open_selectors
                .insert(token);
            token
        };
        self.affinity_notify.notify_waiters();
        TuiSelectorGuard {
            store: Arc::clone(&self.affinity),
            notify: Arc::clone(&self.affinity_notify),
            key,
            token: Some(token),
        }
    }

    pub(crate) fn select_tui_target(
        &self,
        key_id: &str,
        peer_ip: Option<IpAddr>,
        asset: Asset,
    ) -> (TuiTargetGuard, watch::Receiver<bool>) {
        let key = TuiAffinityKey::new(key_id, peer_ip);
        let (closed_tx, closed_rx) = watch::channel(false);
        let token = {
            let mut store = self.affinity.lock().expect("tui affinity mutex poisoned");
            let token = store.alloc_token();
            store
                .entries
                .entry(key.clone())
                .or_default()
                .selected_targets
                .push(SelectedTuiTarget {
                    token,
                    asset,
                    closed_tx,
                });
            token
        };
        self.affinity_notify.notify_waiters();
        (
            TuiTargetGuard {
                store: Arc::clone(&self.affinity),
                notify: Arc::clone(&self.affinity_notify),
                key,
                token: Some(token),
            },
            closed_rx,
        )
    }

    /// Resolve the active TUI-selected asset for `(key_id, peer_ip)`. If a TUI
    /// selector screen is currently open and no target has been chosen yet,
    /// wait up to `wait_timeout` for the user to select an asset or exit TUI.
    pub(crate) async fn resolve_tui_target(
        &self,
        key_id: &str,
        peer_ip: Option<IpAddr>,
        wait_timeout: Duration,
    ) -> Option<ResolvedTuiTarget> {
        let key = TuiAffinityKey::new(key_id, peer_ip);
        let deadline = tokio::time::Instant::now() + wait_timeout;
        loop {
            let notified = self.affinity_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                if self.shutting_down.load(Ordering::Acquire) {
                    return None;
                }
                let store = self.affinity.lock().expect("tui affinity mutex poisoned");
                let entry = store.entries.get(&key)?;
                if let Some(selected) = entry
                    .selected_targets
                    .iter()
                    .rev()
                    .find(|target| !*target.closed_tx.borrow())
                {
                    return Some(ResolvedTuiTarget {
                        asset: selected.asset.clone(),
                        parent_closed: selected.closed_tx.subscribe(),
                    });
                }
                if entry.open_selectors.is_empty() {
                    return None;
                }
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_asset(id: &str, name: &str) -> Asset {
        Asset {
            id: id.to_string(),
            name: name.to_string(),
            protocol: hop_core::ASSET_PROTOCOL_SSH.to_string(),
            hostname: "10.0.0.10".to_string(),
            port: 22,
            display_name: None,
            description: None,
            tags: Vec::new(),
            credential_id: Some("cred-1".to_string()),
            via_asset_id: None,
            allow_tcp_forwarding: true,
            created_at: None,
            updated_at: None,
            health: hop_core::AssetHealth::unknown(id),
        }
    }

    #[tokio::test]
    async fn registry_signals_registered_session() {
        let registry = ActiveSessionRegistry::default();
        let (tx, mut rx) = mpsc::unbounded_channel();
        registry.register("session-1", tx).await;

        assert_eq!(
            registry.terminate("session-1").await,
            TerminateSessionResult::Signaled
        );
        assert_eq!(rx.recv().await, Some(()));
    }

    #[tokio::test]
    async fn registry_reports_missing_session() {
        let registry = ActiveSessionRegistry::default();

        assert_eq!(
            registry.terminate("missing").await,
            TerminateSessionResult::NotFound
        );
    }

    #[tokio::test]
    async fn registry_signals_all_registered_sessions() {
        let registry = ActiveSessionRegistry::default();
        let (first_tx, mut first_rx) = mpsc::unbounded_channel();
        let (second_tx, mut second_rx) = mpsc::unbounded_channel();
        registry.register("session-1", first_tx).await;
        registry.register("session-2", second_tx).await;

        assert_eq!(registry.terminate_all().await, 2);
        assert_eq!(first_rx.recv().await, Some(()));
        assert_eq!(second_rx.recv().await, Some(()));
    }

    #[tokio::test]
    async fn tui_affinity_waits_for_pending_selector_and_signals_followers_on_release() {
        let registry = ActiveSessionRegistry::default();
        let v4: IpAddr = "10.253.0.1".parse().unwrap();
        let mapped: IpAddr = "::ffff:10.253.0.1".parse().unwrap();

        assert!(registry
            .resolve_tui_target("key-1", Some(v4), Duration::from_secs(1))
            .await
            .is_none());

        let selector = registry.register_tui_selector("key-1", Some(mapped));
        let waiter_registry = registry.clone();
        let waiter = tokio::spawn(async move {
            waiter_registry
                .resolve_tui_target("key-1", Some(v4), Duration::from_secs(5))
                .await
        });

        tokio::time::sleep(Duration::from_millis(30)).await;
        let (target_guard, mut direct_closed_rx) =
            registry.select_tui_target("key-1", Some(mapped), sample_asset("a1", "kwrt"));
        drop(selector);

        let mut resolved = waiter.await.unwrap().expect("expected selected asset");
        assert_eq!(resolved.asset.name, "kwrt");
        assert!(!*resolved.parent_closed.borrow());
        assert!(!*direct_closed_rx.borrow());

        // Different key or IP never sees this selection.
        assert!(registry
            .resolve_tui_target("key-2", Some(v4), Duration::from_millis(50))
            .await
            .is_none());
        assert!(registry
            .resolve_tui_target(
                "key-1",
                Some("10.253.0.2".parse().unwrap()),
                Duration::from_millis(50)
            )
            .await
            .is_none());

        drop(target_guard);
        assert!(*resolved.parent_closed.borrow_and_update());
        assert!(*direct_closed_rx.borrow_and_update());
        assert!(registry
            .resolve_tui_target("key-1", Some(v4), Duration::from_millis(50))
            .await
            .is_none());
    }

    #[tokio::test]
    async fn tui_affinity_unblocks_with_none_when_selector_closes_without_selection() {
        let registry = ActiveSessionRegistry::default();
        let ip: IpAddr = "192.168.11.50".parse().unwrap();
        let selector = registry.register_tui_selector("key-1", Some(ip));

        let waiter_registry = registry.clone();
        let waiter = tokio::spawn(async move {
            waiter_registry
                .resolve_tui_target("key-1", Some(ip), Duration::from_secs(5))
                .await
        });

        tokio::time::sleep(Duration::from_millis(25)).await;
        drop(selector);

        let resolved = tokio::time::timeout(Duration::from_millis(250), waiter)
            .await
            .expect("waiter must unblock promptly when TUI closes")
            .unwrap();
        assert!(resolved.is_none());
    }
}
