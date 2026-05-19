//! NIP-98 bearer token builder for iroh-relay admission.
//!
//! Signs a kind:27235 event over the canonical iroh-relay URL using the
//! user's Nostr identity, then base64-encodes the event JSON. The receiving
//! relay's `sprout_relay::iroh_relay` access callback decodes + verifies
//! this exact bearer string.
//!
//! Both sides use the same `sprout_auth::nip98_canonical_url` helper, so
//! path-prefix / trailing-slash / localhost-vs-127.0.0.1 drift cannot
//! create undebuggable per-connection denials.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag};

const IROH_RELAY_PATH: &str = "/relay";
const NIP98_METHOD: &str = "GET";

/// Errors produced while building a NIP-98 bearer.
#[derive(Debug, thiserror::Error)]
pub enum Nip98BearerError {
    /// `iroh_relay_url` from NIP-11 wasn't a parseable URL.
    #[error("invalid iroh relay URL: {0}")]
    InvalidUrl(String),
    /// `nostr` library failed to construct/sign the event.
    #[error("event signing failed: {0}")]
    Sign(String),
    /// Tag construction failed (should never happen for static "u"/"method").
    #[error("tag construction failed: {0}")]
    Tag(String),
}

/// Build the `Authorization: Bearer <token>` value for an iroh-relay
/// admission request.
///
/// `iroh_relay_public_url` is the value taken verbatim from the target
/// relay's NIP-11 `iroh_relay_url` field. We canonicalise it the same way
/// the relay does before signing the `u` tag.
pub fn build_nip98_bearer(
    keys: &Keys,
    iroh_relay_public_url: &str,
) -> Result<String, Nip98BearerError> {
    let canonical = sprout_auth::nip98_canonical_url(iroh_relay_public_url, IROH_RELAY_PATH)
        .ok_or_else(|| Nip98BearerError::InvalidUrl(iroh_relay_public_url.to_string()))?;

    let tags = vec![
        Tag::parse(["u", &canonical]).map_err(|e| Nip98BearerError::Tag(e.to_string()))?,
        Tag::parse(["method", NIP98_METHOD]).map_err(|e| Nip98BearerError::Tag(e.to_string()))?,
    ];

    let event = EventBuilder::new(Kind::HttpAuth, "")
        .tags(tags)
        .sign_with_keys(keys)
        .map_err(|e| Nip98BearerError::Sign(e.to_string()))?;

    let json = event.as_json();
    Ok(STANDARD.encode(json))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_for_canonical_url() {
        let keys = Keys::generate();
        let token =
            build_nip98_bearer(&keys, "https://relay.example.com/iroh").expect("build bearer");
        // Round-trip through base64 -> JSON to confirm the event has the
        // canonical URL in its `u` tag.
        let bytes = STANDARD.decode(&token).expect("base64 decode");
        let json = String::from_utf8(bytes).expect("utf8");
        assert!(
            json.contains("\"u\""),
            "bearer event should carry `u` tag: {json}",
        );
        assert!(
            json.contains("https://relay.example.com/iroh/relay"),
            "bearer event should canonicalise the URL: {json}",
        );
        assert!(
            json.contains("\"method\""),
            "bearer event should carry `method` tag",
        );
    }

    #[test]
    fn rejects_unparseable_url() {
        let keys = Keys::generate();
        let err = build_nip98_bearer(&keys, "definitely not a url").expect_err("should fail");
        match err {
            Nip98BearerError::InvalidUrl(_) => {}
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn different_keys_produce_different_bearers() {
        let a = Keys::generate();
        let b = Keys::generate();
        let ta = build_nip98_bearer(&a, "https://relay.example.com/iroh").unwrap();
        let tb = build_nip98_bearer(&b, "https://relay.example.com/iroh").unwrap();
        assert_ne!(ta, tb);
    }
}
