// The pairing code lives in the Keychain, readable after the first unlock
// (so a widget can refresh while the phone is locked) and never synced off
// this device. It is stored in the App Group's access group so the widget
// extension can read it; if that group isn't available (a build signed
// without it) it falls back to the app's own group and only the app sees it.
import Foundation
import Security

/** The App Group shared by the app and its widgets (Info.plist `TCAppGroup`). */
enum AppGroup {
    static let id: String? = (Bundle.main.object(forInfoDictionaryKey: "TCAppGroup") as? String).flatMap { $0.isEmpty ? nil : $0 }

    static var container: URL? {
        id.flatMap { FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: $0) }
    }
}

enum CodeStore {
    private static let service = "tokencounter.sync"
    private static let account = "pairing-code"

    // No access group: matches the item in whichever group it is in.
    private static func query() -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
    }

    static func load() -> String? {
        var q = query()
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var out: CFTypeRef?
        guard SecItemCopyMatching(q as CFDictionary, &out) == errSecSuccess, let data = out as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }

    @discardableResult
    static func save(_ code: String) -> Bool {
        delete()
        var add = query()
        add[kSecValueData as String] = Data(code.utf8)
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        if let group = AppGroup.id {
            var shared = add
            shared[kSecAttrAccessGroup as String] = group
            if SecItemAdd(shared as CFDictionary, nil) == errSecSuccess { return true }
        }
        return SecItemAdd(add as CFDictionary, nil) == errSecSuccess
    }

    static func delete() {
        SecItemDelete(query() as CFDictionary)
    }

    /** Move a code saved outside the shared group (by a build without one)
     *  into it, so the widgets can read it. */
    static func shareWithWidgets() {
        guard let group = AppGroup.id else { return }
        var q = query()
        q[kSecAttrAccessGroup as String] = group
        if SecItemCopyMatching(q as CFDictionary, nil) == errSecSuccess { return }
        if let code = load() { save(code) }
    }
}
