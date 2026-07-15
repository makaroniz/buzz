import Foundation

/// Pure NIP-PL client lifecycle decision machine.
///
/// This encodes the B2/B4 rules from the iOS lifecycle cold review as a
/// side-effect-free function of (persisted state, observed APNs token,
/// clock), so every rule is unit-testable without App Attest, Keychain, or
/// network:
///
/// - **Rotate only on real token change.** iOS re-delivers the device token
///   on every launch; an unconditional rotate bumps the server epoch each
///   launch and invalidates every outstanding relay delegation. The machine
///   compares fingerprints and answers `.noop` for a re-delivered token.
/// - **Persist-then-confirm epoch.** A rotation is two-phase: the caller
///   persists the `pendingRotation` intent *before* sending
///   `POST /v1/installations/endpoint`, and commits/clears it only on a known
///   outcome. A crash in between leaves the intent in place; the machine
///   answers `.resumeRotation` on the next launch instead of desyncing.
/// - **Zombie-key recovery.** After backup-restore to a new device the
///   restored state names an App Attest key that no longer exists (keys are
///   device-bound), so every assertion fails. `apply(outcome:)` maps
///   attestation-level rejection to `.clearAndReenroll` instead of retrying
///   forever.
/// - **Renewal.** Installations expire (`expires_at`); an expired or
///   near-expiry installation re-enrolls instead of silently failing rotate
///   and delegate calls with a stale handle.
public enum BuzzPushLifecycle {
    /// How far before `expires_at` the client proactively re-enrolls.
    /// 7 days against the 90-day default lifetime: early enough to cover
    /// long app dormancy between launches, tiny relative to the lease.
    public static let renewalLeadTimeSeconds: Int64 = 7 * 24 * 3600

    // MARK: Launch decision

    /// What to do when APNs (re)delivers a device token.
    public enum TokenDecision: Equatable {
        /// No persisted installation (or it must be discarded): run full
        /// enrollment for the given token fingerprint.
        case enroll
        /// Installation valid and the token is unchanged: do nothing.
        case noop
        /// Installation expired or inside the renewal window: re-enroll
        /// (fresh attest key + handle), then discard the old state.
        case reenroll
        /// Token genuinely changed: persist `stateWithIntent` FIRST, then
        /// send rotate with `endpoint_epoch = stateWithIntent.endpointEpoch`
        /// and `new_endpoint_epoch = pending.newEndpointEpoch`.
        case beginRotation(stateWithIntent: BuzzPushInstallationState)
        /// A previous rotation intent survived (crash window): re-send the
        /// same rotate. The intent already pins the target epoch, so this is
        /// idempotent from the client's perspective; `apply(outcome:)`
        /// resolves whether the server had already committed.
        case resumeRotation(state: BuzzPushInstallationState)
    }

    /// Decide the launch/token action. `tokenFingerprint` is
    /// `BuzzPushInstallationState.fingerprint(ofEndpoint:)` of the freshly
    /// delivered token; `now` is unix seconds.
    public static func onDeviceToken(
        state: BuzzPushInstallationState?,
        tokenFingerprint: String,
        now: Int64
    ) -> TokenDecision {
        guard let state else { return .enroll }
        if now >= state.expiresAt - renewalLeadTimeSeconds {
            return .reenroll
        }
        if let pending = state.pendingRotation {
            // A surviving intent wins over fingerprint comparison: the local
            // committed fingerprint may or may not match the server, and the
            // intent records where we were headed.
            if pending.newEndpointFingerprint == tokenFingerprint {
                return .resumeRotation(state: state)
            }
            // Token moved again while a rotation was in flight. Re-point the
            // intent at the new token but keep the same target epoch — the
            // old intent never confirmed, so the epoch step is still ours.
            var next = state
            next.pendingRotation = .init(
                newEndpointEpoch: pending.newEndpointEpoch,
                newEndpointFingerprint: tokenFingerprint
            )
            return .beginRotation(stateWithIntent: next)
        }
        if state.endpointFingerprint == tokenFingerprint {
            return .noop
        }
        var next = state
        next.pendingRotation = .init(
            newEndpointEpoch: state.endpointEpoch + 1,
            newEndpointFingerprint: tokenFingerprint
        )
        return .beginRotation(stateWithIntent: next)
    }

    // MARK: Rotation outcome

    /// Gateway response classification for a rotate attempt. The caller maps
    /// HTTP status/error codes to this enum.
    public enum RotateOutcome: Equatable {
        /// `200 {"status":"rotated"}`.
        case rotated
        /// `404 not_authorized` — epoch mismatch, consumed challenge, or
        /// missing/expired installation. Indistinguishable by design.
        case notAuthorized
        /// `401 invalid_attestation` — assertion failed to verify. With a
        /// device-bound key this is the zombie-key (backup-restore) or
        /// tampered-state signature.
        case invalidAttestation
        /// Transport failure / 5xx / timeout: outcome unknown.
        case transient
    }

    /// What the caller must do after a rotate attempt resolves.
    public enum RotateResolution: Equatable {
        /// Persist this state (intent committed or advanced).
        case persist(BuzzPushInstallationState)
        /// Keep the persisted intent as-is and retry later (with backoff).
        case retainIntent
        /// Local state is unrecoverable: delete it and run full enrollment.
        case clearAndReenroll
    }

    /// Resolve a rotate attempt that was sent with
    /// `endpoint_epoch = state.endpointEpoch` and
    /// `new_endpoint_epoch = state.pendingRotation!.newEndpointEpoch`.
    ///
    /// `notAuthorized` on a *resumed* intent is the ambiguous crash-window
    /// case: the server may have committed the previous attempt (so our old
    /// epoch no longer matches). We cannot read the server epoch back, so
    /// the deterministic reconvergence is: adopt the pending epoch as
    /// committed, then immediately begin a fresh rotation FROM it — if the
    /// server had committed, this succeeds and both sides converge; if the
    /// installation is actually gone, the next attempt maps to
    /// `.clearAndReenroll` via `escalated`.
    public static func applyRotate(
        state: BuzzPushInstallationState,
        outcome: RotateOutcome,
        escalated: Bool = false
    ) -> RotateResolution {
        guard let pending = state.pendingRotation else {
            // No intent recorded: nothing to commit; treat as corrupt state.
            return .clearAndReenroll
        }
        switch outcome {
        case .rotated:
            var next = state
            next.endpointEpoch = pending.newEndpointEpoch
            next.endpointFingerprint = pending.newEndpointFingerprint
            next.pendingRotation = nil
            return .persist(next)
        case .transient:
            return .retainIntent
        case .invalidAttestation:
            // Assertion itself failed: the attest key no longer signs for
            // this state (restored backup / wiped key). Retrying is futile.
            return .clearAndReenroll
        case .notAuthorized:
            if escalated {
                // Already adopted-and-retried once; the installation is gone
                // or expired server-side.
                return .clearAndReenroll
            }
            // Adopt the pending epoch and step the intent forward one epoch;
            // the caller persists this and sends one escalated rotate.
            var next = state
            next.endpointEpoch = pending.newEndpointEpoch
            next.endpointFingerprint = pending.newEndpointFingerprint
            next.pendingRotation = .init(
                newEndpointEpoch: pending.newEndpointEpoch + 1,
                newEndpointFingerprint: pending.newEndpointFingerprint
            )
            return .persist(next)
        }
    }

    // MARK: Enrollment outcome

    /// What the caller must do after an enrollment attempt resolves.
    public enum EnrollResolution: Equatable {
        /// Persist this freshly enrolled state (epoch is always 1).
        case persist(BuzzPushInstallationState)
        /// Retry later with backoff; keep whatever state existed.
        case retryLater
        /// Give up on this attest key and generate a new one next attempt.
        case discardKeyAndRetry
    }

    /// Resolve an enrollment attempt. `keyId`/`appProfile` are what was
    /// attested; the handle/epoch/expiry come from the `201` response.
    public static func applyEnroll(
        response: (installationHandle: UUID, endpointEpoch: Int64, expiresAt: Int64)?,
        keyId: String,
        appProfile: String,
        endpointFingerprint: String,
        invalidAttestation: Bool
    ) -> EnrollResolution {
        if let r = response {
            return .persist(
                BuzzPushInstallationState(
                    installationHandle: r.installationHandle,
                    keyId: keyId,
                    appProfile: appProfile,
                    endpointEpoch: r.endpointEpoch,
                    endpointFingerprint: endpointFingerprint,
                    expiresAt: r.expiresAt
                )
            )
        }
        // A rejected attestation on ENROLL means the key/attestation object
        // itself was refused (or the key was already enrolled) — minting a
        // fresh key is the only forward path. Transient failures retry with
        // the same key.
        return invalidAttestation ? .discardKeyAndRetry : .retryLater
    }
}
