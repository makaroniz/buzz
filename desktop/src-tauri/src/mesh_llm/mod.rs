//! Mesh-LLM client: discover, dial, and publish kind:31990 offers.
//!
//! This module is the desktop-side counterpart to `sprout-relay`'s embedded
//! iroh-relay (see `crates/sprout-relay/src/iroh_relay.rs`). The relay gates
//! admission with NIP-98 + relay membership; this module signs that bearer
//! token, dials offers advertised under kind:31990, and publishes our own
//! offer when the user enables compute-sharing.
//!
//! ## Submodules
//!
//! - [`endpoint`]: long-lived iroh endpoint keypair persisted at
//!   `{app_data_dir}/mesh_iroh.key`.
//! - [`nip98`]: build the NIP-98 bearer event signed with the user's Nostr
//!   key for a given canonical relay URL.
//! - [`offer`]: load/save the user's mesh-LLM offer preferences
//!   (VRAM/RAM/concurrency caps, models).

pub mod endpoint;
pub mod nip98;
pub mod offer;

pub use endpoint::{load_or_create_endpoint_key, EndpointKeyError};
pub use nip98::{build_nip98_bearer, Nip98BearerError};
pub use offer::{ComputeSharingPrefs, OfferPrefsError};
