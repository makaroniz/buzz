//! Error types for buzz-auth.

/// All errors that can occur during authentication and authorization.
///
/// Variants are designed to be safe to return to callers without leaking
/// internal implementation details. Do **not** include raw token values,
/// database contents, or stack traces in error messages.
///
/// # Wire-mapping contract
///
/// This enum describes **what happened**; how it renders on the wire is the
/// relay's concern. The single sanctioned conversion lives in
/// `buzz_relay::auth_wire::auth_error_wire`, which collapses every variant
/// into one of four `AuthErrorWireCategory` values:
///
/// - `AuthFailed` — all verification-class variants (`InvalidSignature`,
///   `ChallengeMismatch`, `RelayUrlMismatch`, `EventExpired`, `Nip98Invalid`,
///   `Nip98Replay`, `PubkeyMismatch`) MUST be byte-indistinguishable on the
///   wire. Distinguishing any pair turns the community-scoped replay seen-set
///   or membership state into a presence oracle. See the audit note at
///   `RESEARCH/RELAY_REWRITE_AUTH_ERROR_ORACLE_AUDIT.md` (policies P1–P5).
/// - `InsufficientScope` / `ChannelAccessDenied` — authorization class,
///   remediation-distinct, carry no tenant-scoped detail.
/// - `InternalRedacted` — `Internal(_)` MUST NEVER stringify on the wire; the
///   inner string can carry community-prefixed Redis keys (existence oracle).
///   The construction site logs detail via `tracing::warn!`; the wire sees
///   the category only.
///
/// The exhaustive match in `auth_error_wire` (no wildcard arm) is the
/// compile-time fence: adding any variant here fails to compile in
/// `buzz-relay` until the wire-class decision is made.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// The NIP-42 event signature is invalid or the event is structurally malformed.
    ///
    /// **WIRE class:** `AuthFailed`. Byte-identical with all other verification-class variants.
    #[error("invalid signature or malformed auth event")]
    InvalidSignature,

    /// The `challenge` tag in the AUTH event does not match the relay's issued challenge.
    ///
    /// **WIRE class:** `AuthFailed`. Byte-identical with all other verification-class variants.
    #[error("challenge mismatch")]
    ChallengeMismatch,

    /// The `relay` tag in the AUTH event does not match this relay's URL.
    ///
    /// **WIRE class:** `AuthFailed`. Byte-identical with all other verification-class variants.
    #[error("relay url mismatch")]
    RelayUrlMismatch,

    /// The AUTH event's `created_at` timestamp is more than ±60 seconds from now.
    ///
    /// **WIRE class:** `AuthFailed`. Byte-identical with all other verification-class variants.
    #[error("auth event timestamp outside ±60s window")]
    EventExpired,

    /// NIP-98 HTTP Auth event (kind:27235) failed verification.
    ///
    /// The inner string describes the specific failure (signature, timestamp, URL, etc.)
    /// and is safe to include in server logs. Do **not** forward raw event content to clients.
    ///
    /// **WIRE class:** `AuthFailed`. Byte-identical with all other verification-class variants;
    /// the inner string is log-only.
    #[error("NIP-98 HTTP Auth verification failed: {0}")]
    Nip98Invalid(String),

    /// A NIP-98 event with the same id has already been observed within the
    /// replay-prevention window. The event itself was structurally valid; the
    /// rejection is on freshness, not validity.
    ///
    /// **WIRE class:** `AuthFailed`. MUST be byte-indistinguishable from
    /// `Nip98Invalid` on the wire — distinguishing them turns the
    /// community-scoped seen-set into a presence oracle on event ids.
    #[error("NIP-98 replay: event id already seen within window")]
    Nip98Replay,

    /// The pubkey in the auth event does not match the expected identity.
    ///
    /// **WIRE class:** `AuthFailed`. Byte-identical with all other verification-class variants.
    #[error("pubkey mismatch: event pubkey does not match authenticated identity")]
    PubkeyMismatch,

    /// The authenticated context does not have the required scope for this operation.
    ///
    /// **WIRE class:** `InsufficientScope`. Authorization-class, remediation-distinct;
    /// the `required`/`have` fields carry no tenant-scoped detail and are safe on the wire.
    #[error("insufficient scope: required {required}, have {have:?}")]
    InsufficientScope {
        /// The scope that was required.
        required: String,
        /// The scopes the caller actually holds.
        have: Vec<String>,
    },

    /// The authenticated user is not a member of the requested channel.
    ///
    /// **WIRE class:** `ChannelAccessDenied`. Authorization-class, remediation-distinct.
    #[error("channel access denied")]
    ChannelAccessDenied,

    /// An unexpected internal error occurred (e.g. a `spawn_blocking` panic).
    ///
    /// **WIRE class:** `InternalRedacted`. The inner string MUST NEVER appear on the wire —
    /// it can carry community-prefixed Redis keys, downstream error chains, or other
    /// existence-oracle surfaces. Log detail at the construction site; the wire sees the
    /// category only.
    #[error("internal auth error: {0}")]
    Internal(String),
}
