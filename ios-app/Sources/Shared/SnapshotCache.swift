// The last good snapshot, kept for instant display and offline use (the web
// app's `sync-snapshot-v1`). Tagged with its channel so a re-pair never shows
// another channel's numbers, and validated again when read back: a cache from
// an older build must not be able to break every launch.
import Foundation

struct CachedSnapshot: Sendable {
    var channel: String
    var version: Int
    var fetchedAt: Date
    var snapshot: Snapshot
}

enum SnapshotCache {
    /** In the App Group container when there is one, so the widgets can show
     *  the last snapshot without a network round trip. */
    static var fileURL: URL {
        let dir = AppGroup.container
            ?? (try? FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true))
            ?? FileManager.default.temporaryDirectory
        return dir.appendingPathComponent("sync-snapshot-v1.json")
    }

    // The snapshot is stored as the exact bytes that were validated (base64),
    // never re-serialized: JSONSerialization reads a literal like -1e999 as
    // -infinity and then raises an uncatchable exception when asked to write it.
    static func read(url: URL = fileURL) -> CachedSnapshot? {
        guard let data = try? Data(contentsOf: url) else { return nil }
        do {
            guard let o = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let channel = o["channel"] as? String,
                  let version = JSONValue.number(o["version"]).flatMap({ Int(exactly: $0) }),
                  let fetchedAt = JSONValue.number(o["fetchedAt"]),
                  let bytes = (o["snapshot"] as? String).flatMap({ Data(base64Encoded: $0) })
            else { throw CocoaError(.fileReadCorruptFile) }
            return CachedSnapshot(
                channel: channel,
                version: version,
                fetchedAt: Date(timeIntervalSince1970: fetchedAt / 1000),
                snapshot: try Snapshot.parse(bytes)
            )
        } catch {
            clear(url: url)
            return nil
        }
    }

    static func write(_ c: CachedSnapshot, url: URL = fileURL) {
        let o: [String: Any] = [
            "channel": c.channel,
            "version": c.version,
            "fetchedAt": (c.fetchedAt.timeIntervalSince1970 * 1000).rounded(),
            "snapshot": c.snapshot.source.base64EncodedString(),
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: o) else { return }
        try? data.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
    }

    static func clear(url: URL = fileURL) {
        try? FileManager.default.removeItem(at: url)
    }
}
