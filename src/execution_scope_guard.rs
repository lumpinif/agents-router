use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use crate::provider_inbound::ProviderInboundReady;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ExecutionScopeKey {
    source_type: String,
    source_id: String,
    source_session_id: String,
}

impl ExecutionScopeKey {
    pub(crate) fn from_ready(ready: &ProviderInboundReady) -> Self {
        Self {
            source_type: ready.surface.source_type.clone(),
            source_id: ready.surface.source_id.clone(),
            source_session_id: ready.surface.source_session_id.clone(),
        }
    }

    pub(crate) fn source_type(&self) -> &str {
        &self.source_type
    }

    pub(crate) fn source_id(&self) -> &str {
        &self.source_id
    }

    pub(crate) fn source_session_id(&self) -> &str {
        &self.source_session_id
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ExecutionScopeGuard {
    active: Arc<Mutex<HashSet<ExecutionScopeKey>>>,
}

impl ExecutionScopeGuard {
    pub(crate) fn try_acquire(
        &self,
        key: ExecutionScopeKey,
    ) -> anyhow::Result<Option<ExecutionScopeLease>> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| anyhow::anyhow!("execution scope guard lock is poisoned"))?;
        if active.contains(&key) {
            return Ok(None);
        }

        active.insert(key.clone());
        Ok(Some(ExecutionScopeLease {
            key,
            active: Arc::clone(&self.active),
        }))
    }
}

#[derive(Debug)]
pub(crate) struct ExecutionScopeLease {
    key: ExecutionScopeKey,
    active: Arc<Mutex<HashSet<ExecutionScopeKey>>>,
}

impl Drop for ExecutionScopeLease {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_source_session_allows_only_one_active_lease() {
        let guard = ExecutionScopeGuard::default();
        let key = ExecutionScopeKey {
            source_type: "codex_desktop".to_string(),
            source_id: "codex_desktop".to_string(),
            source_session_id: "session-1".to_string(),
        };

        let first = guard
            .try_acquire(key.clone())
            .expect("first acquire should not fail");
        let second = guard
            .try_acquire(key.clone())
            .expect("second acquire should not fail");

        assert!(first.is_some());
        assert!(second.is_none());

        drop(first);
        assert!(
            guard
                .try_acquire(key)
                .expect("third acquire should not fail")
                .is_some()
        );
    }
}
