// The synced snapshot for one pairing code, like the web app's remote.ts:
//   - polls tc_head every 30 s while the app is in the foreground, and right
//     away when it comes back (at most every 5 s);
//   - downloads (tc_get) only when the version moved, or when forced;
//   - remembers a version that failed to open and doesn't download it again
//     until the version moves or the user forces a refresh;
//   - keeps the last good snapshot on disk for instant, offline display.
import Foundation
import Observation
import WidgetKit

@MainActor
@Observable
final class RemoteStore {
    private(set) var snapshot: Snapshot?
    private(set) var version: Int?
    /** When the server last confirmed `version` is current. */
    private(set) var fetchedAt: Date?
    /** Last attempt's error; the snapshot (if any) stays alongside it. */
    private(set) var error: SyncError?
    private(set) var refreshing = false

    let keys: SyncKeys
    private let client: SyncClient
    @ObservationIgnored private var failed: (version: Int, error: SyncError)?
    @ObservationIgnored private var inFlight: Task<Void, Never>?
    @ObservationIgnored private var queued: Bool?
    @ObservationIgnored private var lastAttempt = Date.distantPast
    @ObservationIgnored private var poller: Task<Void, Never>?
    /** Set when the phone unpairs or pairs again: a request still in flight
     *  must not put a snapshot back on disk afterwards. */
    @ObservationIgnored private var disposed = false

    static let pollInterval: Duration = .seconds(30)
    static let minRecheck: TimeInterval = 5

    init(keys: SyncKeys) {
        self.keys = keys
        client = SyncClient(keys: keys)
        if let c = SnapshotCache.read(), c.channel == keys.channel {
            snapshot = c.snapshot
            version = c.version
            fetchedAt = c.fetchedAt
        }
    }

    /** A snapshot fetched while pairing, so it isn't downloaded twice. */
    func seed(_ snapshot: Snapshot, version: Int) {
        let now = Date()
        self.snapshot = snapshot
        self.version = version
        fetchedAt = now
        error = nil
        SnapshotCache.write(CachedSnapshot(channel: keys.channel, version: version, fetchedAt: now, snapshot: snapshot))
        WidgetCenter.shared.reloadAllTimelines()
    }

    /** Foreground: refresh now and every 30 s. */
    func start() {
        guard poller == nil, !disposed else { return }
        poller = Task { [weak self] in
            await self?.refreshIfDue()
            while !Task.isCancelled {
                try? await Task.sleep(for: Self.pollInterval)
                if Task.isCancelled { break }
                await self?.refresh()
            }
        }
    }

    func stop() {
        poller?.cancel()
        poller = nil
    }

    /** Stop for good (unpair, or replaced by another pairing). */
    func dispose() {
        disposed = true
        stop()
    }

    private func refreshIfDue() async {
        if Date().timeIntervalSince(lastAttempt) >= Self.minRecheck { await refresh() }
    }

    /** Forget the snapshot on screen and its cache, then download again. */
    func clear() {
        SnapshotCache.clear()
        failed = nil
        snapshot = nil
        version = nil
        fetchedAt = nil
        error = nil
        Task { await refresh(force: true) }
    }

    /** head → (get + open when the version moved, or when forced). A call
     *  while one is running queues exactly one more run. */
    func refresh(force: Bool = false) async {
        if let running = inFlight {
            queued = (queued ?? false) || force
            await running.value
            return
        }
        let task = Task { await run(force: force) }
        inFlight = task
        await task.value
        inFlight = nil
        if let again = queued {
            queued = nil
            if !disposed { await refresh(force: again) }
        }
    }

    private func run(force: Bool) async {
        lastAttempt = Date()
        refreshing = true
        defer { refreshing = false }
        do {
            let h = try await client.head()
            if disposed { return }
            if h.version == 0 { throw SyncError(kind: .empty, message: "Nothing has been uploaded yet.") }
            if force || h.version != failed?.version { failed = nil }
            // The same version already failed to open: show its error again
            // (next to the last good snapshot, which is stale, so fetchedAt
            // stays) and don't download it again.
            if let f = failed {
                error = f.error
                return
            }
            if !force, snapshot != nil, version == h.version {
                fetchedAt = Date()
                error = nil
                return
            }
            let g = try await client.get()
            if disposed { return }
            let snap: Snapshot
            do {
                snap = try SyncProtocol.openEnvelope(g.payload, keys: keys)
            } catch {
                let e = error as? SyncError ?? SyncError(kind: .badSnapshot, message: error.localizedDescription)
                failed = (g.head.version, e)
                throw e
            }
            let now = Date()
            snapshot = snap
            version = g.head.version
            fetchedAt = now
            error = nil
            SnapshotCache.write(CachedSnapshot(channel: keys.channel, version: g.head.version, fetchedAt: now, snapshot: snap))
            // The widgets read that cache; show the new numbers there too.
            WidgetCenter.shared.reloadAllTimelines()
        } catch is CancellationError {
            return
        } catch let e as SyncError {
            if !disposed { error = e }
        } catch {
            if !disposed { self.error = SyncError(kind: .network, message: error.localizedDescription) }
        }
    }
}
