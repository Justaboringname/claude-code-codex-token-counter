// What the phone screen shows, computed from a snapshot: the web app's
// PopoverContent.tsx helpers (models, tool tiles, limits, freshness, price
// note), kept free of SwiftUI so they can be tested against the TypeScript.
import Foundation

enum Dashboard {
    static let canonicalTools = ["Read", "Edit", "Bash", "Grep", "WebFetch", "Glob"]
    static let staleSnapshot: TimeInterval = 15 * 60

    // MARK: by model

    struct ModelRow: Equatable, Identifiable {
        var id: String
        var name: String
        var color: String // already lifted for dark
        var cost: Double
        var sharePct: Double
        var inK: Double
        var outK: Double
        var cacheReadK: Double
        /** "in 250K", "out 62K", "cache 19.6M"; empty for cost-only rows. */
        var detail: [String] {
            var d: [String] = []
            if inK + outK > 0 { d += ["in \(Fmt.tokens(inK))", "out \(Fmt.tokens(outK))"] }
            if cacheReadK > 0 { d.append("cache \(Fmt.tokens(cacheReadK))") }
            return d
        }
    }

    /** Cost > 0, by cost descending (ties by id), top 10. Shares are of the
     *  range's totalCost, not of the visible rows. */
    static func modelRows(_ usage: Usage?, dark: Bool) -> [ModelRow] {
        guard let usage else { return [] }
        let rows = usage.byModel.map { entry -> ModelRow in
            let meta = Models.meta(entry.id)
            let cost = Models.cost(entry.id, entry.usage)
            return ModelRow(
                id: entry.id,
                name: meta.name,
                color: dark ? Models.liftForDark(meta.color) : meta.color,
                cost: cost,
                sharePct: usage.totalCost > 0 ? cost / usage.totalCost * 100 : 0,
                inK: entry.usage.inK,
                outK: entry.usage.outK,
                cacheReadK: entry.usage.cacheReadK
            )
        }
        return Array(
            rows.filter { $0.cost > 0 }
                .sorted { $0.cost != $1.cost ? $0.cost > $1.cost : $0.id < $1.id }
                .prefix(10)
        )
    }

    // MARK: tools

    /** Exactly 6 tiles: used tools by calls (stable), then unused canonical ones. */
    static func toolTiles(_ usage: Usage?) -> [ToolCount] {
        let used = (usage?.byTool ?? []).enumerated()
            .sorted { $0.element.calls != $1.element.calls ? $0.element.calls > $1.element.calls : $0.offset < $1.offset }
            .map(\.element)
        let names = Set(used.map(\.name))
        let filler = canonicalTools.filter { !names.contains($0) }.map { ToolCount(name: $0, calls: 0, tokens: 0) }
        return Array((used + filler).prefix(6))
    }

    enum ToolIcon: String { case read, edit, bash, search, web }

    static func toolIcon(_ name: String) -> ToolIcon {
        switch name.lowercased() {
        case "read": .read
        case "edit", "write", "multiedit": .edit
        case "bash", "bashoutput", "killshell": .bash
        case "webfetch", "websearch": .web
        default: .search
        }
    }

    // MARK: limits

    private static func clampPct(_ p: Double) -> Double { min(100, max(0, p)) }

    /** A window whose reset is in the past rolled over: 0%. */
    static func liveIsoPct(_ w: LimitWindow?, now: Date) -> Double? {
        guard let w else { return nil }
        if let reset = ISODate.parse(w.resetsAt), reset <= now { return 0 }
        return clampPct(w.usedPct)
    }

    static func liveEpochPct(_ resetsAt: Double?, _ pct: Double, now: Date) -> Double {
        if let r = resetsAt, r * 1000 <= now.timeIntervalSince1970 * 1000 { return 0 }
        return clampPct(pct)
    }

    struct LimitsRow: Equatable {
        var title: String
        var fivePct: Double?
        var weeklyPct: Double?
        /** Set when the reading is not current: meters dimmed, "as of …". */
        var staleAsOf: String?
        var codex: Bool
    }

    static func claudeLimits(_ q: LiveQuota?, now: Date) -> LimitsRow? {
        guard let q, q.fiveHour != nil || q.weekly != nil else { return nil }
        return LimitsRow(
            title: "Claude limits",
            fivePct: liveIsoPct(q.fiveHour, now: now),
            weeklyPct: liveIsoPct(q.weekly, now: now),
            staleAsOf: q.status != "ok" ? q.fetchedAt : nil,
            codex: false
        )
    }

    /** The live Codex quota if it has a window, else the range's log fallback. */
    static func codexLimits(_ live: LiveQuota?, log: LogQuota?, now: Date) -> LimitsRow? {
        if let q = live, q.fiveHour != nil || q.weekly != nil {
            return LimitsRow(
                title: "Codex limits",
                fivePct: liveIsoPct(q.fiveHour, now: now),
                weeklyPct: liveIsoPct(q.weekly, now: now),
                staleAsOf: q.status != "ok" ? q.fetchedAt : nil,
                codex: true
            )
        }
        guard let l = log else { return nil }
        return LimitsRow(
            title: "Codex limits",
            fivePct: l.primaryWindowMinutes != 0 ? liveEpochPct(l.primaryResetsAt, l.primaryUsedPct, now: now) : nil,
            weeklyPct: l.secondaryWindowMinutes != 0 ? liveEpochPct(l.secondaryResetsAt, l.secondaryUsedPct, now: now) : nil,
            staleAsOf: nil,
            codex: true
        )
    }

    // MARK: hero

    static func heroLabel(_ usage: Usage?) -> String {
        "\(usage?.rangeLabel ?? "—") spend".uppercased()
    }

    /** "+13%" / "-0%"; up (≥ 0) is green. */
    static func delta(_ pct: Double) -> (up: Bool, text: String) {
        let up = pct >= 0
        return (up, (up ? "+" : "") + JS.toFixed(pct, 0) + "%")
    }

    /** The sparkline series to draw, bottom first: Codex under Claude, only
     *  when it has spend. A missing Claude series is 14 zeros. */
    static func sparkSeries(_ usage: Usage?) -> (claude: [Double], codex: [Double]?) {
        let claude = usage?.sparkline ?? Array(repeating: 0, count: 14)
        let codex = usage?.sparklineCodex.flatMap { $0.contains { $0 > 0 } ? $0 : nil }
        return (claude, codex)
    }

    // MARK: freshness

    struct Freshness: Equatable {
        var text: String
        var alert: Bool
    }

    /** "From Mac Studio · updated 2 min ago", or amber "Mac Studio last
     *  synced 3 h ago", plus " · Offline" etc. for a real sync error. */
    static func freshness(_ s: Snapshot, error: SyncError?, now: Date) -> Freshness {
        let gen = ISODate.parse(s.generatedAt)
        let name = s.deviceName
        let stale = gen.map { now.timeIntervalSince($0) > staleSnapshot } ?? false
        let age = gen.map { Fmt.relativeAge($0, now: now) } ?? "at an unknown time"
        let err = error.flatMap { $0.isReal ? $0.describe(deviceName: name, hasSnapshot: true) : nil }
        let text = (stale ? "\(name) last synced \(age)" : "From \(name) · updated \(age)")
            + (err.map { " · \($0.short)" } ?? "")
        return Freshness(text: text, alert: stale || err != nil)
    }

    // MARK: widgets

    /** A widget that iOS has not refreshed for this long marks its numbers as old. */
    static let widgetUnrefreshed: TimeInterval = 2 * 3600

    /** When a widget's numbers are from, if they should be marked as old; nil while they are current.
     *  A widget is refreshed only every 15–60 minutes, so "older than 15 minutes
     *  right now" would flag it all the time. Instead: the snapshot was already
     *  15 minutes old when the widget last looked (`checkedAt`; the Mac uploads
     *  at least every 10 minutes, so it had stopped), or the widget itself has
     *  not been refreshed for two hours. */
    static func widgetStaleSince(_ s: Snapshot, checkedAt: Date, now: Date) -> Date? {
        let gen = ISODate.parse(s.generatedAt)
        let old = gen.map { checkedAt.timeIntervalSince($0) > staleSnapshot } ?? false
        let unrefreshed = now.timeIntervalSince(checkedAt) > widgetUnrefreshed
        return old || unrefreshed ? (gen ?? checkedAt) : nil
    }

    /** Moments in (now, now + window] at which a limit window rolls over (its
     *  meter drops to 0) without new data. One second late, to be past the
     *  edge. The widgets schedule a timeline entry at each. */
    static func changeMoments(_ s: Snapshot?, range: UsageRange, after now: Date, within window: TimeInterval) -> [Date] {
        guard let s else { return [] }
        var dates: [Date] = []
        for q in [s.claudeQuota, s.codexQuota] {
            for w in [q?.fiveHour, q?.weekly] {
                if let d = ISODate.parse(w?.resetsAt) { dates.append(d) }
            }
        }
        if let log = s.usage[range]?.codexQuota {
            for r in [log.primaryResetsAt, log.secondaryResetsAt] {
                if let r { dates.append(Date(timeIntervalSince1970: r)) }
            }
        }
        let end = now.addingTimeInterval(window)
        return Array(Set(dates.map { $0.addingTimeInterval(1) })).filter { $0 > now && $0 <= end }.sorted()
    }

    // MARK: price note

    /** "3h" / "2d" since an ISO time; nil when missing, unparseable or in the future. */
    static func fmtAge(_ iso: String?, now: Date) -> String? {
        guard let d = ISODate.parse(iso) else { return nil }
        let ms = (now.timeIntervalSince1970 - d.timeIntervalSince1970) * 1000
        guard ms.isFinite, ms >= 0 else { return nil }
        let h = ms / 3_600_000
        if h < 1 { return "\(Int(max(1, JS.round(ms / 60000))))m" }
        if h < 48 { return "\(Int(h.rounded(.down)))h" }
        return "\(Int((h / 24).rounded(.down)))d"
    }

    static func priceNote(_ h: PriceHealth?, now: Date) -> (text: String, warn: Bool)? {
        guard let h, h.status != "fresh" else { return nil }
        let age = fmtAge(h.fetchedAt, now: now)
        switch h.status {
        case "builtin": return ("Built-in prices", h.lastError != nil)
        case "error": return ("Price update failed", true)
        case "aging", "stale": return (age.map { "Prices \($0) old" } ?? "Prices out of date", h.status == "stale")
        default: return nil
        }
    }

    // MARK: times

    /** Local time today, else "Nov 8" (formatAsOf). */
    static func asOf(_ iso: String, now: Date = Date()) -> String {
        guard let d = ISODate.parse(iso) else { return "Invalid Date" }
        return asOf(d, now: now)
    }

    static func asOf(_ d: Date, now: Date = Date()) -> String {
        if Calendar.current.isDate(d, inSameDayAs: now) {
            return d.formatted(date: .omitted, time: .shortened)
        }
        return d.formatted(.dateTime.month(.abbreviated).day())
    }

    /** Local time today, else date + time (clockTime). */
    static func clockTime(_ d: Date, now: Date = Date()) -> String {
        if Calendar.current.isDate(d, inSameDayAs: now) { return d.formatted(date: .omitted, time: .shortened) }
        return d.formatted(.dateTime.month(.abbreviated).day().hour().minute())
    }
}
