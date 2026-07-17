use super::*;
use std::collections::BTreeMap;

fn definition(
    id: &str,
    model: Option<&str>,
    provider: Option<&str>,
    prompt: &str,
) -> AgentDefinition {
    AgentDefinition {
        id: id.to_string(),
        display_name: "Test Definition".to_string(),
        avatar_url: None,
        system_prompt: prompt.to_string(),
        runtime: None,
        model: model.map(str::to_string),
        provider: provider.map(str::to_string),
        name_pool: vec![],
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        env_vars: BTreeMap::new(),
        respond_to: None,
        respond_to_allowlist: vec![],
        parallelism: None,
        created_at: "".to_string(),
        updated_at: "".to_string(),
    }
}

fn record(
    persona_id: Option<&str>,
    model: Option<&str>,
    provider: Option<&str>,
    prompt: Option<&str>,
) -> ManagedAgentRecord {
    use crate::managed_agents::{BackendKind, RespondTo};
    ManagedAgentRecord {
        pubkey: "agent-pk".to_string(),
        name: "Agent".to_string(),
        persona_id: persona_id.map(str::to_string),
        private_key_nsec: "".to_string(),
        auth_tag: None,
        relay_url: "ws://localhost:3000".to_string(),
        avatar_url: None,
        acp_command: "buzz-acp".to_string(),
        agent_command: "goose".to_string(),
        agent_command_override: None,
        agent_args: vec![],
        mcp_command: "".to_string(),
        turn_timeout_seconds: 300,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: prompt.map(str::to_string),
        model: model.map(str::to_string),
        provider: provider.map(str::to_string),
        persona_source_version: None,
        env_vars: BTreeMap::new(),
        start_on_app_launch: false,
        runtime_pid: None,
        backend: BackendKind::Local,
        backend_agent_id: None,
        provider_binary_path: None,
        team_id: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        created_at: "".to_string(),
        updated_at: "".to_string(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
        respond_to: RespondTo::OwnerOnly,
        respond_to_allowlist: vec![],
        display_name: None,
        slug: None,
        runtime: None,
        name_pool: vec![],
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        relay_mesh: None,
        auto_restart_on_config_change: false,
        definition_respond_to: None,
        definition_respond_to_allowlist: vec![],
        definition_parallelism: None,
    }
}

fn global(model: Option<&str>, provider: Option<&str>) -> GlobalAgentConfig {
    GlobalAgentConfig {
        model: model.map(str::to_string),
        provider: provider.map(str::to_string),
        ..Default::default()
    }
}

// ── Linked instance: definition → global, record ignored ──

#[test]
fn linked_definition_model_wins_over_stale_record() {
    let rec = record(
        Some("d1"),
        Some("stale-model"),
        Some("stale-prov"),
        Some("stale prompt"),
    );
    let defs = vec![definition(
        "d1",
        Some("def-model"),
        Some("def-prov"),
        "def prompt",
    )];
    let g = global(Some("global-model"), Some("global-prov"));

    let result = resolve_effective_config(&rec, &defs, &g);
    let cfg = match result {
        EffectiveConfigResult::Resolved(c) => c,
        other => panic!("expected Resolved, got {:?}", other),
    };

    assert_eq!(cfg.model.value.as_deref(), Some("def-model"));
    assert_eq!(cfg.model.source, ConfigSource::Definition);
    assert_eq!(cfg.provider.value.as_deref(), Some("def-prov"));
    assert_eq!(cfg.provider.source, ConfigSource::Definition);
    assert_eq!(cfg.system_prompt.value.as_deref(), Some("def prompt"));
    assert_eq!(cfg.system_prompt.source, ConfigSource::Definition);
}

#[test]
fn linked_inherit_global_when_definition_blank() {
    let rec = record(
        Some("d1"),
        Some("stale-model"),
        Some("stale-prov"),
        Some("stale prompt"),
    );
    let defs = vec![definition("d1", None, None, "")];
    let g = global(Some("global-model"), Some("global-prov"));

    let result = resolve_effective_config(&rec, &defs, &g);
    let cfg = match result {
        EffectiveConfigResult::Resolved(c) => c,
        other => panic!("expected Resolved, got {:?}", other),
    };

    assert_eq!(cfg.model.value.as_deref(), Some("global-model"));
    assert_eq!(cfg.model.source, ConfigSource::Global);
    assert_eq!(cfg.provider.value.as_deref(), Some("global-prov"));
    assert_eq!(cfg.provider.source, ConfigSource::Global);
    assert_eq!(cfg.system_prompt.value, None);
    assert_eq!(cfg.system_prompt.source, ConfigSource::Definition);
}

#[test]
fn linked_stale_record_model_is_inert() {
    let rec = record(Some("d1"), Some("stale-model"), Some("stale-prov"), None);
    let defs = vec![definition("d1", None, None, "")];
    let g = global(None, None);

    let result = resolve_effective_config(&rec, &defs, &g);
    let cfg = match result {
        EffectiveConfigResult::Resolved(c) => c,
        other => panic!("expected Resolved, got {:?}", other),
    };

    assert_eq!(cfg.model.value, None);
    assert_eq!(cfg.model.source, ConfigSource::Global);
    assert_eq!(cfg.provider.value, None);
    assert_eq!(cfg.provider.source, ConfigSource::Global);
}

#[test]
fn linked_definition_model_set_provider_inherits() {
    let rec = record(Some("d1"), None, None, None);
    let defs = vec![definition("d1", Some("def-model"), None, "prompt")];
    let g = global(None, Some("global-prov"));

    let result = resolve_effective_config(&rec, &defs, &g);
    let cfg = match result {
        EffectiveConfigResult::Resolved(c) => c,
        other => panic!("expected Resolved, got {:?}", other),
    };

    assert_eq!(cfg.model.value.as_deref(), Some("def-model"));
    assert_eq!(cfg.model.source, ConfigSource::Definition);
    assert_eq!(cfg.provider.value.as_deref(), Some("global-prov"));
    assert_eq!(cfg.provider.source, ConfigSource::Global);
}

#[test]
fn linked_blank_prompt_means_no_prompt() {
    let rec = record(Some("d1"), None, None, Some("stale prompt on record"));
    let defs = vec![definition("d1", None, None, "")];
    let g = global(None, None);

    let result = resolve_effective_config(&rec, &defs, &g);
    let cfg = match result {
        EffectiveConfigResult::Resolved(c) => c,
        other => panic!("expected Resolved, got {:?}", other),
    };

    assert_eq!(cfg.system_prompt.value, None);
    assert_eq!(cfg.system_prompt.source, ConfigSource::Definition);
}

#[test]
fn linked_whitespace_only_definition_model_inherits_global() {
    let rec = record(Some("d1"), Some("stale"), None, None);
    let defs = vec![definition("d1", Some("  "), Some("  \t"), "")];
    let g = global(Some("global-model"), Some("global-prov"));

    let result = resolve_effective_config(&rec, &defs, &g);
    let cfg = match result {
        EffectiveConfigResult::Resolved(c) => c,
        other => panic!("expected Resolved, got {:?}", other),
    };

    assert_eq!(cfg.model.value.as_deref(), Some("global-model"));
    assert_eq!(cfg.model.source, ConfigSource::Global);
    assert_eq!(cfg.provider.value.as_deref(), Some("global-prov"));
    assert_eq!(cfg.provider.source, ConfigSource::Global);
}

// ── Orphaned instance ──

#[test]
fn orphaned_linked_instance_returns_error() {
    let rec = record(Some("missing-def"), None, None, None);
    let defs = vec![];
    let g = global(Some("global-model"), None);

    let result = resolve_effective_config(&rec, &defs, &g);
    match result {
        EffectiveConfigResult::OrphanedInstance {
            record_pubkey,
            missing_persona_id,
        } => {
            assert_eq!(record_pubkey, "agent-pk");
            assert_eq!(missing_persona_id, "missing-def");
        }
        other => panic!("expected OrphanedInstance, got {:?}", other),
    }
}

// ── Definition-less instance: instance → global ──

#[test]
fn definition_less_uses_own_fields() {
    let rec = record(None, Some("my-model"), Some("my-prov"), Some("my prompt"));
    let defs = vec![];
    let g = global(Some("global-model"), Some("global-prov"));

    let result = resolve_effective_config(&rec, &defs, &g);
    let cfg = match result {
        EffectiveConfigResult::Resolved(c) => c,
        other => panic!("expected Resolved, got {:?}", other),
    };

    assert_eq!(cfg.model.value.as_deref(), Some("my-model"));
    assert_eq!(cfg.model.source, ConfigSource::InstanceLegacy);
    assert_eq!(cfg.provider.value.as_deref(), Some("my-prov"));
    assert_eq!(cfg.provider.source, ConfigSource::InstanceLegacy);
    assert_eq!(cfg.system_prompt.value.as_deref(), Some("my prompt"));
    assert_eq!(cfg.system_prompt.source, ConfigSource::InstanceLegacy);
}

#[test]
fn definition_less_falls_back_to_global() {
    let rec = record(None, None, None, None);
    let defs = vec![];
    let g = global(Some("global-model"), Some("global-prov"));

    let result = resolve_effective_config(&rec, &defs, &g);
    let cfg = match result {
        EffectiveConfigResult::Resolved(c) => c,
        other => panic!("expected Resolved, got {:?}", other),
    };

    assert_eq!(cfg.model.value.as_deref(), Some("global-model"));
    assert_eq!(cfg.model.source, ConfigSource::Global);
    assert_eq!(cfg.provider.value.as_deref(), Some("global-prov"));
    assert_eq!(cfg.provider.source, ConfigSource::Global);
}

#[test]
fn definition_less_blank_record_fields_fall_through() {
    let rec = record(None, Some("  "), Some(""), Some("  "));
    let defs = vec![];
    let g = global(Some("g-model"), None);

    let result = resolve_effective_config(&rec, &defs, &g);
    let cfg = match result {
        EffectiveConfigResult::Resolved(c) => c,
        other => panic!("expected Resolved, got {:?}", other),
    };

    assert_eq!(cfg.model.value.as_deref(), Some("g-model"));
    assert_eq!(cfg.model.source, ConfigSource::Global);
    assert_eq!(cfg.provider.value, None);
    assert_eq!(cfg.provider.source, ConfigSource::Global);
    assert_eq!(cfg.system_prompt.value, None);
}

// ── Convenience helper ──

#[test]
fn model_provider_pair_returns_none_for_orphan() {
    let rec = record(Some("missing"), None, None, None);
    assert_eq!(
        resolve_effective_model_provider_pair(&rec, &[], &global(None, None)),
        None
    );
}

#[test]
fn model_provider_pair_returns_resolved_values() {
    let rec = record(Some("d1"), None, None, None);
    let defs = vec![definition("d1", Some("m"), Some("p"), "")];
    let g = global(None, None);

    let pair = resolve_effective_model_provider_pair(&rec, &defs, &g);
    assert_eq!(pair, Some((Some("m".to_string()), Some("p".to_string()))));
}

// ── require_resolved: the shared orphan-refusal contract used by
// build_deploy_payload and spawn_agent_child ──

#[test]
fn require_resolved_returns_shared_error_for_orphan() {
    let rec = record(Some("missing"), None, None, None);
    let error = resolve_effective_config(&rec, &[], &global(None, None))
        .require_resolved()
        .expect_err("orphan must not resolve");
    assert_eq!(error, ORPHANED_INSTANCE_ERROR);
}

#[test]
fn require_resolved_returns_config_for_resolved() {
    let rec = record(Some("d1"), None, None, None);
    let defs = vec![definition("d1", Some("m"), Some("p"), "prompt")];
    let cfg = resolve_effective_config(&rec, &defs, &global(None, None))
        .require_resolved()
        .expect("linked instance with a live definition must resolve");
    assert_eq!(cfg.model.value.as_deref(), Some("m"));
}

#[test]
fn require_resolved_refuses_orphan_only() {
    let orphan = record(Some("missing"), None, None, None);
    assert_eq!(
        resolve_effective_config(&orphan, &[], &global(None, None))
            .require_resolved()
            .unwrap_err(),
        ORPHANED_INSTANCE_ERROR,
    );

    let linked = record(Some("d1"), None, None, None);
    let defs = vec![definition("d1", Some("m"), None, "")];
    assert!(
        resolve_effective_config(&linked, &defs, &global(None, None))
            .require_resolved()
            .is_ok()
    );

    // Definition-less instances are never orphaned regardless of how bare
    // their own fields are — orphan status only applies to a dangling link.
    let bare = record(None, None, None, None);
    assert!(resolve_effective_config(&bare, &[], &global(None, None))
        .require_resolved()
        .is_ok());
}

// ── Morgan's exact regression sequence ──

#[test]
fn morgans_sequence_inherit_explicit_inherit() {
    let g = global(Some("claude-opus-4-6"), Some("anthropic"));

    // Step 1: fresh agent with inherited model → resolves global
    let rec_step1 = record(Some("d1"), None, None, None);
    let defs = vec![definition("d1", None, None, "agent prompt")];
    let cfg1 = match resolve_effective_config(&rec_step1, &defs, &g) {
        EffectiveConfigResult::Resolved(c) => c,
        other => panic!("step 1: {:?}", other),
    };
    assert_eq!(cfg1.model.value.as_deref(), Some("claude-opus-4-6"));
    assert_eq!(cfg1.model.source, ConfigSource::Global);

    // Step 2: set explicit model on definition
    let defs_explicit = vec![definition(
        "d1",
        Some("goose-gpt-5-6-sol"),
        Some("databricks"),
        "agent prompt",
    )];
    let cfg2 = match resolve_effective_config(&rec_step1, &defs_explicit, &g) {
        EffectiveConfigResult::Resolved(c) => c,
        other => panic!("step 2: {:?}", other),
    };
    assert_eq!(cfg2.model.value.as_deref(), Some("goose-gpt-5-6-sol"));
    assert_eq!(cfg2.model.source, ConfigSource::Definition);

    // Step 3: switch back to inherit — even with stale record bytes
    let rec_stale = record(
        Some("d1"),
        Some("goose-gpt-5-6-sol"),
        Some("databricks"),
        None,
    );
    let defs_inherit = vec![definition("d1", None, None, "agent prompt")];
    let cfg3 = match resolve_effective_config(&rec_stale, &defs_inherit, &g) {
        EffectiveConfigResult::Resolved(c) => c,
        other => panic!("step 3: {:?}", other),
    };
    assert_eq!(cfg3.model.value.as_deref(), Some("claude-opus-4-6"));
    assert_eq!(cfg3.model.source, ConfigSource::Global);
    assert_eq!(cfg3.provider.value.as_deref(), Some("anthropic"));
    assert_eq!(cfg3.provider.source, ConfigSource::Global);
}
