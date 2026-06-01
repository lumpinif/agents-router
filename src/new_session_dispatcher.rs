use chrono::{DateTime, Utc};

use crate::bridge_control::BridgeNewSessionCommand;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewSessionWork {
    pub command: BridgeNewSessionCommand,
    pub dispatched_at: DateTime<Utc>,
}

pub(crate) trait NewSessionDispatcher: Send + Sync {
    fn dispatch(&self, work: NewSessionWork) -> anyhow::Result<()>;
}
