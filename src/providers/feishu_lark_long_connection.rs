use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;

use anyhow::{Context, ensure};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use prost::Message as ProstMessage;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio::time::{Duration, Instant, sleep, sleep_until};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use tracing::{debug, info, warn};

#[cfg(test)]
use crate::agent_controller::{
    AgentControllerClosedLoopDecision, AgentControllerRuntime, ProviderThreadReplyAdapter,
};
#[cfg(test)]
use crate::agent_integration_catalog::AgentIntegrationDescriptor;
use crate::agent_integration_catalog::agent_integration_for_source;
#[cfg(test)]
use crate::bridge_binding_ledger::ThreadSessionBindingInput;
use crate::bridge_binding_ledger::{BridgeBindingLedger, BridgeBindingLedgerStore};
use crate::bridge_control::{
    BridgeControlOutcome, BridgeControlReply, BridgeRoomMessage, disconnected_lark_thread_reply,
    handle_provider_control_command, handle_provider_room_event,
};
use crate::config::{
    FeishuLarkAppDomain, FeishuLarkProviderConfig, ProviderConfig, ProviderConfigDetail,
    ProviderType, SourceType, ValidatedConfig,
};
use crate::continuation_dispatcher::{ClaimedContinuationWork, ContinuationDispatcher};
use crate::lark_personal_agent_channel;
use crate::new_session_dispatcher::{NewSessionDispatcher, NewSessionWork};
#[cfg(test)]
use crate::provider_catalog::ProviderModeCapability;
use crate::provider_catalog::{
    ProviderMode, provider_config_mode_capability, provider_mode_capability,
};
use crate::provider_inbound::{
    NormalizedProviderControlCommand, ProviderControlCommand, ProviderControlNormalizeResult,
    ProviderControlSkipReason, ProviderInboundDecision, ProviderInboundNormalizeResult,
    ProviderInboundReady, ProviderInboundSkipReason, lookup_and_claim_provider_surface_reply,
    lookup_and_claim_provider_thread_session_reply,
    normalize_feishu_lark_long_connection_control_command,
    normalize_feishu_lark_long_connection_surface_reply, provider_event_id_stable_hash,
    thread_binding_query_for_reply,
};
use crate::providers::feishu_lark_continuation::FeishuLarkContinuationDispatcher;
use crate::providers::feishu_lark_control::{
    dispatch_feishu_lark_control_reply, dispatch_feishu_lark_room_message,
};
use crate::providers::feishu_lark_new_session::FeishuLarkNewSessionDispatcher;
use crate::response_surface_exposure::evaluate_response_surface_exposure;
#[cfg(test)]
use crate::response_surface_ledger::ResponseSurfaceLedger;
use crate::response_surface_ledger::ResponseSurfaceLedgerStore;
use crate::runtime::RuntimeState;

const LARK_LONG_CONNECTION_ENDPOINT_PATH: &str = "/callback/ws/endpoint";
const HEADER_TYPE: &str = "type";
const HEADER_MESSAGE_ID: &str = "message_id";
const HEADER_SUM: &str = "sum";
const HEADER_SEQ: &str = "seq";
const HEADER_TRACE_ID: &str = "trace_id";
const HEADER_BIZ_RT: &str = "biz_rt";
const MESSAGE_TYPE_EVENT: &str = "event";
const FRAME_METHOD_CONTROL: i32 = 0;
const FRAME_METHOD_DATA: i32 = 1;
const PLATFORM_ACK_CODE_OK: u16 = 200;
const HIDDEN_LONG_CONNECTION_IDLE_RETRY: Duration = Duration::from_secs(5);
const HIDDEN_LONG_CONNECTION_ERROR_RETRY: Duration = Duration::from_secs(3);

type FeishuLarkTransportFuture<'a, T> =
    Pin<Box<dyn Future<Output = anyhow::Result<T>> + Send + 'a>>;

#[derive(Clone)]
pub(crate) struct FeishuLarkLongConnectionConfig {
    pub provider_id: String,
    pub domain: FeishuLarkAppDomain,
    pub app_id: String,
    app_secret: String,
}

impl fmt::Debug for FeishuLarkLongConnectionConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FeishuLarkLongConnectionConfig")
            .field("provider_id", &self.provider_id)
            .field("domain", &self.domain)
            .field("app_id", &self.app_id)
            .field("app_secret", &"[redacted]")
            .finish()
    }
}

impl FeishuLarkLongConnectionConfig {
    pub fn from_provider_config(provider: &ProviderConfig) -> anyhow::Result<Self> {
        let ProviderConfigDetail::FeishuLark(FeishuLarkProviderConfig::AppBot(detail)) =
            &provider.detail
        else {
            anyhow::bail!(
                "Feishu/Lark long connection runtime requires `mode = \"app_bot\"` app credentials"
            );
        };

        let app_secret = detail
            .app_secret
            .resolve_runtime_value(
                &provider.id,
                ProviderType::FeishuLark.as_str(),
                "app_secret",
            )
            .context("failed to resolve Feishu/Lark app secret")?;

        ensure!(
            !detail.app_id.trim().is_empty(),
            "Feishu/Lark long connection runtime requires app_id"
        );
        ensure!(
            !app_secret.trim().is_empty(),
            "Feishu/Lark long connection runtime requires app_secret"
        );

        Ok(Self {
            provider_id: provider.id.clone(),
            domain: detail.domain,
            app_id: detail.app_id.clone(),
            app_secret,
        })
    }

    pub fn app_secret(&self) -> &str {
        &self.app_secret
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FeishuLarkLongConnectionEvent<'a> {
    pub raw_event: &'a [u8],
    pub received_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FeishuLarkLongConnectionEndpoint {
    pub url: String,
    pub client_config: Option<FeishuLarkLongConnectionClientConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FeishuLarkLongConnectionClientConfig {
    pub reconnect_count: Option<i64>,
    pub reconnect_interval: Option<i64>,
    pub reconnect_nonce: Option<i64>,
    pub ping_interval: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FeishuLarkLongConnectionTransportMessage {
    Binary(Vec<u8>),
    Text(String),
    Closed,
}

pub(crate) trait FeishuLarkLongConnectionTransport: Send {
    fn receive<'a>(
        &'a mut self,
    ) -> FeishuLarkTransportFuture<'a, FeishuLarkLongConnectionTransportMessage>;
    fn send_binary<'a>(&'a mut self, data: Vec<u8>) -> FeishuLarkTransportFuture<'a, ()>;
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FeishuLarkPlatformAck {
    Acknowledge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FeishuLarkLongConnectionDecision {
    ReadyAfterLocalClaim(Box<ProviderInboundReady>),
    ControlReply(BridgeControlReply),
    RoomMessage(BridgeRoomMessage),
    NewSession(Box<crate::bridge_control::BridgeNewSessionCommand>),
    Skip(ProviderInboundSkipReason),
    ControlSkip(ProviderControlSkipReason),
}

#[cfg(test)]
impl FeishuLarkLongConnectionDecision {
    pub fn platform_ack(&self) -> FeishuLarkPlatformAck {
        FeishuLarkPlatformAck::Acknowledge
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FeishuLarkLongConnectionRuntime {
    config: FeishuLarkLongConnectionConfig,
    bot_identity: Option<FeishuLarkLongConnectionBotIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FeishuLarkLongConnectionBotIdentity {
    pub open_id: String,
    pub name: String,
}

impl FeishuLarkLongConnectionRuntime {
    pub fn from_provider_config(provider: &ProviderConfig) -> anyhow::Result<Self> {
        Ok(Self {
            config: FeishuLarkLongConnectionConfig::from_provider_config(provider)?,
            bot_identity: None,
        })
    }

    #[cfg(test)]
    pub fn connection_config(&self) -> &FeishuLarkLongConnectionConfig {
        &self.config
    }

    pub(crate) fn with_bot_identity(
        mut self,
        bot_identity: FeishuLarkLongConnectionBotIdentity,
    ) -> Self {
        self.bot_identity = Some(bot_identity);
        self
    }

    fn bot_open_id(&self) -> anyhow::Result<&str> {
        self.bot_identity
            .as_ref()
            .map(|identity| identity.open_id.as_str())
            .context("Feishu/Lark long connection runtime must resolve bot identity before receiving events")
    }

    pub async fn fetch_bot_identity_hidden(
        &self,
    ) -> anyhow::Result<FeishuLarkLongConnectionBotIdentity> {
        let token = self.fetch_tenant_access_token_hidden().await?;
        let response = reqwest::Client::new()
            .get(format!(
                "{}/open-apis/bot/v3/info",
                long_connection_api_base_url(self.config.domain)
            ))
            .bearer_auth(token)
            .send()
            .await
            .context("failed to request Feishu/Lark bot identity")?;
        let status = response.status();
        let response_body = response
            .text()
            .await
            .context("failed to read Feishu/Lark bot identity response")?;
        ensure!(
            status.is_success(),
            "Feishu/Lark bot identity request returned HTTP status {}",
            status
        );

        let provider_response: FeishuLarkBotInfoResponse = serde_json::from_str(&response_body)
            .context("Feishu/Lark bot identity response returned invalid JSON")?;
        ensure!(
            provider_response.code == 0,
            "Feishu/Lark bot identity request returned code {}: {}",
            provider_response.code,
            provider_response
                .msg
                .unwrap_or_else(|| "unknown error".to_string())
        );
        let bot = provider_response
            .bot
            .context("Feishu/Lark bot identity response did not include bot")?;
        let open_id = present_owned(bot.open_id)
            .context("Feishu/Lark bot identity response did not include bot.open_id")?;
        let name = present_owned(bot.app_name).unwrap_or_else(|| "bot".to_string());

        Ok(FeishuLarkLongConnectionBotIdentity { open_id, name })
    }

    async fn fetch_tenant_access_token_hidden(&self) -> anyhow::Result<String> {
        let response = reqwest::Client::new()
            .post(format!(
                "{}/open-apis/auth/v3/tenant_access_token/internal",
                long_connection_api_base_url(self.config.domain)
            ))
            .json(&FeishuLarkTenantAccessTokenRequest {
                app_id: &self.config.app_id,
                app_secret: self.config.app_secret(),
            })
            .send()
            .await
            .context("failed to request Feishu/Lark tenant access token")?;
        let status = response.status();
        let response_body = response
            .text()
            .await
            .context("failed to read Feishu/Lark tenant access token response")?;
        ensure!(
            status.is_success(),
            "Feishu/Lark tenant access token request returned HTTP status {}",
            status
        );

        let provider_response: FeishuLarkTenantAccessTokenResponse =
            serde_json::from_str(&response_body)
                .context("Feishu/Lark tenant access token response returned invalid JSON")?;
        ensure!(
            provider_response.code == 0,
            "Feishu/Lark tenant access token request returned code {}: {}",
            provider_response.code,
            provider_response
                .msg
                .unwrap_or_else(|| "unknown error".to_string())
        );
        ensure!(
            provider_response.expire.unwrap_or_default() > 0,
            "Feishu/Lark tenant access token response returned invalid expiry"
        );
        present_owned(provider_response.tenant_access_token)
            .context("Feishu/Lark tenant access token response did not include token")
    }

    pub async fn discover_official_endpoint_hidden(
        &self,
    ) -> anyhow::Result<FeishuLarkLongConnectionEndpoint> {
        let endpoint_url = format!(
            "{}{}",
            long_connection_api_base_url(self.config.domain),
            LARK_LONG_CONNECTION_ENDPOINT_PATH
        );
        let response = reqwest::Client::new()
            .post(endpoint_url)
            .header("locale", "zh")
            .header("User-Agent", lark_personal_agent_channel::USER_AGENT)
            .json(&FeishuLarkLongConnectionEndpointRequest {
                app_id: &self.config.app_id,
                app_secret: self.config.app_secret(),
            })
            .send()
            .await
            .context("failed to request Feishu/Lark long connection endpoint")?;

        let status = response.status();
        let response_body = response
            .text()
            .await
            .context("failed to read Feishu/Lark long connection endpoint response")?;
        ensure!(
            status.is_success(),
            "Feishu/Lark long connection endpoint returned HTTP status {}",
            status
        );

        let endpoint_response: FeishuLarkLongConnectionEndpointResponse =
            serde_json::from_str(&response_body)
                .context("Feishu/Lark long connection endpoint returned invalid JSON")?;
        ensure!(
            endpoint_response.code == 0,
            "Feishu/Lark long connection endpoint returned code {}: {}",
            endpoint_response.code,
            endpoint_response
                .msg
                .unwrap_or_else(|| "unknown error".to_string())
        );

        let data = endpoint_response
            .data
            .context("Feishu/Lark long connection endpoint response did not include data")?;
        let url = present_owned(data.url)
            .context("Feishu/Lark long connection endpoint response did not include URL")?;

        Ok(FeishuLarkLongConnectionEndpoint {
            url,
            client_config: data.client_config.map(Into::into),
        })
    }

    pub async fn connect_official_transport_hidden(
        &self,
    ) -> anyhow::Result<FeishuLarkOfficialLongConnectionTransport> {
        self.bot_open_id()?;
        let endpoint = self.discover_official_endpoint_hidden().await?;
        FeishuLarkOfficialLongConnectionTransport::connect(endpoint.url, endpoint.client_config)
            .await
    }

    #[cfg(test)]
    pub fn handle_event_before_platform_ack(
        &self,
        ledger: &mut ResponseSurfaceLedger,
        event: FeishuLarkLongConnectionEvent<'_>,
    ) -> anyhow::Result<FeishuLarkLongConnectionDecision> {
        let normalized = normalize_feishu_lark_long_connection_surface_reply(
            &self.config.provider_id,
            self.bot_open_id()?,
            event.raw_event,
        )?;

        let reply = match normalized {
            ProviderInboundNormalizeResult::SurfaceReply(reply) => reply,
            ProviderInboundNormalizeResult::Skip(reason) => {
                return Ok(FeishuLarkLongConnectionDecision::Skip(reason));
            }
        };

        let decision = lookup_and_claim_provider_surface_reply(
            ledger,
            provider_mode_capability(ProviderMode::FeishuLarkAppBot),
            reply,
            event.received_at,
        )?;

        // Platform ack only means the local machine has safely skipped or
        // claimed the event. Agent execution can run longer and finishes later.
        Ok(match decision {
            ProviderInboundDecision::Ready(ready) => {
                FeishuLarkLongConnectionDecision::ReadyAfterLocalClaim(ready)
            }
            ProviderInboundDecision::Skip(reason) => FeishuLarkLongConnectionDecision::Skip(reason),
        })
    }

    pub async fn handle_event_before_platform_ack_with_stores_hidden(
        &self,
        response_surface_ledger_store: &ResponseSurfaceLedgerStore,
        bridge_binding_ledger_store: &BridgeBindingLedgerStore,
        event: FeishuLarkLongConnectionEvent<'_>,
    ) -> anyhow::Result<FeishuLarkLongConnectionDecision> {
        match normalize_feishu_lark_long_connection_control_command(
            &self.config.provider_id,
            self.bot_open_id()?,
            event.raw_event,
        )? {
            ProviderControlNormalizeResult::ControlCommand(command) => {
                return bridge_binding_ledger_store
                    .update(|ledger| {
                        self.handle_control_command_before_platform_ack(
                            ledger,
                            *command,
                            event.received_at,
                        )
                    })
                    .await;
            }
            ProviderControlNormalizeResult::RoomEvent(event) => {
                return self.handle_room_event_before_platform_ack(*event);
            }
            ProviderControlNormalizeResult::Skip(ProviderControlSkipReason::NotControlCommand) => {}
            ProviderControlNormalizeResult::Skip(reason) => {
                return Ok(FeishuLarkLongConnectionDecision::ControlSkip(reason));
            }
        }

        let normalized = normalize_feishu_lark_long_connection_surface_reply(
            &self.config.provider_id,
            self.bot_open_id()?,
            event.raw_event,
        )?;
        let reply = match normalized {
            ProviderInboundNormalizeResult::SurfaceReply(reply) => reply,
            ProviderInboundNormalizeResult::Skip(reason) => {
                return Ok(FeishuLarkLongConnectionDecision::Skip(reason));
            }
        };
        let provider = provider_mode_capability(ProviderMode::FeishuLarkAppBot);
        let surface_decision = response_surface_ledger_store
            .update(|ledger| {
                lookup_and_claim_provider_surface_reply(
                    ledger,
                    provider,
                    reply.clone(),
                    event.received_at,
                )
            })
            .await?;
        match surface_decision {
            ProviderInboundDecision::Ready(ready) => {
                return Ok(FeishuLarkLongConnectionDecision::ReadyAfterLocalClaim(
                    ready,
                ));
            }
            ProviderInboundDecision::Skip(ProviderInboundSkipReason::SurfaceClosed { .. }) => {
                return Ok(FeishuLarkLongConnectionDecision::ControlReply(
                    disconnected_lark_thread_reply(reply),
                ));
            }
            ProviderInboundDecision::Skip(ProviderInboundSkipReason::SurfaceLookupMiss) => {}
            ProviderInboundDecision::Skip(reason) => {
                return Ok(FeishuLarkLongConnectionDecision::Skip(reason));
            }
        }

        let thread_binding = bridge_binding_ledger_store
            .update(|ledger| {
                Ok(ledger.lookup_thread_session(&thread_binding_query_for_reply(&reply)))
            })
            .await?;
        let Some(thread_binding) = thread_binding else {
            return Ok(FeishuLarkLongConnectionDecision::ControlReply(
                disconnected_lark_thread_reply(reply),
            ));
        };

        let thread_decision = response_surface_ledger_store
            .update(|ledger| {
                lookup_and_claim_provider_thread_session_reply(
                    ledger,
                    provider,
                    thread_binding,
                    reply,
                    event.received_at,
                )
            })
            .await?;

        Ok(match thread_decision {
            ProviderInboundDecision::Ready(ready) => {
                FeishuLarkLongConnectionDecision::ReadyAfterLocalClaim(ready)
            }
            ProviderInboundDecision::Skip(reason) => FeishuLarkLongConnectionDecision::Skip(reason),
        })
    }

    #[cfg(test)]
    pub async fn handle_event_before_platform_ack_with_store_hidden(
        &self,
        ledger_store: &ResponseSurfaceLedgerStore,
        event: FeishuLarkLongConnectionEvent<'_>,
    ) -> anyhow::Result<FeishuLarkLongConnectionDecision> {
        ledger_store
            .update(|ledger| self.handle_event_before_platform_ack(ledger, event))
            .await
    }

    fn handle_control_command_before_platform_ack(
        &self,
        ledger: &mut BridgeBindingLedger,
        command: NormalizedProviderControlCommand,
        received_at: DateTime<Utc>,
    ) -> anyhow::Result<FeishuLarkLongConnectionDecision> {
        info!(
            provider.id = %command.provider_id,
            provider.conversation.id = %command.provider_conversation_id,
            provider.thread.id = %command.provider_thread_id,
            event.hash = %provider_event_id_stable_hash(&command.provider_event_id),
            control.command = control_command_name(&command.command),
            control.surface = control_command_surface(&command.command),
            event = "feishu_lark.long_connection.live.control.accepted",
        );
        match handle_provider_control_command(ledger, command, received_at)? {
            BridgeControlOutcome::Reply(reply) => {
                Ok(FeishuLarkLongConnectionDecision::ControlReply(reply))
            }
            BridgeControlOutcome::RoomMessage(message) => {
                Ok(FeishuLarkLongConnectionDecision::RoomMessage(message))
            }
            BridgeControlOutcome::NewSession(command) => Ok(
                FeishuLarkLongConnectionDecision::NewSession(Box::new(command)),
            ),
        }
    }

    fn handle_room_event_before_platform_ack(
        &self,
        event: crate::provider_inbound::NormalizedProviderRoomEvent,
    ) -> anyhow::Result<FeishuLarkLongConnectionDecision> {
        info!(
            provider.id = %event.provider_id,
            provider.conversation.id = %event.provider_conversation_id,
            event.hash = %provider_event_id_stable_hash(&event.provider_event_id),
            event.kind = ?event.event,
            event = "feishu_lark.long_connection.live.room_event.accepted",
        );
        match handle_provider_room_event(event)? {
            BridgeControlOutcome::RoomMessage(message) => {
                Ok(FeishuLarkLongConnectionDecision::RoomMessage(message))
            }
            BridgeControlOutcome::Reply(reply) => {
                Ok(FeishuLarkLongConnectionDecision::ControlReply(reply))
            }
            BridgeControlOutcome::NewSession(command) => Ok(
                FeishuLarkLongConnectionDecision::NewSession(Box::new(command)),
            ),
        }
    }

    #[cfg(test)]
    pub async fn receive_event_before_platform_ack_hidden(
        &self,
        ledger: &mut ResponseSurfaceLedger,
        transport: &mut dyn FeishuLarkLongConnectionTransport,
        payload_buffer: &mut FeishuLarkLongConnectionPayloadBuffer,
        received_at: DateTime<Utc>,
    ) -> anyhow::Result<FeishuLarkLongConnectionDecision> {
        loop {
            let message = transport.receive().await?;
            let Some(mut frame) = lark_frame_from_transport_message(message)? else {
                anyhow::bail!("Feishu/Lark long connection closed before receiving an event");
            };

            if frame.method == FRAME_METHOD_CONTROL {
                continue;
            }
            ensure!(
                frame.method == FRAME_METHOD_DATA,
                "Feishu/Lark long connection frame used unsupported method {}",
                frame.method
            );

            log_lark_data_frame_received(&self.config.provider_id, &frame);

            if header_value(&frame, HEADER_TYPE).as_deref() != Some(MESSAGE_TYPE_EVENT) {
                continue;
            }

            let Some(raw_event) = payload_buffer.accept_frame(&frame)? else {
                continue;
            };
            info!(
                provider.id = %self.config.provider_id,
                event = "feishu_lark.long_connection.live.event.received",
            );
            let decision = self.handle_event_before_platform_ack(
                ledger,
                FeishuLarkLongConnectionEvent {
                    raw_event: &raw_event,
                    received_at,
                },
            )?;
            frame.headers.push(FeishuLarkLongConnectionFrameHeader {
                key: HEADER_BIZ_RT.to_string(),
                value: "0".to_string(),
            });
            frame.payload = Some(
                serde_json::to_vec(&FeishuLarkLongConnectionPlatformAck {
                    code: PLATFORM_ACK_CODE_OK,
                })
                .context("failed to serialize Feishu/Lark platform ack")?,
            );
            transport.send_binary(frame.encode_to_vec()).await?;
            log_platform_ack_sent(&self.config.provider_id, &decision);
            return Ok(decision);
        }
    }

    pub async fn receive_event_before_platform_ack_with_stores_hidden(
        &self,
        response_surface_ledger_store: &ResponseSurfaceLedgerStore,
        bridge_binding_ledger_store: &BridgeBindingLedgerStore,
        transport: &mut dyn FeishuLarkLongConnectionTransport,
        payload_buffer: &mut FeishuLarkLongConnectionPayloadBuffer,
    ) -> anyhow::Result<FeishuLarkLongConnectionDecision> {
        loop {
            let message = transport.receive().await?;
            let Some(mut frame) = lark_frame_from_transport_message(message)? else {
                anyhow::bail!("Feishu/Lark long connection closed before receiving an event");
            };

            if frame.method == FRAME_METHOD_CONTROL {
                continue;
            }
            ensure!(
                frame.method == FRAME_METHOD_DATA,
                "Feishu/Lark long connection frame used unsupported method {}",
                frame.method
            );

            log_lark_data_frame_received(&self.config.provider_id, &frame);

            if header_value(&frame, HEADER_TYPE).as_deref() != Some(MESSAGE_TYPE_EVENT) {
                continue;
            }

            let Some(raw_event) = payload_buffer.accept_frame(&frame)? else {
                continue;
            };
            info!(
                provider.id = %self.config.provider_id,
                event = "feishu_lark.long_connection.live.event.received",
            );
            let received_at = Utc::now();
            let decision = self
                .handle_event_before_platform_ack_with_stores_hidden(
                    response_surface_ledger_store,
                    bridge_binding_ledger_store,
                    FeishuLarkLongConnectionEvent {
                        raw_event: &raw_event,
                        received_at,
                    },
                )
                .await?;
            frame.headers.push(FeishuLarkLongConnectionFrameHeader {
                key: HEADER_BIZ_RT.to_string(),
                value: "0".to_string(),
            });
            frame.payload = Some(
                serde_json::to_vec(&FeishuLarkLongConnectionPlatformAck {
                    code: PLATFORM_ACK_CODE_OK,
                })
                .context("failed to serialize Feishu/Lark platform ack")?,
            );
            transport.send_binary(frame.encode_to_vec()).await?;
            log_platform_ack_sent(&self.config.provider_id, &decision);
            return Ok(decision);
        }
    }

    #[cfg(test)]
    pub(crate) async fn continue_claimed_event_hidden_with_test_policy_facts(
        &self,
        config: &ValidatedConfig,
        ledger: &mut ResponseSurfaceLedger,
        ready: ProviderInboundReady,
        controller_runtime: &AgentControllerRuntime<'_>,
        provider_reply: &dyn ProviderThreadReplyAdapter,
        now: DateTime<Utc>,
        agent_integration_override: Option<AgentIntegrationDescriptor>,
        provider_capability_override: Option<&'static ProviderModeCapability>,
    ) -> anyhow::Result<AgentControllerClosedLoopDecision> {
        self.ensure_claimed_event_matches_runtime(&ready)?;
        controller_runtime
            .run_inbound_continuation_closed_loop_with_test_policy_facts(
                config,
                ledger,
                ready,
                provider_reply,
                now,
                agent_integration_override,
                provider_capability_override,
            )
            .await
    }

    #[cfg(test)]
    fn ensure_claimed_event_matches_runtime(
        &self,
        ready: &ProviderInboundReady,
    ) -> anyhow::Result<()> {
        ensure!(
            ready.reply.provider_id == self.config.provider_id,
            "claimed Feishu/Lark event provider does not match long connection runtime"
        );
        ensure!(
            ready.reply.provider_mode == ProviderMode::FeishuLarkAppBot,
            "claimed Feishu/Lark event must come from `mode = \"app_bot\"` app credentials"
        );
        Ok(())
    }
}

pub async fn run_live_lark_long_connection(runtime: RuntimeState) -> anyhow::Result<()> {
    info!(event = "feishu_lark.long_connection.live.supervisor.started");

    loop {
        let snapshot = runtime.current()?;
        let targets = lark_app_bot_response_surface_targets(&snapshot.config);
        drop(snapshot);

        if targets.is_empty() {
            debug!(
                event = "feishu_lark.long_connection.live.supervisor.idle",
                reason = "no_lark_app_bot_response_surface_route",
            );
            sleep(HIDDEN_LONG_CONNECTION_IDLE_RETRY).await;
            continue;
        }

        for provider in targets {
            if let Err(error) =
                run_live_lark_long_connection_provider(runtime.clone(), provider.clone()).await
            {
                warn!(
                    provider.id = %provider.id,
                    provider.type = %provider.provider_type().as_str(),
                    error = %error,
                    event = "feishu_lark.long_connection.live.provider.failed",
                );
                sleep(HIDDEN_LONG_CONNECTION_ERROR_RETRY).await;
            }
        }
    }
}

async fn run_live_lark_long_connection_provider(
    runtime_state: RuntimeState,
    provider: ProviderConfig,
) -> anyhow::Result<()> {
    let long_connection = FeishuLarkLongConnectionRuntime::from_provider_config(&provider)?;
    let bot_identity = long_connection.fetch_bot_identity_hidden().await?;
    info!(
        provider.id = %provider.id,
        bot.open_id = %bot_identity.open_id,
        bot.name = %bot_identity.name,
        event = "feishu_lark.long_connection.live.bot_identity.resolved",
    );
    let long_connection = long_connection.with_bot_identity(bot_identity);
    let mut transport = long_connection.connect_official_transport_hidden().await?;
    let mut payload_buffer = FeishuLarkLongConnectionPayloadBuffer::default();
    let dispatcher = FeishuLarkContinuationDispatcher::new(runtime_state.clone());
    let control_reply_dispatcher = LiveFeishuLarkControlReplyDispatcher::new(runtime_state.clone());
    let room_message_dispatcher = LiveFeishuLarkRoomMessageDispatcher::new(runtime_state.clone());
    let new_session_dispatcher = FeishuLarkNewSessionDispatcher::new(runtime_state.clone());

    info!(
        provider.id = %provider.id,
        event = "feishu_lark.long_connection.live.provider.connected",
    );

    loop {
        let response_surface_ledger_store = runtime_state.response_surface_ledger();
        let bridge_binding_ledger_store = runtime_state.bridge_binding_ledger();
        let dispatch_context = FeishuLarkLiveDispatchContext {
            response_surface_ledger_store: &response_surface_ledger_store,
            bridge_binding_ledger_store: &bridge_binding_ledger_store,
            dispatcher: &dispatcher,
            control_reply_dispatcher: &control_reply_dispatcher,
            room_message_dispatcher: &room_message_dispatcher,
            new_session_dispatcher: &new_session_dispatcher,
        };
        receive_and_dispatch_live_lark_event_hidden(
            &long_connection,
            &mut transport,
            &mut payload_buffer,
            dispatch_context,
        )
        .await?;
    }
}

struct FeishuLarkLiveDispatchContext<'a> {
    response_surface_ledger_store: &'a ResponseSurfaceLedgerStore,
    bridge_binding_ledger_store: &'a BridgeBindingLedgerStore,
    dispatcher: &'a dyn ContinuationDispatcher,
    control_reply_dispatcher: &'a dyn FeishuLarkControlReplyDispatcher,
    room_message_dispatcher: &'a dyn FeishuLarkRoomMessageDispatcher,
    new_session_dispatcher: &'a dyn NewSessionDispatcher,
}

async fn receive_and_dispatch_live_lark_event_hidden(
    long_connection: &FeishuLarkLongConnectionRuntime,
    transport: &mut dyn FeishuLarkLongConnectionTransport,
    payload_buffer: &mut FeishuLarkLongConnectionPayloadBuffer,
    dispatch_context: FeishuLarkLiveDispatchContext<'_>,
) -> anyhow::Result<()> {
    let decision = long_connection
        .receive_event_before_platform_ack_with_stores_hidden(
            dispatch_context.response_surface_ledger_store,
            dispatch_context.bridge_binding_ledger_store,
            transport,
            payload_buffer,
        )
        .await?;

    match decision {
        FeishuLarkLongConnectionDecision::Skip(reason) => {
            log_lark_inbound_skip(&long_connection.config.provider_id, &reason);
        }
        FeishuLarkLongConnectionDecision::ReadyAfterLocalClaim(ready) => {
            info!(
                provider.id = %long_connection.config.provider_id,
                surface.id = %ready.surface.surface_id,
                event.hash = %ready.provider_event_id_hash,
                event = "feishu_lark.long_connection.live.event.claimed",
            );
            dispatch_context
                .dispatcher
                .dispatch(ClaimedContinuationWork {
                    ready: *ready,
                    dispatched_at: Utc::now(),
                })?;
        }
        FeishuLarkLongConnectionDecision::ControlReply(reply) => {
            dispatch_context.control_reply_dispatcher.dispatch(reply)?;
        }
        FeishuLarkLongConnectionDecision::RoomMessage(message) => {
            dispatch_context.room_message_dispatcher.dispatch(message)?;
        }
        FeishuLarkLongConnectionDecision::NewSession(command) => {
            dispatch_context
                .new_session_dispatcher
                .dispatch(NewSessionWork {
                    command: *command,
                    dispatched_at: Utc::now(),
                })?;
        }
        FeishuLarkLongConnectionDecision::ControlSkip(reason) => {
            log_lark_control_skip(&long_connection.config.provider_id, &reason);
        }
    }

    Ok(())
}

trait FeishuLarkControlReplyDispatcher: Send + Sync {
    fn dispatch(&self, reply: BridgeControlReply) -> anyhow::Result<()>;
}

trait FeishuLarkRoomMessageDispatcher: Send + Sync {
    fn dispatch(&self, message: BridgeRoomMessage) -> anyhow::Result<()>;
}

#[derive(Clone)]
struct LiveFeishuLarkControlReplyDispatcher {
    runtime_state: RuntimeState,
}

impl LiveFeishuLarkControlReplyDispatcher {
    fn new(runtime_state: RuntimeState) -> Self {
        Self { runtime_state }
    }
}

impl FeishuLarkControlReplyDispatcher for LiveFeishuLarkControlReplyDispatcher {
    fn dispatch(&self, reply: BridgeControlReply) -> anyhow::Result<()> {
        dispatch_feishu_lark_control_reply(self.runtime_state.clone(), reply);
        Ok(())
    }
}

#[derive(Clone)]
struct LiveFeishuLarkRoomMessageDispatcher {
    runtime_state: RuntimeState,
}

impl LiveFeishuLarkRoomMessageDispatcher {
    fn new(runtime_state: RuntimeState) -> Self {
        Self { runtime_state }
    }
}

impl FeishuLarkRoomMessageDispatcher for LiveFeishuLarkRoomMessageDispatcher {
    fn dispatch(&self, message: BridgeRoomMessage) -> anyhow::Result<()> {
        dispatch_feishu_lark_room_message(self.runtime_state.clone(), message);
        Ok(())
    }
}

fn lark_app_bot_response_surface_targets(config: &ValidatedConfig) -> Vec<ProviderConfig> {
    config
        .providers
        .iter()
        .filter(|provider| {
            matches!(
                &provider.detail,
                ProviderConfigDetail::FeishuLark(FeishuLarkProviderConfig::AppBot(_))
            ) && has_codex_desktop_response_surface_route(config, provider)
        })
        .cloned()
        .collect()
}

fn has_codex_desktop_response_surface_route(
    config: &ValidatedConfig,
    provider: &ProviderConfig,
) -> bool {
    let Some(source) = config.source("codex_desktop") else {
        return false;
    };
    if source.source_type != SourceType::CodexDesktop {
        return false;
    }
    let Some(agent) = agent_integration_for_source(&source.id, source.source_type) else {
        return false;
    };
    let provider_capability = provider_config_mode_capability(provider);

    config.routes.iter().any(|route| {
        route.sources.iter().any(|source| source == "codex_desktop")
            && route
                .providers
                .iter()
                .any(|route_provider| route_provider == &provider.id)
            && evaluate_response_surface_exposure(agent, provider_capability, route).is_eligible()
    })
}

fn log_platform_ack_sent(provider_id: &str, decision: &FeishuLarkLongConnectionDecision) {
    match decision {
        FeishuLarkLongConnectionDecision::ReadyAfterLocalClaim(ready) => {
            info!(
                provider.id = %provider_id,
                surface.id = %ready.surface.surface_id,
                event.hash = %ready.provider_event_id_hash,
                event = "feishu_lark.long_connection.live.platform_ack.sent",
            );
        }
        FeishuLarkLongConnectionDecision::Skip(reason) => {
            debug!(
                provider.id = %provider_id,
                reason = ?reason,
                event = "feishu_lark.long_connection.live.platform_ack.sent",
            );
        }
        FeishuLarkLongConnectionDecision::ControlReply(reply) => {
            info!(
                provider.id = %provider_id,
                provider.conversation.id = %reply.provider_conversation_id,
                provider.thread.id = %reply.provider_thread_id,
                event.hash = %reply.provider_event_id_hash,
                event = "feishu_lark.long_connection.live.platform_ack.sent",
            );
        }
        FeishuLarkLongConnectionDecision::RoomMessage(message) => {
            info!(
                provider.id = %provider_id,
                provider.conversation.id = %message.provider_conversation_id,
                event.hash = %message.provider_event_id_hash,
                event = "feishu_lark.long_connection.live.platform_ack.sent",
            );
        }
        FeishuLarkLongConnectionDecision::NewSession(command) => {
            info!(
                provider.id = %provider_id,
                provider.conversation.id = %command.provider_conversation_id,
                provider.thread.id = %command.provider_thread_id,
                project.path = %command.project_path,
                event.hash = %command.provider_event_id_hash,
                event = "feishu_lark.long_connection.live.platform_ack.sent",
            );
        }
        FeishuLarkLongConnectionDecision::ControlSkip(reason) => {
            debug!(
                provider.id = %provider_id,
                reason = ?reason,
                event = "feishu_lark.long_connection.live.platform_ack.sent",
            );
        }
    }
}

fn control_command_name(command: &ProviderControlCommand) -> &'static str {
    match command {
        ProviderControlCommand::BindProject { .. } => "bind",
        ProviderControlCommand::DirectBindProject { .. } => "direct_bind",
        ProviderControlCommand::DirectChatGuidance => "direct_guidance",
        ProviderControlCommand::DirectHelp => "direct_help",
        ProviderControlCommand::DirectNewSession { .. } => "direct_new",
        ProviderControlCommand::DirectStatus => "direct_status",
        ProviderControlCommand::DirectUnbindProject => "direct_unbind",
        ProviderControlCommand::Help => "help",
        ProviderControlCommand::NewSession { .. } => "new",
        ProviderControlCommand::RoomGuidance => "room_guidance",
        ProviderControlCommand::Status => "status",
        ProviderControlCommand::UnbindProject { .. } => "unbind",
        ProviderControlCommand::Invalid { .. } => "invalid",
    }
}

fn control_command_surface(command: &ProviderControlCommand) -> &'static str {
    match command {
        ProviderControlCommand::DirectBindProject { .. }
        | ProviderControlCommand::DirectChatGuidance
        | ProviderControlCommand::DirectHelp
        | ProviderControlCommand::DirectNewSession { .. }
        | ProviderControlCommand::DirectStatus
        | ProviderControlCommand::DirectUnbindProject => "direct_chat",
        ProviderControlCommand::BindProject { .. }
        | ProviderControlCommand::Help
        | ProviderControlCommand::NewSession { .. }
        | ProviderControlCommand::RoomGuidance
        | ProviderControlCommand::Status
        | ProviderControlCommand::UnbindProject { .. } => "project_room",
        ProviderControlCommand::Invalid { .. } => "invalid",
    }
}

fn log_lark_inbound_skip(provider_id: &str, reason: &ProviderInboundSkipReason) {
    match reason {
        ProviderInboundSkipReason::SurfaceLookupMiss
        | ProviderInboundSkipReason::SurfaceClosed { .. }
        | ProviderInboundSkipReason::DuplicateEvent { .. }
        | ProviderInboundSkipReason::EventAlreadyProcessing { .. } => {
            info!(
                provider.id = %provider_id,
                reason = ?reason,
                event = "feishu_lark.long_connection.live.event.skipped",
            );
        }
        _ => {
            debug!(
                provider.id = %provider_id,
                reason = ?reason,
                event = "feishu_lark.long_connection.live.event.skipped",
            );
        }
    }
}

fn log_lark_control_skip(provider_id: &str, reason: &ProviderControlSkipReason) {
    debug!(
        provider.id = %provider_id,
        reason = ?reason,
        event = "feishu_lark.long_connection.live.control.skipped",
    );
}

fn log_lark_data_frame_received(provider_id: &str, frame: &FeishuLarkLongConnectionFrame) {
    let payload_bytes = frame.payload.as_ref().map_or(0, Vec::len);
    info!(
        provider.id = %provider_id,
        frame.method = frame.method,
        frame.service = frame.service,
        frame.type = ?header_value(frame, HEADER_TYPE),
        frame.message_id = ?header_value(frame, HEADER_MESSAGE_ID),
        frame.sum = ?header_value(frame, HEADER_SUM),
        frame.seq = ?header_value(frame, HEADER_SEQ),
        frame.trace_id = ?header_value(frame, HEADER_TRACE_ID),
        payload.bytes = payload_bytes,
        event = "feishu_lark.long_connection.live.data_frame.received",
    );
}

pub(crate) struct FeishuLarkOfficialLongConnectionTransport {
    websocket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    service_id: i32,
    protocol_ping_interval: Option<Duration>,
    next_protocol_ping: Option<Instant>,
}

impl FeishuLarkOfficialLongConnectionTransport {
    async fn connect(
        url: String,
        client_config: Option<FeishuLarkLongConnectionClientConfig>,
    ) -> anyhow::Result<Self> {
        let service_id = lark_long_connection_service_id(&url)?;
        let protocol_ping_interval = client_config
            .and_then(|config| config.ping_interval)
            .and_then(|seconds| u64::try_from(seconds).ok())
            .filter(|seconds| *seconds > 0)
            .map(Duration::from_secs);
        let (websocket, _) = connect_async(&url)
            .await
            .context("failed to connect Feishu/Lark official long connection WebSocket")?;
        Ok(Self {
            websocket,
            service_id,
            protocol_ping_interval,
            next_protocol_ping: protocol_ping_interval.map(|_| Instant::now()),
        })
    }

    async fn receive_websocket_message(
        &mut self,
        message: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
    ) -> anyhow::Result<FeishuLarkLongConnectionTransportMessage> {
        let Some(message) = message else {
            return Ok(FeishuLarkLongConnectionTransportMessage::Closed);
        };
        match message.context("Feishu/Lark long connection receive failed")? {
            Message::Binary(bytes) => Ok(FeishuLarkLongConnectionTransportMessage::Binary(
                bytes.to_vec(),
            )),
            Message::Text(text) => Ok(FeishuLarkLongConnectionTransportMessage::Text(
                text.to_string(),
            )),
            Message::Close(_) => Ok(FeishuLarkLongConnectionTransportMessage::Closed),
            Message::Ping(bytes) => {
                self.websocket
                    .send(Message::Pong(bytes))
                    .await
                    .context("failed to send Feishu/Lark WebSocket pong")?;
                self.receive().await
            }
            Message::Pong(_) | Message::Frame(_) => self.receive().await,
        }
    }

    async fn send_protocol_ping(&mut self) -> anyhow::Result<()> {
        let frame = lark_protocol_ping_frame(self.service_id);
        self.websocket
            .send(Message::Binary(frame.encode_to_vec().into()))
            .await
            .context("failed to send Feishu/Lark protocol ping")?;
        self.arm_next_protocol_ping();
        Ok(())
    }

    fn arm_next_protocol_ping(&mut self) {
        self.next_protocol_ping = self
            .protocol_ping_interval
            .map(|interval| Instant::now() + interval);
    }
}

impl FeishuLarkLongConnectionTransport for FeishuLarkOfficialLongConnectionTransport {
    fn receive<'a>(
        &'a mut self,
    ) -> FeishuLarkTransportFuture<'a, FeishuLarkLongConnectionTransportMessage> {
        Box::pin(async move {
            loop {
                if let Some(deadline) = self.next_protocol_ping {
                    tokio::select! {
                        _ = sleep_until(deadline) => {
                            self.send_protocol_ping().await?;
                        }
                        message = self.websocket.next() => {
                            return self.receive_websocket_message(message).await;
                        }
                    }
                } else {
                    let message = self.websocket.next().await;
                    return self.receive_websocket_message(message).await;
                }
            }
        })
    }

    fn send_binary<'a>(&'a mut self, data: Vec<u8>) -> FeishuLarkTransportFuture<'a, ()> {
        Box::pin(async move {
            self.websocket
                .send(Message::Binary(data.into()))
                .await
                .context("failed to send Feishu/Lark long connection frame")?;
            Ok(())
        })
    }
}

#[derive(Debug, Default)]
pub(crate) struct FeishuLarkLongConnectionPayloadBuffer {
    fragments: HashMap<String, Vec<Option<Vec<u8>>>>,
}

impl FeishuLarkLongConnectionPayloadBuffer {
    pub fn accept_frame(
        &mut self,
        frame: &FeishuLarkLongConnectionFrame,
    ) -> anyhow::Result<Option<Vec<u8>>> {
        let payload = frame
            .payload
            .clone()
            .context("Feishu/Lark event frame did not include payload")?;
        let sum = header_value(frame, HEADER_SUM)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(1);
        if sum <= 1 {
            return Ok(Some(payload));
        }

        let message_id = required_header(frame, HEADER_MESSAGE_ID)?;
        let seq = required_header(frame, HEADER_SEQ)?
            .parse::<usize>()
            .context("Feishu/Lark event frame seq header was not a number")?;
        ensure!(
            seq < sum,
            "Feishu/Lark event frame seq {} was outside fragment sum {}",
            seq,
            sum
        );

        let entry = self
            .fragments
            .entry(message_id.clone())
            .or_insert_with(|| vec![None; sum]);
        ensure!(
            entry.len() == sum,
            "Feishu/Lark event frame fragment sum changed for message `{}`",
            message_id
        );
        entry[seq] = Some(payload);

        if entry.iter().any(Option::is_none) {
            return Ok(None);
        }

        let completed = entry
            .iter()
            .filter_map(|fragment| fragment.as_ref())
            .flat_map(|fragment| fragment.iter().copied())
            .collect::<Vec<_>>();
        self.fragments.remove(&message_id);
        Ok(Some(completed))
    }
}

#[derive(Clone, PartialEq, ProstMessage)]
pub(crate) struct FeishuLarkLongConnectionFrameHeader {
    #[prost(string, required, tag = "1")]
    pub key: String,
    #[prost(string, required, tag = "2")]
    pub value: String,
}

#[derive(Clone, PartialEq, ProstMessage)]
pub(crate) struct FeishuLarkLongConnectionFrame {
    #[prost(uint64, required, tag = "1")]
    pub seq_id: u64,
    #[prost(uint64, required, tag = "2")]
    pub log_id: u64,
    #[prost(int32, required, tag = "3")]
    pub service: i32,
    #[prost(int32, required, tag = "4")]
    pub method: i32,
    #[prost(message, repeated, tag = "5")]
    pub headers: Vec<FeishuLarkLongConnectionFrameHeader>,
    #[prost(string, optional, tag = "6")]
    pub payload_encoding: Option<String>,
    #[prost(string, optional, tag = "7")]
    pub payload_type: Option<String>,
    #[prost(bytes, optional, tag = "8")]
    pub payload: Option<Vec<u8>>,
    #[prost(string, optional, tag = "9")]
    pub log_id_new: Option<String>,
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

#[derive(Debug, Deserialize)]
struct FeishuLarkBotInfoResponse {
    code: i64,
    msg: Option<String>,
    bot: Option<FeishuLarkBotInfo>,
}

#[derive(Debug, Deserialize)]
struct FeishuLarkBotInfo {
    open_id: Option<String>,
    app_name: Option<String>,
}

#[derive(Debug, Serialize)]
struct FeishuLarkLongConnectionEndpointRequest<'a> {
    #[serde(rename = "AppID")]
    app_id: &'a str,
    #[serde(rename = "AppSecret")]
    app_secret: &'a str,
}

#[derive(Debug, Deserialize)]
struct FeishuLarkLongConnectionEndpointResponse {
    code: i64,
    msg: Option<String>,
    data: Option<FeishuLarkLongConnectionEndpointData>,
}

#[derive(Debug, Deserialize)]
struct FeishuLarkLongConnectionEndpointData {
    #[serde(rename = "URL")]
    url: Option<String>,
    #[serde(rename = "ClientConfig")]
    client_config: Option<FeishuLarkLongConnectionEndpointClientConfig>,
}

#[derive(Debug, Deserialize)]
struct FeishuLarkLongConnectionEndpointClientConfig {
    #[serde(rename = "ReconnectCount")]
    reconnect_count: Option<i64>,
    #[serde(rename = "ReconnectInterval")]
    reconnect_interval: Option<i64>,
    #[serde(rename = "ReconnectNonce")]
    reconnect_nonce: Option<i64>,
    #[serde(rename = "PingInterval")]
    ping_interval: Option<i64>,
}

impl From<FeishuLarkLongConnectionEndpointClientConfig> for FeishuLarkLongConnectionClientConfig {
    fn from(value: FeishuLarkLongConnectionEndpointClientConfig) -> Self {
        Self {
            reconnect_count: value.reconnect_count,
            reconnect_interval: value.reconnect_interval,
            reconnect_nonce: value.reconnect_nonce,
            ping_interval: value.ping_interval,
        }
    }
}

#[derive(Debug, Serialize)]
struct FeishuLarkLongConnectionPlatformAck {
    code: u16,
}

fn lark_frame_from_transport_message(
    message: FeishuLarkLongConnectionTransportMessage,
) -> anyhow::Result<Option<FeishuLarkLongConnectionFrame>> {
    match message {
        FeishuLarkLongConnectionTransportMessage::Binary(bytes) => {
            FeishuLarkLongConnectionFrame::decode(bytes.as_slice())
                .map(Some)
                .context("failed to decode Feishu/Lark long connection protobuf frame")
        }
        FeishuLarkLongConnectionTransportMessage::Text(_) => {
            anyhow::bail!("Feishu/Lark long connection returned unsupported text frame")
        }
        FeishuLarkLongConnectionTransportMessage::Closed => Ok(None),
    }
}

fn header_value(frame: &FeishuLarkLongConnectionFrame, key: &str) -> Option<String> {
    frame
        .headers
        .iter()
        .find(|header| header.key == key)
        .map(|header| header.value.clone())
}

fn required_header(frame: &FeishuLarkLongConnectionFrame, key: &str) -> anyhow::Result<String> {
    header_value(frame, key)
        .with_context(|| format!("Feishu/Lark long connection frame was missing `{key}` header"))
}

fn lark_protocol_ping_frame(service_id: i32) -> FeishuLarkLongConnectionFrame {
    FeishuLarkLongConnectionFrame {
        seq_id: 0,
        log_id: 0,
        service: service_id,
        method: FRAME_METHOD_CONTROL,
        headers: vec![FeishuLarkLongConnectionFrameHeader {
            key: HEADER_TYPE.to_string(),
            value: "ping".to_string(),
        }],
        payload_encoding: None,
        payload_type: None,
        payload: None,
        log_id_new: None,
    }
}

fn lark_long_connection_service_id(connect_url: &str) -> anyhow::Result<i32> {
    let url = Url::parse(connect_url).context("Feishu/Lark long connection URL was invalid")?;
    let service_id = url
        .query_pairs()
        .find_map(|(key, value)| (key == "service_id").then_some(value.into_owned()))
        .context("Feishu/Lark long connection URL did not include service_id")?;
    service_id
        .parse::<i32>()
        .context("Feishu/Lark long connection service_id was not a number")
}

fn long_connection_api_base_url(domain: FeishuLarkAppDomain) -> &'static str {
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

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use chrono::{Duration, TimeZone};
    use tempfile::tempdir;

    use super::*;
    use crate::bridge_binding_ledger::RoomBindingQuery;
    use crate::config::{
        CliConfig, FeishuLarkAppBotProviderConfig, FeishuLarkCustomBotProviderConfig, LogConfig,
        NotificationConfig, RawConfig, RawProviderConfig, RouteConfig, SecretSource, SourceConfig,
        UrlSource,
    };
    use crate::continuation_dispatcher::{ClaimedContinuationWork, ContinuationDispatcher};
    use crate::response_surface_ledger::NewResponseSurface;

    const TEST_LARK_BOT_OPEN_ID: &str = "ou_test_bot";

    #[test]
    fn app_bot_runtime_uses_official_long_connection_credentials_only() {
        let runtime = app_bot_runtime();

        assert_eq!(runtime.connection_config().provider_id, "lark-app");
        assert_eq!(
            runtime.connection_config().domain,
            FeishuLarkAppDomain::Lark
        );
        assert_eq!(runtime.connection_config().app_id, "cli_test_app");
        assert_eq!(runtime.connection_config().app_secret(), "test-secret");
        assert!(!format!("{:?}", runtime.connection_config()).contains("test-secret"));
    }

    #[test]
    fn personal_agent_without_default_room_can_start_long_connection_runtime() {
        let runtime =
            FeishuLarkLongConnectionRuntime::from_provider_config(&personal_agent_provider())
                .expect("Personal Agent should build long connection runtime without a room");

        assert_eq!(runtime.connection_config().provider_id, "lark-app");
        assert_eq!(runtime.connection_config().app_id, "cli_test_app");
        assert_eq!(runtime.connection_config().app_secret(), "test-secret");
    }

    #[test]
    fn custom_bot_runtime_is_rejected_instead_of_inferred_from_fields() {
        let error = FeishuLarkLongConnectionRuntime::from_provider_config(&ProviderConfig {
            id: "lark-custom".to_string(),
            detail: ProviderConfigDetail::FeishuLark(FeishuLarkProviderConfig::CustomBot(
                FeishuLarkCustomBotProviderConfig {
                    url: UrlSource::Inline(
                        "https://open.larksuite.com/open-apis/bot/v2/hook/test".to_string(),
                    ),
                    secret: None,
                },
            )),
        })
        .expect_err("Custom Bot must not enter long connection runtime");

        assert!(error.to_string().contains("mode = \"app_bot\""));
    }

    #[test]
    fn live_targets_are_driven_by_catalog_provider_facts_and_route_config() {
        let config = lark_response_surface_config(true, true);

        let targets = lark_app_bot_response_surface_targets(&config);

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, "lark-app");
    }

    #[test]
    fn live_targets_include_personal_agent_without_default_room() {
        let config = personal_agent_response_surface_config();

        let targets = lark_app_bot_response_surface_targets(&config);

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, "lark-app");
    }

    #[test]
    fn live_targets_skip_when_route_replies_are_disabled() {
        let config = lark_response_surface_config(false, true);

        assert!(lark_app_bot_response_surface_targets(&config).is_empty());
    }

    #[test]
    fn live_targets_skip_custom_bot_even_when_route_replies_are_enabled() {
        let config = lark_response_surface_config(true, false);

        assert!(lark_app_bot_response_surface_targets(&config).is_empty());
    }

    #[test]
    fn surface_reply_is_claimed_before_platform_ack_ready() {
        let now = test_time();
        let runtime = app_bot_runtime();
        let mut ledger = ledger_with_lark_surface(now);

        let decision = runtime
            .handle_event_before_platform_ack(&mut ledger, event_at(now + Duration::seconds(1)))
            .expect("event handling should succeed");

        assert_eq!(decision.platform_ack(), FeishuLarkPlatformAck::Acknowledge);
        let FeishuLarkLongConnectionDecision::ReadyAfterLocalClaim(ready) = decision else {
            panic!("surface reply should be ready after local claim");
        };
        assert_eq!(ready.surface.source_session_id, "session-1");
        assert_eq!(ready.reply.provider_id, "lark-app");
        assert_eq!(ready.reply.provider_thread_id, "om_root_message_id");
        assert_eq!(ready.reply.reply_text, "continue with README");

        let duplicate = runtime
            .handle_event_before_platform_ack(&mut ledger, event_at(now + Duration::seconds(2)))
            .expect("duplicate event handling should succeed");
        assert_eq!(duplicate.platform_ack(), FeishuLarkPlatformAck::Acknowledge);
        assert!(matches!(
            duplicate,
            FeishuLarkLongConnectionDecision::Skip(
                ProviderInboundSkipReason::EventAlreadyProcessing { .. }
            )
        ));
    }

    #[test]
    fn lookup_miss_is_ack_skipped_without_claiming_event() {
        let now = test_time();
        let runtime = app_bot_runtime();
        let mut ledger = ResponseSurfaceLedger::in_memory();

        let decision = runtime
            .handle_event_before_platform_ack(&mut ledger, event_at(now + Duration::seconds(1)))
            .expect("lookup miss should not fail");

        assert_eq!(decision.platform_ack(), FeishuLarkPlatformAck::Acknowledge);
        assert_eq!(
            decision,
            FeishuLarkLongConnectionDecision::Skip(ProviderInboundSkipReason::SurfaceLookupMiss)
        );

        ledger
            .create_surface_at(lark_surface(now), now + Duration::seconds(2))
            .expect("surface should be creatable after lookup miss");
        let later = runtime
            .handle_event_before_platform_ack(&mut ledger, event_at(now + Duration::seconds(3)))
            .expect("event should still be claimable after surface appears");
        assert!(matches!(
            later,
            FeishuLarkLongConnectionDecision::ReadyAfterLocalClaim(_)
        ));
    }

    #[test]
    fn rootless_mention_is_ack_skipped_without_surface_trigger() {
        let now = test_time();
        let runtime = app_bot_runtime();
        let mut ledger = ledger_with_lark_surface(now);

        let decision = runtime
            .handle_event_before_platform_ack(
                &mut ledger,
                FeishuLarkLongConnectionEvent {
                    raw_event: br#"{
                        "schema": "2.0",
                        "header": {
                            "event_id": "event-1",
                            "event_type": "im.message.receive_v1",
                            "tenant_key": "2ca1d211f64f6438"
                        },
                        "event": {
                            "sender": { "sender_type": "user" },
                            "message": {
                                "message_id": "om_rootless_reply",
                                "root_id": "",
                                "chat_id": "oc_5ce6d572455d361153b7xx51da133945",
                                "message_type": "text",
                                "content": "{\"text\":\"@_user_1 continue\"}"
                            }
                        }
                    }"#,
                    received_at: now + Duration::seconds(1),
                },
            )
            .expect("rootless mention should parse as skip");

        assert_eq!(decision.platform_ack(), FeishuLarkPlatformAck::Acknowledge);
        assert_eq!(
            decision,
            FeishuLarkLongConnectionDecision::Skip(ProviderInboundSkipReason::NotSurfaceReply)
        );
    }

    #[tokio::test]
    async fn hidden_transport_receives_event_then_claims_and_acks_platform() {
        let now = test_time();
        let runtime = app_bot_runtime();
        let mut ledger = ledger_with_lark_surface(now);
        let mut transport = RecordingTransport::with_messages(vec![
            FeishuLarkLongConnectionTransportMessage::Binary(event_frame(
                include_bytes!(
                    "../../tests/fixtures/provider_inbound/feishu_lark_surface_reply.json"
                )
                .to_vec(),
            )),
        ]);
        let mut payload_buffer = FeishuLarkLongConnectionPayloadBuffer::default();

        let decision = runtime
            .receive_event_before_platform_ack_hidden(
                &mut ledger,
                &mut transport,
                &mut payload_buffer,
                now + Duration::seconds(1),
            )
            .await
            .expect("hidden transport should receive and ack event");

        let FeishuLarkLongConnectionDecision::ReadyAfterLocalClaim(ready) = decision else {
            panic!("surface reply should be ready after local claim");
        };
        assert_eq!(ready.reply.provider_thread_id, "om_root_message_id");
        assert_eq!(ready.reply.reply_text, "continue with README");

        let sent = transport.sent_messages();
        assert_eq!(sent.len(), 1);
        let ack = FeishuLarkLongConnectionFrame::decode(sent[0].as_slice())
            .expect("platform ack should be a protobuf frame");
        assert_eq!(ack.method, FRAME_METHOD_DATA);
        assert_eq!(header_value(&ack, HEADER_BIZ_RT).as_deref(), Some("0"));
        let payload = ack.payload.expect("ack should include JSON payload");
        let payload: serde_json::Value =
            serde_json::from_slice(&payload).expect("ack payload should be JSON");
        assert_eq!(payload["code"], PLATFORM_ACK_CODE_OK);
    }

    #[tokio::test]
    async fn hidden_runtime_claims_event_from_shared_ledger_store() {
        let now = test_time();
        let dir = tempdir().expect("temp dir should exist");
        let ledger_store = ResponseSurfaceLedgerStore::new(dir.path().join("ledger.json"))
            .expect("ledger store should build");
        ledger_store
            .update(|ledger| {
                ledger.create_surface_at(lark_surface(now), now)?;
                Ok(())
            })
            .await
            .expect("surface should be stored");
        let runtime = app_bot_runtime();

        let decision = runtime
            .handle_event_before_platform_ack_with_store_hidden(
                &ledger_store,
                event_at(now + Duration::seconds(1)),
            )
            .await
            .expect("event should claim through shared ledger store");

        let FeishuLarkLongConnectionDecision::ReadyAfterLocalClaim(ready) = decision else {
            panic!("surface reply should be ready after local claim");
        };
        assert_eq!(ready.surface.source_session_id, "session-1");
        let duplicate = runtime
            .handle_event_before_platform_ack_with_store_hidden(
                &ledger_store,
                event_at(now + Duration::seconds(2)),
            )
            .await
            .expect("duplicate should be read from same store");
        assert!(matches!(
            duplicate,
            FeishuLarkLongConnectionDecision::Skip(
                ProviderInboundSkipReason::EventAlreadyProcessing { .. }
            )
        ));
    }

    #[tokio::test]
    async fn hidden_transport_dispatches_claimed_event_without_blocking_next_receive() {
        let now = test_time();
        let dir = tempdir().expect("temp dir should exist");
        let ledger_store = ResponseSurfaceLedgerStore::new(dir.path().join("ledger.json"))
            .expect("ledger store should build");
        let bridge_binding_ledger_store =
            BridgeBindingLedgerStore::new(dir.path().join("bridge-bindings.json"))
                .expect("binding ledger store should build");
        ledger_store
            .update(|ledger| {
                ledger.create_surface_at(lark_surface(now), now)?;
                Ok(())
            })
            .await
            .expect("surface should be stored");
        let runtime = app_bot_runtime();
        let mut transport = RecordingTransport::with_messages(vec![
            FeishuLarkLongConnectionTransportMessage::Binary(event_frame(lark_reply_payload(
                "om_reply_message_id",
                "@_user_1 continue with README",
            ))),
            FeishuLarkLongConnectionTransportMessage::Binary(event_frame(lark_reply_payload(
                "om_second_reply_message_id",
                "@_user_1 also update the logs",
            ))),
        ]);
        let mut payload_buffer = FeishuLarkLongConnectionPayloadBuffer::default();
        let dispatcher = RecordingContinuationDispatcher::default();
        let control_reply_dispatcher = RecordingControlReplyDispatcher::default();
        let room_message_dispatcher = RecordingRoomMessageDispatcher::default();
        let new_session_dispatcher = RecordingNewSessionDispatcher::default();

        receive_and_dispatch_live_lark_event_hidden(
            &runtime,
            &mut transport,
            &mut payload_buffer,
            test_dispatch_context(
                &ledger_store,
                &bridge_binding_ledger_store,
                &dispatcher,
                &control_reply_dispatcher,
                &room_message_dispatcher,
                &new_session_dispatcher,
            ),
        )
        .await
        .expect("first event should be acknowledged and dispatched");
        assert_eq!(transport.sent_messages().len(), 1);
        assert_eq!(dispatcher.dispatched().len(), 1);

        receive_and_dispatch_live_lark_event_hidden(
            &runtime,
            &mut transport,
            &mut payload_buffer,
            test_dispatch_context(
                &ledger_store,
                &bridge_binding_ledger_store,
                &dispatcher,
                &control_reply_dispatcher,
                &room_message_dispatcher,
                &new_session_dispatcher,
            ),
        )
        .await
        .expect("second event should be received without waiting for first continuation");

        let dispatched = dispatcher.dispatched();
        assert_eq!(transport.sent_messages().len(), 2);
        assert_eq!(dispatched.len(), 2);
        assert_eq!(dispatched[0].ready.reply.reply_text, "continue with README");
        assert_eq!(dispatched[1].ready.reply.reply_text, "also update the logs");
    }

    #[tokio::test]
    async fn hidden_transport_binds_room_command_before_ack_and_dispatches_control_reply() {
        let dir = tempdir().expect("temp dir should exist");
        let project_dir = tempdir().expect("project dir should exist");
        let project_path = project_dir.path().to_string_lossy().to_string();
        let ledger_store = ResponseSurfaceLedgerStore::new(dir.path().join("ledger.json"))
            .expect("ledger store should build");
        let bridge_binding_ledger_store =
            BridgeBindingLedgerStore::new(dir.path().join("bridge-bindings.json"))
                .expect("binding ledger store should build");
        let runtime = app_bot_runtime();
        let mut transport = RecordingTransport::with_messages(vec![
            FeishuLarkLongConnectionTransportMessage::Binary(event_frame(lark_root_payload(
                "om_bind_message_id",
                &format!("@_user_1 /bind {project_path}"),
            ))),
        ]);
        let mut payload_buffer = FeishuLarkLongConnectionPayloadBuffer::default();
        let dispatcher = RecordingContinuationDispatcher::default();
        let control_reply_dispatcher = RecordingControlReplyDispatcher::default();
        let room_message_dispatcher = RecordingRoomMessageDispatcher::default();
        let new_session_dispatcher = RecordingNewSessionDispatcher::default();

        receive_and_dispatch_live_lark_event_hidden(
            &runtime,
            &mut transport,
            &mut payload_buffer,
            test_dispatch_context(
                &ledger_store,
                &bridge_binding_ledger_store,
                &dispatcher,
                &control_reply_dispatcher,
                &room_message_dispatcher,
                &new_session_dispatcher,
            ),
        )
        .await
        .expect("bind command should ack and dispatch control reply");

        assert_eq!(transport.sent_messages().len(), 1);
        assert!(dispatcher.dispatched().is_empty());
        let replies = control_reply_dispatcher.dispatched();
        assert_eq!(replies.len(), 1);
        assert_eq!(
            replies[0].text,
            format!(
                "Done. This room is connected to:\n{project_path}\n\nNew Codex updates for this folder will appear here."
            )
        );
        assert!(new_session_dispatcher.dispatched().is_empty());
        let connected = bridge_binding_ledger_store
            .update(|ledger| {
                Ok(ledger.connected_projects_for_room(&RoomBindingQuery {
                    provider_id: "lark-app".to_string(),
                    provider_type: "feishu_lark".to_string(),
                    provider_account_id: "2ca1d211f64f6438".to_string(),
                    provider_conversation_id: "oc_5ce6d572455d361153b7xx51da133945".to_string(),
                }))
            })
            .await
            .expect("binding ledger should read");
        assert_eq!(connected.len(), 1);
        assert_eq!(connected[0].project_path, project_path);
    }

    #[tokio::test]
    async fn hidden_transport_dispatches_room_welcome_for_bot_added_event() {
        let dir = tempdir().expect("temp dir should exist");
        let ledger_store = ResponseSurfaceLedgerStore::new(dir.path().join("ledger.json"))
            .expect("ledger store should build");
        let bridge_binding_ledger_store =
            BridgeBindingLedgerStore::new(dir.path().join("bridge-bindings.json"))
                .expect("binding ledger store should build");
        let runtime = app_bot_runtime();
        let mut transport = RecordingTransport::with_messages(vec![
            FeishuLarkLongConnectionTransportMessage::Binary(event_frame(
                br#"{
                    "schema": "2.0",
                    "header": {
                        "event_id": "event-bot-added",
                        "event_type": "im.chat.member.bot.added_v1",
                        "tenant_key": "2ca1d211f64f6438"
                    },
                    "event": {
                        "chat_id": "oc_project_room"
                    }
                }"#
                .to_vec(),
            )),
        ]);
        let mut payload_buffer = FeishuLarkLongConnectionPayloadBuffer::default();
        let dispatcher = RecordingContinuationDispatcher::default();
        let control_reply_dispatcher = RecordingControlReplyDispatcher::default();
        let room_message_dispatcher = RecordingRoomMessageDispatcher::default();
        let new_session_dispatcher = RecordingNewSessionDispatcher::default();

        receive_and_dispatch_live_lark_event_hidden(
            &runtime,
            &mut transport,
            &mut payload_buffer,
            test_dispatch_context(
                &ledger_store,
                &bridge_binding_ledger_store,
                &dispatcher,
                &control_reply_dispatcher,
                &room_message_dispatcher,
                &new_session_dispatcher,
            ),
        )
        .await
        .expect("bot-added event should ack and dispatch a room message");

        assert_eq!(transport.sent_messages().len(), 1);
        assert!(dispatcher.dispatched().is_empty());
        assert!(control_reply_dispatcher.dispatched().is_empty());
        assert!(new_session_dispatcher.dispatched().is_empty());
        let messages = room_message_dispatcher.dispatched();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_conversation_id, "oc_project_room");
        assert!(messages[0].text.contains("Hi, I can bring Codex updates"));
        assert!(
            messages[0]
                .text
                .contains("`@your-bot /bind /path/to/project`")
        );
        assert!(messages[0].text.contains("Use Lark's @ menu to select me"));
    }

    #[tokio::test]
    async fn hidden_transport_dispatches_new_session_command_for_bound_room() {
        let dir = tempdir().expect("temp dir should exist");
        let project_dir = tempdir().expect("project dir should exist");
        let project_path = project_dir.path().to_string_lossy().to_string();
        let ledger_store = ResponseSurfaceLedgerStore::new(dir.path().join("ledger.json"))
            .expect("ledger store should build");
        let bridge_binding_ledger_store =
            BridgeBindingLedgerStore::new(dir.path().join("bridge-bindings.json"))
                .expect("binding ledger store should build");
        bridge_binding_ledger_store
            .update(|ledger| {
                ledger.connect_room_project_at(
                    crate::bridge_binding_ledger::RoomProjectBindingInput {
                        provider_id: "lark-app".to_string(),
                        provider_type: "feishu_lark".to_string(),
                        provider_account_id: "2ca1d211f64f6438".to_string(),
                        provider_conversation_id: "oc_5ce6d572455d361153b7xx51da133945".to_string(),
                        project_path: project_path.clone(),
                    },
                    test_time(),
                )?;
                Ok(())
            })
            .await
            .expect("room binding should persist");
        let runtime = app_bot_runtime();
        let mut transport = RecordingTransport::with_messages(vec![
            FeishuLarkLongConnectionTransportMessage::Binary(event_frame(lark_root_payload(
                "om_new_message_id",
                "@_user_1 /new Reply exactly OK.",
            ))),
        ]);
        let mut payload_buffer = FeishuLarkLongConnectionPayloadBuffer::default();
        let dispatcher = RecordingContinuationDispatcher::default();
        let control_reply_dispatcher = RecordingControlReplyDispatcher::default();
        let room_message_dispatcher = RecordingRoomMessageDispatcher::default();
        let new_session_dispatcher = RecordingNewSessionDispatcher::default();

        receive_and_dispatch_live_lark_event_hidden(
            &runtime,
            &mut transport,
            &mut payload_buffer,
            test_dispatch_context(
                &ledger_store,
                &bridge_binding_ledger_store,
                &dispatcher,
                &control_reply_dispatcher,
                &room_message_dispatcher,
                &new_session_dispatcher,
            ),
        )
        .await
        .expect("new command should ack and dispatch work");

        assert_eq!(transport.sent_messages().len(), 1);
        assert!(dispatcher.dispatched().is_empty());
        assert!(control_reply_dispatcher.dispatched().is_empty());
        let dispatched = new_session_dispatcher.dispatched();
        assert_eq!(dispatched.len(), 1);
        assert_eq!(dispatched[0].command.project_path, project_path);
        assert_eq!(
            dispatched[0].command.provider_thread_id,
            "om_new_message_id"
        );
        assert_eq!(dispatched[0].command.prompt, "Reply exactly OK.");
    }

    #[tokio::test]
    async fn hidden_transport_dispatches_thread_binding_when_surface_is_missing() {
        let now = test_time();
        let dir = tempdir().expect("temp dir should exist");
        let ledger_store = ResponseSurfaceLedgerStore::new(dir.path().join("ledger.json"))
            .expect("ledger store should build");
        let bridge_binding_ledger_store =
            BridgeBindingLedgerStore::new(dir.path().join("bridge-bindings.json"))
                .expect("binding ledger store should build");
        bridge_binding_ledger_store
            .update(|ledger| {
                ledger.bind_thread_session_at(
                    ThreadSessionBindingInput {
                        provider_id: "lark-app".to_string(),
                        provider_type: ProviderType::FeishuLark.as_str().to_string(),
                        provider_account_id: "2ca1d211f64f6438".to_string(),
                        provider_conversation_id: "oc_5ce6d572455d361153b7xx51da133945".to_string(),
                        provider_thread_id: "om_root_message_id".to_string(),
                        project_path: "/Users/tester/projects/agents-router".to_string(),
                        source_id: "codex_desktop".to_string(),
                        source_type: "codex_desktop".to_string(),
                        source_session_id: "session-1".to_string(),
                    },
                    now,
                )?;
                Ok(())
            })
            .await
            .expect("thread binding should persist");
        let runtime = app_bot_runtime();
        let mut transport = RecordingTransport::with_messages(vec![
            FeishuLarkLongConnectionTransportMessage::Binary(event_frame(lark_reply_payload(
                "om_reply_without_surface",
                "@_user_1 continue from binding",
            ))),
        ]);
        let mut payload_buffer = FeishuLarkLongConnectionPayloadBuffer::default();
        let dispatcher = RecordingContinuationDispatcher::default();
        let control_reply_dispatcher = RecordingControlReplyDispatcher::default();
        let room_message_dispatcher = RecordingRoomMessageDispatcher::default();
        let new_session_dispatcher = RecordingNewSessionDispatcher::default();

        receive_and_dispatch_live_lark_event_hidden(
            &runtime,
            &mut transport,
            &mut payload_buffer,
            test_dispatch_context(
                &ledger_store,
                &bridge_binding_ledger_store,
                &dispatcher,
                &control_reply_dispatcher,
                &room_message_dispatcher,
                &new_session_dispatcher,
            ),
        )
        .await
        .expect("thread-bound reply should ack and dispatch continuation");

        assert_eq!(transport.sent_messages().len(), 1);
        assert!(control_reply_dispatcher.dispatched().is_empty());
        let dispatched = dispatcher.dispatched();
        assert_eq!(dispatched.len(), 1);
        assert_eq!(
            dispatched[0].ready.reply.reply_text,
            "continue from binding"
        );
        assert_eq!(dispatched[0].ready.surface.source_session_id, "session-1");
        assert!(
            dispatched[0]
                .ready
                .surface
                .surface_id
                .starts_with("bridge-thread-")
        );
    }

    #[tokio::test]
    async fn hidden_transport_replies_when_lark_thread_is_not_connected() {
        let dir = tempdir().expect("temp dir should exist");
        let ledger_store = ResponseSurfaceLedgerStore::new(dir.path().join("ledger.json"))
            .expect("ledger store should build");
        let bridge_binding_ledger_store =
            BridgeBindingLedgerStore::new(dir.path().join("bridge-bindings.json"))
                .expect("binding ledger store should build");
        let runtime = app_bot_runtime();
        let mut transport = RecordingTransport::with_messages(vec![
            FeishuLarkLongConnectionTransportMessage::Binary(event_frame(lark_reply_payload(
                "om_unbound_reply",
                "@_user_1 continue from old thread",
            ))),
        ]);
        let mut payload_buffer = FeishuLarkLongConnectionPayloadBuffer::default();
        let dispatcher = RecordingContinuationDispatcher::default();
        let control_reply_dispatcher = RecordingControlReplyDispatcher::default();
        let room_message_dispatcher = RecordingRoomMessageDispatcher::default();
        let new_session_dispatcher = RecordingNewSessionDispatcher::default();

        receive_and_dispatch_live_lark_event_hidden(
            &runtime,
            &mut transport,
            &mut payload_buffer,
            test_dispatch_context(
                &ledger_store,
                &bridge_binding_ledger_store,
                &dispatcher,
                &control_reply_dispatcher,
                &room_message_dispatcher,
                &new_session_dispatcher,
            ),
        )
        .await
        .expect("unbound thread reply should ack and dispatch a helpful reply");

        assert_eq!(transport.sent_messages().len(), 1);
        assert!(dispatcher.dispatched().is_empty());
        assert!(room_message_dispatcher.dispatched().is_empty());
        assert!(new_session_dispatcher.dispatched().is_empty());
        let replies = control_reply_dispatcher.dispatched();
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].provider_thread_id, "om_root_message_id");
        assert!(replies[0].text.contains("no longer connected to Codex"));
    }

    #[tokio::test]
    async fn hidden_transport_ack_skips_non_surface_event_without_controller_trigger() {
        let now = test_time();
        let runtime = app_bot_runtime();
        let mut ledger = ledger_with_lark_surface(now);
        let mut transport = RecordingTransport::with_messages(vec![
            FeishuLarkLongConnectionTransportMessage::Binary(event_frame(
                br#"{
                    "schema": "2.0",
                    "header": {
                        "event_id": "event-1",
                        "event_type": "im.message.receive_v1",
                        "tenant_key": "2ca1d211f64f6438"
                    },
                    "event": {
                        "sender": { "sender_type": "user" },
                        "message": {
                            "message_id": "om_rootless_reply",
                            "root_id": "",
                            "chat_id": "oc_5ce6d572455d361153b7xx51da133945",
                            "message_type": "text",
                            "content": "{\"text\":\"@_user_1 continue\"}"
                        }
                    }
                }"#
                .to_vec(),
            )),
        ]);
        let mut payload_buffer = FeishuLarkLongConnectionPayloadBuffer::default();

        let decision = runtime
            .receive_event_before_platform_ack_hidden(
                &mut ledger,
                &mut transport,
                &mut payload_buffer,
                now + Duration::seconds(1),
            )
            .await
            .expect("hidden transport should ack skipped event");

        assert_eq!(
            decision,
            FeishuLarkLongConnectionDecision::Skip(ProviderInboundSkipReason::NotSurfaceReply)
        );
        assert_eq!(transport.sent_messages().len(), 1);
    }

    #[test]
    fn fragmented_event_frames_are_reassembled_before_normalization() {
        let payload = br#"{"schema":"2.0"}"#;
        let first = fragment_frame("message-1", 2, 0, payload[..8].to_vec());
        let second = fragment_frame("message-1", 2, 1, payload[8..].to_vec());
        let mut buffer = FeishuLarkLongConnectionPayloadBuffer::default();

        assert_eq!(
            buffer
                .accept_frame(&first)
                .expect("first fragment should be accepted"),
            None
        );
        assert_eq!(
            buffer
                .accept_frame(&second)
                .expect("second fragment should complete payload"),
            Some(payload.to_vec())
        );
    }

    #[test]
    fn parses_official_long_connection_service_id_from_url() {
        let service_id = lark_long_connection_service_id(
            "wss://example.invalid/ws?device_id=device-1&service_id=42",
        )
        .expect("service_id should parse");

        assert_eq!(service_id, 42);
    }

    #[test]
    fn protocol_ping_frame_matches_official_channel_shape() {
        let frame = lark_protocol_ping_frame(42);

        assert_eq!(frame.seq_id, 0);
        assert_eq!(frame.log_id, 0);
        assert_eq!(frame.service, 42);
        assert_eq!(frame.method, FRAME_METHOD_CONTROL);
        assert_eq!(header_value(&frame, HEADER_TYPE).as_deref(), Some("ping"));
        assert!(frame.payload.is_none());
    }

    fn app_bot_provider() -> ProviderConfig {
        ProviderConfig {
            id: "lark-app".to_string(),
            detail: ProviderConfigDetail::FeishuLark(FeishuLarkProviderConfig::AppBot(
                FeishuLarkAppBotProviderConfig {
                    domain: FeishuLarkAppDomain::Lark,
                    app_id: "cli_test_app".to_string(),
                    app_secret: SecretSource::Inline("test-secret".to_string()),
                    app_registration_source: None,
                    tenant_key: Some("2ca1d211f64f6438".to_string()),
                    chat_id: Some("oc_5ce6d572455d361153b7xx51da133945".to_string()),
                },
            )),
        }
    }

    fn app_bot_runtime() -> FeishuLarkLongConnectionRuntime {
        FeishuLarkLongConnectionRuntime::from_provider_config(&app_bot_provider())
            .expect("App Bot should build hidden long connection runtime")
            .with_bot_identity(test_bot_identity())
    }

    fn test_bot_identity() -> FeishuLarkLongConnectionBotIdentity {
        FeishuLarkLongConnectionBotIdentity {
            open_id: TEST_LARK_BOT_OPEN_ID.to_string(),
            name: "Agents Router".to_string(),
        }
    }

    fn personal_agent_provider() -> ProviderConfig {
        ProviderConfig {
            id: "lark-app".to_string(),
            detail: ProviderConfigDetail::FeishuLark(FeishuLarkProviderConfig::AppBot(
                FeishuLarkAppBotProviderConfig {
                    domain: FeishuLarkAppDomain::Lark,
                    app_id: "cli_test_app".to_string(),
                    app_secret: SecretSource::Inline("test-secret".to_string()),
                    app_registration_source: Some(
                        lark_personal_agent_channel::REGISTRATION_SOURCE.to_string(),
                    ),
                    tenant_key: None,
                    chat_id: None,
                },
            )),
        }
    }

    fn lark_response_surface_config(
        response_surface_enabled: bool,
        app_bot: bool,
    ) -> ValidatedConfig {
        let mut provider = RawProviderConfig::new("lark-app", ProviderType::FeishuLark);
        if app_bot {
            provider.mode = Some("app_bot".to_string());
            provider.domain = Some("lark".to_string());
            provider.app_id = Some("cli_test_app".to_string());
            provider.app_secret = Some("test-secret".to_string());
            provider.tenant_key = Some("2ca1d211f64f6438".to_string());
            provider.chat_id = Some("oc_5ce6d572455d361153b7xx51da133945".to_string());
        } else {
            provider.url =
                Some("https://open.larksuite.com/open-apis/bot/v2/hook/test".to_string());
        }

        let mut route = RouteConfig::new(
            vec!["codex_desktop".to_string()],
            vec!["lark-app".to_string()],
        );
        route.response_surface.enabled = response_surface_enabled;

        RawConfig {
            schema_version: crate::config::CONFIG_SCHEMA_VERSION,
            cli: CliConfig::default(),
            log: LogConfig::default(),
            notification: NotificationConfig::default(),
            sources: vec![SourceConfig {
                id: "codex_desktop".to_string(),
                source_type: SourceType::CodexDesktop,
            }],
            providers: vec![provider],
            routes: vec![route],
        }
        .validate()
        .expect("test config should validate")
    }

    fn personal_agent_response_surface_config() -> ValidatedConfig {
        let mut provider = RawProviderConfig::new("lark-app", ProviderType::FeishuLark);
        provider.mode = Some("app_bot".to_string());
        provider.domain = Some("lark".to_string());
        provider.app_id = Some("cli_test_app".to_string());
        provider.app_secret = Some("test-secret".to_string());

        let mut route = RouteConfig::new(
            vec!["codex_desktop".to_string()],
            vec!["lark-app".to_string()],
        );
        route.response_surface.enabled = true;

        RawConfig {
            schema_version: crate::config::CONFIG_SCHEMA_VERSION,
            cli: CliConfig::default(),
            log: LogConfig::default(),
            notification: NotificationConfig::default(),
            sources: vec![SourceConfig {
                id: "codex_desktop".to_string(),
                source_type: SourceType::CodexDesktop,
            }],
            providers: vec![provider],
            routes: vec![route],
        }
        .validate()
        .expect("Personal Agent config should validate without a fixed room")
    }

    fn ledger_with_lark_surface(now: DateTime<Utc>) -> ResponseSurfaceLedger {
        let mut ledger = ResponseSurfaceLedger::in_memory();
        ledger
            .create_surface_at(lark_surface(now), now)
            .expect("surface should be created");
        ledger
    }

    fn lark_surface(_now: DateTime<Utc>) -> NewResponseSurface {
        NewResponseSurface {
            signal_id: "signal-1".to_string(),
            delivery_id: "delivery-1".to_string(),
            source_id: "codex_desktop".to_string(),
            source_type: "codex_desktop".to_string(),
            source_session_id: "session-1".to_string(),
            source_turn_id: Some("turn-1".to_string()),
            provider_id: "lark-app".to_string(),
            provider_type: ProviderType::FeishuLark.as_str().to_string(),
            provider_mode: ProviderMode::FeishuLarkAppBot,
            provider_account_id: "2ca1d211f64f6438".to_string(),
            provider_conversation_id: "oc_5ce6d572455d361153b7xx51da133945".to_string(),
            provider_message_id: "om_root_message_id".to_string(),
            provider_thread_id: "om_root_message_id".to_string(),
            route_binding_hash: None,
        }
    }

    fn event_at(received_at: DateTime<Utc>) -> FeishuLarkLongConnectionEvent<'static> {
        FeishuLarkLongConnectionEvent {
            raw_event: include_bytes!(
                "../../tests/fixtures/provider_inbound/feishu_lark_surface_reply.json"
            ),
            received_at,
        }
    }

    fn lark_reply_payload(message_id: &str, reply_text: &str) -> Vec<u8> {
        let mut payload: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../tests/fixtures/provider_inbound/feishu_lark_surface_reply.json"
        ))
        .expect("fixture should be valid JSON");
        payload["event"]["message"]["message_id"] = serde_json::json!(message_id);
        payload["event"]["message"]["content"] =
            serde_json::json!(serde_json::json!({ "text": reply_text }).to_string());
        serde_json::to_vec(&payload).expect("payload should serialize")
    }

    fn lark_root_payload(message_id: &str, text: &str) -> Vec<u8> {
        let mut payload: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../tests/fixtures/provider_inbound/feishu_lark_surface_reply.json"
        ))
        .expect("fixture should be valid JSON");
        payload["event"]["message"]["message_id"] = serde_json::json!(message_id);
        payload["event"]["message"]["root_id"] = serde_json::json!("");
        payload["event"]["message"]["content"] =
            serde_json::json!(serde_json::json!({ "text": text }).to_string());
        serde_json::to_vec(&payload).expect("payload should serialize")
    }

    fn test_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 20, 1, 2, 3)
            .single()
            .expect("test time should be valid")
    }

    #[derive(Default)]
    struct RecordingContinuationDispatcher {
        dispatched: Arc<Mutex<Vec<ClaimedContinuationWork>>>,
    }

    impl RecordingContinuationDispatcher {
        fn dispatched(&self) -> Vec<ClaimedContinuationWork> {
            self.dispatched
                .lock()
                .expect("dispatcher record lock should not be poisoned")
                .clone()
        }
    }

    impl ContinuationDispatcher for RecordingContinuationDispatcher {
        fn dispatch(&self, work: ClaimedContinuationWork) -> anyhow::Result<()> {
            self.dispatched
                .lock()
                .expect("dispatcher record lock should not be poisoned")
                .push(work);
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingControlReplyDispatcher {
        dispatched: Arc<Mutex<Vec<BridgeControlReply>>>,
    }

    impl RecordingControlReplyDispatcher {
        fn dispatched(&self) -> Vec<BridgeControlReply> {
            self.dispatched
                .lock()
                .expect("control dispatcher record lock should not be poisoned")
                .clone()
        }
    }

    impl FeishuLarkControlReplyDispatcher for RecordingControlReplyDispatcher {
        fn dispatch(&self, reply: BridgeControlReply) -> anyhow::Result<()> {
            self.dispatched
                .lock()
                .expect("control dispatcher record lock should not be poisoned")
                .push(reply);
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingRoomMessageDispatcher {
        dispatched: Arc<Mutex<Vec<BridgeRoomMessage>>>,
    }

    impl RecordingRoomMessageDispatcher {
        fn dispatched(&self) -> Vec<BridgeRoomMessage> {
            self.dispatched
                .lock()
                .expect("room message dispatcher record lock should not be poisoned")
                .clone()
        }
    }

    impl FeishuLarkRoomMessageDispatcher for RecordingRoomMessageDispatcher {
        fn dispatch(&self, message: BridgeRoomMessage) -> anyhow::Result<()> {
            self.dispatched
                .lock()
                .expect("room message dispatcher record lock should not be poisoned")
                .push(message);
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingNewSessionDispatcher {
        dispatched: Arc<Mutex<Vec<NewSessionWork>>>,
    }

    impl RecordingNewSessionDispatcher {
        fn dispatched(&self) -> Vec<NewSessionWork> {
            self.dispatched
                .lock()
                .expect("new session dispatcher record lock should not be poisoned")
                .clone()
        }
    }

    impl NewSessionDispatcher for RecordingNewSessionDispatcher {
        fn dispatch(&self, work: NewSessionWork) -> anyhow::Result<()> {
            self.dispatched
                .lock()
                .expect("new session dispatcher record lock should not be poisoned")
                .push(work);
            Ok(())
        }
    }

    fn test_dispatch_context<'a>(
        response_surface_ledger_store: &'a ResponseSurfaceLedgerStore,
        bridge_binding_ledger_store: &'a BridgeBindingLedgerStore,
        dispatcher: &'a RecordingContinuationDispatcher,
        control_reply_dispatcher: &'a RecordingControlReplyDispatcher,
        room_message_dispatcher: &'a RecordingRoomMessageDispatcher,
        new_session_dispatcher: &'a RecordingNewSessionDispatcher,
    ) -> FeishuLarkLiveDispatchContext<'a> {
        FeishuLarkLiveDispatchContext {
            response_surface_ledger_store,
            bridge_binding_ledger_store,
            dispatcher,
            control_reply_dispatcher,
            room_message_dispatcher,
            new_session_dispatcher,
        }
    }

    struct RecordingTransport {
        received: VecDeque<FeishuLarkLongConnectionTransportMessage>,
        sent: Vec<Vec<u8>>,
    }

    impl RecordingTransport {
        fn with_messages(messages: Vec<FeishuLarkLongConnectionTransportMessage>) -> Self {
            Self {
                received: messages.into(),
                sent: Vec::new(),
            }
        }

        fn sent_messages(&self) -> Vec<Vec<u8>> {
            self.sent.clone()
        }
    }

    impl FeishuLarkLongConnectionTransport for RecordingTransport {
        fn receive<'a>(
            &'a mut self,
        ) -> FeishuLarkTransportFuture<'a, FeishuLarkLongConnectionTransportMessage> {
            Box::pin(async move {
                Ok(self
                    .received
                    .pop_front()
                    .unwrap_or(FeishuLarkLongConnectionTransportMessage::Closed))
            })
        }

        fn send_binary<'a>(&'a mut self, data: Vec<u8>) -> FeishuLarkTransportFuture<'a, ()> {
            Box::pin(async move {
                self.sent.push(data);
                Ok(())
            })
        }
    }

    fn event_frame(payload: Vec<u8>) -> Vec<u8> {
        base_event_frame(
            payload,
            vec![header(HEADER_SUM, "1"), header(HEADER_SEQ, "0")],
        )
        .encode_to_vec()
    }

    fn fragment_frame(
        message_id: &str,
        sum: usize,
        seq: usize,
        payload: Vec<u8>,
    ) -> FeishuLarkLongConnectionFrame {
        base_event_frame(
            payload,
            vec![
                header(HEADER_MESSAGE_ID, message_id),
                header(HEADER_SUM, &sum.to_string()),
                header(HEADER_SEQ, &seq.to_string()),
            ],
        )
    }

    fn base_event_frame(
        payload: Vec<u8>,
        extra_headers: Vec<FeishuLarkLongConnectionFrameHeader>,
    ) -> FeishuLarkLongConnectionFrame {
        let mut headers = vec![header(HEADER_TYPE, MESSAGE_TYPE_EVENT)];
        headers.extend(extra_headers);
        FeishuLarkLongConnectionFrame {
            seq_id: 1,
            log_id: 2,
            service: 3,
            method: FRAME_METHOD_DATA,
            headers,
            payload_encoding: None,
            payload_type: None,
            payload: Some(payload),
            log_id_new: None,
        }
    }

    fn header(key: &str, value: &str) -> FeishuLarkLongConnectionFrameHeader {
        FeishuLarkLongConnectionFrameHeader {
            key: key.to_string(),
            value: value.to_string(),
        }
    }
}
