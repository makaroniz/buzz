//! Multi-tenant conformance harness.
//!
//! This file mirrors the obligation table in `docs/multi-tenant-conformance.md`
//! **one row per module**. It is the executable form of the conformance
//! contract: the current single-community relay is the wire-parity *oracle*, and
//! these tests prove two things the rewrite must never break:
//!
//!   1. **N=1 parity** — with one configured host → one default community, every
//!      existing client observes byte-identical behavior. This is asserted by the
//!      *existing* e2e suites (`e2e_relay`, `e2e_rest_api`, `e2e_media`, …) run
//!      with `RELAY_URL` pointed at the new relay; no new test is needed here,
//!      only the documented obligation that those suites stay green unchanged.
//!
//!   2. **A/B isolation** — with two hosts → two communities on the *same* relay
//!      deployment, no tenant-observable state crosses the boundary. These are
//!      the new tests below. They require a running multi-tenant relay with two
//!      host mappings, so they are `#[ignore]` by default and selected with
//!      `--ignored`.
//!
//! # Running the A/B isolation suite
//!
//! ```text
//! RELAY_URL_A=http://a.localhost:3000 \
//! RELAY_URL_B=http://b.localhost:3000 \
//! cargo test -p buzz-test-client --test conformance_multitenant -- --ignored
//! ```
//!
//! Both URLs MUST address the same relay process (same pod, same Postgres, same
//! Redis); only the `Host` header differs. That is the whole point: one binary,
//! one DB, two communities, provably isolated by `community_id` derived from the
//! host — never from caller input.
//!
//! # Status of each row
//!
//! A row is `todo!()`-stubbed until the lane it depends on lands on the
//! integration branch. The stub is intentional and load-bearing: it names the
//! exact obligation so the lane owner fills in *their* row, and a green run can
//! never be faked by an empty body. Lane ownership is noted per module.

#![allow(clippy::todo, unused)]

use std::time::Duration;

/// Host A's base URL (community A). Defaults to a local two-host relay.
fn url_a() -> String {
    std::env::var("RELAY_URL_A").unwrap_or_else(|_| "http://a.localhost:3000".to_string())
}

/// Host B's base URL (community B), same relay process, different host.
fn url_b() -> String {
    std::env::var("RELAY_URL_B").unwrap_or_else(|_| "http://b.localhost:3000".to_string())
}

/// An unmapped/unknown host on the same relay process. No community row maps to
/// it, so the relay must fail closed (404) rather than fall through to a default
/// tenant. `*.localhost` resolves to 127.0.0.1, so this addresses the same relay
/// as `url_a`/`url_b` but presents a `Host` no community is bound to.
fn url_unknown() -> String {
    std::env::var("RELAY_URL_UNKNOWN")
        .unwrap_or_else(|_| "http://unknown.localhost:3000".to_string())
}

/// Marker for a conformance obligation whose lane has not yet landed on the
/// integration branch. Centralizes the "not yet wired" panic so the harvest of
/// remaining work is one grep: `rg pending_lane conformance_multitenant.rs`.
#[track_caller]
fn pending_lane(lane: &str, obligation: &str) -> ! {
    todo!("conformance pending [{lane}]: {obligation}");
}

// ---------------------------------------------------------------------------
// Row zero: request community binding (Eva — relay-wiring)
// ---------------------------------------------------------------------------
mod row_zero_host_binding {
    use super::*;

    /// Obligation: an unknown/unmapped host fails closed with a *generic*
    /// rejection and never falls through to a default tenant.
    #[tokio::test]
    #[ignore]
    async fn unmapped_host_fails_closed_generically() {
        pending_lane(
            "relay-wiring",
            "unmapped host → generic rejection, no default tenant, no host echo",
        );
    }

    /// Obligation: a client-supplied `h` tag / token community stamp can never
    /// override the host-derived community; a disagreeing stamp is rejected.
    #[tokio::test]
    #[ignore]
    async fn client_supplied_community_cannot_override_host() {
        pending_lane(
            "relay-wiring",
            "token/h-tag community disagreeing with resolve_host(host) → reject",
        );
    }
}

// ---------------------------------------------------------------------------
// NIP-11 relay info and relay `self` (Eva — relay-wiring)
// ---------------------------------------------------------------------------
mod nip11_relay_info {
    use super::*;

    /// Fetch the NIP-11 relay information document from `base_url`'s root with
    /// `Accept: application/nostr+json`. Returns `(status, body)`; `body` is the
    /// raw response text (parsed by callers as needed).
    ///
    /// The `Host` header is implied by `base_url` — `a.localhost`/`b.localhost`
    /// both resolve to 127.0.0.1, so reqwest addresses the same relay process
    /// and the relay derives the community from the host. That host-derivation
    /// is exactly what this row exercises; nothing here is caller-supplied.
    async fn fetch_nip11(base_url: &str) -> (reqwest::StatusCode, String) {
        let client = reqwest::Client::builder()
            .build()
            .expect("build reqwest client");
        let resp = client
            .get(base_url)
            .header(reqwest::header::ACCEPT, "application/nostr+json")
            .send()
            .await
            .unwrap_or_else(|e| panic!("NIP-11 GET {base_url} failed: {e}"));
        let status = resp.status();
        let body = resp.text().await.expect("read NIP-11 body");
        (status, body)
    }

    /// Obligation: unauthenticated NIP-11 reads must not become an enumeration
    /// oracle for other communities; `RelayInfo::build` takes only static +
    /// host-scoped inputs (the static-input lint backs this at compile/CI time).
    ///
    /// This is the *black-box* complement to that compile-time fence
    /// (`crates/buzz-relay/src/nip11.rs::_RELAY_INFO_BUILD_STATIC_INPUT_FENCE`):
    /// the fence proves `RelayInfo::build` *cannot* take an unscoped DB/search
    /// input; this test proves the *observable wire behavior* — that the served
    /// document carries nothing that distinguishes one community from another.
    ///
    /// Because `RelayInfo::build` is genuinely static-input today, host A's and
    /// host B's NIP-11 bodies are byte-identical, and that identity *is* the
    /// proof: no field varies by community, so an unauthenticated reader cannot
    /// use the document to probe whether (or how) community B is configured.
    /// The moment a per-community value leaks into the doc, the two bodies
    /// diverge and this assertion fails — that is the mutate-bite this row
    /// guards (seed a community-distinguishing field into the served doc → the
    /// A≡B assertion goes red).
    #[tokio::test]
    #[ignore]
    async fn nip11_is_not_a_cross_community_enumeration_oracle() {
        let (status_a, body_a) = fetch_nip11(&url_a()).await;
        let (status_b, body_b) = fetch_nip11(&url_b()).await;

        assert_eq!(
            status_a,
            reqwest::StatusCode::OK,
            "host A must serve its NIP-11 document"
        );
        assert_eq!(
            status_b,
            reqwest::StatusCode::OK,
            "host B must serve its NIP-11 document"
        );

        // Both bodies must be valid NIP-11 JSON — a relay-info object, not an
        // error page or a host echo.
        let json_a: serde_json::Value =
            serde_json::from_str(&body_a).expect("host A NIP-11 is valid JSON");
        let json_b: serde_json::Value =
            serde_json::from_str(&body_b).expect("host B NIP-11 is valid JSON");
        assert!(
            json_a.get("supported_nips").is_some(),
            "host A NIP-11 must be a relay-info document (has supported_nips)"
        );
        assert!(
            json_b.get("supported_nips").is_some(),
            "host B NIP-11 must be a relay-info document (has supported_nips)"
        );

        // The enumeration-oracle obligation: no field of the served document
        // varies by community. Identical bodies are the proof that the doc
        // cannot be used to distinguish or probe another tenant.
        assert_eq!(
            json_a, json_b,
            "NIP-11 from host A and host B must be identical: any community-\
             distinguishing field would make the unauthenticated relay-info \
             document an enumeration oracle for other tenants"
        );

        // An *unmapped* host must get the SAME document too — not a 404. NIP-11
        // is intentionally host-agnostic (served from static facts BEFORE host
        // binding; see `router.rs::nip11_or_ws_handler`). If an unknown host
        // 404'd here while a mapped host returned 200, that status difference
        // would itself be the enumeration oracle — a caller could probe which
        // hosts are configured by watching for 404-vs-200. Serving the identical
        // static doc to every host, mapped or not, is precisely what denies that
        // oracle. (Fail-closed host binding lives on the WS-upgrade / non-
        // `nostr+json` path and is asserted by `row_zero_host_binding`.)
        let (status_unknown, body_unknown) = fetch_nip11(&url_unknown()).await;
        assert_eq!(
            status_unknown,
            reqwest::StatusCode::OK,
            "an unmapped host must still receive the static NIP-11 document, not \
             a 404 — a status difference between mapped and unmapped hosts would \
             itself be a community-enumeration oracle"
        );
        let json_unknown: serde_json::Value = serde_json::from_str(&body_unknown)
            .expect("unmapped-host NIP-11 is valid JSON");
        assert_eq!(
            json_a, json_unknown,
            "NIP-11 served to an unmapped host must be byte-identical to a mapped \
             host's document: the relay-info doc carries no host-derived field, \
             so it cannot reveal whether a given host is configured"
        );
    }
}

// ---------------------------------------------------------------------------
// API tokens and NIP-98 replay (Sami — buzz-auth)
// ---------------------------------------------------------------------------
mod api_tokens_nip98_replay {
    use super::*;

    /// Obligation: token hash uniqueness/lookup is `(community_id, token_hash)`;
    /// a token minted in A does not authorize the same hash in B.
    #[tokio::test]
    #[ignore]
    async fn token_minted_in_a_does_not_authorize_in_b() {
        pending_lane(
            "buzz-auth",
            "identical token_hash in A and B → A's token rejected against B",
        );
    }

    /// Obligation: NIP-98 replay seen-set is shared (any-pod) AND community
    /// scoped: a nonce spent in A is still spendable in B, but a replay within A
    /// is rejected from any pod.
    #[tokio::test]
    #[ignore]
    async fn nip98_replay_seenset_is_shared_and_community_scoped() {
        pending_lane(
            "buzz-auth",
            "replay key (community_id, event_id) in shared store; u-host must match req.community",
        );
    }
}

// ---------------------------------------------------------------------------
// Membership / allowlist / archived identities (Sami — buzz-auth)
// ---------------------------------------------------------------------------
mod membership_allowlist {
    use super::*;

    /// Obligation: membership/allowlist/archive keyed `(community_id, pubkey)`;
    /// archiving a key in A cannot hide/archive it in B; errors stay generic.
    #[tokio::test]
    #[ignore]
    async fn archive_in_a_does_not_affect_b() {
        pending_lane(
            "buzz-auth",
            "archived_identities (community_id, pubkey) — A's archive invisible to B",
        );
    }
}

// ---------------------------------------------------------------------------
// Users / profiles / NIP-05 / user search (Sami+Quinn — auth+search)
// ---------------------------------------------------------------------------
mod users_profiles_nip05 {
    use super::*;

    /// Obligation: same pubkey can hold a different profile per community; kind:0
    /// replacement is scoped by `(community_id, pubkey)`.
    #[tokio::test]
    #[ignore]
    async fn same_pubkey_distinct_profiles_in_two_communities() {
        pending_lane(
            "buzz-auth",
            "kind:0 replace scoped by (community_id, pubkey); no cross-community inheritance",
        );
    }

    /// Obligation: the same NIP-05 local part can exist on two hosts; lookup only
    /// resolves handles for the requested host/community.
    #[tokio::test]
    #[ignore]
    async fn same_nip05_local_part_on_two_hosts_is_independent() {
        pending_lane(
            "buzz-auth",
            "NIP-05 unique (community_id, lower(handle)); host A lookup never resolves B's",
        );
    }
}

// ---------------------------------------------------------------------------
// Channel-less global events and DMs (Mari+Max — db+pubsub)
// ---------------------------------------------------------------------------
mod channelless_global_events_dms {
    use super::*;

    /// Obligation: same event id / d-tag / pubkey can co-exist in two
    /// communities; direct `GET /api/events/{id}` and REQ filter by community
    /// first; NIP-33 uniqueness is `(community_id, kind, pubkey, d_tag)`.
    #[tokio::test]
    #[ignore]
    async fn same_event_id_and_dtag_coexist_across_communities() {
        pending_lane(
            "buzz-db",
            "same id/d-tag in A and B both retrievable, each scoped; no cross-fetch",
        );
    }

    /// Obligation: a DM `#p` in A does not cross-deliver to B.
    #[tokio::test]
    #[ignore]
    async fn dm_does_not_cross_deliver_between_communities() {
        pending_lane(
            "buzz-pubsub",
            "DM addressed in A never fans out to the same pubkey's B subscription",
        );
    }
}

// ---------------------------------------------------------------------------
// Channels and channel membership (Mari — buzz-db)
// ---------------------------------------------------------------------------
mod channels_membership {
    use super::*;

    /// Obligation: the same channel UUID legitimately co-exists in two
    /// communities (DB PK `(community_id, id)`); an `h` tag resolving to a
    /// channel in another community is rejected generically.
    #[tokio::test]
    #[ignore]
    async fn same_channel_uuid_in_two_communities_is_isolated() {
        pending_lane(
            "buzz-db",
            "channel UUID U exists in A and B; member/post in A never touches B's U",
        );
    }
}

// ---------------------------------------------------------------------------
// Workflows / runs / approvals / webhooks / schedules (Mari+Max)
// ---------------------------------------------------------------------------
mod workflows {
    use super::*;

    /// Obligation: identical workflow UUID / approval-token hash in two
    /// communities are independent; trigger evaluation only sees same-community
    /// events; schedule execution is isolated (at-most-once per community).
    #[tokio::test]
    #[ignore]
    async fn identical_workflow_and_approval_token_are_isolated() {
        pending_lane(
            "buzz-db",
            "same workflow UUID + approval hash in A and B act only within their community",
        );
    }
}

// ---------------------------------------------------------------------------
// Search / FTS (Quinn — buzz-search)
// ---------------------------------------------------------------------------
mod search_fts {
    use super::*;

    /// Obligation: every search `filter` includes `community_id`; same
    /// id/content in A and B return only same-community hits; deleting in A does
    /// not delete the B document. Postgres FTS (search_tsv/GIN), not Typesense.
    #[tokio::test]
    #[ignore]
    async fn search_results_and_deletes_are_community_scoped() {
        pending_lane(
            "buzz-search",
            "FTS query scoped by community_id; delete in A leaves B hit intact",
        );
    }
}

// ---------------------------------------------------------------------------
// Redis pub/sub, presence, typing, cache invalidation (Max — buzz-pubsub)
// ---------------------------------------------------------------------------
mod pubsub_presence_typing {
    use super::*;

    /// Obligation: keys are `buzz:{community}:…`; cross-node fan-out never
    /// delivers an A event to a B subscription, even for the same channel UUID;
    /// the same pubkey can be online in A and away in B independently.
    #[tokio::test]
    #[ignore]
    async fn fanout_and_presence_do_not_cross_communities() {
        pending_lane(
            "buzz-pubsub",
            "event on A's channel UUID never reaches B's subscription on the same UUID",
        );
    }
}

// ---------------------------------------------------------------------------
// Media / Blossom / S3 (Perci — buzz-media)
// ---------------------------------------------------------------------------
mod media_blossom {
    use super::*;

    /// Obligation: public blob `GET/HEAD /media/{sha256.ext}` stays
    /// unauthenticated (N=1 compat, shared CAS bytes). The community boundary is
    /// the metadata/descriptor/upload-auth/quota/audit layer: B's private upload
    /// metadata/errors must not be observable from A, even when the blob bytes
    /// are deduplicated and shared.
    #[tokio::test]
    #[ignore]
    async fn media_metadata_boundary_holds_while_blob_bytes_shared() {
        pending_lane(
            "buzz-media",
            "shared SHA bytes OK; A cannot read B's upload metadata/quota/audit; errors generic",
        );
    }
}

// ---------------------------------------------------------------------------
// Git hosting / NIP-34 / object storage (Perci — buzz-media/git)
// ---------------------------------------------------------------------------
mod git_hosting {
    use super::*;

    /// Obligation: pointer/name keys include community; same owner/repo in two
    /// communities are independent; a push in A does not advance B's pointer.
    #[tokio::test]
    #[ignore]
    async fn same_owner_repo_isolated_push_does_not_cross() {
        pending_lane(
            "buzz-media",
            "repos/{community}/{owner}/{repo}/pointer; push in A leaves B pointer unchanged",
        );
    }
}

// ---------------------------------------------------------------------------
// Mesh / agents / ACP / MCP / CLI (Eva — relay-wiring smoke)
// ---------------------------------------------------------------------------
mod mesh_agents_cli {
    use super::*;

    /// Obligation: one portable key may join multiple communities, but
    /// memberships, DMs, profiles, jobs, and presence do not bleed across them.
    #[tokio::test]
    #[ignore]
    async fn one_key_two_communities_no_bleed() {
        pending_lane(
            "relay-wiring",
            "same key joins A and B; CLI/ACP smoke shows distinct memberships/profiles",
        );
    }
}

// ---------------------------------------------------------------------------
// Audit log and observability (Dawn — buzz-audit)
// ---------------------------------------------------------------------------
mod audit_log {
    //! Obligation: audit reads verify exactly one community chain
    //! (`(community_id, seq)` / `(community_id, hash)`); error strings must not
    //! leak cross-community IDs, constraint names, or existence facts.
    //!
    //! **This row is doc-only — and that is the strongest statement in the file.**
    //!
    //! Every other row here proves a black-box property: the relay serves a wire
    //! response, and the test asserts that response denies a cross-community
    //! oracle. The audit log has no such response to assert against — it has **no
    //! client-reachable wire surface at all**. There is no `/audit` route in
    //! `crates/buzz-relay/src/router.rs` (the route list is `/`, `/info`,
    //! `/.well-known/nostr.json`, the health probes, `/events`, `/query`,
    //! `/count`, `/hooks`, the media and git sub-routers, and the audio WS — no
    //! audit endpoint). Audit is written as an ingest side-effect
    //! (`handlers/event.rs`, `dispatch_persistent_event`) and read only via
    //! `buzz_audit::AuditService::{verify_chain, get_entries}`, which are
    //! operator-internal (consumed by `buzz-admin`). `crates/buzz-audit/src/
    //! error.rs` states it directly: `AuditError` is "never relayed to a client
    //! on the wire," and "no variant embeds a `community_id`."
    //!
    //! So where other rows prove *the oracle is denied*, audit proves *the
    //! oracle's surface does not exist* — a strictly stronger isolation claim,
    //! and the honest way to state it is to cite the facts, not to invent a wire
    //! observation that the architecture does not offer. Reaching behind the
    //! wire into Postgres here would also break this file's black-box contract
    //! (its deps are `buzz-ws-client`/`reqwest`/`tokio-tungstenite`/`s3` — no
    //! `sqlx`, no `buzz-audit`), and a DB-direct read can never catch a
    //! wire-layer bug because it never traverses the wire read path.
    //!
    //! The two halves of the obligation are proven in their proper homes, where
    //! direct Postgres access is in-convention:
    //!
    //!   1. **One chain per community** —
    //!      `buzz_audit::service::tests::chains_are_independent_per_community`
    //!      (direct `AuditService::log`) proves interleaved A/B writes keep
    //!      independent `(community_id, seq)` chains, each starting at seq 1 with
    //!      its own `prev_hash`, and that `verify_chain`/`get_entries` scoped to
    //!      one community never traverse another. The *integrated* path — that a
    //!      community resolved from the request's `TenantContext` at relay ingest
    //!      lands in the correct chain and stays isolated — is proven by
    //!      `buzz_relay::handlers::event::tests::
    //!      audit_chain_is_isolated_per_tenant_through_relay_ingest`, driving
    //!      `dispatch_persistent_event` under two tenants against a shared
    //!      Postgres (no WS-AUTH dependency).
    //!   2. **Errors don't leak** —
    //!      `buzz_audit::error::tests::audit_error_text_carries_no_community_id_or_constraint`
    //!      asserts no `AuditError` variant's rendered text embeds a
    //!      `community_id`, constraint name, or cross-community object id.
    //!
    //! Substrate on PR head: `crates/buzz-audit/src/entry.rs` keys `AuditEntry`
    //! `(community_id, seq)` with per-community `prev_hash`; `NewAuditEntry.
    //! community_id` is typed `CommunityId` (server-resolved, never client
    //! input).
}

// ---------------------------------------------------------------------------
// Migration gate 5: N=1 conformance (Eva — orchestration)
// ---------------------------------------------------------------------------
mod n1_parity {
    //! N=1 parity is asserted by the *existing* e2e suites run against the new
    //! relay with one configured host → one default community. The obligation:
    //! no existing client needs new tags, paths, event fields, CLI flags, or
    //! protocol messages. This module documents the gate; the assertion lives in
    //! the unchanged `e2e_*` suites passing green under `RELAY_URL=<new relay>`.
    //!
    //! Parity runner (driven from the integration job, not a unit test):
    //!   1. Boot new relay, one host → default community, backfill existing data.
    //!   2. Run the full `e2e_*` ignored suite with `RELAY_URL` at the new relay.
    //!   3. Every suite green, unchanged === N=1 parity proven.
}
