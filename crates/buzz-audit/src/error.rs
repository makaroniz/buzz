use thiserror::Error;

/// Errors that can occur during audit log operations.
///
/// These are **operator-internal** diagnostics (logged by the audit worker, or
/// returned to an operator-scoped verification call) — they are never relayed to
/// a client on the wire. Even so, no variant embeds a `community_id` or any
/// cross-community object identifier: a `seq` is per-community and meaningless
/// without its chain, and hashes are opaque. An error raised while verifying
/// community A's chain therefore cannot reveal a fact about community B.
#[derive(Debug, Error)]
pub enum AuditError {
    /// A database operation failed.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    /// The `prev_hash` of an entry does not match the hash of the preceding
    /// entry in the same community's chain.
    #[error(
        "hash chain integrity violation at seq {seq}: prev_hash does not match preceding entry"
    )]
    ChainViolation {
        /// Per-community sequence number of the offending entry.
        seq: i64,
    },

    /// The stored hash of an entry does not match the recomputed hash.
    #[error("hash mismatch at seq {seq}: stored hash does not match recomputed hash")]
    HashMismatch {
        /// Per-community sequence number of the offending entry.
        seq: i64,
    },

    /// An unrecognised action string was found in the database.
    #[error("unknown audit action in database")]
    UnknownAction,

    /// A JSON serialization error occurred (e.g. while canonicalising `detail`).
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}
