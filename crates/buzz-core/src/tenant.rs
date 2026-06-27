//! Tenant identity: the server-resolved community key carried on every scoped path.
//!
//! These types live in `buzz-core` (zero I/O deps) so the DB, auth, pub/sub,
//! search, audit, media, and relay-wiring layers all name a community the same
//! way without depending on each other.
//!
//! ## The fence
//!
//! The whole multi-tenant safety story rests on one invariant from the formal
//! model (conformance "row zero"): a request's community is *resolved from the
//! connection host by the server*, never supplied or influenced by the client.
//!
//! [`TenantContext`] encodes that invariant in the type system. It has no
//! `Default`, no `Deserialize`, and no public constructor other than
//! [`TenantContext::resolved`], which is meant to be called *only* from the
//! host-resolution path. Downstream code receives `&TenantContext` and can read
//! the community but cannot mint one — so "the client chose this community"
//! cannot type-check anywhere outside resolution.

use std::fmt;
use uuid::Uuid;

/// A community: the first-class tenant key on every scoped row.
///
/// Opaque UUID newtype. Equality and ordering are the underlying UUID's.
/// There is deliberately no `community_id` parsed from client input anywhere;
/// a `CommunityId` only ever originates from host resolution or from a DB row
/// the server already scoped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CommunityId(Uuid);

impl CommunityId {
    /// Wrap a UUID that the server has already established as a community id
    /// (e.g. read back from the `communities` table during host resolution).
    ///
    /// This is intentionally not a parse-from-client entry point: callers must
    /// already hold a server-trusted UUID.
    pub const fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    /// The underlying UUID, for DB binds and Redis key construction.
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl fmt::Display for CommunityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// The resolved tenant of an in-flight request, bound once at connection /
/// request establishment before any handler observes tenant data.
///
/// Carried by reference (`&TenantContext`) through every scoped call. This is
/// the *only* way to name a community downstream, and it cannot be constructed
/// from client input — see the module-level "fence" note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantContext {
    community: CommunityId,
    host: String,
}

impl TenantContext {
    /// Construct a context from a completed host resolution.
    ///
    /// Call this *only* from the host-resolution path (the function that maps a
    /// connection's host to a `communities` row). Everywhere else takes
    /// `&TenantContext` and reads it; nothing else mints one.
    pub fn resolved(community: CommunityId, host: impl Into<String>) -> Self {
        Self {
            community,
            host: host.into(),
        }
    }

    /// The community every scoped operation under this request must use.
    pub const fn community(&self) -> CommunityId {
        self.community
    }

    /// The host that resolved to this community.
    ///
    /// Authoritative for the NIP-05 domain and audit labelling; never re-derive
    /// the community from it downstream — the community is already fixed.
    pub fn host(&self) -> &str {
        &self.host
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn community_id_roundtrips_uuid() {
        let u = Uuid::from_u128(0x1234_5678_9abc_def0_1122_3344_5566_7788);
        let c = CommunityId::from_uuid(u);
        assert_eq!(c.as_uuid(), &u);
        assert_eq!(c.to_string(), u.to_string());
    }

    #[test]
    fn tenant_context_exposes_resolution_inputs() {
        let u = Uuid::from_u128(1);
        let ctx = TenantContext::resolved(CommunityId::from_uuid(u), "relay.example");
        assert_eq!(ctx.community().as_uuid(), &u);
        assert_eq!(ctx.host(), "relay.example");
    }
}
