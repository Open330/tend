// Where the device token lives: the Keychain, on this device only.
//
// `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`: never in iCloud Keychain,
// never restored to another phone from a backup, unreadable while the phone
// is locked. The token is never written to UserDefaults or logged; the
// dashboard address is not a secret and sits next to it in the item.
import Foundation
import Security

struct Pairing: Codable, Hashable, Sendable {
    let origin: ServerOrigin
    let device: PairedDevice
    let token: String
    /// What the dashboard issued; a pairing kept by build 1 has none and
    /// may only decide.
    var scope: DeviceScope

    init(origin: ServerOrigin, device: PairedDevice, token: String, scope: DeviceScope = .decide) {
        self.origin = origin
        self.device = device
        self.token = token
        self.scope = scope
    }

    enum CodingKeys: String, CodingKey { case origin, device, token, scope }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        origin = try values.decode(ServerOrigin.self, forKey: .origin)
        device = try values.decode(PairedDevice.self, forKey: .device)
        token = try values.decode(String.self, forKey: .token)
        scope = (try? values.decodeIfPresent(DeviceScope.self, forKey: .scope)) ?? .decide
    }
}

enum CredentialStore {
    private static let service = "dev.tend.ios.device"
    private static let account = "pairing"

    private static var query: [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
    }

    static func load() -> Pairing? {
        var item = query
        item[kSecReturnData as String] = true
        item[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        guard SecItemCopyMatching(item as CFDictionary, &result) == errSecSuccess,
              let data = result as? Data
        else { return nil }
        return try? JSONDecoder().decode(Pairing.self, from: data)
    }

    #if DEBUG
    /// Tests: make the next saves fail, as a full or locked Keychain would.
    nonisolated(unsafe) static var failSaves = false
    #endif

    /// Replace the stored pairing in one step: a failed write leaves the
    /// one stored before in place, never nothing.
    @discardableResult
    static func save(_ pairing: Pairing) -> Bool {
        #if DEBUG
        if failSaves { return false }
        #endif
        guard let data = try? JSONEncoder().encode(pairing) else { return false }
        let update: [String: Any] = [
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
        ]
        let updated = SecItemUpdate(query as CFDictionary, update as CFDictionary)
        if updated == errSecSuccess { return true }
        guard updated == errSecItemNotFound else { return false }
        var item = query
        item.merge(update) { $1 }
        return SecItemAdd(item as CFDictionary, nil) == errSecSuccess
    }

    static func clear() {
        SecItemDelete(query as CFDictionary)
    }
}
