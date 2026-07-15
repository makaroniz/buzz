import Foundation
import XCTest

@testable import BuzzPushKit

/// Unit tests for the pure lifecycle decision machine. Each test names the
/// cold-review finding (B2/B4) it pins.
final class BuzzPushLifecycleTests: XCTestCase {
    // MARK: Fixtures

    static let handle = UUID(uuidString: "22222222-2222-4222-8222-222222222222")!
    static let tokenA = BuzzPushInstallationState.fingerprint(
        ofEndpoint: String(repeating: "01", count: 32))
    static let tokenB = BuzzPushInstallationState.fingerprint(
        ofEndpoint: String(repeating: "02", count: 32))
    static let tokenC = BuzzPushInstallationState.fingerprint(
        ofEndpoint: String(repeating: "03", count: 32))
    static let now: Int64 = 1_752_620_000

    func freshState(
        epoch: Int64 = 3,
        fingerprint: String = tokenA,
        expiresAt: Int64 = now + 60 * 24 * 3600,
        pending: BuzzPushInstallationState.PendingRotation? = nil
    ) -> BuzzPushInstallationState {
        BuzzPushInstallationState(
            installationHandle: Self.handle,
            keyId: "qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqo=",
            appProfile: "buzz-ios-production",
            endpointEpoch: epoch,
            endpointFingerprint: fingerprint,
            expiresAt: expiresAt,
            pendingRotation: pending
        )
    }

    // MARK: Token fingerprinting

    func testFingerprintIsCaseInsensitiveOverHex() {
        XCTAssertEqual(
            BuzzPushInstallationState.fingerprint(ofEndpoint: "0A0B0C"),
            BuzzPushInstallationState.fingerprint(ofEndpoint: "0a0b0c")
        )
    }

    // MARK: B4 — rotate only on real token change

    func testNoStateEnrolls() {
        XCTAssertEqual(
            BuzzPushLifecycle.onDeviceToken(state: nil, tokenFingerprint: Self.tokenA, now: Self.now),
            .enroll
        )
    }

    func testRedeliveredTokenIsNoop() {
        // iOS re-delivers the same token every launch; this MUST NOT rotate
        // (B4: unconditional rotate invalidates all delegations per launch).
        XCTAssertEqual(
            BuzzPushLifecycle.onDeviceToken(
                state: freshState(), tokenFingerprint: Self.tokenA, now: Self.now),
            .noop
        )
    }

    func testChangedTokenBeginsRotationWithPersistedIntent() {
        let decision = BuzzPushLifecycle.onDeviceToken(
            state: freshState(), tokenFingerprint: Self.tokenB, now: Self.now)
        guard case let .beginRotation(next) = decision else {
            return XCTFail("expected beginRotation, got \(decision)")
        }
        // Committed fields untouched until the gateway confirms.
        XCTAssertEqual(next.endpointEpoch, 3)
        XCTAssertEqual(next.endpointFingerprint, Self.tokenA)
        XCTAssertEqual(next.pendingRotation,
                       .init(newEndpointEpoch: 4, newEndpointFingerprint: Self.tokenB))
    }

    // MARK: B4 — renewal window

    func testExpiredInstallationReenrolls() {
        XCTAssertEqual(
            BuzzPushLifecycle.onDeviceToken(
                state: freshState(expiresAt: Self.now - 1),
                tokenFingerprint: Self.tokenA, now: Self.now),
            .reenroll
        )
    }

    func testInsideRenewalLeadTimeReenrolls() {
        XCTAssertEqual(
            BuzzPushLifecycle.onDeviceToken(
                state: freshState(expiresAt: Self.now + BuzzPushLifecycle.renewalLeadTimeSeconds - 1),
                tokenFingerprint: Self.tokenA, now: Self.now),
            .reenroll
        )
    }

    func testOutsideRenewalLeadTimeDoesNotReenroll() {
        XCTAssertEqual(
            BuzzPushLifecycle.onDeviceToken(
                state: freshState(expiresAt: Self.now + BuzzPushLifecycle.renewalLeadTimeSeconds + 1),
                tokenFingerprint: Self.tokenA, now: Self.now),
            .noop
        )
    }

    // MARK: B2 — crash window (persist-then-confirm)

    func testSurvivingIntentResumesRotation() {
        let state = freshState(
            pending: .init(newEndpointEpoch: 4, newEndpointFingerprint: Self.tokenB))
        XCTAssertEqual(
            BuzzPushLifecycle.onDeviceToken(
                state: state, tokenFingerprint: Self.tokenB, now: Self.now),
            .resumeRotation(state: state)
        )
    }

    func testTokenMovedAgainDuringInFlightRotationKeepsTargetEpoch() {
        let state = freshState(
            pending: .init(newEndpointEpoch: 4, newEndpointFingerprint: Self.tokenB))
        let decision = BuzzPushLifecycle.onDeviceToken(
            state: state, tokenFingerprint: Self.tokenC, now: Self.now)
        guard case let .beginRotation(next) = decision else {
            return XCTFail("expected beginRotation, got \(decision)")
        }
        // The unconfirmed epoch step is reused; only the destination token moves.
        XCTAssertEqual(next.pendingRotation,
                       .init(newEndpointEpoch: 4, newEndpointFingerprint: Self.tokenC))
    }

    func testSurvivingIntentMatchingCommittedFingerprintStillResumes() {
        // Crash AFTER server commit but BEFORE local commit, then the token
        // reverts (or never actually changed): the intent must still resolve
        // through the rotate path, never be dropped on the floor.
        let state = freshState(
            fingerprint: Self.tokenA,
            pending: .init(newEndpointEpoch: 4, newEndpointFingerprint: Self.tokenA))
        XCTAssertEqual(
            BuzzPushLifecycle.onDeviceToken(
                state: state, tokenFingerprint: Self.tokenA, now: Self.now),
            .resumeRotation(state: state)
        )
    }

    // MARK: Rotate outcomes

    func testRotatedCommitsEpochAndClearsIntent() {
        let state = freshState(
            pending: .init(newEndpointEpoch: 4, newEndpointFingerprint: Self.tokenB))
        let resolution = BuzzPushLifecycle.applyRotate(state: state, outcome: .rotated)
        guard case let .persist(next) = resolution else {
            return XCTFail("expected persist, got \(resolution)")
        }
        XCTAssertEqual(next.endpointEpoch, 4)
        XCTAssertEqual(next.endpointFingerprint, Self.tokenB)
        XCTAssertNil(next.pendingRotation)
    }

    func testTransientRetainsIntent() {
        let state = freshState(
            pending: .init(newEndpointEpoch: 4, newEndpointFingerprint: Self.tokenB))
        XCTAssertEqual(
            BuzzPushLifecycle.applyRotate(state: state, outcome: .transient),
            .retainIntent
        )
    }

    // MARK: B2 — zombie key (backup-restore)

    func testInvalidAttestationClearsAndReenrolls() {
        let state = freshState(
            pending: .init(newEndpointEpoch: 4, newEndpointFingerprint: Self.tokenB))
        XCTAssertEqual(
            BuzzPushLifecycle.applyRotate(state: state, outcome: .invalidAttestation),
            .clearAndReenroll
        )
    }

    // MARK: B2 — epoch desync reconvergence

    func testNotAuthorizedAdoptsPendingEpochAndEscalates() {
        // Ambiguous crash window: the server may have committed epoch 4.
        // First notAuthorized adopts 4 and points a fresh intent at 5.
        let state = freshState(
            pending: .init(newEndpointEpoch: 4, newEndpointFingerprint: Self.tokenB))
        let resolution = BuzzPushLifecycle.applyRotate(state: state, outcome: .notAuthorized)
        guard case let .persist(next) = resolution else {
            return XCTFail("expected persist, got \(resolution)")
        }
        XCTAssertEqual(next.endpointEpoch, 4)
        XCTAssertEqual(next.endpointFingerprint, Self.tokenB)
        XCTAssertEqual(next.pendingRotation,
                       .init(newEndpointEpoch: 5, newEndpointFingerprint: Self.tokenB))
    }

    func testEscalatedNotAuthorizedClearsAndReenrolls() {
        let state = freshState(
            epoch: 4, fingerprint: Self.tokenB,
            pending: .init(newEndpointEpoch: 5, newEndpointFingerprint: Self.tokenB))
        XCTAssertEqual(
            BuzzPushLifecycle.applyRotate(state: state, outcome: .notAuthorized, escalated: true),
            .clearAndReenroll
        )
    }

    func testRotateWithoutIntentIsCorruptState() {
        XCTAssertEqual(
            BuzzPushLifecycle.applyRotate(state: freshState(), outcome: .rotated),
            .clearAndReenroll
        )
    }

    // MARK: Enrollment outcomes

    func testEnrollSuccessPersistsEpochOneState() {
        let resolution = BuzzPushLifecycle.applyEnroll(
            response: (Self.handle, 1, Self.now + 90 * 24 * 3600),
            keyId: "key",
            appProfile: "buzz-ios-production",
            endpointFingerprint: Self.tokenA,
            invalidAttestation: false
        )
        guard case let .persist(state) = resolution else {
            return XCTFail("expected persist, got \(resolution)")
        }
        XCTAssertEqual(state.endpointEpoch, 1)
        XCTAssertEqual(state.endpointFingerprint, Self.tokenA)
        XCTAssertNil(state.pendingRotation)
    }

    func testEnrollInvalidAttestationDiscardsKey() {
        XCTAssertEqual(
            BuzzPushLifecycle.applyEnroll(
                response: nil, keyId: "key", appProfile: "buzz-ios-production",
                endpointFingerprint: Self.tokenA, invalidAttestation: true),
            .discardKeyAndRetry
        )
    }

    func testEnrollTransientRetries() {
        XCTAssertEqual(
            BuzzPushLifecycle.applyEnroll(
                response: nil, keyId: "key", appProfile: "buzz-ios-production",
                endpointFingerprint: Self.tokenA, invalidAttestation: false),
            .retryLater
        )
    }

    // MARK: Store round-trip (codec + in-memory store)

    func testStateStoreRoundTrip() throws {
        let store = InMemoryPushStateStore()
        XCTAssertNil(try store.load())
        let state = freshState(
            pending: .init(newEndpointEpoch: 4, newEndpointFingerprint: Self.tokenB))
        try store.save(state)
        XCTAssertEqual(try store.load(), state)
        try store.clear()
        XCTAssertNil(try store.load())
    }

    func testCorruptEnvelopeThrowsCorrupt() {
        XCTAssertThrowsError(try BuzzPushStateCodec.decode(Data("{}".utf8))) { error in
            XCTAssertEqual(error as? BuzzPushStateStoreError, .corrupt)
        }
        XCTAssertThrowsError(try BuzzPushStateCodec.decode(Data("not json".utf8))) { error in
            XCTAssertEqual(error as? BuzzPushStateStoreError, .corrupt)
        }
    }

    func testFutureVersionEnvelopeIsCorruptNotCrash() throws {
        let state = freshState()
        var object = try JSONSerialization.jsonObject(
            with: BuzzPushStateCodec.encode(state)) as! [String: Any]
        object["version"] = 999
        let data = try JSONSerialization.data(withJSONObject: object)
        XCTAssertThrowsError(try BuzzPushStateCodec.decode(data)) { error in
            XCTAssertEqual(error as? BuzzPushStateStoreError, .corrupt)
        }
    }
}
