import Foundation
#if canImport(Security)
import Security
#endif

/// Persistence seam for `BuzzPushInstallationState`.
///
/// The production implementation is `KeychainPushStateStore`; tests use
/// `InMemoryPushStateStore`. Kept deliberately tiny: load / save / clear.
public protocol BuzzPushStateStore {
    func load() throws -> BuzzPushInstallationState?
    func save(_ state: BuzzPushInstallationState) throws
    func clear() throws
}

public enum BuzzPushStateStoreError: Error, Equatable {
    /// Underlying Keychain call failed with the given OSStatus.
    case keychain(OSStatus)
    /// Persisted bytes did not decode; the caller should treat this as
    /// no-state (clear + re-enroll) rather than crash.
    case corrupt
}

/// JSON codec shared by every store. Versioned envelope so a future schema
/// change can migrate instead of tripping `corrupt`.
enum BuzzPushStateCodec {
    struct Envelope: Codable {
        var version: Int
        var state: BuzzPushInstallationState
    }

    static let currentVersion = 1

    static func encode(_ state: BuzzPushInstallationState) throws -> Data {
        try JSONEncoder().encode(Envelope(version: currentVersion, state: state))
    }

    static func decode(_ data: Data) throws -> BuzzPushInstallationState {
        guard let envelope = try? JSONDecoder().decode(Envelope.self, from: data),
              envelope.version == currentVersion
        else {
            throw BuzzPushStateStoreError.corrupt
        }
        return envelope.state
    }
}

/// Test double / non-Darwin fallback.
public final class InMemoryPushStateStore: BuzzPushStateStore {
    private var data: Data?

    public init() {}

    public func load() throws -> BuzzPushInstallationState? {
        try data.map(BuzzPushStateCodec.decode)
    }

    public func save(_ state: BuzzPushInstallationState) throws {
        data = try BuzzPushStateCodec.encode(state)
    }

    public func clear() throws {
        data = nil
    }
}

#if canImport(Security)
/// Keychain-backed store (kSecClassGenericPassword).
///
/// Attribute choices are load-bearing (cold review B2):
///
/// - `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`:
///   * *AfterFirstUnlock* — the Notification Service Extension and background
///     launches must read this while the device is locked (post-first-unlock),
///     so `WhenUnlocked` is wrong.
///   * *ThisDeviceOnly* — App Attest keys are device-bound. If this record
///     migrated through backup/restore to a new device it would name a key
///     that can never sign again (the zombie-installation failure). Excluding
///     it from device transfers kills that class at the storage layer;
///     `BuzzPushLifecycle`'s `invalidAttestation → clearAndReenroll` remains
///     as defense in depth.
/// - `kSecAttrAccessGroup` (optional) shares the item with the NSE via an
///   App Group / keychain access group.
///
/// NOTE: this state is *authorization bookkeeping*, not key material — the
/// actual private key lives in the Secure Enclave under App Attest.
public final class KeychainPushStateStore: BuzzPushStateStore {
    private let service: String
    private let account: String
    private let accessGroup: String?

    public init(
        service: String = "xyz.buzz.push.installation",
        account: String = "nip-pl-state",
        accessGroup: String? = nil
    ) {
        self.service = service
        self.account = account
        self.accessGroup = accessGroup
    }

    private func baseQuery() -> [String: Any] {
        var q: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        if let accessGroup {
            q[kSecAttrAccessGroup as String] = accessGroup
        }
        return q
    }

    public func load() throws -> BuzzPushInstallationState? {
        var q = baseQuery()
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var out: CFTypeRef?
        let status = SecItemCopyMatching(q as CFDictionary, &out)
        switch status {
        case errSecSuccess:
            guard let data = out as? Data else { throw BuzzPushStateStoreError.corrupt }
            return try BuzzPushStateCodec.decode(data)
        case errSecItemNotFound:
            return nil
        default:
            throw BuzzPushStateStoreError.keychain(status)
        }
    }

    public func save(_ state: BuzzPushInstallationState) throws {
        let data = try BuzzPushStateCodec.encode(state)
        var add = baseQuery()
        add[kSecValueData as String] = data
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        let status = SecItemAdd(add as CFDictionary, nil)
        if status == errSecDuplicateItem {
            let update: [String: Any] = [
                kSecValueData as String: data,
                kSecAttrAccessible as String:
                    kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
            ]
            let updateStatus = SecItemUpdate(baseQuery() as CFDictionary, update as CFDictionary)
            guard updateStatus == errSecSuccess else {
                throw BuzzPushStateStoreError.keychain(updateStatus)
            }
        } else if status != errSecSuccess {
            throw BuzzPushStateStoreError.keychain(status)
        }
    }

    public func clear() throws {
        let status = SecItemDelete(baseQuery() as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw BuzzPushStateStoreError.keychain(status)
        }
    }
}
#endif
