import SwiftUI
import WidgetKit

@main
struct TokenCounterApp: App {
    @State private var model = AppModel()

    var body: some Scene {
        WindowGroup {
            RootView().environment(model)
        }
    }
}

struct RootView: View {
    @Environment(AppModel.self) private var model
    @AppStorage("theme") private var themeMode = "auto"

    var body: some View {
        content
            .preferredColorScheme(themeMode == "light" ? .light : themeMode == "dark" ? .dark : nil)
            .tint(Color(hex: "#C2603A"))
    }

    @ViewBuilder private var content: some View {
        #if DEBUG && targetEnvironment(simulator)
        if let preview = DebugPreview.data {
            if let mode = DebugPreview.widgets, let snapshot = preview.snapshot {
                DebugWidgetGallery(snapshot: snapshot, stale: mode == "stale")
            } else {
                DashboardView(data: preview, actions: DashboardActions())
            }
        } else if DebugPreview.forcePairing {
            PairingView()
        } else {
            paired.task { await DebugPreview.pairFromFileIfAsked(model) }
        }
        #else
        paired
        #endif
    }

    @ViewBuilder private var paired: some View {
        if let store = model.store {
            DashboardScreen(store: store).id(ObjectIdentifier(store))
        } else {
            PairingView()
        }
    }
}

/** The dashboard bound to a live store; polls while the app is active. */
struct DashboardScreen: View {
    let store: RemoteStore
    @Environment(AppModel.self) private var model
    @Environment(\.scenePhase) private var phase

    var body: some View {
        DashboardView(
            data: DashboardData(
                snapshot: store.snapshot,
                version: store.version,
                fetchedAt: store.fetchedAt,
                error: store.error,
                refreshing: store.refreshing,
                serverHost: model.serverHost
            ),
            actions: DashboardActions(
                refresh: { [store] force in await store.refresh(force: force) },
                unpair: { [model] in model.unpair() }
            )
        )
        .onChange(of: phase, initial: true) { _, p in
            if p == .active { store.start() } else if p == .background { store.stop() }
        }
        .onDisappear { store.stop() }
    }
}

#if DEBUG && targetEnvironment(simulator)
/** Simulator-only hooks for looking at states without a server (set with
 *  `SIMCTL_CHILD_<NAME>=… xcrun simctl launch …`):
 *    TC_PREVIEW_SNAPSHOT=<snapshot.json on the Mac>  show that snapshot
 *    TC_PREVIEW_AGE=<seconds>      pretend it was generated that long ago
 *    TC_PREVIEW_ERROR=<kind>       network | empty | forbidden | decryptFailed | badSnapshot
 *    TC_PREVIEW_NOSNAP=1           no snapshot (placeholder states)
 *    TC_PREVIEW_PAIRING=1          the pairing screen
 *    TC_PREVIEW_WIDGETS=1|stale    with a snapshot: every widget family at its real size
 *    TC_DEBUG_CODE_FILE=<file>     pair with the code in that file (never logged) */
@MainActor
enum DebugPreview {
    private static let env = ProcessInfo.processInfo.environment

    static let forcePairing = env["TC_PREVIEW_PAIRING"] == "1"
    static let widgets = env["TC_PREVIEW_WIDGETS"]

    static let data: DashboardData? = {
        let error = env["TC_PREVIEW_ERROR"].flatMap(SyncErrorKind.init(rawValue:)).map { kind in
            SyncError(kind: kind, message: kind == .network ? "Can't reach the sync server." : "The sync server rejected this pairing code.")
        }
        if env["TC_PREVIEW_NOSNAP"] == "1" {
            return DashboardData(fetchedAt: error == nil ? nil : Date(), error: error, refreshing: env["TC_PREVIEW_ERROR"] == nil)
        }
        guard let path = env["TC_PREVIEW_SNAPSHOT"], let raw = FileManager.default.contents(atPath: path),
              var json = try? JSONSerialization.jsonObject(with: raw) as? [String: Any]
        else { return nil }
        if let inner = json["json"] as? [String: Any] { json = inner } // a test fixture wraps it
        if let age = env["TC_PREVIEW_AGE"].flatMap(Double.init) {
            json["generatedAt"] = ISO8601DateFormatter().string(from: Date().addingTimeInterval(-age))
        }
        guard let bytes = try? JSONSerialization.data(withJSONObject: json), let snap = try? Snapshot.parse(bytes) else { return nil }
        return DashboardData(snapshot: snap, version: 7, fetchedAt: Date(), error: error, serverHost: "example.supabase.co")
    }()

    static func pairFromFileIfAsked(_ model: AppModel) async {
        guard !model.isPaired, let path = env["TC_DEBUG_CODE_FILE"], let raw = FileManager.default.contents(atPath: path),
              let code = String(data: raw, encoding: .utf8)
        else { return }
        try? await model.pair(code)
    }
}

/** The widget views laid out at the sizes iOS gives them on a large iPhone
 *  (the Lock Screen's own vibrant rendering is not reproduced here). */
struct DebugWidgetGallery: View {
    let snapshot: Snapshot
    let stale: Bool

    var body: some View {
        let now = Date()
        let checked = stale ? now.addingTimeInterval(-3 * 3600) : now
        let entry = UsageEntry(date: now, snapshot: snapshot, paired: true, range: .today, checkedAt: checked)
        let unpaired = UsageEntry(date: now, snapshot: nil, paired: false, range: .today, checkedAt: now)
        ScrollView {
            VStack(spacing: 14) {
                HStack(spacing: 14) {
                    home(entry, .systemSmall, 170, 170)
                    home(unpaired, .systemSmall, 170, 170)
                }
                home(entry, .systemMedium, 364, 170)
                VStack(spacing: 12) {
                    UsageWidgetView(entry: entry, family: .accessoryInline).font(.system(size: 15, weight: .medium))
                    HStack(spacing: 12) {
                        lock(entry, .accessoryRectangular, 172, 76)
                        lock(entry, .accessoryCircular, 76, 76)
                        lock(unpaired, .accessoryCircular, 76, 76)
                    }
                    lock(unpaired, .accessoryRectangular, 172, 76)
                }
                .foregroundStyle(.white)
                .environment(\.colorScheme, .dark)
                .padding(16)
                .frame(maxWidth: .infinity)
                .background(LinearGradient(colors: [Color(hex: "#5B4636"), Color(hex: "#1E2238")], startPoint: .top, endPoint: .bottom), in: .rect(cornerRadius: 24))
            }
            .padding(16)
        }
        .background(Color(white: 0.5))
    }

    private func home(_ e: UsageEntry, _ family: WidgetFamily, _ w: CGFloat, _ h: CGFloat) -> some View {
        UsageWidgetView(entry: e, family: family)
            .padding(16)
            .frame(width: w, height: h)
            .background(.background, in: .rect(cornerRadius: 24))
    }

    private func lock(_ e: UsageEntry, _ family: WidgetFamily, _ w: CGFloat, _ h: CGFloat) -> some View {
        UsageWidgetView(entry: e, family: family)
            .frame(width: w, height: h)
            .overlay(RoundedRectangle(cornerRadius: family == .accessoryCircular ? h / 2 : 10).stroke(.white.opacity(0.25), style: StrokeStyle(lineWidth: 0.5, dash: [3])))
    }
}
#endif
