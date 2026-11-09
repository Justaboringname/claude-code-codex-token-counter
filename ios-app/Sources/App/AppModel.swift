// Pairing state: the stored code and its RemoteStore.
import Foundation
import Observation
import WidgetKit

@MainActor
@Observable
final class AppModel {
    private(set) var store: RemoteStore?
    /** "abcd.supabase.co", for the sync sheet. */
    private(set) var serverHost: String?

    init() {
        if let code = CodeStore.load(), let keys = try? SyncProtocol.deriveKeys(code) {
            CodeStore.shareWithWidgets()
            store = RemoteStore(keys: keys)
            serverHost = URL(string: keys.url)?.host()
        } else {
            // Not paired: nothing may be left over from an earlier pairing.
            SnapshotCache.clear()
        }
    }

    var isPaired: Bool { store != nil }

    /** Check a code against the server (head, then the first snapshot if any)
     *  and keep it. Throws SyncError: badCode / forbidden / network / decryptFailed / badSnapshot. */
    func pair(_ raw: String) async throws {
        let code = SyncProtocol.extractCode(raw)
        let keys = try SyncProtocol.deriveKeys(code)
        let client = SyncClient(keys: keys)
        let head = try await client.head()
        var first: (snapshot: Snapshot, version: Int)?
        if head.version > 0 {
            do {
                first = try await client.fetchSnapshot()
            } catch let e as SyncError where e.kind == .empty {
                first = nil
            }
        }
        guard CodeStore.save(code) else {
            throw SyncError(kind: .badCode, message: "Couldn't save the pairing code on this iPhone.")
        }
        store?.dispose()
        SnapshotCache.clear()
        let s = RemoteStore(keys: keys)
        if let first { s.seed(first.snapshot, version: first.version) }
        store = s
        serverHost = URL(string: keys.url)?.host()
        // Also when nothing was uploaded yet: the widgets must stop saying "pair".
        WidgetCenter.shared.reloadAllTimelines()
    }

    func unpair() {
        store?.dispose()
        store = nil
        serverHost = nil
        CodeStore.delete()
        SnapshotCache.clear()
        WidgetCenter.shared.reloadAllTimelines()
    }
}
