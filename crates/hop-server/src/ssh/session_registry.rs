use std::{collections::HashMap, sync::Arc};

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
}

impl Default for ActiveSessionRegistry {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl ActiveSessionRegistry {
    pub(crate) async fn register(
        &self,
        session_id: impl Into<String>,
        terminate: mpsc::UnboundedSender<()>,
    ) {
        self.inner.lock().await.insert(session_id.into(), terminate);
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
}
