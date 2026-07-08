//! Relay-operator community provisioning HTTP handler support.
//!
//! ## Authorization: operator, not owner
//!
//! Every other admin surface in this relay is community-scoped — the sender's
//! role is looked up in `relay_members (community_id, pubkey)` for the
//! host-resolved tenant. Community *creation* cannot work that way: its effect
//! is the creation of tenancy itself, so the authorizing identity must sit
//! above tenants. The gate here is the deployment-level
//! `RELAY_OPERATOR_PUBKEYS` allowlist (see `Config::relay_operator_pubkeys`).
//! An empty allowlist (the default) disables provisioning entirely.
//!
//! The public surface is `POST /operator/communities`, authenticated by NIP-98
//! and gated by the deployment-level `RELAY_OPERATOR_PUBKEYS` allowlist. The
//! endpoint is intentionally outside the Nostr event ingest data plane: no
//! relay-membership bypass, no special event kind, no storage or fan-out.
//!
//! ## Request shape
//!
//! ```json
//! { "host": "acme.communities.buzz.xyz", "initial_owner_pubkey": "<hex>" }
//! ```
//!
//! `initial_owner_pubkey` is optional. When present for an existing community,
//! it rotates that community owner through the same bootstrap path used by
//! `RELAY_OWNER_PUBKEY`; relay operators are deployment-root authorities.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracing::info;

use buzz_core::tenant::normalize_host;

use crate::state::AppState;

/// Maximum accepted host length. Hostnames cap at 253 octets; leave
/// headroom for a `:port` suffix.
const MAX_HOST_LEN: usize = 260;

/// JSON body for `POST /operator/communities`.
#[derive(Debug, Deserialize)]
pub struct ProvisionCommunityRequest {
    /// Normalized authority for the community to ensure/create.
    pub host: String,
    /// Optional initial owner pubkey. When set on an existing community this
    /// rotates the owner through the same bootstrap path used at startup.
    #[serde(default)]
    pub initial_owner_pubkey: Option<String>,
}

/// JSON response from `POST /operator/communities`.
#[derive(Debug, Serialize)]
pub struct ProvisionCommunityResponse {
    /// UUID of the ensured/created community.
    pub community_id: String,
    /// Canonical host stored on the community row.
    pub host: String,
    /// `created` when the host row was inserted, `existed` when it was already
    /// present and the request converged idempotently.
    pub status: &'static str,
    /// Echoes the validated owner pubkey when an owner bootstrap/rotation ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_pubkey: Option<String>,
}

pub(crate) fn validate_pubkey_hex(value: &str) -> Option<String> {
    let normalized = value.to_ascii_lowercase();
    (normalized.len() == 64 && normalized.chars().all(|c| c.is_ascii_hexdigit()))
        .then_some(normalized)
}

/// Validate a normalized host value for a community.
///
/// The host must already be in normalized shape (`normalize_host` is a
/// no-op on it): lowercase, no default port, no trailing dot. Requiring the
/// caller to send the normalized form keeps the stored `communities.host`
/// value byte-identical to what request-time host resolution will look up.
fn validate_host(host: &str) -> Result<(), String> {
    if host.is_empty() {
        return Err("host is empty".to_string());
    }
    if host.len() > MAX_HOST_LEN {
        return Err(format!(
            "host too long: {} bytes (max {MAX_HOST_LEN})",
            host.len()
        ));
    }
    if host.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("host contains invalid characters".to_string());
    }
    if host.contains('/') || host.contains('?') || host.contains('#') || host.contains('@') {
        return Err(
            "host must be a bare authority (no scheme, path, query, or userinfo)".to_string(),
        );
    }
    if normalize_host(host) != host {
        return Err(format!(
            "host is not normalized: expected {:?}",
            normalize_host(host)
        ));
    }
    Ok(())
}

/// Normalize and validate a host supplied to read-only operator endpoints.
///
/// Unlike create, availability checks may accept non-canonical but normalizable
/// authority values (uppercase host, trailing dot, default port) so kgoose can
/// ask the relay for the canonical spelling before creating. Schemes, paths,
/// userinfo, whitespace/control characters, and oversized values are still
/// rejected.
pub(crate) fn normalize_candidate_host(host: &str) -> Result<String, String> {
    if host.is_empty() {
        return Err("host is empty".to_string());
    }
    if host.len() > MAX_HOST_LEN {
        return Err(format!(
            "host too long: {} bytes (max {MAX_HOST_LEN})",
            host.len()
        ));
    }
    if host.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("host contains invalid characters".to_string());
    }
    if host.contains('/') || host.contains('?') || host.contains('#') || host.contains('@') {
        return Err(
            "host must be a bare authority (no scheme, path, query, or userinfo)".to_string(),
        );
    }

    let normalized = normalize_host(host);
    validate_host(&normalized)?;
    Ok(normalized)
}

/// Validate and execute a relay-operator community provisioning request.
///
/// The caller is an HTTP operator endpoint, not the Nostr event ingest path.
/// That keeps the tenant data-plane fences unchanged: no relay-membership
/// bypass, no special event kind, no command routed ahead of moderation/write
/// blocks. The endpoint authenticates its NIP-98 signer first, then passes the
/// signer here for the deployment-level `RELAY_OPERATOR_PUBKEYS` allowlist.
///
/// Idempotency and owner semantics: the request is idempotent on the host row
/// (re-sending it never duplicates a community). When `initial_owner_pubkey` is
/// present, the owner is (re)bootstrapped via [`buzz_db::Db::bootstrap_owner`]
/// even if the community already existed — any previous owner is demoted to
/// admin, exactly like rotating `RELAY_OWNER_PUBKEY` for the deployment
/// community. This makes a retry after a partial failure (row created, owner
/// bootstrap crashed) converge, at the cost that an operator-signed request can
/// rotate an existing community's owner. The operator allowlist is therefore
/// documented as deployment-root authority, not create-only authority.
pub async fn provision_community(
    state: &Arc<AppState>,
    operator_pubkey: &nostr::PublicKey,
    request: ProvisionCommunityRequest,
) -> Result<ProvisionCommunityResponse, String> {
    let operator_hex = operator_pubkey.to_hex();

    // Operator gate. Deliberately NOT a relay_members lookup: provisioning
    // authority spans tenants and lives in deployment config only. Empty
    // allowlist → everyone is rejected (fail closed).
    if !state
        .config
        .relay_operator_pubkeys
        .iter()
        .any(|pk| pk == &operator_hex)
    {
        return Err("actor not authorized: not a relay operator".to_string());
    }

    validate_host(&request.host)?;

    let initial_owner = request
        .initial_owner_pubkey
        .as_deref()
        .map(|value| {
            validate_pubkey_hex(value).ok_or_else(|| {
                "invalid initial_owner_pubkey: expected 64-char hex pubkey".to_string()
            })
        })
        .transpose()?;

    let existed = state
        .db
        .lookup_community_by_host(&request.host)
        .await
        .map_err(|e| format!("database error: {e}"))?
        .is_some();

    // Same idempotent upsert as the startup seed — creating a community is an
    // INSERT, never DDL (docs/multi-tenant-relay.md §System Model).
    let record = state
        .db
        .ensure_configured_community(&request.host)
        .await
        .map_err(|e| format!("failed to create community: {e}"))?;

    if let Some(owner_hex) = &initial_owner {
        state
            .db
            .bootstrap_owner(record.id, owner_hex)
            .await
            .map_err(|e| format!("community provisioned but owner bootstrap failed: {e}"))?;
    }

    info!(
        operator = %operator_hex,
        community = %record.id,
        host = %record.host,
        owner = initial_owner.as_deref().unwrap_or("<none>"),
        existed,
        "community provisioned via operator endpoint"
    );

    Ok(ProvisionCommunityResponse {
        community_id: record.id.to_string(),
        host: record.host,
        status: if existed { "existed" } else { "created" },
        owner_pubkey: initial_owner,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_valid_bare_domain() {
        assert!(validate_host("acme.communities.buzz.xyz").is_ok());
    }

    #[test]
    fn host_valid_with_port() {
        assert!(validate_host("localhost:3000").is_ok());
    }

    #[test]
    fn host_rejects_empty() {
        assert!(validate_host("").is_err());
    }

    #[test]
    fn host_rejects_uppercase() {
        assert!(validate_host("Acme.example").is_err());
    }

    #[test]
    fn host_rejects_default_port() {
        assert!(validate_host("acme.example:443").is_err());
        assert!(validate_host("acme.example:80").is_err());
    }

    #[test]
    fn host_rejects_trailing_dot() {
        assert!(validate_host("acme.example.").is_err());
    }

    #[test]
    fn host_rejects_scheme_path_userinfo() {
        assert!(validate_host("wss://acme.example").is_err());
        assert!(validate_host("acme.example/path").is_err());
        assert!(validate_host("user@acme.example").is_err());
        assert!(validate_host("acme.example?x=1").is_err());
        assert!(validate_host("acme.example#frag").is_err());
    }

    #[test]
    fn host_rejects_whitespace_and_control() {
        assert!(validate_host("acme .example").is_err());
        assert!(validate_host("acme\n.example").is_err());
    }

    #[test]
    fn host_rejects_oversized() {
        let long = format!("{}.example", "a".repeat(260));
        assert!(validate_host(&long).is_err());
    }

    #[test]
    fn host_accepts_ipv6_bracket_literal() {
        assert!(validate_host("[::1]:3000").is_ok());
    }

    #[test]
    fn candidate_host_normalizes_safe_variants() {
        assert_eq!(
            normalize_candidate_host("Acme.Example:443").unwrap(),
            "acme.example"
        );
        assert_eq!(
            normalize_candidate_host("acme.example.").unwrap(),
            "acme.example"
        );
    }

    #[test]
    fn candidate_host_rejects_non_authorities() {
        assert!(normalize_candidate_host("https://acme.example").is_err());
        assert!(normalize_candidate_host("acme.example/path").is_err());
        assert!(normalize_candidate_host("acme .example").is_err());
    }
}
