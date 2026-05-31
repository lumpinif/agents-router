use chrono::{DateTime, Utc};

use crate::provider_inbound::ProviderInboundReady;

#[derive(Debug, Clone)]
pub(crate) struct ClaimedContinuationWork {
    pub ready: ProviderInboundReady,
    pub dispatched_at: DateTime<Utc>,
}

pub(crate) trait ContinuationDispatcher: Send + Sync {
    fn dispatch(&self, work: ClaimedContinuationWork) -> anyhow::Result<()>;
}
