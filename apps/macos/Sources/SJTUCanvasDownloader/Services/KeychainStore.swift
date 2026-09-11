import Foundation
import Security

/// The key that protects the saved Canvas login, a generic password in the
/// login keychain (service "SJTU Canvas Downloader", account "session-key").
/// The engine encrypts the login cookies with it and only ever receives it
/// in memory; without it the login lasts until the app quits.
enum KeychainStore {
    private static let service = "SJTU Canvas Downloader"
    private static let account = "session-key"

    /// The stored key, or a new one saved now; nil if the keychain refuses.
    static func sessionKey() -> String? {
        if let environment = ProcessInfo.processInfo.environment["SJTU_CANVAS_SESSION_KEY"], !environment.isEmpty {
            return environment
        }
        if let existing = read() {
            return existing
        }
        var bytes = [UInt8](repeating: 0, count: 32)
        guard SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes) == errSecSuccess else { return nil }
        let key = Data(bytes).base64EncodedString()
        var item = baseQuery
        item[kSecValueData as String] = Data(key.utf8)
        item[kSecAttrLabel as String] = "SJTU Canvas Downloader 登录密钥"
        guard SecItemAdd(item as CFDictionary, nil) == errSecSuccess else { return nil }
        return read() == key ? key : nil
    }

    private static func read() -> String? {
        var query = baseQuery
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess,
              let data = result as? Data,
              let key = String(data: data, encoding: .utf8),
              !key.isEmpty
        else {
            return nil
        }
        return key
    }

    private static var baseQuery: [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
    }
}
