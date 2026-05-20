use crate::agent_integration_catalog::{
    AgentIntegrationDescriptor, AgentIntegrationId, DurationSupport, PlatformSupport,
    SourceCapability, agent_integration_descriptor, agent_integration_for_source,
    all_agent_integration_descriptors, default_agent_integration_for_platform,
    setup_agent_integration_descriptors_for_platform,
};
use crate::config::{SourceConfig, SourceType};

pub use crate::agent_integration_catalog::{
    HookCommandTemplate, LocalIntegrationKind, LocalizedSetupText, RuntimePlatform,
    SetupIntegrationKind, SourceIngestFormat, claude_code_hook_command,
    codex_cli_stop_hook_command,
};

pub type SourceIntegrationId = AgentIntegrationId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceIntegrationDescriptor {
    pub id: SourceIntegrationId,
    pub display_name: &'static str,
    pub source_id: &'static str,
    pub source_type: SourceType,
    pub setup_order: u16,
    pub platform_support: PlatformSupport,
    pub duration_support: DurationSupport,
    pub ingest_format: Option<SourceIngestFormat>,
    pub hook_command: Option<HookCommandTemplate>,
    pub local_integration: LocalIntegrationKind,
    pub setup_integration: SetupIntegrationKind,
}

impl SourceIntegrationDescriptor {
    fn from_agent_descriptor(descriptor: &AgentIntegrationDescriptor) -> Self {
        let source = descriptor.source_capability;
        Self::from_source_capability(descriptor.id, source)
    }

    fn from_source_capability(id: SourceIntegrationId, source: SourceCapability) -> Self {
        Self {
            id,
            display_name: source.display_name,
            source_id: source.canonical_source_id,
            source_type: source.source_type,
            setup_order: source.setup_order,
            platform_support: source.platform_support,
            duration_support: source.duration_support,
            ingest_format: source.ingest_format,
            hook_command: source.hook_command,
            local_integration: source.local_integration,
            setup_integration: source.setup_integration,
        }
    }

    pub fn source_config(self) -> SourceConfig {
        SourceConfig {
            id: self.source_id.to_string(),
            source_type: self.source_type,
        }
    }
}

pub fn all_source_integration_descriptors() -> Vec<SourceIntegrationDescriptor> {
    all_agent_integration_descriptors()
        .iter()
        .map(SourceIntegrationDescriptor::from_agent_descriptor)
        .collect()
}

pub fn source_integration_descriptor(id: SourceIntegrationId) -> SourceIntegrationDescriptor {
    SourceIntegrationDescriptor::from_agent_descriptor(agent_integration_descriptor(id))
}

pub fn source_integration_for_source(
    source_id: &str,
    source_type: SourceType,
) -> Option<SourceIntegrationDescriptor> {
    agent_integration_for_source(source_id, source_type)
        .map(SourceIntegrationDescriptor::from_agent_descriptor)
}

pub fn setup_source_integration_descriptors_for_platform(
    platform: RuntimePlatform,
) -> impl Iterator<Item = SourceIntegrationDescriptor> {
    setup_agent_integration_descriptors_for_platform(platform)
        .map(SourceIntegrationDescriptor::from_agent_descriptor)
}

pub fn default_source_integration_for_platform(
    platform: RuntimePlatform,
) -> SourceIntegrationDescriptor {
    SourceIntegrationDescriptor::from_agent_descriptor(default_agent_integration_for_platform(
        platform,
    ))
}
