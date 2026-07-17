use serde::Serialize;

use super::global_config::GlobalAgentConfig;
use super::types::{AgentDefinition, ManagedAgentRecord};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSource {
    Definition,
    Global,
    InstanceLegacy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedField<T> {
    pub value: Option<T>,
    pub source: ConfigSource,
}

#[derive(Debug, Clone)]
pub struct EffectiveAgentConfig {
    pub model: ResolvedField<String>,
    pub provider: ResolvedField<String>,
    pub system_prompt: ResolvedField<String>,
}

#[derive(Debug, Clone)]
pub enum EffectiveConfigResult {
    Resolved(EffectiveAgentConfig),
    OrphanedInstance {
        record_pubkey: String,
        missing_persona_id: String,
    },
}

fn non_blank(v: Option<&str>) -> Option<&str> {
    v.filter(|s| !s.trim().is_empty())
}

fn resolve_linked(
    definition: &AgentDefinition,
    global: &GlobalAgentConfig,
) -> EffectiveAgentConfig {
    let model = match non_blank(definition.model.as_deref()) {
        Some(m) => ResolvedField {
            value: Some(m.to_owned()),
            source: ConfigSource::Definition,
        },
        None => ResolvedField {
            value: global.model.clone(),
            source: ConfigSource::Global,
        },
    };

    let provider = match non_blank(definition.provider.as_deref()) {
        Some(p) => ResolvedField {
            value: Some(p.to_owned()),
            source: ConfigSource::Definition,
        },
        None => ResolvedField {
            value: global.provider.clone(),
            source: ConfigSource::Global,
        },
    };

    let system_prompt = ResolvedField {
        value: non_blank(Some(definition.system_prompt.as_str())).map(str::to_owned),
        source: ConfigSource::Definition,
    };

    EffectiveAgentConfig {
        model,
        provider,
        system_prompt,
    }
}

fn resolve_definition_less(
    record: &ManagedAgentRecord,
    global: &GlobalAgentConfig,
) -> EffectiveAgentConfig {
    let model = match non_blank(record.model.as_deref()) {
        Some(m) => ResolvedField {
            value: Some(m.to_owned()),
            source: ConfigSource::InstanceLegacy,
        },
        None => ResolvedField {
            value: global.model.clone(),
            source: ConfigSource::Global,
        },
    };

    let provider = match non_blank(record.provider.as_deref()) {
        Some(p) => ResolvedField {
            value: Some(p.to_owned()),
            source: ConfigSource::InstanceLegacy,
        },
        None => ResolvedField {
            value: global.provider.clone(),
            source: ConfigSource::Global,
        },
    };

    let system_prompt = ResolvedField {
        value: non_blank(record.system_prompt.as_deref()).map(str::to_owned),
        source: ConfigSource::InstanceLegacy,
    };

    EffectiveAgentConfig {
        model,
        provider,
        system_prompt,
    }
}

pub fn resolve_effective_config(
    record: &ManagedAgentRecord,
    definitions: &[AgentDefinition],
    global: &GlobalAgentConfig,
) -> EffectiveConfigResult {
    match &record.persona_id {
        Some(pid) => match definitions.iter().find(|d| d.id == *pid) {
            Some(def) => EffectiveConfigResult::Resolved(resolve_linked(def, global)),
            None => EffectiveConfigResult::OrphanedInstance {
                record_pubkey: record.pubkey.clone(),
                missing_persona_id: pid.clone(),
            },
        },
        None => EffectiveConfigResult::Resolved(resolve_definition_less(record, global)),
    }
}

pub fn resolve_effective_model_provider_pair(
    record: &ManagedAgentRecord,
    definitions: &[AgentDefinition],
    global: &GlobalAgentConfig,
) -> Option<(Option<String>, Option<String>)> {
    match resolve_effective_config(record, definitions, global) {
        EffectiveConfigResult::Resolved(cfg) => Some((cfg.model.value, cfg.provider.value)),
        EffectiveConfigResult::OrphanedInstance { .. } => None,
    }
}

/// The single user-facing message for a linked instance whose definition no
/// longer exists. Shared by every path that must refuse to act on an orphan:
/// the spawn boundary (`spawn_agent_child`), the interactive start command,
/// and provider deploy.
pub const ORPHANED_INSTANCE_ERROR: &str =
    "This agent's configuration is missing — it may still be \
     syncing or was deleted on another device.";

impl EffectiveConfigResult {
    /// Unwrap into the resolved config, or the shared orphan-refusal error.
    pub fn require_resolved(self) -> Result<EffectiveAgentConfig, String> {
        match self {
            EffectiveConfigResult::Resolved(cfg) => Ok(cfg),
            EffectiveConfigResult::OrphanedInstance { .. } => {
                Err(ORPHANED_INSTANCE_ERROR.to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests;
