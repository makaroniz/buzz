import CryptoKit
import Foundation

/// Durable client-side record of one NIP-PL push installation.
///
/// This is the state the cold review's B2/B4 findings are about: it must
/// survive relaunch, must NOT survive backup-restore onto a new device
/// (App Attest keys are device-bound, so a restored copy is a zombie), and
/// must stay epoch-consistent with the gateway across a crash between
/// `POST /v1/installations/endpoint` succeeding and the local commit.
///
/// The two-phase `pendingRotation` field is the crash-window answer: the
/// intent is persisted *before* the rotate request is sent, and cleared only
/// after the outcome is known. `BuzzPushLifecycle` turns a surviving intent
/// back into a resumable decision.
public struct BuzzPushInstallationState: Codable, Equatable {
    /// Two-phase commit record for an in-flight endpoint rotation.
    public struct PendingRotation: Codable, Equatable {
        /// `endpoint_epoch + 1` at the time the intent was written.
        public var newEndpointEpoch: Int64
        /// Fingerprint of the APNs token the rotation is moving to.
        public var newEndpointFingerprint: String

        public init(newEndpointEpoch: Int64, newEndpointFingerprint: String) {
            self.newEndpointEpoch = newEndpointEpoch
            self.newEndpointFingerprint = newEndpointFingerprint
        }
    }

    /// Gateway-issued installation handle (`installation_handle`).
    public var installationHandle: UUID
    /// App Attest key identifier (standard base64), device-bound.
    public var keyId: String
    /// Registered profile, e.g. `buzz-ios-production`.
    public var appProfile: String
    /// Current committed endpoint epoch (matches gateway on the happy path).
    public var endpointEpoch: Int64
    /// SHA-256 hex of the lowercase-hex APNs token currently enrolled.
    /// Stored instead of the raw token so change detection never requires
    /// retaining the token itself.
    public var endpointFingerprint: String
    /// Installation expiry (unix seconds), as returned by enrollment.
    public var expiresAt: Int64
    /// In-flight rotation intent, if a rotate was started but not confirmed.
    public var pendingRotation: PendingRotation?

    public init(
        installationHandle: UUID,
        keyId: String,
        appProfile: String,
        endpointEpoch: Int64,
        endpointFingerprint: String,
        expiresAt: Int64,
        pendingRotation: PendingRotation? = nil
    ) {
        self.installationHandle = installationHandle
        self.keyId = keyId
        self.appProfile = appProfile
        self.endpointEpoch = endpointEpoch
        self.endpointFingerprint = endpointFingerprint
        self.expiresAt = expiresAt
        self.pendingRotation = pendingRotation
    }

    /// Canonical fingerprint of an APNs token: SHA-256 hex over the ASCII
    /// bytes of the lowercase-hex token string (the same representation the
    /// transcripts carry). Purely local — this is NOT the gateway's
    /// `(app_profile, SHA-256(token))` uniqueness fingerprint.
    public static func fingerprint(ofEndpoint endpointHex: String) -> String {
        SHA256.hash(data: Data(endpointHex.lowercased().utf8))
            .map { String(format: "%02x", $0) }
            .joined()
    }
}
