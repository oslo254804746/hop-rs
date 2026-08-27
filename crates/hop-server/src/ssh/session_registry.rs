use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use tokio::sync::{mpsc, Mutex};

pub(crate) const TERMINATED_BY_MANAGEMENT: &str = "terminated by management request";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerminateSessionResult {
    Signaled,
    NotFound,
}

#[derive(Debug, Clone)]
pub struct ActiveSessionRegistry {
    inner: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<()>>>>,
    shutting_down: Arc<AtomicBool>,
}

impl Default for ActiveSessionRegistry {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            shutting_down: Arc::new(AtomicBool::new(false)),
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
