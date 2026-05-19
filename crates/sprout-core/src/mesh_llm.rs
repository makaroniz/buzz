//! Mesh-LLM compute offer envelope (kind:31990 event content).
//!
//! Published by Sprout members willing to share their local LLM/compute with
//! the rest of the relay. Consumers (other Sprout members) subscribe to
//! kind:31990 events scoped to relay membership and pick an offer that
//! matches their request.
//!
//! # Schema
//!
//! The event content is a JSON-serialised [`MeshLlmOffer`]. The event itself
//! is a NIP-33 parameterized-replaceable event addressed by
//! `(pubkey, kind:31990, d_tag)` where `d_tag` is the [`MeshLlmOffer::d_tag`].
//! This means a member can replace their own offer atomically (e.g. when the
//! VRAM cap changes or a model is loaded/unloaded) without leaking dangling
//! stale offers.
//!
//! # Trust model
//!
//! The signing pubkey of the kind:31990 event is the Nostr identity of the
//! offering member; the event flows through the existing NIP-43 fan-out, so
//! only relay members ever see it. The iroh [`endpoint_id`](MeshLlmOffer::endpoint_id)
//! is a separate ed25519 keypair under the same member's control — the
//! Nostr signature on the kind:31990 event is what binds those two
//! identities together.
//!
//! When a consumer connects to the offered iroh endpoint, the consumer's own
//! NIP-98 bearer (signed with its Nostr key, NOT its iroh key) is what the
//! receiving relay uses to gate admission. So the chain of trust is:
//!
//! - The 31990 event proves "Nostr pubkey N offers compute via iroh endpoint E".
//! - The NIP-98 bearer on the iroh connection proves "Nostr pubkey N' is the
//!   connecting party".
//! - Sprout's [`check_relay_membership`] confirms N' is a relay member.
//!
//! There is no need to also bind N' ↔ iroh-client-endpoint cryptographically:
//! once the membership decision allows the connection, the QUIC stream itself
//! is end-to-end-encrypted between the two iroh endpoints. The offering side
//! sees only `(member-pubkey N', iroh-endpoint E')`, both authenticated.

use serde::{Deserialize, Serialize};

/// The full content of a kind:31990 event.
///
/// Serialized to JSON and placed in the event's `content` field. The event's
/// `d` tag should equal [`MeshLlmOffer::d_tag`] so the event is a stable
/// addressable replacement target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeshLlmOffer {
    /// Schema version. Bumped on breaking changes. Current: `1`.
    pub v: u32,

    /// Stable identifier for *this offering node* under the publisher's
    /// pubkey. A member may publish multiple offers (e.g. one per host they
    /// own, or one per GPU); each gets a distinct `d_tag`.
    ///
    /// MUST be ≤64 chars, ASCII alphanumeric + `-` + `_`. The same value
    /// must be used as the kind:31990 event's `d` tag so replaces are
    /// atomic.
    pub d_tag: String,

    /// Iroh endpoint id (ed25519 public key, base32 z-base form as iroh
    /// renders it) of the offering node's iroh endpoint. Consumers dial
    /// this through an iroh `NodeAddr`.
    pub endpoint_id: String,

    /// Iroh relay URL through which the offering endpoint is reachable.
    ///
    /// This is the *Sprout-hosted* iroh-relay URL — copied verbatim from
    /// the publisher's view of NIP-11 `iroh_relay_url`. If multiple Sprout
    /// relays are bridged into the same membership scope in the future,
    /// this lets a consumer reach an offer behind a different host.
    pub iroh_relay_url: String,

    /// Resource caps the offering side promises to honour for any single
    /// consumer at a time. The publisher should re-publish (replacing the
    /// previous event) whenever these change materially.
    pub caps: ResourceCaps,

    /// Models this node is willing to serve. Empty list = "negotiate at
    /// connect time"; non-empty = the consumer should pick one of these.
    #[serde(default)]
    pub models: Vec<ModelOffer>,

    /// Free-form opaque metadata field, reserved for future extensions
    /// (e.g. region, accelerator type, presence-style state).
    ///
    /// Stored as `serde_json::Value` so additions don't require a schema
    /// bump. `deny_unknown_fields` above keeps the *top-level* schema
    /// strict; freeform extension lives here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

/// Resource caps the offering side commits to for a single consumer.
///
/// Caps are *per-consumer* upper bounds — the offering side may host
/// multiple concurrent consumers, each subject to these caps. The
/// `max_concurrency` field expresses how many concurrent consumers the node
/// will accept across all consumers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceCaps {
    /// Max VRAM (megabytes) the offering side will commit to a single
    /// request. `None` = no cap advertised (consumer decides whether to
    /// proceed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_vram_mb: Option<u32>,

    /// Max system RAM (megabytes) the offering side will commit to a
    /// single request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_ram_mb: Option<u32>,

    /// Max number of concurrent consumers the offering node will accept
    /// across all currently-running requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrency: Option<u32>,
}

/// A single model the offering node is prepared to serve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelOffer {
    /// Model identifier. Convention: HuggingFace-style `org/name[:tag]`,
    /// or `local:<filename>` for ad-hoc local files. Free-form string;
    /// the consumer side is responsible for matching this against its own
    /// requested model.
    pub id: String,

    /// Optional human-readable label for UI surfaces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,

    /// Approximate context window this model serves (tokens). Used for
    /// UI hints; not enforced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u32>,
}

impl MeshLlmOffer {
    /// Maximum length of a `d_tag` string. Mirrors NIP-33's general rule
    /// that `d` tags should be short and stable.
    pub const MAX_D_TAG_LEN: usize = 64;

    /// Validate that a `d_tag` is well-formed: ≤64 chars, ASCII
    /// alphanumeric / `-` / `_`.
    pub fn is_valid_d_tag(d_tag: &str) -> bool {
        !d_tag.is_empty()
            && d_tag.len() <= Self::MAX_D_TAG_LEN
            && d_tag
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    }

    /// Returns true if every required field is well-formed for publishing.
    ///
    /// This is a *publisher-side* sanity check; consumers should be
    /// permissive in what they accept as long as serde-deserialization
    /// succeeds.
    pub fn is_publishable(&self) -> bool {
        self.v == 1
            && Self::is_valid_d_tag(&self.d_tag)
            && !self.endpoint_id.is_empty()
            && !self.iroh_relay_url.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> MeshLlmOffer {
        MeshLlmOffer {
            v: 1,
            d_tag: "node-1".to_string(),
            endpoint_id: "1234abcd".to_string(),
            iroh_relay_url: "https://relay.example.com/iroh".to_string(),
            caps: ResourceCaps {
                max_vram_mb: Some(24_000),
                max_ram_mb: Some(64_000),
                max_concurrency: Some(2),
            },
            models: vec![ModelOffer {
                id: "meta-llama/Llama-3-8B".to_string(),
                label: Some("Llama 3 8B".to_string()),
                context_tokens: Some(8192),
            }],
            extra: None,
        }
    }

    #[test]
    fn round_trip_via_json() {
        let offer = sample();
        let s = serde_json::to_string(&offer).expect("serialise");
        let back: MeshLlmOffer = serde_json::from_str(&s).expect("deserialise");
        assert_eq!(offer, back);
    }

    #[test]
    fn optional_caps_default_to_none() {
        let s = r#"{
            "v": 1,
            "d_tag": "x",
            "endpoint_id": "abc",
            "iroh_relay_url": "https://r/",
            "caps": {}
        }"#;
        let offer: MeshLlmOffer = serde_json::from_str(s).expect("deserialise minimal");
        assert!(offer.caps.max_vram_mb.is_none());
        assert!(offer.caps.max_ram_mb.is_none());
        assert!(offer.caps.max_concurrency.is_none());
        assert!(offer.models.is_empty());
    }

    #[test]
    fn unknown_top_level_field_rejected() {
        // deny_unknown_fields catches schema drift.
        let s = r#"{
            "v": 1,
            "d_tag": "x",
            "endpoint_id": "abc",
            "iroh_relay_url": "https://r",
            "caps": {},
            "wat": "lol"
        }"#;
        assert!(serde_json::from_str::<MeshLlmOffer>(s).is_err());
    }

    #[test]
    fn unknown_caps_field_rejected() {
        let s = r#"{
            "v": 1,
            "d_tag": "x",
            "endpoint_id": "abc",
            "iroh_relay_url": "https://r",
            "caps": { "wat": 7 }
        }"#;
        assert!(serde_json::from_str::<MeshLlmOffer>(s).is_err());
    }

    #[test]
    fn extra_freeform_passes_through() {
        let offer = MeshLlmOffer {
            extra: Some(serde_json::json!({"region": "us-east", "gpu": "H100"})),
            ..sample()
        };
        let s = serde_json::to_string(&offer).unwrap();
        let back: MeshLlmOffer = serde_json::from_str(&s).unwrap();
        assert_eq!(offer, back);
    }

    #[test]
    fn d_tag_validation() {
        assert!(MeshLlmOffer::is_valid_d_tag("node-1"));
        assert!(MeshLlmOffer::is_valid_d_tag("a"));
        assert!(MeshLlmOffer::is_valid_d_tag(&"a".repeat(64)));
        assert!(!MeshLlmOffer::is_valid_d_tag(""));
        assert!(!MeshLlmOffer::is_valid_d_tag(&"a".repeat(65)));
        assert!(!MeshLlmOffer::is_valid_d_tag("node 1"));
        assert!(!MeshLlmOffer::is_valid_d_tag("node/1"));
        assert!(!MeshLlmOffer::is_valid_d_tag("nodé"));
    }

    #[test]
    fn is_publishable_rejects_bad_d_tag() {
        let mut offer = sample();
        offer.d_tag = "bad tag with spaces".to_string();
        assert!(!offer.is_publishable());
    }

    #[test]
    fn is_publishable_rejects_wrong_version() {
        let mut offer = sample();
        offer.v = 2;
        assert!(!offer.is_publishable());
    }

    #[test]
    fn is_publishable_rejects_empty_endpoint() {
        let mut offer = sample();
        offer.endpoint_id = String::new();
        assert!(!offer.is_publishable());
    }
}
