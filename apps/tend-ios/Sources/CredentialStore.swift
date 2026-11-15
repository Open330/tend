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

    @discardableResult
    static func save(_ pairing: Pairing) -> Bool {
        guard let data = try? JSONEncoder().encode(pairing) else { return false }
        clear()
        var item = query
        item[kSecValueData as String] = data
        item[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
        return SecItemAdd(item as CFDictionary, nil) == errSecSuccess
    }

    static func clear() {
        SecItemDelete(query as CFDictionary)
    }
}
