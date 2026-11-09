// What the widgets draw. In Shared (not in the extension) so the app's
// Simulator-only preview can lay the same views out at widget sizes.
import SwiftUI
import WidgetKit

struct UsageEntry: TimelineEntry {
    let date: Date
    let snapshot: Snapshot?
    let paired: Bool
    let range: UsageRange
    /** When the widget last asked the server (or, without a network, looked at its cache). */
    let checkedAt: Date
}

/** What one entry shows, computed once (the same helpers as the app). */
struct WidgetContent {
    var label: String
    var money: String
    var tokens: String
    var delta: (up: Bool, text: String)?
    /** Set when the numbers are old (see Dashboard.widgetStaleSince): when they are from. */
    var staleSince: Date?
    var claude: Dashboard.LimitsRow?
    var codex: Dashboard.LimitsRow?
    var spark: (claude: [Double], codex: [Double]?)

    init(_ s: Snapshot, range: UsageRange, now: Date, checkedAt: Date) {
        let usage = s.usage[range]
        label = (usage.flatMap { $0.rangeLabel.isEmpty ? nil : $0.rangeLabel } ?? range.label).uppercased()
        money = Fmt.money(usage?.totalCost ?? 0)
        tokens = "\(Fmt.tokens(usage?.totalTokens ?? 0)) tokens"
        delta = usage?.deltaPct.map(Dashboard.delta)
        staleSince = Dashboard.widgetStaleSince(s, checkedAt: checkedAt, now: now)
        claude = Dashboard.claudeLimits(s.claudeQuota, now: now).flatMap { $0.fivePct == nil && $0.weeklyPct == nil ? nil : $0 }
        codex = Dashboard.codexLimits(s.codexQuota, log: usage?.codexQuota, now: now).flatMap { $0.fivePct == nil && $0.weeklyPct == nil ? nil : $0 }
        spark = Dashboard.sparkSeries(usage)
    }

    /** The row a one-row widget shows: Claude's, else Codex's. */
    var mainLimits: Dashboard.LimitsRow? { claude ?? codex }

    /** The line under the amount is the token count (no stale note, no delta). */
    var headlineShowsTokens: Bool { staleSince == nil && delta == nil }
}

struct UsageWidgetView: View {
    var entry: UsageEntry
    var family: WidgetFamily

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let t = Theme(scheme)
        if let s = entry.snapshot {
            let c = WidgetContent(s, range: entry.range, now: entry.date, checkedAt: entry.checkedAt)
            switch family {
            case .systemMedium: medium(c, t)
            case .accessoryRectangular: rectangular(c)
            case .accessoryCircular: circular(c)
            case .accessoryInline: inline(c)
            default: small(c, t)
            }
        } else {
            empty
        }
    }

    private func asOf(_ since: Date) -> String { "as of \(Dashboard.asOf(since, now: entry.date))" }

    // MARK: home screen

    private func small(_ c: WidgetContent, _ t: Theme) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            headline(c, t, size: 30)
            Spacer(minLength: 6)
            if let row = c.mainLimits {
                VStack(spacing: 6) { meters(row, t) }
            } else if !c.headlineShowsTokens {
                Text(c.tokens).font(.system(size: 12)).foregroundStyle(.secondary)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private func medium(_ c: WidgetContent, _ t: Theme) -> some View {
        HStack(alignment: .top, spacing: 16) {
            VStack(alignment: .leading, spacing: 0) {
                headline(c, t, size: 32)
                Spacer(minLength: 6)
                SparklineView(claude: c.spark.claude, codex: c.spark.codex, claudeColor: t.claude, codexColor: t.codex, height: 36)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            if c.claude != nil || c.codex != nil {
                VStack(alignment: .leading, spacing: 6) {
                    if let row = c.claude { group("Claude", row, t) }
                    if let row = c.codex { group("Codex", row, t) }
                }
                .frame(width: 140, alignment: .topLeading)
                .frame(maxHeight: .infinity, alignment: .center)
            }
        }
    }

    private func headline(_ c: WidgetContent, _ t: Theme, size: CGFloat) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(c.label)
                .font(.system(size: 11, weight: .semibold))
                .tracking(0.8)
                .foregroundStyle(.secondary)
                .lineLimit(1)
            Text(c.money)
                .font(.system(size: size, weight: .semibold))
                .tracking(-0.8)
                .monospacedDigit()
                .lineLimit(1)
                .minimumScaleFactor(0.5)
                .contentTransition(.numericText())
            if let since = c.staleSince {
                // The Mac is asleep or offline: say when the numbers are from.
                Label(asOf(since), systemImage: "exclamationmark.triangle")
                    .labelStyle(.titleAndIcon)
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(t.warn)
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
            } else if let d = c.delta {
                HStack(spacing: 3) {
                    Image(systemName: d.up ? "arrow.up" : "arrow.down").font(.system(size: 9, weight: .bold))
                    Text(d.text).fontWeight(.semibold).monospacedDigit()
                    Text("vs yesterday").foregroundStyle(Color.secondary)
                }
                .font(.system(size: 12))
                .foregroundStyle(d.up ? t.deltaUp : t.deltaDown)
                .lineLimit(1)
                .minimumScaleFactor(0.8)
            } else {
                Text(c.tokens).font(.system(size: 12)).foregroundStyle(.secondary).lineLimit(1)
            }
        }
        .accessibilityElement(children: .combine)
    }

    private func group(_ title: String, _ row: Dashboard.LimitsRow, _ t: Theme) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title.uppercased()).font(.system(size: 10, weight: .semibold)).tracking(0.8).foregroundStyle(.secondary)
            meters(row, t)
        }
    }

    @ViewBuilder
    private func meters(_ row: Dashboard.LimitsRow, _ t: Theme) -> some View {
        if let p = row.fivePct { meter("5h", p, row, t) }
        if let p = row.weeklyPct { meter("wk", p, row, t) }
    }

    private func meter(_ label: String, _ pct: Double, _ row: Dashboard.LimitsRow, _ t: Theme) -> some View {
        HStack(spacing: 6) {
            Text(label).foregroundStyle(.secondary).frame(width: 17, alignment: .leading)
            BarTrack(pct: pct, color: row.codex ? t.codex : t.claude, track: t.track, height: 5).widgetAccentable()
            Text(JS.toFixed(pct, 0) + "%").monospacedDigit().frame(minWidth: 30, alignment: .trailing)
        }
        .font(.system(size: 11, weight: .medium))
        .opacity(row.staleAsOf != nil ? 0.55 : 1)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(row.title), \(label == "5h" ? "5 hour" : "weekly")")
        .accessibilityValue(JS.toFixed(pct, 0) + " percent")
    }

    // MARK: lock screen

    private func rectangular(_ c: WidgetContent) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline, spacing: 4) {
                Group {
                    if let since = c.staleSince {
                        Label(asOf(since), systemImage: "exclamationmark.triangle").labelStyle(.titleAndIcon)
                    } else {
                        Text(c.label.capitalized)
                    }
                }
                .font(.system(size: 13, weight: .semibold))
                .widgetAccentable()
                .lineLimit(1)
                .minimumScaleFactor(0.7)
                Spacer(minLength: 2)
                Text(c.money).font(.system(size: 15, weight: .semibold)).monospacedDigit().lineLimit(1).minimumScaleFactor(0.7)
            }
            if let row = c.mainLimits {
                Group {
                    if let p = row.fivePct { lockMeter("5h", p) }
                    if let p = row.weeklyPct { lockMeter("wk", p) }
                }
                .opacity(row.staleAsOf != nil || c.staleSince != nil ? 0.55 : 1)
            } else {
                Text(c.tokens).font(.system(size: 12)).foregroundStyle(.secondary)
            }
        }
    }

    private func lockMeter(_ label: String, _ pct: Double) -> some View {
        HStack(spacing: 5) {
            Text(label).frame(width: 17, alignment: .leading)
            BarTrack(pct: pct, color: .primary, track: .primary.opacity(0.25), height: 4).widgetAccentable()
            Text(JS.toFixed(pct, 0) + "%").monospacedDigit().frame(minWidth: 28, alignment: .trailing)
        }
        .font(.system(size: 11, weight: .medium))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(label == "5h" ? "5 hour limit" : "Weekly limit")
        .accessibilityValue(JS.toFixed(pct, 0) + " percent")
    }

    @ViewBuilder
    private func circular(_ c: WidgetContent) -> some View {
        let row = c.mainLimits
        let five = row?.fivePct
        if let row, let pct = five ?? row.weeklyPct {
            Gauge(value: min(100, max(0, pct)), in: 0...100) {
                Text(five != nil ? "5h" : "wk")
            } currentValueLabel: {
                Text(JS.toFixed(pct, 0))
            }
            .gaugeStyle(.accessoryCircular)
            // Old numbers or a stale limit reading: drawn fainter, like the app.
            .opacity(row.staleAsOf != nil || c.staleSince != nil ? 0.55 : 1)
            .accessibilityLabel("\(row.title), \(five != nil ? "5 hour" : "weekly")")
            .accessibilityValue(JS.toFixed(pct, 0) + " percent")
        } else {
            // No limit is known: the spend, not a gauge reading 0.
            ZStack {
                AccessoryWidgetBackground()
                VStack(spacing: 0) {
                    Text(c.label.capitalized).font(.system(size: 9, weight: .semibold)).lineLimit(1).minimumScaleFactor(0.6)
                    Text(c.money).font(.system(size: 14, weight: .semibold)).monospacedDigit().lineLimit(1).minimumScaleFactor(0.4)
                }
                .padding(.horizontal, 5)
                .opacity(c.staleSince != nil ? 0.55 : 1)
            }
        }
    }

    private func inline(_ c: WidgetContent) -> some View {
        let tail: String
        if let since = c.staleSince {
            tail = " · \(asOf(since))"
        } else {
            tail = c.mainLimits?.fivePct.map { " · 5h \(JS.toFixed($0, 0))%" } ?? ""
        }
        return Text("\(c.label.capitalized) \(c.money)\(tail)")
    }

    // MARK: nothing to show

    @ViewBuilder private var empty: some View {
        let text = entry.paired ? "Waiting for the Mac" : "Open Tokens to pair"
        switch family {
        case .accessoryInline:
            Text(text)
        case .accessoryCircular:
            Image(systemName: "icloud.slash").font(.system(size: 20)).accessibilityLabel(text)
        default:
            VStack(alignment: .leading, spacing: 6) {
                Image(systemName: entry.paired ? "icloud.and.arrow.down" : "qrcode.viewfinder").font(.system(size: 20)).foregroundStyle(.secondary)
                Text(text).font(.system(size: 13, weight: .medium))
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
        }
    }
}
