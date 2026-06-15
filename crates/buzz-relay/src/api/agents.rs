//! Agent ownership lookup — GET /api/agents/:pubkey/ownership (NIP-98 auth).
//!
//! Returns the relay-authoritative `agent_owner_pubkey` mapping and whether
//! the authenticated caller is the registered owner. Used by the desktop to
//! gate observer activity visibility without relying on channel membership or
//! local managed-agent store state.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde::Serialize;

use crate::state::AppState;

use super::bridge::{canonical_url, check_nip98_replay, verify_bridge_auth};
use super::{api_error, internal_error};

#[derive(Debug, Serialize)]
pub struct AgentOwnershipResponse {
    pub agent_pubkey: String,
    pub owner_pubkey: Option<String>,
    pub is_owner: bool,
}

/// Resolve whether the authenticated user owns `agent_pubkey` per relay DB.
pub async fn get_agent_ownership(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(agent_pubkey): Path<String>,
) -> Result<Json<AgentOwnershipResponse>, (StatusCode, Json<serde_json::Value>)> {
    let agent_hex = agent_pubkey.trim().to_ascii_lowercase();
    if agent_hex.len() != 64 || !agent_hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(api_error(StatusCode::BAD_REQUEST, "invalid agent pubkey"));
    }

    let agent_bytes = hex::decode(&agent_hex)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid agent pubkey hex"))?;

    let path = format!("/api/agents/{agent_hex}/ownership");
    let url = canonical_url(&state.config.relay_url, &path);
    let (actor_pubkey, event_id_bytes) =
        verify_bridge_auth(&headers, "GET", &url, None, state.config.require_auth_token)?;
    check_nip98_replay(&state, event_id_bytes)?;

    let actor_bytes = actor_pubkey.to_bytes().to_vec();
    let auth_tag = headers.get("x-auth-tag").and_then(|v| v.to_str().ok());
    super::relay_members::enforce_relay_membership(&state, &actor_bytes, auth_tag).await?;

    let owner_pubkey = state
        .db
        .get_agent_channel_policy(&agent_bytes)
        .await
        .map_err(|e| internal_error(&format!("ownership lookup failed: {e}")))?
        .and_then(|(_policy, owner)| owner);

    let is_owner = state
        .db
        .is_agent_owner(&agent_bytes, &actor_bytes)
        .await
        .map_err(|e| internal_error(&format!("ownership check failed: {e}")))?;

    Ok(Json(AgentOwnershipResponse {
        agent_pubkey: agent_hex,
        owner_pubkey: owner_pubkey.map(hex::encode),
        is_owner,
    }))
}
