use std::sync::{Arc, Mutex};
use std::time::{Duration as StdDuration, Instant};

use anyhow::{Context, anyhow, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chrono::{DateTime, Local, Utc};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::agent_controller::{
    AgentControllerProgressObserver, AgentControllerSubmitFuture, ProviderThreadReplyAdapter,
    ProviderThreadReplyError, ProviderThreadReplyFuture, ProviderThreadReplyRequest,
    ProviderThreadReplySuccess,
};
use crate::config::{
    FeishuLarkAppBotProviderConfig, FeishuLarkAppDomain, FeishuLarkCustomBotProviderConfig,
    ProviderConfig, ProviderConfigDetail, ProviderType,
};
use crate::delivery::{
    DeliveryError, DeliveryErrorContext, DeliveryErrorKind, ProviderDeliveryReceipt,
    ProviderSendResult, is_retriable_http_status, provider_request_error,
};
use crate::local_machine;
use crate::local_open_bridge::codex_thread_bridge_url;
use crate::provider_urls::validate_feishu_lark_webhook_url;
use crate::providers::http::provider_http_client;
use crate::providers::notification_view::{
    NotificationAction, NotificationFieldKey, NotificationSection, SignalNotificationView,
};
use crate::router::{ProjectRoomPromptFuture, ProjectRoomPromptRequest, ProjectRoomPromptResult};
use crate::router::{Provider, ProviderFuture};
use crate::signal::Signal;

type HmacSha256 = Hmac<Sha256>;
const CODEX_CARD_TEMPLATE: &str = "purple";
const STREAMING_REPLY_ELEMENT_ID: &str = "answer";
const STREAMING_REPLY_INITIAL_TEXT: &str = "Codex is working...";
const STREAMING_PROGRESS_MIN_NEW_BYTES: usize = 24;
const STREAMING_PROGRESS_MIN_INTERVAL: StdDuration = StdDuration::from_millis(350);

#[derive(Debug, Clone)]
pub struct FeishuLarkProvider {
    id: String,
    runtime: FeishuLarkProviderRuntime,
    client: reqwest::Client,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FeishuLarkConversationTextRequest {
    pub provider_id: String,
    pub provider_type: String,
    pub provider_account_id: String,
    pub provider_conversation_id: String,
    pub provider_event_id_hash: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FeishuLarkConversationTextSuccess {
    pub provider_message_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FeishuLarkCreateProjectRoomRequest {
    pub provider_id: String,
    pub provider_type: String,
    pub provider_account_id: String,
    pub operator_open_id: String,
    pub project_name: String,
    pub project_path: String,
    pub proposal_id: String,
    pub proposal_created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FeishuLarkCreateProjectRoomSuccess {
    pub provider_conversation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FeishuLarkStreamingThreadReplyRequest {
    pub provider_id: String,
    pub provider_type: String,
    pub provider_account_id: String,
    pub provider_conversation_id: String,
    pub provider_thread_id: String,
    pub surface_id: String,
    pub provider_event_id_hash: String,
}

#[derive(Debug)]
pub(crate) struct FeishuLarkStreamingThreadReply {
    client: reqwest::Client,
    api_base_url: String,
    token: String,
    provider_id: String,
    provider_type: String,
    surface_id: String,
    provider_event_id_hash: String,
    card_id: String,
    provider_reply_message_id: String,
    sequence: u64,
}

#[derive(Clone)]
pub(crate) struct FeishuLarkLazyStreamingThreadReplyParts {
    provider: FeishuLarkProvider,
    request: FeishuLarkStreamingThreadReplyRequest,
    state: Arc<tokio::sync::Mutex<LazyStreamingThreadReplyState>>,
    provider_id: String,
    provider_type: String,
}

pub(crate) struct FeishuLarkLazyStreamingProgressObserver {
    parts: FeishuLarkLazyStreamingThreadReplyParts,
    state: Mutex<StreamingProgressState>,
}

#[derive(Debug)]
enum LazyStreamingThreadReplyState {
    Pending,
    Ready(Box<FeishuLarkStreamingThreadReply>),
    Disabled,
}

#[derive(Debug)]
struct StreamingProgressState {
    last_sent_len: usize,
    next_update_at: Instant,
}

#[derive(Debug, Clone)]
enum FeishuLarkProviderRuntime {
    CustomBot(FeishuLarkCustomBotRuntime),
    AppBot(FeishuLarkAppBotRuntime),
}

#[derive(Debug, Clone)]
struct FeishuLarkCustomBotRuntime {
    url: String,
    secret: Option<String>,
    computer_name: String,
}

#[derive(Debug, Clone)]
struct FeishuLarkAppBotRuntime {
    api_base_url: String,
    app_id: String,
    app_secret: String,
    operator_open_id: Option<String>,
    tenant_key: Option<String>,
    chat_id: Option<String>,
    computer_name: String,
}

impl FeishuLarkProvider {
    pub fn from_config(config: &ProviderConfig) -> anyhow::Result<Self> {
        let ProviderConfigDetail::FeishuLark(detail) = &config.detail else {
            return Err(anyhow!(
                "provider `{}` is not a feishu_lark provider",
                config.id
            ));
        };
        let runtime = match detail {
            crate::config::FeishuLarkProviderConfig::CustomBot(detail) => {
                FeishuLarkProviderRuntime::CustomBot(runtime_custom_bot(&config.id, detail)?)
            }
            crate::config::FeishuLarkProviderConfig::AppBot(detail) => {
                FeishuLarkProviderRuntime::AppBot(runtime_app_bot(&config.id, detail)?)
            }
        };

        Ok(Self {
            id: config.id.clone(),
            runtime,
            client: provider_http_client()?,
        })
    }

    pub(crate) async fn send_text_to_conversation(
        &self,
        request: FeishuLarkConversationTextRequest,
    ) -> anyhow::Result<FeishuLarkConversationTextSuccess> {
        match &self.runtime {
            FeishuLarkProviderRuntime::CustomBot(_) => {
                anyhow::bail!("Feishu/Lark custom bot mode does not support room messages")
            }
            FeishuLarkProviderRuntime::AppBot(runtime) => {
                self.send_app_bot_conversation_text(request, runtime).await
            }
        }
    }

    pub(crate) async fn create_project_room(
        &self,
        request: FeishuLarkCreateProjectRoomRequest,
    ) -> anyhow::Result<FeishuLarkCreateProjectRoomSuccess> {
        match &self.runtime {
            FeishuLarkProviderRuntime::CustomBot(_) => {
                anyhow::bail!("Feishu/Lark custom bot mode does not support creating rooms")
            }
            FeishuLarkProviderRuntime::AppBot(runtime) => {
                self.create_app_bot_project_room(request, runtime).await
            }
        }
    }

    pub(crate) async fn start_streaming_thread_reply(
        &self,
        request: FeishuLarkStreamingThreadReplyRequest,
    ) -> Result<FeishuLarkStreamingThreadReply, ProviderThreadReplyError> {
        match &self.runtime {
            FeishuLarkProviderRuntime::CustomBot(_) => Err(streaming_thread_reply_error(
                &request,
                "Feishu/Lark custom bot mode does not support streaming thread replies",
            )),
            FeishuLarkProviderRuntime::AppBot(runtime) => {
                self.start_app_bot_streaming_thread_reply(request, runtime)
                    .await
            }
        }
    }
}

impl Provider for FeishuLarkProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn provider_type(&self) -> &str {
        ProviderType::FeishuLark.as_str()
    }

    fn send<'a>(&'a self, signal: &'a Signal) -> ProviderFuture<'a> {
        Box::pin(async move {
            match &self.runtime {
                FeishuLarkProviderRuntime::CustomBot(runtime) => {
                    self.send_custom_bot(signal, runtime).await
                }
                FeishuLarkProviderRuntime::AppBot(runtime) => {
                    self.send_app_bot(signal, runtime).await
                }
            }
        })
    }

    fn can_prompt_for_project_room(&self) -> bool {
        matches!(&self.runtime, FeishuLarkProviderRuntime::AppBot(_))
    }

    fn send_project_room_prompt<'a>(
        &'a self,
        request: ProjectRoomPromptRequest,
    ) -> Option<ProjectRoomPromptFuture<'a>> {
        match &self.runtime {
            FeishuLarkProviderRuntime::CustomBot(_) => None,
            FeishuLarkProviderRuntime::AppBot(runtime) => Some(Box::pin(async move {
                self.send_app_bot_project_room_prompt(request, runtime)
                    .await
            })),
        }
    }

    fn send_to_provider_conversation<'a>(
        &'a self,
        signal: &'a Signal,
        provider_conversation_id: &'a str,
    ) -> Option<ProviderFuture<'a>> {
        match &self.runtime {
            FeishuLarkProviderRuntime::CustomBot(_) => None,
            FeishuLarkProviderRuntime::AppBot(runtime) => {
                let mut runtime = runtime.clone();
                runtime.chat_id = Some(provider_conversation_id.to_string());
                Some(Box::pin(async move {
                    self.send_app_bot(signal, &runtime).await
                }))
            }
        }
    }

    fn send_to_provider_thread<'a>(
        &'a self,
        signal: &'a Signal,
        provider_account_id: &'a str,
        provider_conversation_id: &'a str,
        provider_thread_id: &'a str,
    ) -> Option<ProviderFuture<'a>> {
        match &self.runtime {
            FeishuLarkProviderRuntime::CustomBot(_) => None,
            FeishuLarkProviderRuntime::AppBot(runtime) => {
                let mut runtime = runtime.clone();
                runtime.chat_id = Some(provider_conversation_id.to_string());
                Some(Box::pin(async move {
                    self.send_app_bot_thread_notification(
                        signal,
                        &runtime,
                        provider_account_id,
                        provider_conversation_id,
                        provider_thread_id,
                    )
                    .await
                }))
            }
        }
    }
}

impl ProviderThreadReplyAdapter for FeishuLarkProvider {
    fn provider_id(&self) -> &str {
        &self.id
    }

    fn provider_type(&self) -> &str {
        ProviderType::FeishuLark.as_str()
    }

    fn send_thread_reply<'a>(
        &'a self,
        request: ProviderThreadReplyRequest,
    ) -> ProviderThreadReplyFuture<'a> {
        Box::pin(async move {
            match &self.runtime {
                FeishuLarkProviderRuntime::CustomBot(_) => Err(thread_reply_error(
                    &request,
                    "Feishu/Lark custom bot mode does not support thread replies",
                )),
                FeishuLarkProviderRuntime::AppBot(runtime) => {
                    self.send_app_bot_thread_reply(request, runtime).await
                }
            }
        })
    }
}

impl FeishuLarkStreamingThreadReply {
    pub(crate) async fn set_markdown(
        &mut self,
        markdown: &str,
    ) -> Result<(), ProviderThreadReplyError> {
        self.sequence += 1;
        let content = if markdown.trim().is_empty() {
            STREAMING_REPLY_INITIAL_TEXT
        } else {
            markdown
        };
        let request = FeishuLarkCardKitContentUpdateRequest {
            content,
            sequence: self.sequence,
            uuid: streaming_card_update_uuid(&self.card_id, self.sequence),
        };
        let response = self
            .client
            .put(format!(
                "{}/open-apis/cardkit/v1/cards/{}/elements/{}/content",
                self.api_base_url, self.card_id, STREAMING_REPLY_ELEMENT_ID
            ))
            .bearer_auth(&self.token)
            .json(&request)
            .send()
            .await
            .map_err(|error| {
                self.error(format!(
                    "failed to update Feishu/Lark streaming reply card: {}",
                    error.without_url()
                ))
                .with_retriable(true)
            })?;
        self.ensure_cardkit_update_response(response, "content update")
            .await
    }

    pub(crate) async fn finish(
        &mut self,
        summary: Option<&str>,
    ) -> Result<ProviderThreadReplySuccess, ProviderThreadReplyError> {
        self.sequence += 1;
        let settings = json!({
            "config": {
                "streaming_mode": false,
                "summary": {
                    "content": summary
                        .map(short_card_summary)
                        .unwrap_or_else(|| "Codex finished".to_string())
                }
            }
        });
        let request = FeishuLarkCardKitSettingsUpdateRequest {
            settings: serde_json::to_string(&settings).map_err(|error| {
                self.error(format!("failed to serialize card settings: {error}"))
            })?,
            sequence: self.sequence,
            uuid: streaming_card_update_uuid(&self.card_id, self.sequence),
        };
        let response = self
            .client
            .patch(format!(
                "{}/open-apis/cardkit/v1/cards/{}/settings",
                self.api_base_url, self.card_id
            ))
            .bearer_auth(&self.token)
            .json(&request)
            .send()
            .await
            .map_err(|error| {
                self.error(format!(
                    "failed to finish Feishu/Lark streaming reply card: {}",
                    error.without_url()
                ))
                .with_retriable(true)
            })?;
        self.ensure_cardkit_update_response(response, "settings update")
            .await?;

        Ok(ProviderThreadReplySuccess {
            provider_reply_message_id: Some(self.provider_reply_message_id.clone()),
        })
    }

    async fn ensure_cardkit_update_response(
        &self,
        response: reqwest::Response,
        action: &'static str,
    ) -> Result<(), ProviderThreadReplyError> {
        let status = response.status();
        let body = response.text().await.map_err(|error| {
            self.error(format!(
                "failed to read Feishu/Lark streaming reply card {action} response: {}",
                error.without_url()
            ))
            .with_retriable(true)
        })?;
        if !status.is_success() {
            let status_code = status.as_u16();
            return Err(self
                .error(format!(
                    "Feishu/Lark streaming reply card {action} returned HTTP status {}",
                    status
                ))
                .with_http_status(status_code)
                .with_retriable(is_retriable_http_status(status_code)));
        }

        let provider_response: FeishuLarkCardKitUpdateResponse = serde_json::from_str(&body)
            .map_err(|error| {
                self.error(format!(
                    "Feishu/Lark streaming reply card {action} returned invalid JSON: {error}"
                ))
            })?;
        if provider_response.code != 0 {
            return Err(self.error(format!(
                "Feishu/Lark streaming reply card {action} returned code {}: {}",
                provider_response.code,
                provider_response
                    .msg
                    .unwrap_or_else(|| "unknown error".to_string())
            )));
        }
        Ok(())
    }

    fn error(&self, message: impl Into<String>) -> ProviderThreadReplyError {
        ProviderThreadReplyError {
            provider_id: self.provider_id.clone(),
            provider_type: self.provider_type.clone(),
            surface_id: self.surface_id.clone(),
            provider_event_id_hash: self.provider_event_id_hash.clone(),
            message: message.into().into_boxed_str(),
            http_status: None,
            retriable: false,
        }
    }
}

impl FeishuLarkLazyStreamingThreadReplyParts {
    pub(crate) fn new(
        provider: FeishuLarkProvider,
        request: FeishuLarkStreamingThreadReplyRequest,
    ) -> Self {
        let provider_id = request.provider_id.clone();
        let provider_type = request.provider_type.clone();
        Self {
            provider,
            request,
            state: Arc::new(tokio::sync::Mutex::new(
                LazyStreamingThreadReplyState::Pending,
            )),
            provider_id,
            provider_type,
        }
    }

    pub(crate) fn progress_observer(&self) -> FeishuLarkLazyStreamingProgressObserver {
        FeishuLarkLazyStreamingProgressObserver {
            parts: self.clone(),
            state: Mutex::new(StreamingProgressState {
                last_sent_len: 0,
                next_update_at: Instant::now(),
            }),
        }
    }

    pub(crate) async fn start(&self) -> Result<bool, ProviderThreadReplyError> {
        self.ensure_started().await
    }

    async fn set_markdown(&self, text: &str) -> Result<bool, ProviderThreadReplyError> {
        if !self.ensure_started().await? {
            return Ok(false);
        }

        let mut state = self.state.lock().await;
        let LazyStreamingThreadReplyState::Ready(handle) = &mut *state else {
            return Ok(false);
        };
        handle.set_markdown(text).await?;
        tracing::debug!(
            provider.id = %handle.provider_id,
            provider.type = %handle.provider_type,
            provider.reply.message_id = %handle.provider_reply_message_id,
            card.id = %handle.card_id,
            surface.id = %handle.surface_id,
            event.hash = %handle.provider_event_id_hash,
            text.chars = text.chars().count(),
            event = "provider_thread_streaming_reply.updated",
        );
        Ok(true)
    }

    async fn finish_or_send_text(
        &self,
        request: ProviderThreadReplyRequest,
    ) -> Result<ProviderThreadReplySuccess, ProviderThreadReplyError> {
        match self.ensure_started().await {
            Ok(true) => {
                let streaming_result = {
                    let mut state = self.state.lock().await;
                    let LazyStreamingThreadReplyState::Ready(handle) = &mut *state else {
                        tracing::warn!(
                            provider.id = %request.provider_id,
                            provider.type = %request.provider_type,
                            provider.thread.id = %request.provider_thread_id,
                            surface.id = %request.surface_id,
                            event.hash = %request.provider_event_id_hash,
                            event = "provider_thread_streaming_reply.text_fallback",
                            reason = "streaming_state_not_ready",
                        );
                        return self.provider.send_thread_reply(request).await;
                    };

                    match handle.set_markdown(&request.text).await {
                        Ok(()) => handle.finish(Some(&request.text)).await,
                        Err(error) => Err(error),
                    }
                };

                match streaming_result {
                    Ok(success) => {
                        tracing::info!(
                            provider.id = %request.provider_id,
                            provider.type = %request.provider_type,
                            provider.thread.id = %request.provider_thread_id,
                            provider.reply.message_id = success.provider_reply_message_id.as_deref(),
                            surface.id = %request.surface_id,
                            event.hash = %request.provider_event_id_hash,
                            event = "provider_thread_streaming_reply.finished",
                        );
                        Ok(success)
                    }
                    Err(error) => {
                        *self.state.lock().await = LazyStreamingThreadReplyState::Disabled;
                        tracing::warn!(
                            provider.id = %error.provider_id,
                            provider.type = %error.provider_type,
                            surface.id = %error.surface_id,
                            event.hash = %error.provider_event_id_hash,
                            http.status = error.http_status,
                            error.retriable = error.retriable,
                            error = %error.message,
                            event = "provider_thread_streaming_reply.finish.failed",
                        );
                        tracing::info!(
                            provider.id = %request.provider_id,
                            provider.type = %request.provider_type,
                            provider.thread.id = %request.provider_thread_id,
                            surface.id = %request.surface_id,
                            event.hash = %request.provider_event_id_hash,
                            event = "provider_thread_streaming_reply.text_fallback",
                            reason = "streaming_finish_failed",
                        );
                        self.provider.send_thread_reply(request).await
                    }
                }
            }
            Ok(false) => {
                tracing::info!(
                    provider.id = %request.provider_id,
                    provider.type = %request.provider_type,
                    provider.thread.id = %request.provider_thread_id,
                    surface.id = %request.surface_id,
                    event.hash = %request.provider_event_id_hash,
                    event = "provider_thread_streaming_reply.text_fallback",
                    reason = "streaming_disabled",
                );
                self.provider.send_thread_reply(request).await
            }
            Err(error) => {
                tracing::warn!(
                    provider.id = %error.provider_id,
                    provider.type = %error.provider_type,
                    surface.id = %error.surface_id,
                    event.hash = %error.provider_event_id_hash,
                    http.status = error.http_status,
                    error.retriable = error.retriable,
                    error = %error.message,
                    event = "provider_thread_streaming_reply.start.failed",
                );
                self.provider.send_thread_reply(request).await
            }
        }
    }

    async fn ensure_started(&self) -> Result<bool, ProviderThreadReplyError> {
        let mut state = self.state.lock().await;
        match &*state {
            LazyStreamingThreadReplyState::Ready(_) => Ok(true),
            LazyStreamingThreadReplyState::Disabled => Ok(false),
            LazyStreamingThreadReplyState::Pending => {
                match self
                    .provider
                    .start_streaming_thread_reply(self.request.clone())
                    .await
                {
                    Ok(handle) => {
                        tracing::info!(
                            provider.id = %handle.provider_id,
                            provider.type = %handle.provider_type,
                            provider.reply.message_id = %handle.provider_reply_message_id,
                            card.id = %handle.card_id,
                            surface.id = %handle.surface_id,
                            event.hash = %handle.provider_event_id_hash,
                            event = "provider_thread_streaming_reply.started",
                        );
                        *state = LazyStreamingThreadReplyState::Ready(Box::new(handle));
                        Ok(true)
                    }
                    Err(error) => {
                        *state = LazyStreamingThreadReplyState::Disabled;
                        Err(error)
                    }
                }
            }
        }
    }
}

impl ProviderThreadReplyAdapter for FeishuLarkLazyStreamingThreadReplyParts {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn provider_type(&self) -> &str {
        &self.provider_type
    }

    fn send_thread_reply<'a>(
        &'a self,
        request: ProviderThreadReplyRequest,
    ) -> ProviderThreadReplyFuture<'a> {
        Box::pin(async move {
            if request.provider_id != self.provider_id
                || request.provider_type != self.provider_type
            {
                return Err(thread_reply_error(
                    &request,
                    "lazy streaming thread reply adapter does not match inbound provider",
                ));
            }
            self.finish_or_send_text(request).await
        })
    }
}

impl AgentControllerProgressObserver for FeishuLarkLazyStreamingProgressObserver {
    fn execution_started<'a>(&'a self) -> AgentControllerSubmitFuture<'a> {
        Box::pin(async move {
            if let Err(error) = self.parts.start().await {
                tracing::warn!(
                    provider.id = %error.provider_id,
                    provider.type = %error.provider_type,
                    surface.id = %error.surface_id,
                    event.hash = %error.provider_event_id_hash,
                    http.status = error.http_status,
                    error.retriable = error.retriable,
                    error = %error.message,
                    event = "provider_thread_streaming_reply.start.failed",
                );
            }
            Ok(())
        })
    }

    fn text_snapshot<'a>(&'a self, text: &'a str) -> AgentControllerSubmitFuture<'a> {
        Box::pin(async move {
            if !self.should_send(text) {
                return Ok(());
            }
            if let Err(error) = self.parts.set_markdown(text).await {
                tracing::warn!(
                    provider.id = %error.provider_id,
                    provider.type = %error.provider_type,
                    surface.id = %error.surface_id,
                    event.hash = %error.provider_event_id_hash,
                    http.status = error.http_status,
                    error.retriable = error.retriable,
                    error = %error.message,
                    event = "provider_thread_streaming_reply.progress_update.failed",
                );
            }
            Ok(())
        })
    }
}

impl FeishuLarkLazyStreamingProgressObserver {
    fn should_send(&self, text: &str) -> bool {
        should_send_streaming_progress(&self.state, text)
    }
}

fn should_send_streaming_progress(state: &Mutex<StreamingProgressState>, text: &str) -> bool {
    if text.trim().is_empty() {
        return false;
    }

    let now = Instant::now();
    let Ok(mut state) = state.lock() else {
        return true;
    };
    let new_len = text.len();
    if new_len <= state.last_sent_len {
        return false;
    }

    let enough_new_content =
        new_len.saturating_sub(state.last_sent_len) >= STREAMING_PROGRESS_MIN_NEW_BYTES;
    let interval_elapsed = now >= state.next_update_at;
    let paragraph_boundary = text.ends_with('\n');
    if !(enough_new_content || interval_elapsed || paragraph_boundary) {
        return false;
    }

    state.last_sent_len = new_len;
    state.next_update_at = now + STREAMING_PROGRESS_MIN_INTERVAL;
    true
}

impl FeishuLarkProvider {
    async fn send_custom_bot(
        &self,
        signal: &Signal,
        runtime: &FeishuLarkCustomBotRuntime,
    ) -> Result<ProviderSendResult, DeliveryError> {
        let provider_type = ProviderType::FeishuLark.as_str();
        let request = FeishuLarkInteractiveRequest::from_signal(
            signal,
            runtime.secret.as_deref(),
            &runtime.computer_name,
        );
        let response = self
            .client
            .post(&runtime.url)
            .json(&request)
            .send()
            .await
            .map_err(|error| {
                let is_timeout = error.is_timeout();
                provider_request_error(
                    signal,
                    &self.id,
                    provider_type,
                    "feishu_lark",
                    is_timeout,
                    error.without_url(),
                )
            })?;

        let status = response.status();
        let status_code = status.as_u16();
        if !status.is_success() {
            return Err(DeliveryError::new(
                DeliveryErrorKind::ProviderRejected,
                DeliveryErrorContext::provider_send(signal, &self.id, provider_type),
                format!(
                    "feishu_lark provider `{}` returned HTTP status {}",
                    self.id, status
                ),
            )
            .with_http_status(status_code)
            .with_retriable(is_retriable_http_status(status_code)));
        }

        let response_body = response.text().await.map_err(|error| {
            DeliveryError::new(
                DeliveryErrorKind::Network,
                DeliveryErrorContext::provider_send(signal, &self.id, provider_type),
                format!("feishu_lark provider `{}` failed to read response", self.id),
            )
            .with_http_status(status_code)
            .with_retriable(true)
            .with_source(error.without_url())
        })?;
        let provider_response: FeishuLarkResponse =
            serde_json::from_str(&response_body).map_err(|error| {
                DeliveryError::new(
                    DeliveryErrorKind::ProviderResponse,
                    DeliveryErrorContext::provider_send(signal, &self.id, provider_type),
                    format!(
                        "feishu_lark provider `{}` returned invalid response JSON",
                        self.id
                    ),
                )
                .with_http_status(status_code)
                .with_source(error)
            })?;

        if provider_response.code != 0 {
            let provider_code = provider_response.code.to_string();
            return Err(DeliveryError::new(
                DeliveryErrorKind::ProviderRejected,
                DeliveryErrorContext::provider_send(signal, &self.id, provider_type),
                format!(
                    "feishu_lark provider `{}` returned code {}: {}",
                    self.id,
                    provider_response.code,
                    provider_response
                        .msg
                        .unwrap_or_else(|| "unknown error".to_string())
                ),
            )
            .with_http_status(status_code)
            .with_provider_code(provider_code));
        }

        Ok(ProviderSendResult::sent(&self.id, provider_type, signal).with_http_status(status_code))
    }

    async fn send_app_bot(
        &self,
        signal: &Signal,
        runtime: &FeishuLarkAppBotRuntime,
    ) -> Result<ProviderSendResult, DeliveryError> {
        let provider_type = ProviderType::FeishuLark.as_str();
        let token = self.fetch_tenant_access_token(signal, runtime).await?;
        let chat_id = runtime.chat_id.as_deref().ok_or_else(|| {
            DeliveryError::new(
                DeliveryErrorKind::Config,
                DeliveryErrorContext::provider_send(signal, &self.id, provider_type),
                format!(
                    "feishu_lark provider `{}` has no fixed room configured; connect a project room with `/bind /absolute/project/path`, or set `chat_id` only for the advanced fixed-room setup",
                    self.id
                ),
            )
        })?;
        let content =
            serde_json::to_string(&FeishuLarkCard::from_signal(signal, &runtime.computer_name))
                .map_err(|error| {
                    DeliveryError::new(
                DeliveryErrorKind::Internal,
                DeliveryErrorContext::provider_send(signal, &self.id, provider_type),
                format!(
                    "feishu_lark provider `{}` failed to serialize Feishu/Lark app message content",
                    self.id
                ),
            )
            .with_source(error)
                })?;
        let request = FeishuLarkAppBotSendMessageRequest {
            receive_id: chat_id,
            msg_type: "interactive",
            content,
        };

        let response = self
            .client
            .post(format!("{}/open-apis/im/v1/messages", runtime.api_base_url))
            .query(&[("receive_id_type", "chat_id")])
            .bearer_auth(token)
            .json(&request)
            .send()
            .await
            .map_err(|error| {
                let is_timeout = error.is_timeout();
                provider_request_error(
                    signal,
                    &self.id,
                    provider_type,
                    "feishu_lark",
                    is_timeout,
                    error.without_url(),
                )
            })?;

        let status = response.status();
        let status_code = status.as_u16();
        let response_body = response.text().await.map_err(|error| {
            DeliveryError::new(
                DeliveryErrorKind::Network,
                DeliveryErrorContext::provider_send(signal, &self.id, provider_type),
                format!(
                    "feishu_lark provider `{}` failed to read Feishu/Lark app send response",
                    self.id
                ),
            )
            .with_http_status(status_code)
            .with_retriable(true)
            .with_source(error.without_url())
        })?;

        if !status.is_success() {
            return Err(DeliveryError::new(
                DeliveryErrorKind::ProviderRejected,
                DeliveryErrorContext::provider_send(signal, &self.id, provider_type),
                format!(
                    "feishu_lark provider `{}` returned HTTP status {} while sending Feishu/Lark app message",
                    self.id, status
                ),
            )
            .with_http_status(status_code)
            .with_retriable(is_retriable_http_status(status_code)));
        }

        let provider_response: FeishuLarkAppBotSendMessageResponse =
            serde_json::from_str(&response_body).map_err(|error| {
                DeliveryError::new(
                DeliveryErrorKind::ProviderResponse,
                DeliveryErrorContext::provider_send(signal, &self.id, provider_type),
                format!(
                    "feishu_lark provider `{}` returned invalid Feishu/Lark app send response JSON",
                    self.id
                ),
            )
            .with_http_status(status_code)
            .with_source(error)
            })?;

        if provider_response.code != 0 {
            let provider_code = provider_response.code.to_string();
            return Err(DeliveryError::new(
                DeliveryErrorKind::ProviderRejected,
                DeliveryErrorContext::provider_send(signal, &self.id, provider_type),
                format!(
                    "feishu_lark provider `{}` returned code {} while sending Feishu/Lark app message: {}",
                    self.id,
                    provider_response.code,
                    provider_response
                        .msg
                        .unwrap_or_else(|| "unknown error".to_string())
                ),
            )
            .with_http_status(status_code)
            .with_provider_code(provider_code));
        }

        let data = provider_response.data.ok_or_else(|| {
            DeliveryError::new(
                DeliveryErrorKind::ProviderResponse,
                DeliveryErrorContext::provider_send(signal, &self.id, provider_type),
                format!(
                    "feishu_lark provider `{}` Feishu/Lark app send response did not include message data",
                    self.id
                ),
            )
            .with_http_status(status_code)
        })?;
        let receipt = app_bot_surface_ready_receipt(signal, &self.id, provider_type, runtime, data)
            .map_err(|error| (*error).with_http_status(status_code))?;
        let provider_message_id = receipt.provider_message_id.clone();
        let mut result = ProviderSendResult::sent(&self.id, provider_type, signal)
            .with_http_status(status_code)
            .with_delivery_receipt(receipt);
        if let Some(provider_message_id) = provider_message_id {
            result = result.with_provider_message_id(provider_message_id);
        }

        Ok(result)
    }

    async fn send_app_bot_thread_notification(
        &self,
        signal: &Signal,
        runtime: &FeishuLarkAppBotRuntime,
        provider_account_id: &str,
        provider_conversation_id: &str,
        provider_thread_id: &str,
    ) -> Result<ProviderSendResult, DeliveryError> {
        let provider_type = ProviderType::FeishuLark.as_str();
        let token = self.fetch_tenant_access_token(signal, runtime).await?;
        let card = FeishuLarkCard::from_signal(signal, &runtime.computer_name);
        let content = serde_json::to_string(&card).map_err(|error| {
            DeliveryError::new(
                DeliveryErrorKind::Internal,
                DeliveryErrorContext::provider_send(signal, &self.id, provider_type),
                format!(
                    "feishu_lark provider `{}` failed to serialize Feishu/Lark thread message content",
                    self.id
                ),
            )
            .with_source(error)
        })?;
        let body = FeishuLarkAppBotReplyMessageRequest {
            content,
            msg_type: "interactive",
            reply_in_thread: true,
            uuid: thread_notification_uuid(&signal.id, provider_thread_id),
        };

        let response = self
            .client
            .post(format!(
                "{}/open-apis/im/v1/messages/{}/reply",
                runtime.api_base_url, provider_thread_id
            ))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                let is_timeout = error.is_timeout();
                provider_request_error(
                    signal,
                    &self.id,
                    provider_type,
                    "feishu_lark",
                    is_timeout,
                    error.without_url(),
                )
            })?;

        let status = response.status();
        let status_code = status.as_u16();
        let response_body = response.text().await.map_err(|error| {
            DeliveryError::new(
                DeliveryErrorKind::Network,
                DeliveryErrorContext::provider_send(signal, &self.id, provider_type),
                format!(
                    "feishu_lark provider `{}` failed to read Feishu/Lark thread message response",
                    self.id
                ),
            )
            .with_http_status(status_code)
            .with_retriable(true)
            .with_source(error.without_url())
        })?;

        if !status.is_success() {
            return Err(DeliveryError::new(
                DeliveryErrorKind::ProviderRejected,
                DeliveryErrorContext::provider_send(signal, &self.id, provider_type),
                format!(
                    "feishu_lark provider `{}` returned HTTP status {} while sending Feishu/Lark thread message",
                    self.id, status
                ),
            )
            .with_http_status(status_code)
            .with_retriable(is_retriable_http_status(status_code)));
        }

        let provider_response: FeishuLarkAppBotReplyMessageResponse =
            serde_json::from_str(&response_body).map_err(|error| {
                DeliveryError::new(
                    DeliveryErrorKind::ProviderResponse,
                    DeliveryErrorContext::provider_send(signal, &self.id, provider_type),
                    format!(
                        "feishu_lark provider `{}` returned invalid Feishu/Lark thread message response JSON",
                        self.id
                    ),
                )
                .with_http_status(status_code)
                .with_source(error)
            })?;
        if provider_response.code != 0 {
            let provider_code = provider_response.code.to_string();
            return Err(DeliveryError::new(
                DeliveryErrorKind::ProviderRejected,
                DeliveryErrorContext::provider_send(signal, &self.id, provider_type),
                format!(
                    "feishu_lark provider `{}` returned code {} while sending Feishu/Lark thread message: {}",
                    self.id,
                    provider_response.code,
                    provider_response
                        .msg
                        .unwrap_or_else(|| "unknown error".to_string())
                ),
            )
            .with_http_status(status_code)
            .with_provider_code(provider_code));
        }

        let data = provider_response.data.ok_or_else(|| {
            DeliveryError::new(
                DeliveryErrorKind::ProviderResponse,
                DeliveryErrorContext::provider_send(signal, &self.id, provider_type),
                format!(
                    "feishu_lark provider `{}` Feishu/Lark thread message response did not include message data",
                    self.id
                ),
            )
            .with_http_status(status_code)
        })?;
        let receipt = app_bot_thread_surface_ready_receipt(
            signal,
            &self.id,
            provider_type,
            provider_account_id,
            provider_conversation_id,
            provider_thread_id,
            data,
        )
        .map_err(|error| (*error).with_http_status(status_code))?;
        let provider_message_id = receipt.provider_message_id.clone();
        let mut result = ProviderSendResult::sent(&self.id, provider_type, signal)
            .with_http_status(status_code)
            .with_delivery_receipt(receipt);
        if let Some(provider_message_id) = provider_message_id {
            result = result.with_provider_message_id(provider_message_id);
        }

        Ok(result)
    }

    async fn fetch_tenant_access_token(
        &self,
        signal: &Signal,
        runtime: &FeishuLarkAppBotRuntime,
    ) -> Result<String, DeliveryError> {
        let provider_type = ProviderType::FeishuLark.as_str();
        let request = FeishuLarkTenantAccessTokenRequest {
            app_id: &runtime.app_id,
            app_secret: &runtime.app_secret,
        };
        let response = self
            .client
            .post(format!(
                "{}/open-apis/auth/v3/tenant_access_token/internal",
                runtime.api_base_url
            ))
            .json(&request)
            .send()
            .await
            .map_err(|error| {
                let is_timeout = error.is_timeout();
                provider_request_error(
                    signal,
                    &self.id,
                    provider_type,
                    "feishu_lark",
                    is_timeout,
                    error.without_url(),
                )
            })?;

        let status = response.status();
        let status_code = status.as_u16();
        let response_body = response.text().await.map_err(|error| {
            DeliveryError::new(
                DeliveryErrorKind::Network,
                DeliveryErrorContext::provider_send(signal, &self.id, provider_type),
                format!(
                    "feishu_lark provider `{}` failed to read tenant access token response",
                    self.id
                ),
            )
            .with_http_status(status_code)
            .with_retriable(true)
            .with_source(error.without_url())
        })?;

        if !status.is_success() {
            return Err(DeliveryError::new(
                DeliveryErrorKind::ProviderRejected,
                DeliveryErrorContext::provider_send(signal, &self.id, provider_type),
                format!(
                    "feishu_lark provider `{}` returned HTTP status {} while fetching tenant access token",
                    self.id, status
                ),
            )
            .with_http_status(status_code)
            .with_retriable(is_retriable_http_status(status_code)));
        }

        let provider_response: FeishuLarkTenantAccessTokenResponse = match serde_json::from_str(
            &response_body,
        ) {
            Ok(response) => response,
            Err(error) => {
                return Err(DeliveryError::new(
                        DeliveryErrorKind::ProviderResponse,
                        DeliveryErrorContext::provider_send(signal, &self.id, provider_type),
                        format!(
                            "feishu_lark provider `{}` returned invalid tenant access token response JSON",
                            self.id
                        ),
                    )
                    .with_http_status(status_code)
                    .with_source(error));
            }
        };

        if provider_response.code != 0 {
            let provider_code = provider_response.code.to_string();
            return Err(DeliveryError::new(
                DeliveryErrorKind::ProviderRejected,
                DeliveryErrorContext::provider_send(signal, &self.id, provider_type),
                format!(
                    "feishu_lark provider `{}` returned code {} while fetching tenant access token: {}",
                    self.id,
                    provider_response.code,
                    provider_response.msg.unwrap_or_else(|| "unknown error".to_string())
                ),
            )
            .with_http_status(status_code)
            .with_provider_code(provider_code));
        }

        let Some(token) = present_owned(provider_response.tenant_access_token) else {
            return Err(DeliveryError::new(
                DeliveryErrorKind::ProviderResponse,
                DeliveryErrorContext::provider_send(signal, &self.id, provider_type),
                format!(
                    "feishu_lark provider `{}` tenant access token response did not include a token",
                    self.id
                ),
            )
            .with_http_status(status_code));
        };
        if provider_response.expire.unwrap_or_default() <= 0 {
            return Err(DeliveryError::new(
                DeliveryErrorKind::ProviderResponse,
                DeliveryErrorContext::provider_send(signal, &self.id, provider_type),
                format!(
                    "feishu_lark provider `{}` tenant access token response had an invalid expiry",
                    self.id
                ),
            )
            .with_http_status(status_code));
        }

        Ok(token)
    }

    async fn send_app_bot_thread_reply(
        &self,
        request: ProviderThreadReplyRequest,
        runtime: &FeishuLarkAppBotRuntime,
    ) -> Result<ProviderThreadReplySuccess, ProviderThreadReplyError> {
        validate_app_bot_thread_reply_request(&request, &self.id, runtime)?;

        let token = self
            .fetch_tenant_access_token_for_thread_reply(&request, runtime)
            .await?;
        let content = serde_json::to_string(&FeishuLarkTextMessageContent {
            text: request.text.as_str(),
        })
        .map_err(|error| {
            thread_reply_error(&request, format!("failed to serialize text reply: {error}"))
        })?;
        let body = FeishuLarkAppBotReplyMessageRequest {
            content,
            msg_type: "text",
            reply_in_thread: true,
            uuid: thread_reply_uuid(&request.provider_event_id_hash),
        };

        let response = self
            .client
            .post(format!(
                "{}/open-apis/im/v1/messages/{}/reply",
                runtime.api_base_url, request.provider_thread_id
            ))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                thread_reply_error(
                    &request,
                    format!(
                        "failed to send Feishu/Lark thread reply: {}",
                        error.without_url()
                    ),
                )
                .with_retriable(true)
            })?;

        let status = response.status();
        let response_body = response.text().await.map_err(|error| {
            thread_reply_error(
                &request,
                format!(
                    "failed to read Feishu/Lark thread reply response: {}",
                    error.without_url()
                ),
            )
            .with_retriable(true)
        })?;
        if !status.is_success() {
            let status_code = status.as_u16();
            return Err(thread_reply_error(
                &request,
                format!("Feishu/Lark thread reply returned HTTP status {}", status),
            )
            .with_http_status(status_code)
            .with_retriable(is_retriable_http_status(status_code)));
        }

        let provider_response: FeishuLarkAppBotReplyMessageResponse =
            serde_json::from_str(&response_body).map_err(|error| {
                thread_reply_error(
                    &request,
                    format!("Feishu/Lark thread reply returned invalid JSON: {error}"),
                )
            })?;
        if provider_response.code != 0 {
            return Err(thread_reply_error(
                &request,
                format!(
                    "Feishu/Lark thread reply returned code {}: {}",
                    provider_response.code,
                    provider_response
                        .msg
                        .unwrap_or_else(|| "unknown error".to_string())
                ),
            ));
        }

        let data = provider_response.data.ok_or_else(|| {
            thread_reply_error(
                &request,
                "Feishu/Lark thread reply did not include message data",
            )
        })?;
        let provider_reply_message_id =
            required_thread_reply_response_field(&request, "message_id", data.message_id)?;
        let root_id = required_thread_reply_response_field(&request, "root_id", data.root_id)?;
        if root_id != request.provider_thread_id {
            return Err(thread_reply_error(
                &request,
                "Feishu/Lark thread reply root_id did not match the response surface thread",
            ));
        }

        Ok(ProviderThreadReplySuccess {
            provider_reply_message_id: Some(provider_reply_message_id),
        })
    }

    async fn start_app_bot_streaming_thread_reply(
        &self,
        request: FeishuLarkStreamingThreadReplyRequest,
        runtime: &FeishuLarkAppBotRuntime,
    ) -> Result<FeishuLarkStreamingThreadReply, ProviderThreadReplyError> {
        validate_app_bot_streaming_thread_reply_request(&request, &self.id, runtime)?;

        let token = self
            .fetch_tenant_access_token_for_streaming_thread_reply(&request, runtime)
            .await?;
        let card = streaming_markdown_card(STREAMING_REPLY_INITIAL_TEXT, true);
        let create_body = FeishuLarkCardKitCreateRequest {
            card_type: "card_json",
            data: serde_json::to_string(&card).map_err(|error| {
                streaming_thread_reply_error(
                    &request,
                    format!("failed to serialize streaming reply card: {error}"),
                )
            })?,
        };
        let create_response = self
            .client
            .post(format!(
                "{}/open-apis/cardkit/v1/cards",
                runtime.api_base_url
            ))
            .bearer_auth(&token)
            .json(&create_body)
            .send()
            .await
            .map_err(|error| {
                streaming_thread_reply_error(
                    &request,
                    format!(
                        "failed to create Feishu/Lark streaming reply card: {}",
                        error.without_url()
                    ),
                )
                .with_retriable(true)
            })?;
        let create_status = create_response.status();
        let create_body = create_response.text().await.map_err(|error| {
            streaming_thread_reply_error(
                &request,
                format!(
                    "failed to read Feishu/Lark streaming reply card create response: {}",
                    error.without_url()
                ),
            )
            .with_retriable(true)
        })?;
        if !create_status.is_success() {
            let status_code = create_status.as_u16();
            return Err(streaming_thread_reply_error(
                &request,
                format!(
                    "Feishu/Lark streaming reply card create returned HTTP status {}",
                    create_status
                ),
            )
            .with_http_status(status_code)
            .with_retriable(is_retriable_http_status(status_code)));
        }
        let create_response: FeishuLarkCardKitCreateResponse = serde_json::from_str(&create_body)
            .map_err(|error| {
            streaming_thread_reply_error(
                &request,
                format!("Feishu/Lark streaming reply card create returned invalid JSON: {error}"),
            )
        })?;
        if create_response.code != 0 {
            return Err(streaming_thread_reply_error(
                &request,
                format!(
                    "Feishu/Lark streaming reply card create returned code {}: {}",
                    create_response.code,
                    create_response
                        .msg
                        .unwrap_or_else(|| "unknown error".to_string())
                ),
            ));
        }
        let card_id = create_response
            .data
            .and_then(|data| present_owned(data.card_id))
            .ok_or_else(|| {
                streaming_thread_reply_error(
                    &request,
                    "Feishu/Lark streaming reply card create did not include card_id",
                )
            })?;

        let content = serde_json::to_string(&FeishuLarkCardReferenceContent {
            content_type: "card",
            data: FeishuLarkCardReferenceData {
                card_id: card_id.as_str(),
            },
        })
        .map_err(|error| {
            streaming_thread_reply_error(
                &request,
                format!("failed to serialize streaming card reference: {error}"),
            )
        })?;
        let reply_body = FeishuLarkAppBotReplyMessageRequest {
            content,
            msg_type: "interactive",
            reply_in_thread: true,
            uuid: streaming_thread_reply_uuid(&request.provider_event_id_hash),
        };
        let reply_response = self
            .client
            .post(format!(
                "{}/open-apis/im/v1/messages/{}/reply",
                runtime.api_base_url, request.provider_thread_id
            ))
            .bearer_auth(&token)
            .json(&reply_body)
            .send()
            .await
            .map_err(|error| {
                streaming_thread_reply_error(
                    &request,
                    format!(
                        "failed to send Feishu/Lark streaming reply card: {}",
                        error.without_url()
                    ),
                )
                .with_retriable(true)
            })?;
        let reply_status = reply_response.status();
        let reply_body = reply_response.text().await.map_err(|error| {
            streaming_thread_reply_error(
                &request,
                format!(
                    "failed to read Feishu/Lark streaming reply card message response: {}",
                    error.without_url()
                ),
            )
            .with_retriable(true)
        })?;
        if !reply_status.is_success() {
            let status_code = reply_status.as_u16();
            return Err(streaming_thread_reply_error(
                &request,
                format!(
                    "Feishu/Lark streaming reply card message returned HTTP status {}",
                    reply_status
                ),
            )
            .with_http_status(status_code)
            .with_retriable(is_retriable_http_status(status_code)));
        }
        let reply_response: FeishuLarkAppBotReplyMessageResponse =
            serde_json::from_str(&reply_body).map_err(|error| {
                streaming_thread_reply_error(
                    &request,
                    format!(
                        "Feishu/Lark streaming reply card message returned invalid JSON: {error}"
                    ),
                )
            })?;
        if reply_response.code != 0 {
            return Err(streaming_thread_reply_error(
                &request,
                format!(
                    "Feishu/Lark streaming reply card message returned code {}: {}",
                    reply_response.code,
                    reply_response
                        .msg
                        .unwrap_or_else(|| "unknown error".to_string())
                ),
            ));
        }
        let data = reply_response.data.ok_or_else(|| {
            streaming_thread_reply_error(
                &request,
                "Feishu/Lark streaming reply card message did not include message data",
            )
        })?;
        let provider_reply_message_id = required_streaming_thread_reply_response_field(
            &request,
            "message_id",
            data.message_id,
        )?;
        let root_id =
            required_streaming_thread_reply_response_field(&request, "root_id", data.root_id)?;
        if root_id != request.provider_thread_id {
            return Err(streaming_thread_reply_error(
                &request,
                "Feishu/Lark streaming reply card root_id did not match the response surface thread",
            ));
        }

        Ok(FeishuLarkStreamingThreadReply {
            client: self.client.clone(),
            api_base_url: runtime.api_base_url.clone(),
            token,
            provider_id: request.provider_id,
            provider_type: request.provider_type,
            surface_id: request.surface_id,
            provider_event_id_hash: request.provider_event_id_hash,
            card_id,
            provider_reply_message_id,
            sequence: 0,
        })
    }

    async fn send_app_bot_conversation_text(
        &self,
        request: FeishuLarkConversationTextRequest,
        runtime: &FeishuLarkAppBotRuntime,
    ) -> anyhow::Result<FeishuLarkConversationTextSuccess> {
        validate_app_bot_conversation_text_request(&request, &self.id, runtime)?;
        let token = self
            .fetch_tenant_access_token_for_conversation_text(&request, runtime)
            .await?;
        let content = serde_json::to_string(&FeishuLarkTextMessageContent {
            text: request.text.as_str(),
        })
        .context("failed to serialize Feishu/Lark room text content")?;
        let body = FeishuLarkAppBotSendMessageRequest {
            receive_id: &request.provider_conversation_id,
            msg_type: "text",
            content,
        };

        let response = self
            .client
            .post(format!("{}/open-apis/im/v1/messages", runtime.api_base_url))
            .query(&[("receive_id_type", "chat_id")])
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                anyhow!(
                    "failed to send Feishu/Lark room message: {}",
                    error.without_url()
                )
            })?;
        let status = response.status();
        let response_body = response.text().await.map_err(|error| {
            anyhow!(
                "failed to read Feishu/Lark room message response: {}",
                error.without_url()
            )
        })?;
        ensure!(
            status.is_success(),
            "Feishu/Lark room message returned HTTP status {}",
            status
        );

        let provider_response: FeishuLarkAppBotSendMessageResponse =
            serde_json::from_str(&response_body)
                .context("Feishu/Lark room message response returned invalid JSON")?;
        ensure!(
            provider_response.code == 0,
            "Feishu/Lark room message returned code {}: {}",
            provider_response.code,
            provider_response
                .msg
                .unwrap_or_else(|| "unknown error".to_string())
        );
        let data = provider_response
            .data
            .context("Feishu/Lark room message response did not include message data")?;
        let message_id = present_owned(data.message_id)
            .context("Feishu/Lark room message response did not include message_id")?;
        let chat_id = present_owned(data.chat_id)
            .context("Feishu/Lark room message response did not include chat_id")?;
        ensure!(
            chat_id == request.provider_conversation_id,
            "Feishu/Lark room message response chat_id did not match target room"
        );
        if let Some(sender) = data.sender
            && let Some(tenant_key) = present_owned(sender.tenant_key)
        {
            ensure!(
                tenant_key == request.provider_account_id,
                "Feishu/Lark room message response tenant_key did not match target tenant"
            );
        }

        Ok(FeishuLarkConversationTextSuccess {
            provider_message_id: Some(message_id),
        })
    }

    async fn send_app_bot_project_room_prompt(
        &self,
        request: ProjectRoomPromptRequest,
        runtime: &FeishuLarkAppBotRuntime,
    ) -> Result<ProjectRoomPromptResult, DeliveryError> {
        let provider_type = ProviderType::FeishuLark.as_str();
        let context = || {
            DeliveryErrorContext::provider_control(&request.proposal_id, &self.id, provider_type)
        };
        let prompt_conversation_id = request
            .prompt_conversation_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(|value| ("chat_id", value));
        let operator_open_id = runtime
            .operator_open_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(|value| ("open_id", value));
        let (receive_id_type, receive_id) = prompt_conversation_id
            .or(operator_open_id)
            .ok_or_else(|| {
                DeliveryError::new(
                    DeliveryErrorKind::Config,
                    context(),
                    format!(
                        "feishu_lark provider `{}` cannot prompt for project rooms because neither prompt_conversation_id nor operator_open_id is configured",
                        self.id
                    ),
                )
        })?;
        let token = self
            .fetch_tenant_access_token_for_project_room_prompt(&request, runtime)
            .await?;
        let content =
            serde_json::to_string(&project_room_prompt_card(&request)).map_err(|error| {
                DeliveryError::new(
                    DeliveryErrorKind::Internal,
                    context(),
                    format!(
                        "feishu_lark provider `{}` failed to serialize project room prompt card",
                        self.id
                    ),
                )
                .with_source(error)
            })?;
        let body = FeishuLarkAppBotSendMessageRequest {
            receive_id,
            msg_type: "interactive",
            content,
        };

        let response = self
            .client
            .post(format!("{}/open-apis/im/v1/messages", runtime.api_base_url))
            .query(&[("receive_id_type", receive_id_type)])
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                let is_timeout = error.is_timeout();
                let kind = if is_timeout {
                    DeliveryErrorKind::Timeout
                } else {
                    DeliveryErrorKind::Network
                };
                DeliveryError::new(
                    kind,
                    context(),
                    format!(
                        "feishu_lark provider `{}` request failed while sending project room prompt",
                        self.id
                    ),
                )
                .with_retriable(true)
                .with_source(error.without_url())
            })?;
        let status = response.status();
        let status_code = status.as_u16();
        let response_body = response.text().await.map_err(|error| {
            DeliveryError::new(
                DeliveryErrorKind::Network,
                context(),
                format!(
                    "feishu_lark provider `{}` failed to read project room prompt response",
                    self.id
                ),
            )
            .with_http_status(status_code)
            .with_retriable(true)
            .with_source(error.without_url())
        })?;

        if !status.is_success() {
            return Err(DeliveryError::new(
                DeliveryErrorKind::ProviderRejected,
                context(),
                format!(
                    "feishu_lark provider `{}` returned HTTP status {} while sending project room prompt",
                    self.id, status
                ),
            )
            .with_http_status(status_code)
            .with_retriable(is_retriable_http_status(status_code)));
        }

        let provider_response: FeishuLarkAppBotSendMessageResponse =
            serde_json::from_str(&response_body).map_err(|error| {
                DeliveryError::new(
                DeliveryErrorKind::ProviderResponse,
                context(),
                format!(
                    "feishu_lark provider `{}` returned invalid project room prompt response JSON",
                    self.id
                ),
            )
            .with_http_status(status_code)
            .with_source(error)
            })?;
        if provider_response.code != 0 {
            let provider_code = provider_response.code.to_string();
            return Err(DeliveryError::new(
                DeliveryErrorKind::ProviderRejected,
                context(),
                format!(
                    "feishu_lark provider `{}` returned code {} while sending project room prompt: {}",
                    self.id,
                    provider_response.code,
                    provider_response
                        .msg
                        .unwrap_or_else(|| "unknown error".to_string())
                ),
            )
            .with_http_status(status_code)
            .with_provider_code(provider_code));
        }
        let data = provider_response.data.ok_or_else(|| {
            DeliveryError::new(
                DeliveryErrorKind::ProviderResponse,
                context(),
                format!(
                    "feishu_lark provider `{}` project room prompt response did not include message data",
                    self.id
                ),
            )
            .with_http_status(status_code)
        })?;
        let provider_account_id = data
            .sender
            .and_then(|sender| sender.tenant_key)
            .and_then(|tenant_key| present_owned(Some(tenant_key)));

        Ok(ProjectRoomPromptResult {
            provider_account_id,
            provider_conversation_id: data
                .chat_id
                .and_then(|chat_id| present_owned(Some(chat_id))),
            provider_message_id: data
                .message_id
                .and_then(|message_id| present_owned(Some(message_id))),
        })
    }

    async fn create_app_bot_project_room(
        &self,
        request: FeishuLarkCreateProjectRoomRequest,
        runtime: &FeishuLarkAppBotRuntime,
    ) -> anyhow::Result<FeishuLarkCreateProjectRoomSuccess> {
        validate_app_bot_create_project_room_request(&request, &self.id, runtime)?;
        let token = self
            .fetch_tenant_access_token_for_project_room_create(&request, runtime)
            .await?;
        let body = FeishuLarkCreateChatRequest {
            name: project_room_name(&request.project_name),
            description: project_room_description(&request.project_path),
            owner_id: request.operator_open_id.clone(),
            user_id_list: vec![request.operator_open_id.clone()],
            group_message_type: "chat",
            chat_mode: "group",
            chat_type: "private",
            join_message_visibility: "all_members",
            leave_message_visibility: "all_members",
        };
        let uuid = project_room_create_uuid(&request.proposal_id, request.proposal_created_at);

        let response = self
            .client
            .post(format!("{}/open-apis/im/v1/chats", runtime.api_base_url))
            .query(&[
                ("user_id_type", "open_id"),
                ("set_bot_manager", "true"),
                ("uuid", uuid.as_str()),
            ])
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                anyhow!(
                    "failed to create Feishu/Lark project room for `{}`: {}",
                    request.project_path,
                    error.without_url()
                )
            })?;
        let status = response.status();
        let response_body = response.text().await.map_err(|error| {
            anyhow!(
                "failed to read Feishu/Lark project room create response for `{}`: {}",
                request.project_path,
                error.without_url()
            )
        })?;
        ensure!(
            status.is_success(),
            "Feishu/Lark project room create returned HTTP status {}",
            status
        );

        let provider_response: FeishuLarkCreateChatResponse = serde_json::from_str(&response_body)
            .context("Feishu/Lark project room create response returned invalid JSON")?;
        ensure!(
            provider_response.code == 0,
            "Feishu/Lark project room create returned code {}: {}",
            provider_response.code,
            provider_response
                .msg
                .unwrap_or_else(|| "unknown error".to_string())
        );
        let data = provider_response
            .data
            .context("Feishu/Lark project room create response did not include data")?;
        let chat_id = present_owned(data.chat_id)
            .context("Feishu/Lark project room create response did not include chat_id")?;

        Ok(FeishuLarkCreateProjectRoomSuccess {
            provider_conversation_id: chat_id,
        })
    }

    async fn fetch_tenant_access_token_for_project_room_prompt(
        &self,
        request: &ProjectRoomPromptRequest,
        runtime: &FeishuLarkAppBotRuntime,
    ) -> Result<String, DeliveryError> {
        let context = || {
            DeliveryErrorContext::provider_control(
                &request.proposal_id,
                &self.id,
                ProviderType::FeishuLark.as_str(),
            )
        };
        let token_request = FeishuLarkTenantAccessTokenRequest {
            app_id: &runtime.app_id,
            app_secret: &runtime.app_secret,
        };
        let response = self
            .client
            .post(format!(
                "{}/open-apis/auth/v3/tenant_access_token/internal",
                runtime.api_base_url
            ))
            .json(&token_request)
            .send()
            .await
            .map_err(|error| {
                let kind = if error.is_timeout() {
                    DeliveryErrorKind::Timeout
                } else {
                    DeliveryErrorKind::Network
                };
                DeliveryError::new(
                    kind,
                    context(),
                    format!(
                        "feishu_lark provider `{}` failed to fetch tenant access token for project room prompt",
                        self.id
                    ),
                )
                .with_retriable(true)
                .with_source(error.without_url())
            })?;
        let status = response.status();
        let status_code = status.as_u16();
        let response_body = response.text().await.map_err(|error| {
            DeliveryError::new(
                DeliveryErrorKind::Network,
                context(),
                format!(
                    "feishu_lark provider `{}` failed to read tenant access token response for project room prompt",
                    self.id
                ),
            )
            .with_http_status(status_code)
            .with_retriable(true)
            .with_source(error.without_url())
        })?;
        if !status.is_success() {
            return Err(DeliveryError::new(
                DeliveryErrorKind::ProviderRejected,
                context(),
                format!(
                    "tenant access token request for Feishu/Lark project room prompt returned HTTP status {}",
                    status
                ),
            )
            .with_http_status(status_code)
            .with_retriable(is_retriable_http_status(status_code)));
        }

        let provider_response: FeishuLarkTenantAccessTokenResponse =
            serde_json::from_str(&response_body).map_err(|error| {
                DeliveryError::new(
                DeliveryErrorKind::ProviderResponse,
                context(),
                "tenant access token response for Feishu/Lark project room prompt was invalid JSON",
            )
            .with_http_status(status_code)
            .with_source(error)
            })?;
        if provider_response.code != 0 {
            let provider_code = provider_response.code.to_string();
            return Err(DeliveryError::new(
                DeliveryErrorKind::ProviderRejected,
                context(),
                format!(
                    "tenant access token request for Feishu/Lark project room prompt returned code {}: {}",
                    provider_response.code,
                    provider_response
                        .msg
                        .unwrap_or_else(|| "unknown error".to_string())
                ),
            )
            .with_http_status(status_code)
            .with_provider_code(provider_code));
        }
        let token = present_owned(provider_response.tenant_access_token).ok_or_else(|| {
            DeliveryError::new(
                DeliveryErrorKind::ProviderResponse,
                context(),
                "tenant access token response for Feishu/Lark project room prompt did not include a token",
            )
            .with_http_status(status_code)
        })?;
        if provider_response.expire.unwrap_or_default() <= 0 {
            return Err(DeliveryError::new(
                DeliveryErrorKind::ProviderResponse,
                context(),
                "tenant access token response for Feishu/Lark project room prompt had an invalid expiry",
            )
            .with_http_status(status_code));
        }

        Ok(token)
    }

    async fn fetch_tenant_access_token_for_thread_reply(
        &self,
        request: &ProviderThreadReplyRequest,
        runtime: &FeishuLarkAppBotRuntime,
    ) -> Result<String, ProviderThreadReplyError> {
        let token_request = FeishuLarkTenantAccessTokenRequest {
            app_id: &runtime.app_id,
            app_secret: &runtime.app_secret,
        };
        let response = self
            .client
            .post(format!(
                "{}/open-apis/auth/v3/tenant_access_token/internal",
                runtime.api_base_url
            ))
            .json(&token_request)
            .send()
            .await
            .map_err(|error| {
                thread_reply_error(
                    request,
                    format!(
                        "failed to fetch tenant access token: {}",
                        error.without_url()
                    ),
                )
                .with_retriable(true)
            })?;

        let status = response.status();
        let response_body = response.text().await.map_err(|error| {
            thread_reply_error(
                request,
                format!(
                    "failed to read tenant access token response: {}",
                    error.without_url()
                ),
            )
            .with_retriable(true)
        })?;
        if !status.is_success() {
            let status_code = status.as_u16();
            return Err(thread_reply_error(
                request,
                format!(
                    "tenant access token request returned HTTP status {}",
                    status
                ),
            )
            .with_http_status(status_code)
            .with_retriable(is_retriable_http_status(status_code)));
        }

        let provider_response: FeishuLarkTenantAccessTokenResponse =
            serde_json::from_str(&response_body).map_err(|error| {
                thread_reply_error(
                    request,
                    format!("tenant access token response was invalid JSON: {error}"),
                )
            })?;
        if provider_response.code != 0 {
            return Err(thread_reply_error(
                request,
                format!(
                    "tenant access token request returned code {}: {}",
                    provider_response.code,
                    provider_response
                        .msg
                        .unwrap_or_else(|| "unknown error".to_string())
                ),
            ));
        }
        let Some(token) = present_owned(provider_response.tenant_access_token) else {
            return Err(thread_reply_error(
                request,
                "tenant access token response did not include a token",
            ));
        };
        if provider_response.expire.unwrap_or_default() <= 0 {
            return Err(thread_reply_error(
                request,
                "tenant access token response had an invalid expiry",
            ));
        }

        Ok(token)
    }

    async fn fetch_tenant_access_token_for_streaming_thread_reply(
        &self,
        request: &FeishuLarkStreamingThreadReplyRequest,
        runtime: &FeishuLarkAppBotRuntime,
    ) -> Result<String, ProviderThreadReplyError> {
        let token_request = FeishuLarkTenantAccessTokenRequest {
            app_id: &runtime.app_id,
            app_secret: &runtime.app_secret,
        };
        let response = self
            .client
            .post(format!(
                "{}/open-apis/auth/v3/tenant_access_token/internal",
                runtime.api_base_url
            ))
            .json(&token_request)
            .send()
            .await
            .map_err(|error| {
                streaming_thread_reply_error(
                    request,
                    format!(
                        "failed to fetch tenant access token: {}",
                        error.without_url()
                    ),
                )
                .with_retriable(true)
            })?;

        let status = response.status();
        let response_body = response.text().await.map_err(|error| {
            streaming_thread_reply_error(
                request,
                format!(
                    "failed to read tenant access token response: {}",
                    error.without_url()
                ),
            )
            .with_retriable(true)
        })?;
        if !status.is_success() {
            let status_code = status.as_u16();
            return Err(streaming_thread_reply_error(
                request,
                format!(
                    "tenant access token request returned HTTP status {}",
                    status
                ),
            )
            .with_http_status(status_code)
            .with_retriable(is_retriable_http_status(status_code)));
        }

        let provider_response: FeishuLarkTenantAccessTokenResponse =
            serde_json::from_str(&response_body).map_err(|error| {
                streaming_thread_reply_error(
                    request,
                    format!("tenant access token response was invalid JSON: {error}"),
                )
            })?;
        if provider_response.code != 0 {
            return Err(streaming_thread_reply_error(
                request,
                format!(
                    "tenant access token request returned code {}: {}",
                    provider_response.code,
                    provider_response
                        .msg
                        .unwrap_or_else(|| "unknown error".to_string())
                ),
            ));
        }
        let Some(token) = present_owned(provider_response.tenant_access_token) else {
            return Err(streaming_thread_reply_error(
                request,
                "tenant access token response did not include a token",
            ));
        };
        if provider_response.expire.unwrap_or_default() <= 0 {
            return Err(streaming_thread_reply_error(
                request,
                "tenant access token response had an invalid expiry",
            ));
        }

        Ok(token)
    }

    async fn fetch_tenant_access_token_for_conversation_text(
        &self,
        request: &FeishuLarkConversationTextRequest,
        runtime: &FeishuLarkAppBotRuntime,
    ) -> anyhow::Result<String> {
        let token_request = FeishuLarkTenantAccessTokenRequest {
            app_id: &runtime.app_id,
            app_secret: &runtime.app_secret,
        };
        let response = self
            .client
            .post(format!(
                "{}/open-apis/auth/v3/tenant_access_token/internal",
                runtime.api_base_url
            ))
            .json(&token_request)
            .send()
            .await
            .map_err(|error| {
                anyhow!(
                    "failed to fetch tenant access token for Feishu/Lark room message `{}`: {}",
                    request.provider_event_id_hash,
                    error.without_url()
                )
            })?;
        let status = response.status();
        let response_body = response.text().await.map_err(|error| {
            anyhow!(
                "failed to read tenant access token response for Feishu/Lark room message `{}`: {}",
                request.provider_event_id_hash,
                error.without_url()
            )
        })?;
        ensure!(
            status.is_success(),
            "tenant access token request for Feishu/Lark room message returned HTTP status {}",
            status
        );

        let provider_response: FeishuLarkTenantAccessTokenResponse = serde_json::from_str(
            &response_body,
        )
        .context("tenant access token response for Feishu/Lark room message was invalid JSON")?;
        ensure!(
            provider_response.code == 0,
            "tenant access token request for Feishu/Lark room message returned code {}: {}",
            provider_response.code,
            provider_response
                .msg
                .unwrap_or_else(|| "unknown error".to_string())
        );
        let token = present_owned(provider_response.tenant_access_token).context(
            "tenant access token response for Feishu/Lark room message did not include a token",
        )?;
        ensure!(
            provider_response.expire.unwrap_or_default() > 0,
            "tenant access token response for Feishu/Lark room message had an invalid expiry"
        );
        Ok(token)
    }

    async fn fetch_tenant_access_token_for_project_room_create(
        &self,
        request: &FeishuLarkCreateProjectRoomRequest,
        runtime: &FeishuLarkAppBotRuntime,
    ) -> anyhow::Result<String> {
        let token_request = FeishuLarkTenantAccessTokenRequest {
            app_id: &runtime.app_id,
            app_secret: &runtime.app_secret,
        };
        let response = self
            .client
            .post(format!(
                "{}/open-apis/auth/v3/tenant_access_token/internal",
                runtime.api_base_url
            ))
            .json(&token_request)
            .send()
            .await
            .map_err(|error| {
                anyhow!(
                    "failed to fetch tenant access token for Feishu/Lark project room `{}`: {}",
                    request.project_path,
                    error.without_url()
                )
            })?;
        let status = response.status();
        let response_body = response.text().await.map_err(|error| {
            anyhow!(
                "failed to read tenant access token response for Feishu/Lark project room `{}`: {}",
                request.project_path,
                error.without_url()
            )
        })?;
        ensure!(
            status.is_success(),
            "tenant access token request for Feishu/Lark project room create returned HTTP status {}",
            status
        );

        let provider_response: FeishuLarkTenantAccessTokenResponse =
            serde_json::from_str(&response_body).context(
                "tenant access token response for Feishu/Lark project room create was invalid JSON",
            )?;
        ensure!(
            provider_response.code == 0,
            "tenant access token request for Feishu/Lark project room create returned code {}: {}",
            provider_response.code,
            provider_response
                .msg
                .unwrap_or_else(|| "unknown error".to_string())
        );
        let token = present_owned(provider_response.tenant_access_token).context(
            "tenant access token response for Feishu/Lark project room create did not include a token",
        )?;
        ensure!(
            provider_response.expire.unwrap_or_default() > 0,
            "tenant access token response for Feishu/Lark project room create had an invalid expiry"
        );
        Ok(token)
    }
}

fn runtime_custom_bot(
    provider_id: &str,
    detail: &FeishuLarkCustomBotProviderConfig,
) -> anyhow::Result<FeishuLarkCustomBotRuntime> {
    let url = detail.url.resolve_runtime_value(
        provider_id,
        ProviderType::FeishuLark.as_str(),
        "url_env",
    )?;
    let url = validate_feishu_lark_webhook_url(&url)
        .with_context(|| format!("feishu_lark provider `{provider_id}` webhook URL is invalid"))?;
    let secret = detail
        .secret
        .as_ref()
        .map(|secret| {
            secret.resolve_runtime_value(
                provider_id,
                ProviderType::FeishuLark.as_str(),
                "secret_env",
            )
        })
        .transpose()?;
    let computer_name = local_machine::computer_name()?;

    Ok(FeishuLarkCustomBotRuntime {
        url,
        secret,
        computer_name,
    })
}

fn runtime_app_bot(
    provider_id: &str,
    detail: &FeishuLarkAppBotProviderConfig,
) -> anyhow::Result<FeishuLarkAppBotRuntime> {
    let app_secret = detail.app_secret.resolve_runtime_value(
        provider_id,
        ProviderType::FeishuLark.as_str(),
        "app_secret_env",
    )?;
    let computer_name = local_machine::computer_name()?;

    Ok(FeishuLarkAppBotRuntime {
        api_base_url: app_bot_api_base_url(detail.domain).to_string(),
        app_id: detail.app_id.clone(),
        app_secret,
        operator_open_id: detail.operator_open_id.clone(),
        tenant_key: detail.tenant_key.clone(),
        chat_id: detail.chat_id.clone(),
        computer_name,
    })
}

#[derive(Debug, Serialize)]
struct FeishuLarkInteractiveRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sign: Option<String>,
    msg_type: &'static str,
    card: FeishuLarkCard,
}

impl FeishuLarkInteractiveRequest {
    fn from_signal(signal: &Signal, secret: Option<&str>, computer_name: &str) -> Self {
        let timestamp = secret.map(|_| Utc::now().timestamp().to_string());
        let sign = match (&timestamp, secret) {
            (Some(timestamp), Some(secret)) => Some(sign_request(timestamp, secret)),
            _ => None,
        };

        Self {
            timestamp,
            sign,
            msg_type: "interactive",
            card: FeishuLarkCard::from_signal(signal, computer_name),
        }
    }
}

#[derive(Debug, Serialize)]
struct FeishuLarkCard {
    config: FeishuLarkCardConfig,
    header: FeishuLarkCardHeader,
    elements: Vec<FeishuLarkCardElement>,
}

impl FeishuLarkCard {
    fn from_signal(signal: &Signal, computer_name: &str) -> Self {
        let detail_time = format_local_timestamp(signal.timestamp);
        let view = SignalNotificationView::from_signal(signal, &detail_time);
        let card_body = FeishuLarkCardBody::from_view(&view);

        Self {
            config: FeishuLarkCardConfig {
                wide_screen_mode: true,
            },
            header: FeishuLarkCardHeader {
                template: CODEX_CARD_TEMPLATE,
                title: FeishuLarkPlainText {
                    tag: "plain_text",
                    content: format_header_title(computer_name, &view.title, &card_body),
                },
            },
            elements: format_signal_card_elements(card_body),
        }
    }
}

fn project_room_prompt_card(request: &ProjectRoomPromptRequest) -> FeishuLarkCard {
    FeishuLarkCard {
        config: FeishuLarkCardConfig {
            wide_screen_mode: true,
        },
        header: FeishuLarkCardHeader {
            template: "blue",
            title: FeishuLarkPlainText {
                tag: "plain_text",
                content: "Connect this Codex project?".to_string(),
            },
        },
        elements: vec![
            FeishuLarkCardElement::Markdown {
                content: lark_card_markdown(format!(
                    "**{}**\nCodex just finished work in this project, but no Feishu/Lark room is connected yet.",
                    request.project_name
                )),
            },
            FeishuLarkCardElement::Markdown {
                content: lark_card_markdown(format!("Project folder:\n`{}`", request.project_path)),
            },
            FeishuLarkCardElement::Markdown {
                content: lark_card_markdown(
                    "Choose what should happen to future updates from this project.".to_string(),
                ),
            },
            FeishuLarkCardElement::Action {
                actions: vec![
                    project_room_prompt_button(
                        "Create a room",
                        "create_project_room",
                        &request.proposal_id,
                        "primary",
                    ),
                    project_room_prompt_button(
                        "Use existing room",
                        "use_existing_room",
                        &request.proposal_id,
                        "default",
                    ),
                    project_room_prompt_button(
                        "Ignore",
                        "ignore_project",
                        &request.proposal_id,
                        "default",
                    ),
                ],
            },
        ],
    }
}

fn project_room_prompt_button(
    label: &str,
    action: &str,
    proposal_id: &str,
    button_type: &'static str,
) -> FeishuLarkCardAction {
    FeishuLarkCardAction {
        tag: "button",
        text: FeishuLarkPlainText {
            tag: "plain_text",
            content: label.to_string(),
        },
        url: None,
        value: Some(json!({
            "action": action,
            "proposal_id": proposal_id,
        })),
        button_type,
    }
}

#[derive(Debug, Serialize)]
struct FeishuLarkCardConfig {
    wide_screen_mode: bool,
}

#[derive(Debug, Serialize)]
struct FeishuLarkCardHeader {
    title: FeishuLarkPlainText,
    template: &'static str,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct FeishuLarkPlainText {
    tag: &'static str,
    content: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct FeishuLarkLarkMarkdown {
    tag: &'static str,
    content: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "tag")]
enum FeishuLarkCardElement {
    #[serde(rename = "markdown")]
    Markdown { content: String },
    #[serde(rename = "div")]
    Div { text: FeishuLarkLarkMarkdown },
    #[serde(rename = "column_set")]
    ColumnSet {
        flex_mode: &'static str,
        background_style: &'static str,
        columns: Vec<FeishuLarkCardColumn>,
    },
    #[serde(rename = "hr")]
    Divider,
    #[serde(rename = "action")]
    Action { actions: Vec<FeishuLarkCardAction> },
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct FeishuLarkCardColumn {
    tag: &'static str,
    width: &'static str,
    weight: u8,
    vertical_align: &'static str,
    elements: Vec<FeishuLarkCardElement>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct FeishuLarkCardAction {
    tag: &'static str,
    text: FeishuLarkPlainText,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<serde_json::Value>,
    #[serde(rename = "type")]
    button_type: &'static str,
}

#[derive(Debug, Deserialize)]
struct FeishuLarkResponse {
    code: i64,
    msg: Option<String>,
}

#[derive(Debug, Serialize)]
struct FeishuLarkTenantAccessTokenRequest<'a> {
    app_id: &'a str,
    app_secret: &'a str,
}

#[derive(Debug, Deserialize)]
struct FeishuLarkTenantAccessTokenResponse {
    code: i64,
    msg: Option<String>,
    tenant_access_token: Option<String>,
    expire: Option<i64>,
}

#[derive(Debug, Serialize)]
struct FeishuLarkAppBotSendMessageRequest<'a> {
    receive_id: &'a str,
    msg_type: &'static str,
    content: String,
}

#[derive(Debug, Serialize)]
struct FeishuLarkAppBotReplyMessageRequest {
    content: String,
    msg_type: &'static str,
    reply_in_thread: bool,
    uuid: String,
}

#[derive(Debug, Serialize)]
struct FeishuLarkCardKitCreateRequest {
    #[serde(rename = "type")]
    card_type: &'static str,
    data: String,
}

#[derive(Debug, Serialize)]
struct FeishuLarkCardKitContentUpdateRequest<'a> {
    content: &'a str,
    sequence: u64,
    uuid: String,
}

#[derive(Debug, Serialize)]
struct FeishuLarkCardKitSettingsUpdateRequest {
    settings: String,
    sequence: u64,
    uuid: String,
}

#[derive(Debug, Serialize)]
struct FeishuLarkCardReferenceContent<'a> {
    #[serde(rename = "type")]
    content_type: &'static str,
    data: FeishuLarkCardReferenceData<'a>,
}

#[derive(Debug, Serialize)]
struct FeishuLarkCardReferenceData<'a> {
    card_id: &'a str,
}

#[derive(Debug, Serialize)]
struct FeishuLarkTextMessageContent<'a> {
    text: &'a str,
}

#[derive(Debug, Serialize)]
struct FeishuLarkCreateChatRequest {
    name: String,
    description: String,
    owner_id: String,
    user_id_list: Vec<String>,
    group_message_type: &'static str,
    chat_mode: &'static str,
    chat_type: &'static str,
    join_message_visibility: &'static str,
    leave_message_visibility: &'static str,
}

#[derive(Debug, Deserialize)]
struct FeishuLarkAppBotSendMessageResponse {
    code: i64,
    msg: Option<String>,
    data: Option<FeishuLarkAppBotMessageData>,
}

#[derive(Debug, Deserialize)]
struct FeishuLarkCreateChatResponse {
    code: i64,
    msg: Option<String>,
    data: Option<FeishuLarkCreateChatData>,
}

#[derive(Debug, Deserialize)]
struct FeishuLarkCreateChatData {
    chat_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FeishuLarkAppBotReplyMessageResponse {
    code: i64,
    msg: Option<String>,
    data: Option<FeishuLarkAppBotReplyMessageData>,
}

#[derive(Debug, Deserialize)]
struct FeishuLarkCardKitCreateResponse {
    code: i64,
    msg: Option<String>,
    data: Option<FeishuLarkCardKitCreateData>,
}

#[derive(Debug, Deserialize)]
struct FeishuLarkCardKitCreateData {
    card_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FeishuLarkCardKitUpdateResponse {
    code: i64,
    msg: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FeishuLarkAppBotMessageData {
    message_id: Option<String>,
    chat_id: Option<String>,
    sender: Option<FeishuLarkAppBotMessageSender>,
}

#[derive(Debug, Deserialize)]
struct FeishuLarkAppBotReplyMessageData {
    message_id: Option<String>,
    root_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FeishuLarkAppBotMessageSender {
    tenant_key: Option<String>,
}

fn app_bot_surface_ready_receipt(
    signal: &Signal,
    provider_id: &str,
    provider_type: &str,
    runtime: &FeishuLarkAppBotRuntime,
    data: FeishuLarkAppBotMessageData,
) -> Result<ProviderDeliveryReceipt, Box<DeliveryError>> {
    let message_id = required_app_bot_response_field(
        signal,
        provider_id,
        provider_type,
        "message_id",
        data.message_id,
    )?;
    let chat_id = required_app_bot_response_field(
        signal,
        provider_id,
        provider_type,
        "chat_id",
        data.chat_id,
    )?;
    if runtime
        .chat_id
        .as_deref()
        .is_some_and(|configured_chat_id| configured_chat_id != chat_id)
    {
        return Err(app_bot_response_error(
            signal,
            provider_id,
            provider_type,
            "Feishu/Lark app send response chat_id did not match provider config",
        ));
    }

    let sender = data.sender.ok_or_else(|| {
        app_bot_response_error(
            signal,
            provider_id,
            provider_type,
            "Feishu/Lark app send response did not include sender",
        )
    })?;
    let tenant_key = required_app_bot_response_field(
        signal,
        provider_id,
        provider_type,
        "sender.tenant_key",
        sender.tenant_key,
    )?;
    if runtime
        .tenant_key
        .as_deref()
        .is_some_and(|configured_tenant_key| configured_tenant_key != tenant_key)
    {
        return Err(app_bot_response_error(
            signal,
            provider_id,
            provider_type,
            "Feishu/Lark app send response tenant_key did not match provider config",
        ));
    }

    Ok(ProviderDeliveryReceipt::surface_ready(
        Some(tenant_key),
        Some(chat_id),
        Some(message_id.clone()),
        Some(message_id),
    ))
}

fn app_bot_thread_surface_ready_receipt(
    signal: &Signal,
    provider_id: &str,
    provider_type: &str,
    provider_account_id: &str,
    provider_conversation_id: &str,
    provider_thread_id: &str,
    data: FeishuLarkAppBotReplyMessageData,
) -> Result<ProviderDeliveryReceipt, Box<DeliveryError>> {
    let message_id = required_app_bot_response_field(
        signal,
        provider_id,
        provider_type,
        "message_id",
        data.message_id,
    )?;
    let root_id = required_app_bot_response_field(
        signal,
        provider_id,
        provider_type,
        "root_id",
        data.root_id,
    )?;
    if root_id != provider_thread_id {
        return Err(app_bot_response_error(
            signal,
            provider_id,
            provider_type,
            "Feishu/Lark thread message response root_id did not match target thread",
        ));
    }

    Ok(ProviderDeliveryReceipt::surface_ready(
        Some(provider_account_id.to_string()),
        Some(provider_conversation_id.to_string()),
        Some(message_id),
        Some(root_id),
    ))
}

fn required_app_bot_response_field(
    signal: &Signal,
    provider_id: &str,
    provider_type: &str,
    field: &'static str,
    value: Option<String>,
) -> Result<String, Box<DeliveryError>> {
    present_owned(value).ok_or_else(|| {
        app_bot_response_error(
            signal,
            provider_id,
            provider_type,
            format!("Feishu/Lark app send response did not include {field}"),
        )
    })
}

fn app_bot_response_error(
    signal: &Signal,
    provider_id: &str,
    provider_type: &str,
    message: impl Into<String>,
) -> Box<DeliveryError> {
    Box::new(DeliveryError::new(
        DeliveryErrorKind::ProviderResponse,
        DeliveryErrorContext::provider_send(signal, provider_id, provider_type),
        format!(
            "feishu_lark provider `{provider_id}` returned invalid Feishu/Lark app send response: {}",
            message.into()
        ),
    ))
}

fn validate_app_bot_thread_reply_request(
    request: &ProviderThreadReplyRequest,
    provider_id: &str,
    runtime: &FeishuLarkAppBotRuntime,
) -> Result<(), ProviderThreadReplyError> {
    if request.provider_id != provider_id {
        return Err(thread_reply_error(
            request,
            "provider thread reply request provider_id did not match adapter",
        ));
    }
    if request.provider_type != ProviderType::FeishuLark.as_str() {
        return Err(thread_reply_error(
            request,
            "provider thread reply request provider_type did not match adapter",
        ));
    }
    if runtime
        .tenant_key
        .as_deref()
        .is_some_and(|configured_tenant_key| request.provider_account_id != configured_tenant_key)
    {
        return Err(thread_reply_error(
            request,
            "provider thread reply request tenant did not match Feishu/Lark app config",
        ));
    }
    if present(Some(request.provider_conversation_id.as_str())).is_none() {
        return Err(thread_reply_error(
            request,
            "provider thread reply request did not include provider_conversation_id",
        ));
    }
    if present(Some(request.provider_thread_id.as_str())).is_none() {
        return Err(thread_reply_error(
            request,
            "provider thread reply request did not include provider_thread_id",
        ));
    }
    if present(Some(request.text.as_str())).is_none() {
        return Err(thread_reply_error(
            request,
            "provider thread reply request did not include result text",
        ));
    }

    Ok(())
}

fn validate_app_bot_streaming_thread_reply_request(
    request: &FeishuLarkStreamingThreadReplyRequest,
    provider_id: &str,
    runtime: &FeishuLarkAppBotRuntime,
) -> Result<(), ProviderThreadReplyError> {
    if request.provider_id != provider_id {
        return Err(streaming_thread_reply_error(
            request,
            "streaming thread reply request provider_id did not match adapter",
        ));
    }
    if request.provider_type != ProviderType::FeishuLark.as_str() {
        return Err(streaming_thread_reply_error(
            request,
            "streaming thread reply request provider_type did not match adapter",
        ));
    }
    if runtime
        .tenant_key
        .as_deref()
        .is_some_and(|configured_tenant_key| request.provider_account_id != configured_tenant_key)
    {
        return Err(streaming_thread_reply_error(
            request,
            "streaming thread reply request tenant did not match Feishu/Lark app config",
        ));
    }
    if present(Some(request.provider_account_id.as_str())).is_none() {
        return Err(streaming_thread_reply_error(
            request,
            "streaming thread reply request did not include provider_account_id",
        ));
    }
    if present(Some(request.provider_conversation_id.as_str())).is_none() {
        return Err(streaming_thread_reply_error(
            request,
            "streaming thread reply request did not include provider_conversation_id",
        ));
    }
    if present(Some(request.provider_thread_id.as_str())).is_none() {
        return Err(streaming_thread_reply_error(
            request,
            "streaming thread reply request did not include provider_thread_id",
        ));
    }

    Ok(())
}

fn validate_app_bot_conversation_text_request(
    request: &FeishuLarkConversationTextRequest,
    provider_id: &str,
    runtime: &FeishuLarkAppBotRuntime,
) -> anyhow::Result<()> {
    ensure!(
        request.provider_id == provider_id,
        "Feishu/Lark room message provider_id did not match adapter"
    );
    ensure!(
        request.provider_type == ProviderType::FeishuLark.as_str(),
        "Feishu/Lark room message provider_type did not match adapter"
    );
    if let Some(configured_tenant_key) = runtime.tenant_key.as_deref() {
        ensure!(
            request.provider_account_id == configured_tenant_key,
            "Feishu/Lark room message tenant did not match app config"
        );
    }
    ensure!(
        present(Some(request.provider_account_id.as_str())).is_some(),
        "Feishu/Lark room message did not include provider_account_id"
    );
    ensure!(
        present(Some(request.provider_conversation_id.as_str())).is_some(),
        "Feishu/Lark room message did not include provider_conversation_id"
    );
    ensure!(
        present(Some(request.provider_event_id_hash.as_str())).is_some(),
        "Feishu/Lark room message did not include provider_event_id_hash"
    );
    ensure!(
        present(Some(request.text.as_str())).is_some(),
        "Feishu/Lark room message text was empty"
    );
    Ok(())
}

fn validate_app_bot_create_project_room_request(
    request: &FeishuLarkCreateProjectRoomRequest,
    provider_id: &str,
    runtime: &FeishuLarkAppBotRuntime,
) -> anyhow::Result<()> {
    ensure!(
        request.provider_id == provider_id,
        "Feishu/Lark project room create provider_id did not match adapter"
    );
    ensure!(
        request.provider_type == ProviderType::FeishuLark.as_str(),
        "Feishu/Lark project room create provider_type did not match adapter"
    );
    if let Some(configured_tenant_key) = runtime.tenant_key.as_deref() {
        ensure!(
            request.provider_account_id == configured_tenant_key,
            "Feishu/Lark project room create tenant did not match app config"
        );
    }
    ensure!(
        present(Some(request.provider_account_id.as_str())).is_some(),
        "Feishu/Lark project room create did not include provider_account_id"
    );
    ensure!(
        present(Some(request.operator_open_id.as_str())).is_some(),
        "Feishu/Lark project room create did not include operator_open_id"
    );
    ensure!(
        present(Some(request.project_name.as_str())).is_some(),
        "Feishu/Lark project room create did not include project_name"
    );
    ensure!(
        present(Some(request.project_path.as_str())).is_some(),
        "Feishu/Lark project room create did not include project_path"
    );
    ensure!(
        present(Some(request.proposal_id.as_str())).is_some(),
        "Feishu/Lark project room create did not include proposal_id"
    );
    Ok(())
}

fn project_room_name(project_name: &str) -> String {
    let project_name = project_name.trim();
    if project_name.is_empty() {
        "Codex project".to_string()
    } else {
        format!("Codex · {project_name}")
    }
}

fn project_room_description(project_path: &str) -> String {
    format!("Codex updates from {project_path}")
}

fn project_room_create_uuid(proposal_id: &str, proposal_created_at: DateTime<Utc>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(proposal_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(proposal_created_at.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true));
    let digest = hasher.finalize();
    let hash = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("prc_{}", &hash[..24])
}

fn required_thread_reply_response_field(
    request: &ProviderThreadReplyRequest,
    field: &'static str,
    value: Option<String>,
) -> Result<String, ProviderThreadReplyError> {
    present_owned(value).ok_or_else(|| {
        thread_reply_error(
            request,
            format!("Feishu/Lark thread reply response did not include {field}"),
        )
    })
}

fn required_streaming_thread_reply_response_field(
    request: &FeishuLarkStreamingThreadReplyRequest,
    field: &'static str,
    value: Option<String>,
) -> Result<String, ProviderThreadReplyError> {
    present_owned(value).ok_or_else(|| {
        streaming_thread_reply_error(
            request,
            format!("Feishu/Lark streaming reply response did not include {field}"),
        )
    })
}

fn thread_reply_error(
    request: &ProviderThreadReplyRequest,
    message: impl Into<String>,
) -> ProviderThreadReplyError {
    ProviderThreadReplyError {
        provider_id: request.provider_id.clone(),
        provider_type: request.provider_type.clone(),
        surface_id: request.surface_id.clone(),
        provider_event_id_hash: request.provider_event_id_hash.clone(),
        message: message.into().into_boxed_str(),
        http_status: None,
        retriable: false,
    }
}

fn streaming_thread_reply_error(
    request: &FeishuLarkStreamingThreadReplyRequest,
    message: impl Into<String>,
) -> ProviderThreadReplyError {
    ProviderThreadReplyError {
        provider_id: request.provider_id.clone(),
        provider_type: request.provider_type.clone(),
        surface_id: request.surface_id.clone(),
        provider_event_id_hash: request.provider_event_id_hash.clone(),
        message: message.into().into_boxed_str(),
        http_status: None,
        retriable: false,
    }
}

fn thread_reply_uuid(provider_event_id_hash: &str) -> String {
    let prefix = "ar_";
    let max_hash_len = 50 - prefix.len();
    let hash_prefix = provider_event_id_hash
        .char_indices()
        .nth(max_hash_len)
        .map_or(provider_event_id_hash, |(index, _)| {
            &provider_event_id_hash[..index]
        });
    format!("{prefix}{hash_prefix}")
}

fn streaming_thread_reply_uuid(provider_event_id_hash: &str) -> String {
    let prefix = "ars_";
    let max_hash_len = 50 - prefix.len();
    let hash_prefix = provider_event_id_hash
        .char_indices()
        .nth(max_hash_len)
        .map_or(provider_event_id_hash, |(index, _)| {
            &provider_event_id_hash[..index]
        });
    format!("{prefix}{hash_prefix}")
}

fn streaming_card_update_uuid(card_id: &str, sequence: u64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(card_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(sequence.to_string().as_bytes());
    let digest = hasher.finalize();
    let hash = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("arcu_{}", &hash[..24])
}

fn streaming_markdown_card(content: &str, streaming: bool) -> serde_json::Value {
    json!({
        "schema": "2.0",
        "config": {
            "streaming_mode": streaming,
            "summary": {
                "content": if streaming { STREAMING_REPLY_INITIAL_TEXT } else { "Codex finished" }
            },
            "streaming_config": {
                "print_frequency_ms": { "default": 70 },
                "print_step": { "default": 1 },
                "print_strategy": "fast"
            }
        },
        "body": {
            "elements": [
                {
                    "tag": "markdown",
                    "element_id": STREAMING_REPLY_ELEMENT_ID,
                    "content": content
                }
            ]
        }
    })
}

fn short_card_summary(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return "Codex finished".to_string();
    }
    let mut summary = String::new();
    for ch in trimmed.chars().take(80) {
        if ch.is_control() {
            summary.push(' ');
        } else {
            summary.push(ch);
        }
    }
    if trimmed.chars().count() > 80 {
        summary.push_str("...");
    }
    summary
}

fn thread_notification_uuid(signal_id: &str, provider_thread_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(signal_id.as_bytes());
    hasher.update([0]);
    hasher.update(provider_thread_id.as_bytes());
    let digest = hasher.finalize();
    let hash = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    thread_reply_uuid(&hash)
}

fn app_bot_api_base_url(domain: FeishuLarkAppDomain) -> &'static str {
    match domain {
        FeishuLarkAppDomain::Feishu => "https://open.feishu.cn",
        FeishuLarkAppDomain::Lark => "https://open.larksuite.com",
    }
}

fn present_owned(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn sign_request(timestamp: &str, secret: &str) -> String {
    let string_to_sign = format!("{timestamp}\n{secret}");
    let mac =
        HmacSha256::new_from_slice(string_to_sign.as_bytes()).expect("HMAC accepts any key size");
    STANDARD.encode(mac.finalize().into_bytes())
}

fn format_signal_card_elements(card_body: FeishuLarkCardBody) -> Vec<FeishuLarkCardElement> {
    let mut elements = Vec::new();

    if let Some(title) = card_body.display_title() {
        elements.push(FeishuLarkCardElement::Div {
            text: FeishuLarkLarkMarkdown {
                tag: "lark_md",
                content: lark_card_markdown(format!("**{title}**")),
            },
        });
    }
    if let Some(project_row) = card_body.project_row() {
        elements.push(project_row);
    }
    if let Some(timing_row) = card_body.timing_row() {
        elements.push(timing_row);
    }
    if let Some(session_row) = card_body.session_row() {
        elements.push(session_row);
    }
    if !card_body.other_details.is_empty() {
        elements.push(FeishuLarkCardElement::Markdown {
            content: lark_card_markdown(card_body.other_details.join("\n")),
        });
    }

    if let Some(open_link) = card_body.open_link {
        elements.push(FeishuLarkCardElement::Action {
            actions: vec![FeishuLarkCardAction {
                tag: "button",
                text: FeishuLarkPlainText {
                    tag: "plain_text",
                    content: open_link.label,
                },
                url: Some(open_link.url),
                value: None,
                button_type: "primary",
            }],
        });
    }

    let has_prompt = card_body.prompt.is_some();
    let has_answer = card_body.answer.is_some();
    if has_prompt || has_answer {
        elements.push(FeishuLarkCardElement::Divider);
    }

    if let Some(prompt) = card_body.prompt {
        elements.push(FeishuLarkCardElement::Markdown {
            content: lark_card_markdown(format!("**Prompt**\n{prompt}")),
        });
    }

    if let Some(answer) = card_body.answer {
        if has_prompt {
            elements.push(FeishuLarkCardElement::Divider);
        }
        elements.push(FeishuLarkCardElement::Markdown {
            content: lark_card_markdown(format!("**{}**\n{}", answer.label, answer.content)),
        });
    }

    elements
}

#[cfg(test)]
fn format_signal_card_elements_with_time(
    signal: &Signal,
    formatted_time: &str,
) -> Vec<FeishuLarkCardElement> {
    format_signal_card_elements(FeishuLarkCardBody::from_signal(signal, formatted_time))
}

fn format_header_title(computer_name: &str, title: &str, card_body: &FeishuLarkCardBody) -> String {
    let computer_name = computer_name.trim();
    let mut parts = Vec::new();
    parts.push(title.to_string());

    if let Some(display_title) = card_body.display_title() {
        parts.push(display_title.to_string());
    }
    if !computer_name.is_empty() {
        parts.push(computer_name.to_string());
    }

    parts.join(" · ")
}

#[derive(Debug, PartialEq, Eq)]
struct FeishuLarkCardBody {
    project: Option<String>,
    project_path: Option<String>,
    session_title: Option<String>,
    session_id: Option<String>,
    model: Option<String>,
    duration: Option<String>,
    branch: Option<String>,
    time: Option<String>,
    other_details: Vec<String>,
    open_link: Option<FeishuLarkActionLink>,
    prompt: Option<String>,
    answer: Option<FeishuLarkAnswerBlock>,
}

#[derive(Debug, PartialEq, Eq)]
struct FeishuLarkActionLink {
    label: String,
    url: String,
}

#[derive(Debug, PartialEq, Eq)]
struct FeishuLarkAnswerBlock {
    label: &'static str,
    content: String,
}

impl FeishuLarkCardBody {
    #[cfg(test)]
    fn from_signal(signal: &Signal, formatted_time: &str) -> Self {
        let view = SignalNotificationView::from_signal(signal, formatted_time);
        Self::from_view(&view)
    }

    fn from_view(view: &SignalNotificationView) -> Self {
        let mut other_details = Vec::new();

        if let Some(summary) = view.summary.as_deref() {
            push_markdown_detail(&mut other_details, summary);
        }

        let prompt = view.sections.iter().find_map(|section| match section {
            NotificationSection::Prompt(prompt) => Some(prompt.to_string()),
            NotificationSection::Preview(_) | NotificationSection::Answer(_) => None,
        });
        let answer = view.sections.iter().find_map(|section| match section {
            NotificationSection::Preview(answer) => Some(FeishuLarkAnswerBlock {
                label: "Preview",
                content: answer.to_string(),
            }),
            NotificationSection::Answer(answer) => Some(FeishuLarkAnswerBlock {
                label: "Answer",
                content: answer.to_string(),
            }),
            NotificationSection::Prompt(_) => None,
        });

        Self {
            project: view
                .field_value(NotificationFieldKey::Project)
                .map(ToOwned::to_owned),
            project_path: view
                .field_value(NotificationFieldKey::ProjectPath)
                .map(ToOwned::to_owned),
            session_title: view
                .field_value(NotificationFieldKey::Session)
                .map(ToOwned::to_owned),
            session_id: view
                .field_value(NotificationFieldKey::SessionId)
                .map(ToOwned::to_owned),
            model: view
                .field_value(NotificationFieldKey::Model)
                .map(ToOwned::to_owned),
            duration: view
                .field_value(NotificationFieldKey::Duration)
                .map(ToOwned::to_owned),
            branch: view
                .field_value(NotificationFieldKey::Branch)
                .map(ToOwned::to_owned),
            time: view
                .field_value(NotificationFieldKey::Time)
                .map(ToOwned::to_owned),
            other_details,
            open_link: view.actions.iter().find_map(feishu_action_link),
            prompt,
            answer,
        }
    }

    fn display_title(&self) -> Option<&str> {
        self.session_title.as_deref().or(self.project.as_deref())
    }

    fn project_row(&self) -> Option<FeishuLarkCardElement> {
        let mut columns = Vec::new();

        if let Some(project) = self.project.as_deref() {
            columns.push(metric_column(
                NotificationFieldKey::Project.label(),
                project,
                None,
            ));
        }
        if let Some(branch) = self.branch.as_deref() {
            columns.push(metric_column(
                NotificationFieldKey::Branch.label(),
                branch,
                None,
            ));
        }
        if let Some(model) = self.model.as_deref() {
            columns.push(metric_column(
                NotificationFieldKey::Model.label(),
                model,
                None,
            ));
        }

        if columns.is_empty() {
            None
        } else {
            Some(column_set(columns))
        }
    }

    fn timing_row(&self) -> Option<FeishuLarkCardElement> {
        let mut columns = Vec::new();

        if let Some(duration) = self.duration.as_deref() {
            columns.push(metric_column(
                NotificationFieldKey::Duration.label(),
                duration,
                None,
            ));
        }
        if let Some(time) = self.time.as_deref() {
            columns.push(metric_column(
                NotificationFieldKey::Time.label(),
                strip_numeric_timezone(time),
                None,
            ));
        }

        if columns.is_empty() {
            None
        } else {
            Some(column_set(columns))
        }
    }

    fn session_row(&self) -> Option<FeishuLarkCardElement> {
        self.session_id.as_deref().map(|session_id| {
            column_set(vec![metric_column(
                NotificationFieldKey::SessionId.label(),
                session_id,
                None,
            )])
        })
    }
}

fn push_markdown_detail(details: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if !value.is_empty() {
        details.push(format!("**{value}**"));
    }
}

fn feishu_action_link(action: &NotificationAction) -> Option<FeishuLarkActionLink> {
    let label = action.label.trim();
    if label.is_empty() {
        return None;
    }

    let url = feishu_action_url(&action.url)?;
    Some(FeishuLarkActionLink {
        label: label.to_string(),
        url,
    })
}

fn feishu_action_url(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }

    if let Some(session_id) = url.strip_prefix("codex://threads/") {
        return codex_thread_bridge_url(session_id);
    }

    Some(url.to_string())
}

fn column_set(columns: Vec<FeishuLarkCardColumn>) -> FeishuLarkCardElement {
    FeishuLarkCardElement::ColumnSet {
        flex_mode: "bisect",
        background_style: "default",
        columns,
    }
}

fn metric_column(label: &str, value: &str, secondary_value: Option<&str>) -> FeishuLarkCardColumn {
    let mut content = format!("**{label}**\n{}", value.trim());
    if let Some(secondary_value) = present(secondary_value) {
        content.push('\n');
        content.push('`');
        content.push_str(secondary_value);
        content.push('`');
    }

    FeishuLarkCardColumn {
        tag: "column",
        width: "weighted",
        weight: 1,
        vertical_align: "top",
        elements: vec![FeishuLarkCardElement::Div {
            text: FeishuLarkLarkMarkdown {
                tag: "lark_md",
                content: lark_card_markdown(content),
            },
        }],
    }
}

fn lark_card_markdown(content: impl Into<String>) -> String {
    let content = content.into();
    // Feishu/Lark cards treat `![alt](url)` as an image element and reject it
    // unless the URL is an uploaded image_key. Agent output may include local
    // Markdown image links, so render the marker as text in notification cards.
    content.replace("![", "\\![")
}

fn strip_numeric_timezone(value: &str) -> &str {
    let value = value.trim();
    let Some((timestamp, offset)) = value.rsplit_once(' ') else {
        return value;
    };

    let offset_bytes = offset.as_bytes();
    if offset_bytes.len() == 6
        && matches!(offset_bytes[0], b'+' | b'-')
        && offset_bytes[3] == b':'
        && offset_bytes[1..3].iter().all(u8::is_ascii_digit)
        && offset_bytes[4..6].iter().all(u8::is_ascii_digit)
    {
        timestamp
    } else {
        value
    }
}

fn format_local_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp
        .with_timezone(&Local)
        .format("%Y-%m-%d %H:%M:%S %:z")
        .to_string()
}

fn present(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests;
