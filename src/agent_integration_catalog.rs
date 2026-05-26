use std::fmt;
use std::str::FromStr;

use crate::config::{SourceConfig, SourceType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentIntegrationId {
    CodexDesktop,
    CodexCli,
    ClaudeCode,
    CursorCli,
    OpenCodeCli,
    OpenClaw,
    HermesAgentCli,
    GithubCopilotCli,
    GeminiCli,
    Aider,
}

impl AgentIntegrationId {
    pub fn descriptor(self) -> &'static AgentIntegrationDescriptor {
        agent_integration_descriptor(self)
    }

    pub fn display_name(self) -> &'static str {
        self.descriptor().source_capability.display_name
    }

    pub fn source_id(self) -> &'static str {
        self.descriptor().source_capability.canonical_source_id
    }

    pub fn supports_duration_filter(self) -> bool {
        self.descriptor()
            .source_capability
            .duration_support
            .is_supported()
    }

    pub fn source_config(self) -> SourceConfig {
        self.descriptor().source_config()
    }

    pub fn from_hook_source_id(source_id: &str) -> Option<Self> {
        agent_integration_for_source(source_id, SourceType::AgentHook)
            .map(|descriptor| descriptor.id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentIntegrationDescriptor {
    pub id: AgentIntegrationId,
    pub source_capability: SourceCapability,
    pub continuation_capability: ContinuationCapability,
}

impl AgentIntegrationDescriptor {
    pub fn source_config(self) -> SourceConfig {
        let source = self.source_capability;
        SourceConfig {
            id: source.canonical_source_id.to_string(),
            source_type: source.source_type,
        }
    }

    pub fn source_capability(self) -> SourceCapability {
        self.source_capability
    }

    pub fn continuation_capability(self) -> ContinuationCapability {
        self.continuation_capability
    }

    pub fn continuation_status(self) -> ContinuationSupportStatus {
        self.continuation_capability.status()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceCapability {
    pub display_name: &'static str,
    pub canonical_source_id: &'static str,
    pub source_type: SourceType,
    pub setup_order: u16,
    pub platform_support: PlatformSupport,
    pub duration_support: DurationSupport,
    pub ingest_format: Option<SourceIngestFormat>,
    pub hook_command: Option<HookCommandTemplate>,
    pub local_integration: LocalIntegrationKind,
    pub setup_integration: SetupIntegrationKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContinuationCapability {
    Unsupported,
    Planned(ContinuationTarget),
    Available(ContinuationAvailability),
}

impl ContinuationCapability {
    pub const fn available_experimental(target: ContinuationTarget) -> Self {
        Self::Available(ContinuationAvailability {
            target,
            release_stage: ContinuationReleaseStage::Experimental,
        })
    }

    pub fn status(self) -> ContinuationSupportStatus {
        match self {
            Self::Unsupported => ContinuationSupportStatus::Unsupported,
            Self::Planned(_) => ContinuationSupportStatus::Planned,
            Self::Available(_) => ContinuationSupportStatus::Available,
        }
    }

    pub fn target(self) -> Option<ContinuationTarget> {
        match self {
            Self::Unsupported => None,
            Self::Planned(target) => Some(target),
            Self::Available(availability) => Some(availability.target),
        }
    }

    pub fn is_available(self) -> bool {
        self.status() == ContinuationSupportStatus::Available
    }

    pub fn release_stage(self) -> Option<ContinuationReleaseStage> {
        match self {
            Self::Available(availability) => Some(availability.release_stage),
            Self::Unsupported | Self::Planned(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContinuationAvailability {
    pub target: ContinuationTarget,
    pub release_stage: ContinuationReleaseStage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContinuationTarget {
    pub controller_kind: AgentControllerKind,
    pub session_binding: ContinuationSessionBinding,
    pub active_turn_control: ContinuationActiveTurnControl,
    pub locality_requirement: ContinuationLocalityRequirement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContinuationSupportStatus {
    Unsupported,
    Planned,
    Available,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContinuationReleaseStage {
    Experimental,
    Stable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentControllerKind {
    CodexAppServer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContinuationSessionBinding {
    SignalConversationSessionId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContinuationActiveTurnControl {
    RespectAgentSessionTurnLifecycle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContinuationLocalityRequirement {
    LocalControllerOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimePlatform {
    Macos,
    Linux,
    Windows,
}

impl RuntimePlatform {
    pub fn current() -> Self {
        #[cfg(target_os = "macos")]
        return Self::Macos;

        #[cfg(target_os = "linux")]
        return Self::Linux;

        #[cfg(windows)]
        return Self::Windows;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformSupport {
    All,
    MacosAndWindows,
}

impl PlatformSupport {
    pub fn supports(self, platform: RuntimePlatform) -> bool {
        match self {
            Self::All => true,
            Self::MacosAndWindows => {
                matches!(platform, RuntimePlatform::Macos | RuntimePlatform::Windows)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationSupport {
    Supported,
    Unsupported,
}

impl DurationSupport {
    pub fn is_supported(self) -> bool {
        self == Self::Supported
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalIntegrationKind {
    None,
    CodexCliStopHook,
    ClaudeCodeHooks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalizedSetupText {
    pub english: &'static str,
    pub simplified_chinese: &'static str,
}

impl LocalizedSetupText {
    pub fn english(self) -> &'static str {
        self.english
    }

    pub fn simplified_chinese(self) -> &'static str {
        self.simplified_chinese
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupIntegrationKind {
    CodexDesktopWatch,
    CodexCliStopHook,
    ClaudeCodeHooks,
    ManualHookCommand {
        integration_point: LocalizedSetupText,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceIngestFormat {
    AgentHookEvent,
    ClaudeCodeHook,
    CodexCliStop,
    GeminiCliHook,
    GithubCopilotCliNotification,
    OpencodeCliSession,
}

impl SourceIngestFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AgentHookEvent => "agent_hook_event",
            Self::ClaudeCodeHook => "claude_code_hook",
            Self::CodexCliStop => "codex_cli_stop",
            Self::GeminiCliHook => "gemini_cli_hook",
            Self::GithubCopilotCliNotification => "github_copilot_cli_notification",
            Self::OpencodeCliSession => "opencode_cli_session",
        }
    }
}

impl fmt::Display for SourceIngestFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for SourceIngestFormat {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "agent_hook_event" => Ok(Self::AgentHookEvent),
            "claude_code_hook" => Ok(Self::ClaudeCodeHook),
            "codex_cli_stop" => Ok(Self::CodexCliStop),
            "gemini_cli_hook" => Ok(Self::GeminiCliHook),
            "github_copilot_cli_notification" => Ok(Self::GithubCopilotCliNotification),
            "opencode_cli_session" => Ok(Self::OpencodeCliSession),
            _ => Err(format!("unsupported ingest format `{value}`")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookCommandTemplate {
    Emit {
        source_id: &'static str,
    },
    Ingest {
        source_id: &'static str,
        format: SourceIngestFormat,
    },
}

impl HookCommandTemplate {
    pub fn command(self) -> String {
        match self {
            Self::Emit { source_id } => {
                format!("agents-router emit --source {source_id}")
            }
            Self::Ingest { source_id, format } => {
                format!(
                    "agents-router ingest --source {source_id} --format {}",
                    format.as_str()
                )
            }
        }
    }

    pub fn requires_emit_fields(self) -> bool {
        matches!(self, Self::Emit { .. })
    }
}

const CODEX_DESKTOP_CONTINUATION_TARGET: ContinuationTarget = ContinuationTarget {
    controller_kind: AgentControllerKind::CodexAppServer,
    session_binding: ContinuationSessionBinding::SignalConversationSessionId,
    active_turn_control: ContinuationActiveTurnControl::RespectAgentSessionTurnLifecycle,
    locality_requirement: ContinuationLocalityRequirement::LocalControllerOnly,
};

const CODEX_DESKTOP_CONTINUATION: ContinuationCapability =
    ContinuationCapability::available_experimental(CODEX_DESKTOP_CONTINUATION_TARGET);

const AGENT_INTEGRATION_DESCRIPTORS: &[AgentIntegrationDescriptor] = &[
    AgentIntegrationDescriptor {
        id: AgentIntegrationId::CodexDesktop,
        source_capability: SourceCapability {
            display_name: "Codex Desktop",
            canonical_source_id: "codex_desktop",
            source_type: SourceType::CodexDesktop,
            setup_order: 10,
            platform_support: PlatformSupport::MacosAndWindows,
            duration_support: DurationSupport::Supported,
            ingest_format: None,
            hook_command: None,
            local_integration: LocalIntegrationKind::None,
            setup_integration: SetupIntegrationKind::CodexDesktopWatch,
        },
        continuation_capability: CODEX_DESKTOP_CONTINUATION,
    },
    AgentIntegrationDescriptor {
        id: AgentIntegrationId::CodexCli,
        source_capability: SourceCapability {
            display_name: "Codex CLI",
            canonical_source_id: "codex_cli",
            source_type: SourceType::CodexCli,
            setup_order: 20,
            platform_support: PlatformSupport::All,
            duration_support: DurationSupport::Unsupported,
            ingest_format: Some(SourceIngestFormat::CodexCliStop),
            hook_command: Some(HookCommandTemplate::Ingest {
                source_id: "codex_cli",
                format: SourceIngestFormat::CodexCliStop,
            }),
            local_integration: LocalIntegrationKind::CodexCliStopHook,
            setup_integration: SetupIntegrationKind::CodexCliStopHook,
        },
        continuation_capability: ContinuationCapability::Unsupported,
    },
    AgentIntegrationDescriptor {
        id: AgentIntegrationId::ClaudeCode,
        source_capability: SourceCapability {
            display_name: "Claude Code",
            canonical_source_id: "claude_code",
            source_type: SourceType::ClaudeCode,
            setup_order: 30,
            platform_support: PlatformSupport::All,
            duration_support: DurationSupport::Supported,
            ingest_format: Some(SourceIngestFormat::ClaudeCodeHook),
            hook_command: Some(HookCommandTemplate::Ingest {
                source_id: "claude_code",
                format: SourceIngestFormat::ClaudeCodeHook,
            }),
            local_integration: LocalIntegrationKind::ClaudeCodeHooks,
            setup_integration: SetupIntegrationKind::ClaudeCodeHooks,
        },
        continuation_capability: ContinuationCapability::Unsupported,
    },
    AgentIntegrationDescriptor {
        id: AgentIntegrationId::CursorCli,
        source_capability: SourceCapability {
            display_name: "Cursor CLI",
            canonical_source_id: "cursor_cli",
            source_type: SourceType::AgentHook,
            setup_order: 40,
            platform_support: PlatformSupport::All,
            duration_support: DurationSupport::Unsupported,
            ingest_format: None,
            hook_command: Some(HookCommandTemplate::Emit {
                source_id: "cursor_cli",
            }),
            local_integration: LocalIntegrationKind::None,
            setup_integration: SetupIntegrationKind::ManualHookCommand {
                integration_point: LocalizedSetupText {
                    english: "wrapper after the CLI exits successfully",
                    simplified_chinese: "wrapper 在 CLI 成功退出后",
                },
            },
        },
        continuation_capability: ContinuationCapability::Unsupported,
    },
    AgentIntegrationDescriptor {
        id: AgentIntegrationId::OpenCodeCli,
        source_capability: SourceCapability {
            display_name: "OpenCode CLI",
            canonical_source_id: "opencode_cli",
            source_type: SourceType::AgentHook,
            setup_order: 50,
            platform_support: PlatformSupport::All,
            duration_support: DurationSupport::Unsupported,
            ingest_format: Some(SourceIngestFormat::OpencodeCliSession),
            hook_command: Some(HookCommandTemplate::Ingest {
                source_id: "opencode_cli",
                format: SourceIngestFormat::OpencodeCliSession,
            }),
            local_integration: LocalIntegrationKind::None,
            setup_integration: SetupIntegrationKind::ManualHookCommand {
                integration_point: LocalizedSetupText {
                    english: "plugin when the session becomes idle",
                    simplified_chinese: "plugin 在 session idle 时",
                },
            },
        },
        continuation_capability: ContinuationCapability::Unsupported,
    },
    AgentIntegrationDescriptor {
        id: AgentIntegrationId::OpenClaw,
        source_capability: SourceCapability {
            display_name: "OpenClaw",
            canonical_source_id: "openclaw",
            source_type: SourceType::AgentHook,
            setup_order: 60,
            platform_support: PlatformSupport::All,
            duration_support: DurationSupport::Unsupported,
            ingest_format: None,
            hook_command: Some(HookCommandTemplate::Emit {
                source_id: "openclaw",
            }),
            local_integration: LocalIntegrationKind::None,
            setup_integration: SetupIntegrationKind::ManualHookCommand {
                integration_point: LocalizedSetupText {
                    english: "plugin hook from agent_end",
                    simplified_chinese: "plugin hook 在 agent_end",
                },
            },
        },
        continuation_capability: ContinuationCapability::Unsupported,
    },
    AgentIntegrationDescriptor {
        id: AgentIntegrationId::HermesAgentCli,
        source_capability: SourceCapability {
            display_name: "Hermes Agent CLI",
            canonical_source_id: "hermes_agent_cli",
            source_type: SourceType::AgentHook,
            setup_order: 70,
            platform_support: PlatformSupport::All,
            duration_support: DurationSupport::Unsupported,
            ingest_format: None,
            hook_command: Some(HookCommandTemplate::Emit {
                source_id: "hermes_agent_cli",
            }),
            local_integration: LocalIntegrationKind::None,
            setup_integration: SetupIntegrationKind::ManualHookCommand {
                integration_point: LocalizedSetupText {
                    english: "plugin hook from post_llm_call",
                    simplified_chinese: "plugin hook 在 post_llm_call",
                },
            },
        },
        continuation_capability: ContinuationCapability::Unsupported,
    },
    AgentIntegrationDescriptor {
        id: AgentIntegrationId::GithubCopilotCli,
        source_capability: SourceCapability {
            display_name: "GitHub Copilot CLI",
            canonical_source_id: "github_copilot_cli",
            source_type: SourceType::AgentHook,
            setup_order: 80,
            platform_support: PlatformSupport::All,
            duration_support: DurationSupport::Unsupported,
            ingest_format: Some(SourceIngestFormat::GithubCopilotCliNotification),
            hook_command: Some(HookCommandTemplate::Ingest {
                source_id: "github_copilot_cli",
                format: SourceIngestFormat::GithubCopilotCliNotification,
            }),
            local_integration: LocalIntegrationKind::None,
            setup_integration: SetupIntegrationKind::ManualHookCommand {
                integration_point: LocalizedSetupText {
                    english: "notification hook",
                    simplified_chinese: "notification hook",
                },
            },
        },
        continuation_capability: ContinuationCapability::Unsupported,
    },
    AgentIntegrationDescriptor {
        id: AgentIntegrationId::GeminiCli,
        source_capability: SourceCapability {
            display_name: "Gemini CLI",
            canonical_source_id: "gemini_cli",
            source_type: SourceType::AgentHook,
            setup_order: 90,
            platform_support: PlatformSupport::All,
            duration_support: DurationSupport::Unsupported,
            ingest_format: Some(SourceIngestFormat::GeminiCliHook),
            hook_command: Some(HookCommandTemplate::Ingest {
                source_id: "gemini_cli",
                format: SourceIngestFormat::GeminiCliHook,
            }),
            local_integration: LocalIntegrationKind::None,
            setup_integration: SetupIntegrationKind::ManualHookCommand {
                integration_point: LocalizedSetupText {
                    english: "AfterAgent or Notification hook",
                    simplified_chinese: "AfterAgent 或 Notification hook",
                },
            },
        },
        continuation_capability: ContinuationCapability::Unsupported,
    },
    AgentIntegrationDescriptor {
        id: AgentIntegrationId::Aider,
        source_capability: SourceCapability {
            display_name: "Aider",
            canonical_source_id: "aider",
            source_type: SourceType::AgentHook,
            setup_order: 100,
            platform_support: PlatformSupport::All,
            duration_support: DurationSupport::Unsupported,
            ingest_format: None,
            hook_command: Some(HookCommandTemplate::Emit { source_id: "aider" }),
            local_integration: LocalIntegrationKind::None,
            setup_integration: SetupIntegrationKind::ManualHookCommand {
                integration_point: LocalizedSetupText {
                    english: "notifications-command",
                    simplified_chinese: "notifications-command",
                },
            },
        },
        continuation_capability: ContinuationCapability::Unsupported,
    },
];

pub fn all_agent_integration_descriptors() -> &'static [AgentIntegrationDescriptor] {
    AGENT_INTEGRATION_DESCRIPTORS
}

pub fn agent_integration_descriptor(id: AgentIntegrationId) -> &'static AgentIntegrationDescriptor {
    AGENT_INTEGRATION_DESCRIPTORS
        .iter()
        .find(|descriptor| descriptor.id == id)
        .expect("every AgentIntegrationId must have a AgentIntegrationDescriptor")
}

pub fn agent_integration_for_source(
    source_id: &str,
    source_type: SourceType,
) -> Option<&'static AgentIntegrationDescriptor> {
    AGENT_INTEGRATION_DESCRIPTORS.iter().find(|descriptor| {
        let source = descriptor.source_capability;
        source.canonical_source_id == source_id && source.source_type == source_type
    })
}

pub fn setup_agent_integration_descriptors_for_platform(
    platform: RuntimePlatform,
) -> impl Iterator<Item = &'static AgentIntegrationDescriptor> {
    let mut descriptors = AGENT_INTEGRATION_DESCRIPTORS
        .iter()
        .filter(move |descriptor| {
            descriptor
                .source_capability
                .platform_support
                .supports(platform)
        })
        .collect::<Vec<_>>();
    descriptors.sort_by_key(|descriptor| descriptor.source_capability.setup_order);
    descriptors.into_iter()
}

pub fn default_agent_integration_for_platform(
    platform: RuntimePlatform,
) -> &'static AgentIntegrationDescriptor {
    if RuntimePlatform::Macos == platform || RuntimePlatform::Windows == platform {
        agent_integration_descriptor(AgentIntegrationId::CodexDesktop)
    } else {
        agent_integration_descriptor(AgentIntegrationId::CodexCli)
    }
}

pub fn codex_cli_stop_hook_command() -> String {
    agent_integration_descriptor(AgentIntegrationId::CodexCli)
        .source_capability
        .hook_command
        .expect("Codex CLI hook command must be cataloged")
        .command()
}

pub fn claude_code_hook_command() -> String {
    agent_integration_descriptor(AgentIntegrationId::ClaudeCode)
        .source_capability
        .hook_command
        .expect("Claude Code hook command must be cataloged")
        .command()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_agent_integration_order_matches_existing_ui_order() {
        let ids = setup_agent_integration_descriptors_for_platform(RuntimePlatform::Macos)
            .map(|descriptor| descriptor.source_capability.canonical_source_id)
            .collect::<Vec<_>>();

        assert_eq!(
            ids,
            vec![
                "codex_desktop",
                "codex_cli",
                "claude_code",
                "cursor_cli",
                "opencode_cli",
                "openclaw",
                "hermes_agent_cli",
                "github_copilot_cli",
                "gemini_cli",
                "aider",
            ]
        );
    }

    #[test]
    fn setup_agent_integration_order_is_driven_by_unique_setup_order() {
        let mut setup_orders = all_agent_integration_descriptors()
            .iter()
            .map(|descriptor| descriptor.source_capability.setup_order)
            .collect::<Vec<_>>();
        setup_orders.sort_unstable();
        setup_orders.dedup();
        assert_eq!(
            setup_orders.len(),
            all_agent_integration_descriptors().len()
        );

        let macos_setup_orders =
            setup_agent_integration_descriptors_for_platform(RuntimePlatform::Macos)
                .map(|descriptor| descriptor.source_capability.setup_order)
                .collect::<Vec<_>>();
        assert!(macos_setup_orders.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn agent_integration_descriptors_are_canonical() {
        let cases = [
            (
                AgentIntegrationId::CodexDesktop,
                "Codex Desktop",
                "codex_desktop",
                SourceType::CodexDesktop,
                PlatformSupport::MacosAndWindows,
                DurationSupport::Supported,
                None,
                LocalIntegrationKind::None,
            ),
            (
                AgentIntegrationId::CodexCli,
                "Codex CLI",
                "codex_cli",
                SourceType::CodexCli,
                PlatformSupport::All,
                DurationSupport::Unsupported,
                Some(SourceIngestFormat::CodexCliStop),
                LocalIntegrationKind::CodexCliStopHook,
            ),
            (
                AgentIntegrationId::ClaudeCode,
                "Claude Code",
                "claude_code",
                SourceType::ClaudeCode,
                PlatformSupport::All,
                DurationSupport::Supported,
                Some(SourceIngestFormat::ClaudeCodeHook),
                LocalIntegrationKind::ClaudeCodeHooks,
            ),
            (
                AgentIntegrationId::CursorCli,
                "Cursor CLI",
                "cursor_cli",
                SourceType::AgentHook,
                PlatformSupport::All,
                DurationSupport::Unsupported,
                None,
                LocalIntegrationKind::None,
            ),
            (
                AgentIntegrationId::OpenCodeCli,
                "OpenCode CLI",
                "opencode_cli",
                SourceType::AgentHook,
                PlatformSupport::All,
                DurationSupport::Unsupported,
                Some(SourceIngestFormat::OpencodeCliSession),
                LocalIntegrationKind::None,
            ),
            (
                AgentIntegrationId::OpenClaw,
                "OpenClaw",
                "openclaw",
                SourceType::AgentHook,
                PlatformSupport::All,
                DurationSupport::Unsupported,
                None,
                LocalIntegrationKind::None,
            ),
            (
                AgentIntegrationId::HermesAgentCli,
                "Hermes Agent CLI",
                "hermes_agent_cli",
                SourceType::AgentHook,
                PlatformSupport::All,
                DurationSupport::Unsupported,
                None,
                LocalIntegrationKind::None,
            ),
            (
                AgentIntegrationId::GithubCopilotCli,
                "GitHub Copilot CLI",
                "github_copilot_cli",
                SourceType::AgentHook,
                PlatformSupport::All,
                DurationSupport::Unsupported,
                Some(SourceIngestFormat::GithubCopilotCliNotification),
                LocalIntegrationKind::None,
            ),
            (
                AgentIntegrationId::GeminiCli,
                "Gemini CLI",
                "gemini_cli",
                SourceType::AgentHook,
                PlatformSupport::All,
                DurationSupport::Unsupported,
                Some(SourceIngestFormat::GeminiCliHook),
                LocalIntegrationKind::None,
            ),
            (
                AgentIntegrationId::Aider,
                "Aider",
                "aider",
                SourceType::AgentHook,
                PlatformSupport::All,
                DurationSupport::Unsupported,
                None,
                LocalIntegrationKind::None,
            ),
        ];

        for (
            id,
            display_name,
            source_id,
            source_type,
            platform_support,
            duration_support,
            ingest_format,
            local_integration,
        ) in cases
        {
            let descriptor = agent_integration_descriptor(id);
            let source = descriptor.source_capability;
            assert_eq!(source.display_name, display_name);
            assert_eq!(source.canonical_source_id, source_id);
            assert_eq!(source.source_type, source_type);
            assert_eq!(source.platform_support, platform_support);
            assert_eq!(source.duration_support, duration_support);
            assert_eq!(source.ingest_format, ingest_format);
            assert_eq!(source.local_integration, local_integration);
        }
    }

    #[test]
    fn setup_integration_facts_are_cataloged() {
        assert_eq!(
            agent_integration_descriptor(AgentIntegrationId::CodexDesktop)
                .source_capability
                .setup_integration,
            SetupIntegrationKind::CodexDesktopWatch
        );
        assert_eq!(
            agent_integration_descriptor(AgentIntegrationId::CodexCli)
                .source_capability
                .setup_integration,
            SetupIntegrationKind::CodexCliStopHook
        );
        assert_eq!(
            agent_integration_descriptor(AgentIntegrationId::ClaudeCode)
                .source_capability
                .setup_integration,
            SetupIntegrationKind::ClaudeCodeHooks
        );

        let gemini = agent_integration_descriptor(AgentIntegrationId::GeminiCli).source_capability;
        assert_eq!(
            gemini.setup_integration,
            SetupIntegrationKind::ManualHookCommand {
                integration_point: LocalizedSetupText {
                    english: "AfterAgent or Notification hook",
                    simplified_chinese: "AfterAgent 或 Notification hook",
                },
            }
        );

        for descriptor in all_agent_integration_descriptors() {
            let source = descriptor.source_capability;
            if matches!(
                source.setup_integration,
                SetupIntegrationKind::ManualHookCommand { .. }
            ) {
                assert!(
                    source.hook_command.is_some(),
                    "{} manual setup integration should expose a hook command",
                    source.canonical_source_id
                );
            }
        }
    }

    #[test]
    fn source_capability_facts_are_cataloged_with_descriptors() {
        for descriptor in all_agent_integration_descriptors() {
            let source = descriptor.source_capability();
            assert_eq!(source, descriptor.source_capability);
        }
    }

    #[test]
    fn codex_desktop_continuation_is_available_experimental() {
        let descriptor = agent_integration_descriptor(AgentIntegrationId::CodexDesktop);
        let target = descriptor
            .continuation_capability
            .target()
            .expect("Codex Desktop available continuation should expose target facts");

        assert_eq!(
            descriptor.continuation_status(),
            ContinuationSupportStatus::Available
        );
        assert!(descriptor.continuation_capability.is_available());
        assert_eq!(
            descriptor.continuation_capability.release_stage(),
            Some(ContinuationReleaseStage::Experimental)
        );
        assert_eq!(target.controller_kind, AgentControllerKind::CodexAppServer);
        assert_eq!(
            target.session_binding,
            ContinuationSessionBinding::SignalConversationSessionId
        );
        assert_eq!(
            target.active_turn_control,
            ContinuationActiveTurnControl::RespectAgentSessionTurnLifecycle
        );
        assert_eq!(
            target.locality_requirement,
            ContinuationLocalityRequirement::LocalControllerOnly
        );
    }

    #[test]
    fn only_codex_desktop_continuation_is_available_before_release() {
        for descriptor in all_agent_integration_descriptors() {
            if descriptor.id == AgentIntegrationId::CodexDesktop {
                assert_eq!(
                    descriptor.continuation_status(),
                    ContinuationSupportStatus::Available
                );
            } else {
                assert_ne!(
                    descriptor.continuation_status(),
                    ContinuationSupportStatus::Available,
                    "{} must not expose continuation until it is explicitly promoted",
                    descriptor.source_capability.canonical_source_id
                );
            }
        }
    }

    #[test]
    fn unsupported_continuation_integrations_have_no_controller_target() {
        for id in [
            AgentIntegrationId::CodexCli,
            AgentIntegrationId::ClaudeCode,
            AgentIntegrationId::CursorCli,
            AgentIntegrationId::OpenCodeCli,
            AgentIntegrationId::OpenClaw,
            AgentIntegrationId::HermesAgentCli,
            AgentIntegrationId::GithubCopilotCli,
            AgentIntegrationId::GeminiCli,
            AgentIntegrationId::Aider,
        ] {
            let descriptor = agent_integration_descriptor(id);
            assert_eq!(
                descriptor.continuation_status(),
                ContinuationSupportStatus::Unsupported
            );
            assert_eq!(descriptor.continuation_capability.target(), None);
        }
    }

    #[test]
    fn linux_setup_source_integrations_exclude_codex_desktop() {
        let ids = setup_agent_integration_descriptors_for_platform(RuntimePlatform::Linux)
            .map(|descriptor| descriptor.source_capability.canonical_source_id)
            .collect::<Vec<_>>();

        assert_eq!(ids.first(), Some(&"codex_cli"));
        assert!(!ids.contains(&"codex_desktop"));
    }

    #[test]
    fn default_source_integration_depends_on_platform() {
        assert_eq!(
            default_agent_integration_for_platform(RuntimePlatform::Macos).id,
            AgentIntegrationId::CodexDesktop
        );
        assert_eq!(
            default_agent_integration_for_platform(RuntimePlatform::Windows).id,
            AgentIntegrationId::CodexDesktop
        );
        assert_eq!(
            default_agent_integration_for_platform(RuntimePlatform::Linux).id,
            AgentIntegrationId::CodexCli
        );
    }

    #[test]
    fn hook_commands_are_generated_from_descriptors() {
        assert_eq!(
            codex_cli_stop_hook_command(),
            "agents-router ingest --source codex_cli --format codex_cli_stop"
        );
        assert_eq!(
            claude_code_hook_command(),
            "agents-router ingest --source claude_code --format claude_code_hook"
        );
        assert_eq!(
            agent_integration_descriptor(AgentIntegrationId::GeminiCli)
                .source_capability
                .hook_command
                .expect("Gemini CLI should expose hook command")
                .command(),
            "agents-router ingest --source gemini_cli --format gemini_cli_hook"
        );
    }

    #[test]
    fn emit_hook_commands_are_explicit_prefixes() {
        let command = agent_integration_descriptor(AgentIntegrationId::Aider)
            .source_capability
            .hook_command
            .expect("Aider should expose hook command");

        assert_eq!(command.command(), "agents-router emit --source aider");
        assert!(command.requires_emit_fields());
    }

    #[test]
    fn agent_integration_lookup_requires_matching_source_type() {
        assert_eq!(
            agent_integration_for_source("codex_cli", SourceType::CodexCli)
                .map(|descriptor| descriptor.id),
            Some(AgentIntegrationId::CodexCli)
        );
        assert_eq!(
            agent_integration_for_source("codex_cli", SourceType::AgentHook),
            None
        );
        assert_eq!(
            agent_integration_for_source("claude_code", SourceType::AgentHook),
            None
        );
        assert_eq!(
            agent_integration_for_source("my_desktop", SourceType::CodexDesktop),
            None
        );
        assert_eq!(
            agent_integration_for_source("codex_desktop", SourceType::CodexDesktop)
                .map(|descriptor| descriptor.id),
            Some(AgentIntegrationId::CodexDesktop)
        );
    }

    #[test]
    fn documented_ingest_hook_commands_match_catalog() {
        let docs = [
            include_str!("../docs/agents/codex-cli.md"),
            include_str!("../docs/agents/codex-cli.zh-CN.md"),
            include_str!("../docs/agents/claude-code.md"),
            include_str!("../docs/agents/claude-code.zh-CN.md"),
            include_str!("../docs/agents/gemini-cli.md"),
            include_str!("../docs/agents/gemini-cli.zh-CN.md"),
            include_str!("../docs/agents/github-copilot-cli.md"),
            include_str!("../docs/agents/github-copilot-cli.zh-CN.md"),
            include_str!("../docs/agents/opencode-cli.md"),
        ]
        .join("\n");

        for id in [
            AgentIntegrationId::CodexCli,
            AgentIntegrationId::ClaudeCode,
            AgentIntegrationId::GeminiCli,
            AgentIntegrationId::GithubCopilotCli,
            AgentIntegrationId::OpenCodeCli,
        ] {
            let descriptor = agent_integration_descriptor(id);
            let command = descriptor
                .source_capability
                .hook_command
                .expect("structured integrations should expose hook command")
                .command();
            assert!(
                docs.contains(&command),
                "docs should include catalog command `{command}`"
            );
        }
    }

    #[test]
    fn ingest_format_strings_remain_compatible() {
        for (raw, expected) in [
            ("agent_hook_event", SourceIngestFormat::AgentHookEvent),
            ("claude_code_hook", SourceIngestFormat::ClaudeCodeHook),
            ("codex_cli_stop", SourceIngestFormat::CodexCliStop),
            ("gemini_cli_hook", SourceIngestFormat::GeminiCliHook),
            (
                "github_copilot_cli_notification",
                SourceIngestFormat::GithubCopilotCliNotification,
            ),
            (
                "opencode_cli_session",
                SourceIngestFormat::OpencodeCliSession,
            ),
        ] {
            assert_eq!(raw.parse::<SourceIngestFormat>(), Ok(expected));
            assert_eq!(expected.as_str(), raw);
        }
    }
}
