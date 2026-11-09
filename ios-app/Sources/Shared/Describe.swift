// UI wording for sync errors (the web app's describe.ts).
import Foundation

extension SyncError {
    /** An error worth an amber mark; `empty` is a normal setup state. */
    var isReal: Bool { kind != .empty }

    /** `short` fits the freshness line, `long` a sheet. `hasSnapshot`: a last
     *  good copy is on screen (network errors say so). */
    func describe(deviceName: String? = nil, hasSnapshot: Bool = false) -> (short: String, long: String) {
        let mac = deviceName.flatMap { $0.isEmpty ? nil : $0 } ?? "the main Mac"
        switch kind {
        case .network:
            let then = hasSnapshot ? "Showing the last copy." : "Check your connection and try again."
            return ("Offline", "\(message) \(then)")
        case .empty:
            return ("Waiting for first upload", "Nothing uploaded yet. Turn on “This Mac uploads” on \(mac).")
        case .forbidden:
            return ("Pairing code not accepted", "\(message) Pair again with a current code.")
        case .decryptFailed:
            return ("Pairing code doesn't match", "The pairing code doesn't match the data on the server.")
        case .badCode:
            return ("Invalid pairing code", message)
        case .badSnapshot:
            return newerSchema
                ? ("Update needed", "\(message) Update Token Counter here to show it.")
                : ("Unreadable data", "\(message) Update Token Counter on both devices.")
        }
    }
}
