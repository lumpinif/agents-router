use thiserror::Error;

use crate::signal::Signal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryErrorKind {
    Config,
    Validation,
    Network,
    ProviderRejected,
    ProviderResponse,
    Timeout,
    Internal,
}

impl DeliveryErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Validation => "validation",
            Self::Network => "network",
            Self::ProviderRejected => "provider_rejected",
            Self::ProviderResponse => "provider_response",
            Self::Timeout => "timeout",
            Self::Internal => "internal",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryPhase {
    Route,
    ProviderSend,
}

impl DeliveryPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Route => "route",
            Self::ProviderSend => "provider_send",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryErrorContext {
    pub signal_id: String,
    pub source_id: String,
    pub provider_id: Option<String>,
    pub provider_type: Option<String>,
    pub phase: DeliveryPhase,
}

impl DeliveryErrorContext {
    pub fn route(signal: &Signal) -> Self {
        Self {
            signal_id: signal.id.clone(),
            source_id: signal.source_id().to_string(),
            provider_id: None,
            provider_type: None,
            phase: DeliveryPhase::Route,
        }
    }

    pub fn provider_send(signal: &Signal, provider_id: &str, provider_type: &str) -> Self {
        Self {
            signal_id: signal.id.clone(),
            source_id: signal.source_id().to_string(),
            provider_id: Some(provider_id.to_string()),
            provider_type: Some(provider_type.to_string()),
            phase: DeliveryPhase::ProviderSend,
        }
    }

    pub fn provider_control(operation_id: &str, provider_id: &str, provider_type: &str) -> Self {
        Self {
            signal_id: operation_id.to_string(),
            source_id: "bridge_project_room_discovery".to_string(),
            provider_id: Some(provider_id.to_string()),
            provider_type: Some(provider_type.to_string()),
            phase: DeliveryPhase::ProviderSend,
        }
    }
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct DeliveryError {
    pub kind: DeliveryErrorKind,
    pub context: DeliveryErrorContext,
    pub message: String,
    pub http_status: Option<u16>,
    pub provider_code: Option<String>,
    pub retriable: bool,
    #[source]
    pub source: Option<anyhow::Error>,
}

impl DeliveryError {
    pub fn new(
        kind: DeliveryErrorKind,
        context: DeliveryErrorContext,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            context,
            message: message.into(),
            http_status: None,
            provider_code: None,
            retriable: false,
            source: None,
        }
    }

    pub fn with_http_status(mut self, http_status: u16) -> Self {
        self.http_status = Some(http_status);
        self
    }

    pub fn with_provider_code(mut self, provider_code: impl Into<String>) -> Self {
        self.provider_code = Some(provider_code.into());
        self
    }

    pub fn with_retriable(mut self, retriable: bool) -> Self {
        self.retriable = retriable;
        self
    }

    pub fn with_source(mut self, source: impl Into<anyhow::Error>) -> Self {
        self.source = Some(source.into());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderSendStatus {
    Sent,
}

impl ProviderSendStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sent => "sent",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSendResult {
    pub provider_id: String,
    pub provider_type: String,
    pub signal_id: String,
    pub status: ProviderSendStatus,
    pub provider_message_id: Option<String>,
    pub delivery_receipt: Option<ProviderDeliveryReceipt>,
    pub http_status: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderDeliveryReceiptStatus {
    Candidate,
    SurfaceReady,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderDeliveryReceipt {
    pub status: ProviderDeliveryReceiptStatus,
    pub provider_account_id: Option<String>,
    pub provider_conversation_id: Option<String>,
    pub provider_message_id: Option<String>,
    pub provider_thread_id: Option<String>,
}

impl ProviderDeliveryReceipt {
    pub fn candidate(
        provider_account_id: Option<String>,
        provider_conversation_id: Option<String>,
        provider_message_id: Option<String>,
        provider_thread_id: Option<String>,
    ) -> Self {
        Self {
            status: ProviderDeliveryReceiptStatus::Candidate,
            provider_account_id,
            provider_conversation_id,
            provider_message_id,
            provider_thread_id,
        }
    }

    pub fn surface_ready(
        provider_account_id: Option<String>,
        provider_conversation_id: Option<String>,
        provider_message_id: Option<String>,
        provider_thread_id: Option<String>,
    ) -> Self {
        Self {
            status: ProviderDeliveryReceiptStatus::SurfaceReady,
            provider_account_id,
            provider_conversation_id,
            provider_message_id,
            provider_thread_id,
        }
    }
}

impl ProviderSendResult {
    pub fn sent(provider_id: &str, provider_type: &str, signal: &Signal) -> Self {
        Self {
            provider_id: provider_id.to_string(),
            provider_type: provider_type.to_string(),
            signal_id: signal.id.clone(),
            status: ProviderSendStatus::Sent,
            provider_message_id: None,
            delivery_receipt: None,
            http_status: None,
        }
    }

    pub fn with_http_status(mut self, http_status: u16) -> Self {
        self.http_status = Some(http_status);
        self
    }

    pub fn with_provider_message_id(mut self, provider_message_id: impl Into<String>) -> Self {
        self.provider_message_id = Some(provider_message_id.into());
        self
    }

    pub fn with_delivery_receipt(mut self, delivery_receipt: ProviderDeliveryReceipt) -> Self {
        self.delivery_receipt = Some(delivery_receipt);
        self
    }
}

pub fn is_retriable_http_status(status: u16) -> bool {
    status == 408 || status == 429 || status >= 500
}

pub fn provider_request_error(
    signal: &Signal,
    provider_id: &str,
    provider_type: &str,
    provider_name: &str,
    is_timeout: bool,
    source: impl Into<anyhow::Error>,
) -> DeliveryError {
    let kind = if is_timeout {
        DeliveryErrorKind::Timeout
    } else {
        DeliveryErrorKind::Network
    };
    DeliveryError::new(
        kind,
        DeliveryErrorContext::provider_send(signal, provider_id, provider_type),
        format!("{provider_name} provider `{provider_id}` request failed"),
    )
    .with_retriable(true)
    .with_source(source)
}
