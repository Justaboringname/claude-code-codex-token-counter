// Home-screen and Lock-Screen widgets: spend for one range plus the Claude and
// Codex limit meters, from the same synced snapshot as the app. The views are
// in Sources/Shared/WidgetViews.swift.
//
// The widget asks to be refreshed every 15 minutes; iOS decides the real
// moments (usually 15–60 minutes apart) and the app reloads it whenever it
// gets new numbers. A refresh reads the pairing code from the shared Keychain
// group, asks the server for the current version and downloads only when it
// moved. Without a network it shows the snapshot the app or an earlier
// refresh cached.
import AppIntents
import SwiftUI
import WidgetKit

@main
struct TokensWidgetBundle: WidgetBundle {
    var body: some Widget {
        UsageWidget()
    }
}

// MARK: configuration

enum RangeChoice: String, AppEnum {
    case session, today, week, month, all

    static let typeDisplayRepresentation: TypeDisplayRepresentation = "Range"
    static let caseDisplayRepresentations: [RangeChoice: DisplayRepresentation] = [
        .session: "Current session",
        .today: "Today",
        .week: "This week",
        .month: "This month",
        .all: "All time",
    ]

    var range: UsageRange { UsageRange(rawValue: rawValue) ?? .today }
}

struct RangeIntent: WidgetConfigurationIntent {
    static let title: LocalizedStringResource = "Token Counter"
    static let description = IntentDescription("Choose which range the spend is for.")

    @Parameter(title: "Range", default: .today)
    var range: RangeChoice
}

// MARK: data

enum WidgetData {
    static let refreshEvery: TimeInterval = 15 * 60
    /** How far ahead limit-window resets get their own timeline entry (iOS
     *  may not reload the widget for a good while). */
    static let resetHorizon: TimeInterval = 12 * 3600

    struct Loaded {
        var snapshot: Snapshot?
        var paired: Bool
        var checkedAt: Date
    }

    /** Made-up numbers for the widget gallery, dated now so they read as fresh. */
    static func sample(now: Date) -> Snapshot? {
        let json = #"""
        {"schema":1,"generatedAt":"\#(ISO8601DateFormatter().string(from: now))","device":{"name":"Mac"},"usage":{"today":{"rangeLabel":"Today",
        "totalCost":42.5,"totalTokens":61234,"deltaPct":12,"sparkline":[1,2,1.5,3,2.2,4,3.1,2.4,3.8,5,4.2,3],
        "sparklineCodex":[0.4,0.3,0.5,0.4,0.6,0.5,0.4,0.6,0.5,0.7,0.6,0.5],"byModel":{},"byTool":[]}},
        "claudeQuota":{"fiveHour":{"usedPct":38,"resetsAt":null},"weekly":{"usedPct":54,"resetsAt":null},"status":"ok","fetchedAt":""},
        "codexQuota":{"fiveHour":{"usedPct":12,"resetsAt":null},"weekly":{"usedPct":61,"resetsAt":null},"status":"ok","fetchedAt":""}}
        """#
        return try? Snapshot.parse(Data(json.utf8))
    }

    private static func keys() -> SyncKeys? {
        CodeStore.load().flatMap { try? SyncProtocol.deriveKeys($0) }
    }

    private static func cache(for keys: SyncKeys) -> CachedSnapshot? {
        SnapshotCache.read().flatMap { $0.channel == keys.channel ? $0 : nil }
    }

    /** What is on disk, without asking the server (gallery, transient previews). */
    static func cached() -> Loaded {
        guard let keys = keys() else { return Loaded(snapshot: nil, paired: false, checkedAt: Date()) }
        let c = cache(for: keys)
        return Loaded(snapshot: c?.snapshot, paired: true, checkedAt: c?.fetchedAt ?? Date())
    }

    /** The newest snapshot: the server's when it can be reached, else the cached one. */
    static func load() async -> Loaded {
        let now = Date()
        guard let keys = keys() else { return Loaded(snapshot: nil, paired: false, checkedAt: now) }
        let c = cache(for: keys)
        do {
            let client = SyncClient(keys: keys, timeout: 10)
            let head = try await client.head()
            if head.version == 0 || c?.version == head.version { return Loaded(snapshot: c?.snapshot, paired: true, checkedAt: now) }
            let fresh = try await client.fetchSnapshot()
            // The phone may have been unpaired (or paired with another Mac)
            // while this was downloading: then nothing is written.
            if self.keys()?.channel == keys.channel {
                SnapshotCache.write(CachedSnapshot(channel: keys.channel, version: fresh.version, fetchedAt: now, snapshot: fresh.snapshot))
            }
            return Loaded(snapshot: fresh.snapshot, paired: true, checkedAt: now)
        } catch {
            // Offline or refused: the cached copy, judged by its age.
            return Loaded(snapshot: c?.snapshot, paired: true, checkedAt: now)
        }
    }
}

struct Provider: AppIntentTimelineProvider {
    func placeholder(in context: Context) -> UsageEntry {
        let now = Date()
        return UsageEntry(date: now, snapshot: WidgetData.sample(now: now), paired: true, range: .today, checkedAt: now)
    }

    func snapshot(for configuration: RangeIntent, in context: Context) async -> UsageEntry {
        let c = WidgetData.cached()
        if context.isPreview, c.snapshot == nil { return placeholder(in: context) }
        return UsageEntry(date: .now, snapshot: c.snapshot, paired: c.paired, range: configuration.range.range, checkedAt: c.checkedAt)
    }

    func timeline(for configuration: RangeIntent, in context: Context) async -> Timeline<UsageEntry> {
        let now = Date()
        let r = await WidgetData.load()
        let range = configuration.range.range
        // Besides "now": each limit-window reset (its meter drops to 0), and
        // the moment the numbers count as old if iOS never reloads the widget.
        var dates = [now] + Dashboard.changeMoments(r.snapshot, range: range, after: now, within: WidgetData.resetHorizon)
        if r.snapshot != nil { dates.append(r.checkedAt.addingTimeInterval(Dashboard.widgetUnrefreshed + 1)) }
        return Timeline(
            entries: dates.sorted().map { UsageEntry(date: $0, snapshot: r.snapshot, paired: r.paired, range: range, checkedAt: r.checkedAt) },
            policy: .after(now.addingTimeInterval(WidgetData.refreshEvery))
        )
    }
}

struct UsageWidget: Widget {
    var body: some WidgetConfiguration {
        AppIntentConfiguration(kind: "TokensUsage", intent: RangeIntent.self, provider: Provider()) { entry in
            FamilyView(entry: entry).containerBackground(.background, for: .widget)
        }
        .configurationDisplayName("Token Counter")
        .description("Spend and limits from your main Mac.")
        .supportedFamilies([.systemSmall, .systemMedium, .accessoryRectangular, .accessoryCircular, .accessoryInline])
    }
}

private struct FamilyView: View {
    var entry: UsageEntry
    @Environment(\.widgetFamily) private var family

    var body: some View {
        UsageWidgetView(entry: entry, family: family)
    }
}
