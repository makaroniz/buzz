//! Relay-authoritative agent ownership lookup for activity visibility gates.

use reqwest::Method;
use serde::Serialize;
use tauri::State;

use crate::{
    app_state::AppState,
    relay::{get_relay_json, relay_api_base_url_with_override},
};

#[derive(Debug, Serialize, serde::Deserialize)]
pub struct AgentOwnershipStatus {
    /// Lowercase hex pubkey of the queried agent.
    pub agent_pubkey: String,
    /// Lowercase hex owner pubkey from relay `agent_owner_pubkey`, if set.
    pub owner_pubkey: Option<String>,
    /// True iff the current workspace identity is the relay-recorded owner.
    pub is_owner: bool,
}

/// Resolve whether the current identity owns `agent_pubkey` per relay DB.
#[tauri::command]
pub async fn resolve_agent_ownership(
    agent_pubkey: String,
    state: State<'_, AppState>,
) -> Result<AgentOwnershipStatus, String> {
    let agent_hex = agent_pubkey.trim().to_ascii_lowercase();
    if agent_hex.len() != 64 {
        return Err("agent pubkey must be 64 hex characters".to_string());
    }

    let api_base = relay_api_base_url_with_override(&state);
    let path = format!("/api/agents/{agent_hex}/ownership");
    let url = format!("{api_base}{path}");

    get_relay_json::<AgentOwnershipStatus>(&state, Method::GET, &url, &[]).await
}
