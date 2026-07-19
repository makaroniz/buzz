use std::sync::atomic::Ordering;

use tauri::{AppHandle, Emitter, Manager};

use super::{
    agent_readiness, append_log_marker, current_instance_id, find_managed_agent_mut,
    load_global_agent_config, load_managed_agents, load_personas, managed_agent_runtime_log_path,
    process_is_running, record_agent_command, resolve_effective_agent_env, save_managed_agents,
    spawn_agent_child, terminate_process, write_agent_runtime_receipt, AgentReadiness, BackendKind,
    ManagedAgentPairRuntime, ManagedAgentRuntimeKey, ManagedAgentRuntimeLifecycle,
    ManagedAgentRuntimeReceipt, ManagedAgentRuntimeStatus,
};
use crate::app_state::AppState;

const STATUS_EVENT: &str = "managed-agent-runtime-status";

fn status_for(
    app: &AppHandle,
    record: &super::ManagedAgentRecord,
    key: &ManagedAgentRuntimeKey,
    runtime: Option<&ManagedAgentPairRuntime>,
    requested_relay_url: Option<String>,
) -> ManagedAgentRuntimeStatus {
    let personas = load_personas(app).unwrap_or_default();
    let global = load_global_agent_config(app).unwrap_or_default();
    let command = record_agent_command(record, &personas);
    let metadata = super::known_acp_runtime(&command);
    let effective = resolve_effective_agent_env(record, &personas, metadata, &global);
    let local_setup = matches!(agent_readiness(&effective), AgentReadiness::Ready);
    ManagedAgentRuntimeStatus {
        pubkey: key.pubkey.clone(),
        relay_url: key.relay_url.clone(),
        requested_relay_url,
        local_setup,
        lifecycle: runtime
            .map(|runtime| runtime.lifecycle.clone())
            .unwrap_or(ManagedAgentRuntimeLifecycle::Stopped),
        pid: runtime.map(|runtime| runtime.child.id()),
        error: runtime.and_then(|runtime| runtime.error.clone()),
        log_path: managed_agent_runtime_log_path(app, key)
            .ok()
            .map(|path| path.display().to_string()),
    }
}

fn emit_status(app: &AppHandle, status: &ManagedAgentRuntimeStatus) {
    let _ = app.emit(STATUS_EVENT, status);
}

#[tauri::command]
pub fn list_managed_agent_runtimes(app: AppHandle) -> Result<Vec<ManagedAgentRuntimeStatus>, String> {
    let state = app.state::<AppState>();
    let records = load_managed_agents(&app)?;
    let runtimes = state.managed_agent_processes.lock().map_err(|e| e.to_string())?;
    Ok(runtimes
        .iter()
        .filter_map(|(key, runtime)| {
            let record = records.iter().find(|record| record.pubkey == key.pubkey)?;
            Some(status_for(&app, record, key, Some(runtime), None))
        })
        .collect())
}

#[tauri::command]
pub fn start_managed_agent_runtime(
    pubkey: String,
    relay_url: String,
    app: AppHandle,
) -> Result<ManagedAgentRuntimeStatus, String> {
    start_pair(pubkey, relay_url, true, app)
}

fn start_pair(
    pubkey: String,
    relay_url: String,
    lazy: bool,
    app: AppHandle,
) -> Result<ManagedAgentRuntimeStatus, String> {
    let state = app.state::<AppState>();
    let _transition = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|e| e.to_string())?;
    if state.shutdown_started.load(Ordering::Acquire) {
        return Err("desktop shutdown has started".into());
    }
    let _store = state.managed_agents_store_lock.lock().map_err(|e| e.to_string())?;
    let mut records = load_managed_agents(&app)?;
    let record = find_managed_agent_mut(&mut records, &pubkey)?;
    if record.backend != BackendKind::Local {
        return Err("managed runtime pairs require a local agent".into());
    }
    let key = ManagedAgentRuntimeKey::new(pubkey, &relay_url)?;
    let mut runtimes = state.managed_agent_processes.lock().map_err(|e| e.to_string())?;
    if runtimes.get_mut(&key).is_some_and(|runtime| {
        runtime.child.try_wait().ok().flatten().is_none()
    }) {
        let status = status_for(&app, record, &key, runtimes.get(&key), None);
        return Ok(status);
    }
    runtimes.remove(&key);

    let owner = state.keys.lock().ok().map(|keys| keys.public_key().to_hex());
    let mut process = spawn_agent_child(&app, record, &key.relay_url, lazy, owner.as_deref())?;
    let now = crate::util::now_iso();
    let receipt = ManagedAgentRuntimeReceipt {
        key: key.clone(),
        pid: process.child.id(),
        desktop_instance_id: current_instance_id(&app),
        started_at: now.clone(),
    };
    if let Err(error) = write_agent_runtime_receipt(&app, &receipt) {
        let _ = terminate_process(process.child.id());
        let _ = process.child.wait();
        return Err(error);
    }
    record.runtime_pid = None;
    record.updated_at = now.clone();
    record.last_started_at = Some(now);
    record.last_stopped_at = None;
    record.last_error = None;
    runtimes.insert(key.clone(), ManagedAgentPairRuntime::starting(process));
    let status = status_for(&app, record, &key, runtimes.get(&key), None);
    drop(runtimes);
    save_managed_agents(&app, &records)?;
    emit_status(&app, &status);
    Ok(status)
}

#[tauri::command]
pub fn stop_managed_agent_runtime(
    pubkey: String,
    relay_url: String,
    app: AppHandle,
) -> Result<ManagedAgentRuntimeStatus, String> {
    let state = app.state::<AppState>();
    let _transition = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|e| e.to_string())?;
    let _store = state.managed_agents_store_lock.lock().map_err(|e| e.to_string())?;
    let mut records = load_managed_agents(&app)?;
    let record = find_managed_agent_mut(&mut records, &pubkey)?;
    let key = ManagedAgentRuntimeKey::new(pubkey, &relay_url)?;
    let mut runtimes = state.managed_agent_processes.lock().map_err(|e| e.to_string())?;
    if let Some(mut runtime) = runtimes.remove(&key) {
        if process_is_running(runtime.child.id()) {
            terminate_process(runtime.child.id())?;
        }
        let status = runtime.child.wait().map_err(|e| e.to_string())?;
        record.last_exit_code = status.code();
        let _ = append_log_marker(&runtime.log_path, "=== stopped pair runtime ===");
    }
    super::remove_agent_runtime_receipt(&app, &key);
    state.clear_agent_session_cache(&key);
    record.runtime_pid = None;
    record.updated_at = crate::util::now_iso();
    record.last_stopped_at = Some(record.updated_at.clone());
    let status = status_for(&app, record, &key, None, None);
    drop(runtimes);
    save_managed_agents(&app, &records)?;
    emit_status(&app, &status);
    Ok(status)
}

#[tauri::command]
pub fn restart_managed_agent_runtime(
    pubkey: String,
    relay_url: String,
    app: AppHandle,
) -> Result<ManagedAgentRuntimeStatus, String> {
    stop_managed_agent_runtime(pubkey.clone(), relay_url.clone(), app.clone())?;
    start_pair(pubkey, relay_url, true, app)
}

#[tauri::command]
pub async fn reconcile_managed_agent_runtimes(
    communities: Vec<super::ManagedAgentCommunityTarget>,
    app: AppHandle,
) -> Result<Vec<ManagedAgentRuntimeStatus>, String> {
    // Discovery is Rust-owned. Until membership has been authenticated, never
    // infer an empty membership set as success: return explicit failed rows.
    let records = load_managed_agents(&app)?;
    let mut rows = Vec::new();
    for community in communities {
        for record in records.iter().filter(|record| record.backend == BackendKind::Local) {
            let key = ManagedAgentRuntimeKey::new(record.pubkey.clone(), &community.relay_url)?;
            let mut row = status_for(
                &app,
                record,
                &key,
                None,
                Some(community.relay_url.clone()),
            );
            row.lifecycle = ManagedAgentRuntimeLifecycle::Failed;
            row.error = Some("authenticated membership discovery is not yet available".into());
            rows.push(row);
        }
    }
    Ok(rows)
}
