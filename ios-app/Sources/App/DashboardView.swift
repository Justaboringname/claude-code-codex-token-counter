// The main screen: the synced snapshot, laid out like the phone web app
// (wallpaper, header chips, freshness line, one card with the five ranges).
import SwiftUI

/** Everything the screen draws, as plain values (so any state can be previewed). */
struct DashboardData {
    var snapshot: Snapshot?
    var version: Int?
    var fetchedAt: Date?
    var error: SyncError?
    var refreshing = false
    /** Waiting for the Mac to upload its latest (the user asked). */
    var asking = false
    var serverHost: String?
}

struct DashboardActions: Sendable {
    var refresh: @MainActor @Sendable (_ force: Bool) async -> Void = { _ in }
    var unpair: @MainActor @Sendable () -> Void = {}
}

struct DashboardView: View {
    var data: DashboardData
    var actions: DashboardActions

    @Environment(\.colorScheme) private var scheme
    @Environment(\.cardRadius) private var cardRadius
    @AppStorage("theme") private var themeMode = "auto"
    @State private var range: UsageRange = .today
    @State private var sheetOpen = false

    var body: some View {
        let t = Theme(scheme)
        // Relative times, the 15-minute staleness rule and rolled-over limit
        // windows all depend on the clock.
        TimelineView(.periodic(from: .now, by: 15)) { timeline in
            let now = timeline.date
            GeometryReader { geo in
                ZStack {
                    Wallpaper(dark: t.dark)
                    // One card: what to read scrolls in its upper part, the range
                    // switch sits at its bottom, within the thumb's reach.
                    VStack(spacing: 0) {
                        header(t, now)
                        card(t, now)
                            // The same gap to the screen's bottom edge as to its
                            // sides, so the card's bottom corners sit exactly
                            // inside the screen's (the home indicator then
                            // lies over the card's bottom margin, under the
                            // range switch).
                            .padding(.bottom, Card.margin - geo.safeAreaInsets.bottom)
                    }
                    .frame(maxWidth: 560)
                    .padding(.horizontal, Card.margin)
                    .padding(.top, 6)
                    .frame(maxWidth: .infinity)
                }
            }
        }
        .sheet(isPresented: $sheetOpen) {
            SyncSheet(data: data, actions: actions)
        }
        .sensoryFeedback(.selection, trigger: range)
    }

    // MARK: header

    private func header(_ t: Theme, _ now: Date) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 10) {
                Text("Token Counter")
                    .font(.system(size: 30, weight: .bold))
                    .tracking(-0.6)
                    .foregroundStyle(t.wallInk)
                    .lineLimit(1)
                    .minimumScaleFactor(0.6)
                    .shadow(color: t.dark ? .clear : .rgba(70, 25, 10, 0.30), radius: 5, y: 1)
                    .accessibilityAddTraits(.isHeader)
                Spacer(minLength: 0)
                GlassEffectContainer(spacing: 8) {
                    HStack(spacing: 8) {
                        ChipButton(label: syncLabel, action: { sheetOpen = true }) {
                            Image(systemName: "icloud.and.arrow.down")
                                .font(.system(size: 15, weight: .semibold))
                                .symbolEffect(.pulse, isActive: data.refreshing)
                                .overlay(alignment: .topTrailing) {
                                    if data.error?.isReal == true {
                                        Circle().fill(t.warn).frame(width: 9, height: 9)
                                            .overlay(Circle().stroke(.white.opacity(0.7), lineWidth: 1.5))
                                            .offset(x: 6, y: -5)
                                    }
                                }
                        }
                        ChipButton(label: "Refresh", action: { Task { await actions.refresh(true) } }) {
                            Image(systemName: "arrow.clockwise")
                                .font(.system(size: 15, weight: .semibold))
                                .symbolEffect(.rotate, isActive: data.refreshing)
                        }
                        .disabled(data.refreshing)
                        ChipButton(label: "Appearance: \(themeMode)", action: cycleTheme) {
                            Image(systemName: themeMode == "light" ? "sun.max.fill" : themeMode == "dark" ? "moon.fill" : "circle.lefthalf.filled")
                                .font(.system(size: 15, weight: .semibold))
                                .contentTransition(.symbolEffect(.replace))
                        }
                    }
                    .foregroundStyle(t.wallInk)
                }
            }
            if let s = data.snapshot {
                let fresh = Dashboard.freshness(s, error: data.error, now: now)
                let f = data.asking ? Dashboard.Freshness(text: "Asking \(s.deviceName) for the latest…", alert: false) : fresh
                Button { sheetOpen = true } label: {
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        if f.alert {
                            Image(systemName: "exclamationmark.triangle").font(.system(size: 12, weight: .semibold))
                        }
                        Text(f.text).font(.system(size: 15)).multilineTextAlignment(.leading)
                    }
                    // White text on a bright wallpaper: a soft shadow keeps it readable.
                    .shadow(color: t.dark ? .clear : .rgba(70, 25, 10, 0.30), radius: 3, y: 1)
                    .foregroundStyle(f.alert ? t.wallWarn : t.wallInkSoft)
                    .padding(.vertical, 5)
                    .padding(.leading, f.alert ? 10 : 0)
                    .padding(.trailing, f.alert ? 12 : 0)
                    .background {
                        if f.alert { RoundedRectangle(cornerRadius: 16, style: .continuous).fill(t.wallWarnChip) }
                    }
                }
                .buttonStyle(.plain)
                .accessibilityHint("Sync details")
            }
        }
        .padding(.horizontal, 6)
        .padding(.top, 10)
        .padding(.bottom, 14)
    }

    private var syncLabel: String {
        if let e = data.error, e.isReal {
            return "Sync: \(e.describe(deviceName: data.snapshot?.deviceName, hasSnapshot: data.snapshot != nil).long)"
        }
        return "Sync"
    }

    private func cycleTheme() {
        themeMode = themeMode == "auto" ? "light" : themeMode == "light" ? "dark" : "auto"
    }

    // MARK: card

    private func card(_ t: Theme, _ now: Date) -> some View {
        VStack(spacing: 0) {
            ScrollView {
                VStack(spacing: 0) {
                    if let snapshot = data.snapshot {
                        let usage = snapshot.usage[range]
                        hero(t, usage)
                        spark(t, usage)
                        byModel(t, usage, now)
                        if let row = Dashboard.claudeLimits(snapshot.claudeQuota, now: now) { limits(t, row, now) }
                        if let row = Dashboard.codexLimits(snapshot.codexQuota, log: usage?.codexQuota, now: now) { limits(t, row, now) }
                        tools(t, usage)
                    } else {
                        placeholder(t)
                    }
                }
            }
            .scrollIndicators(.hidden)
            .scrollBounceBehavior(.basedOnSize)
            .refreshable { await actions.refresh(true) }
            // What scrolls fades out just above the range switch instead of
            // being cut by a line.
            .mask {
                VStack(spacing: 0) {
                    Rectangle()
                    LinearGradient(colors: [.black, .clear], startPoint: .top, endPoint: .bottom).frame(height: 18)
                }
            }

            // The range switch is part of the card (not a bar of its own): fixed
            // at the card's bottom while the numbers scroll above it.
            Picker("Range", selection: $range) {
                ForEach(UsageRange.allCases) { r in Text(r.label).tag(r) }
            }
            .pickerStyle(.segmented)
            .controlSize(.large)
            .padding(.horizontal, Card.controlInset(cardRadius))
            .padding(.top, 6)
            .padding(.bottom, Card.controlInset(cardRadius))
        }
        // A whole rounded rectangle on screen; what scrolls is cut to it.
        .clipShape(RoundedRectangle(cornerRadius: cardRadius, style: .continuous))
        .cardSurface(tint: t.cardTint, radius: cardRadius)
        .animation(.snappy(duration: 0.3), value: range)
    }

    private func hero(_ t: Theme, _ usage: Usage?) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(Dashboard.heroLabel(usage))
                .font(.system(size: 12, weight: .semibold))
                .tracking(0.96)
                .foregroundStyle(t.inkSoft)
            let money = Text(Fmt.money(usage?.totalCost ?? 0))
                .font(.system(size: 50, weight: .semibold))
                .tracking(-1.5)
                .monospacedDigit()
                .foregroundStyle(t.ink)
                .lineLimit(1)
                .minimumScaleFactor(0.5)
                .contentTransition(.numericText())
            let tokens = Text("\(Fmt.tokens(usage?.totalTokens ?? 0)) tokens")
                .font(.system(size: 14))
                .foregroundStyle(t.inkSoft)
                .lineLimit(1)
            ViewThatFits(in: .horizontal) {
                HStack(alignment: .firstTextBaseline, spacing: 10) { money.fixedSize(); tokens.fixedSize() }
                VStack(alignment: .leading, spacing: 2) { money; tokens }
            }
            .accessibilityElement(children: .combine)
            if let pct = usage?.deltaPct {
                let d = Dashboard.delta(pct)
                let c = d.up ? t.deltaUp : t.deltaDown
                HStack(spacing: 6) {
                    Image(systemName: d.up ? "arrow.up" : "arrow.down").font(.system(size: 11, weight: .bold)).foregroundStyle(c)
                    Text(d.text).fontWeight(.semibold).monospacedDigit().foregroundStyle(c)
                    Text("vs yesterday")
                }
                .font(.system(size: 14))
                .foregroundStyle(t.inkSoft)
                .accessibilityElement(children: .combine)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 20)
        .padding(.top, 24)
        .padding(.bottom, 14)
    }

    private func spark(_ t: Theme, _ usage: Usage?) -> some View {
        let s = Dashboard.sparkSeries(usage)
        return VStack(spacing: 4) {
            SparklineView(claude: s.claude, codex: s.codex, claudeColor: t.claude, codexColor: t.codex)
            HStack {
                Text(usage?.sparkLeft ?? "")
                Spacer()
                Text(usage?.sparkRight ?? "")
            }
            .font(.system(size: 12))
            .monospacedDigit()
            .foregroundStyle(t.inkMute)
        }
        .padding(.horizontal, 16)
        .padding(.bottom, 16)
    }

    private func byModel(_ t: Theme, _ usage: Usage?, _ now: Date) -> some View {
        let rows = Dashboard.modelRows(usage, dark: t.dark)
        return VStack(alignment: .leading, spacing: 0) {
            SectionLabel(title: "By model", color: t.inkSoft) {
                if let note = Dashboard.priceNote(usage?.priceHealth, now: now) {
                    HStack(spacing: 4) {
                        if note.warn { Image(systemName: "exclamationmark.triangle").font(.system(size: 10, weight: .semibold)) }
                        Text(note.text).monospacedDigit()
                    }
                    .foregroundStyle(note.warn ? t.warn : t.inkMute)
                }
            }
            .padding(.bottom, 12)
            if rows.isEmpty {
                Text("No usage in this range.").font(.system(size: 14)).foregroundStyle(t.inkMute).padding(.vertical, 6)
            }
            ForEach(rows) { m in
                VStack(alignment: .leading, spacing: 0) {
                    HStack(spacing: 8) {
                        Circle().fill(Color(hex: m.color)).frame(width: 9, height: 9)
                        Text(m.name).fontWeight(.medium).lineLimit(1)
                        Spacer(minLength: 4)
                        Text(Fmt.money(m.cost)).fontWeight(.semibold).monospacedDigit()
                        Text(JS.toFixed(m.sharePct, 0) + "%")
                            .font(.system(size: 12.5))
                            .monospacedDigit()
                            .foregroundStyle(t.inkMute)
                            .frame(minWidth: 40, alignment: .trailing)
                    }
                    .font(.system(size: 15))
                    .foregroundStyle(t.ink)
                    BarTrack(pct: m.sharePct, color: Color(hex: m.color), track: t.track).padding(.top, 6)
                    // Chat, Cowork and Duet have a cost but no tokens: no line, and no gap for one.
                    if !m.detail.isEmpty {
                        HStack(spacing: 10) {
                            ForEach(m.detail, id: \.self) { Text($0) }
                        }
                        .font(.system(size: 12))
                        .monospacedDigit()
                        .foregroundStyle(t.inkSoft)
                        .padding(.top, 5)
                    }
                }
                .padding(.bottom, 14)
                .accessibilityElement(children: .combine)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 20)
        .padding(.top, 16)
        .padding(.bottom, 10)
        .overlay(alignment: .top) { Hairline(color: t.hairline) }
    }

    private func limits(_ t: Theme, _ row: Dashboard.LimitsRow, _ now: Date) -> some View {
        VStack(spacing: 9) {
            HStack {
                Text(row.title.uppercased()).fontWeight(.semibold).tracking(1).foregroundStyle(t.inkSoft)
                Spacer()
                if let asOf = row.staleAsOf {
                    Text("as of \(Dashboard.asOf(asOf, now: now))").foregroundStyle(t.inkMute)
                }
            }
            HStack(spacing: 18) {
                if let p = row.fivePct { meter(t, "5h", p, row) }
                if let p = row.weeklyPct { meter(t, "wk", p, row) }
            }
        }
        .font(.system(size: 12.5))
        .padding(.horizontal, 20)
        .padding(.top, 12)
        .padding(.bottom, 14)
        .overlay(alignment: .top) { Hairline(color: t.hairline) }
    }

    private func meter(_ t: Theme, _ label: String, _ pct: Double, _ row: Dashboard.LimitsRow) -> some View {
        HStack(spacing: 8) {
            Text(label).foregroundStyle(t.inkMute).frame(width: 22, alignment: .leading)
            BarTrack(pct: pct, color: row.codex ? t.codex : t.claude, track: t.track)
            Text(JS.toFixed(pct, 0) + "%").monospacedDigit().foregroundStyle(t.inkSoft).frame(minWidth: 38, alignment: .trailing)
        }
        .opacity(row.staleAsOf != nil ? 0.55 : 1)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(row.title), \(label == "5h" ? "5 hour" : "weekly")")
        .accessibilityValue(JS.toFixed(pct, 0) + " percent")
    }

    private func tools(_ t: Theme, _ usage: Usage?) -> some View {
        let tiles = Dashboard.toolTiles(usage)
        let total = tiles.reduce(0) { $0 + $1.calls }
        return VStack(spacing: 10) {
            SectionLabel(title: "Tool calls", color: t.inkSoft) {
                Text("\(total.formatted(.number)) total").monospacedDigit().foregroundStyle(t.inkMute)
            }
            .padding(.horizontal, 4)
            LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 8), count: 3), spacing: 8) {
                ForEach(Array(tiles.enumerated()), id: \.offset) { _, tool in
                    VStack(alignment: .leading, spacing: 2) {
                        HStack(spacing: 6) {
                            Image(systemName: Self.symbol(Dashboard.toolIcon(tool.name)))
                                .font(.system(size: 11, weight: .semibold))
                                .foregroundStyle(Color(hex: Tools.color(tool.name)))
                            Text(Tools.displayName(tool.name)).font(.system(size: 12.5, weight: .medium)).foregroundStyle(t.inkSoft).lineLimit(1)
                        }
                        Text(tool.calls.formatted(.number))
                            .font(.system(size: 18, weight: .semibold))
                            .monospacedDigit()
                            .foregroundStyle(t.ink)
                            .contentTransition(.numericText())
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 10)
                    .background(LinearGradient(colors: [t.tileTop, t.tileBottom], startPoint: .top, endPoint: .bottom), in: Theme.tileShape)
                    .overlay(Theme.tileShape.stroke(t.tileBorder, lineWidth: 0.5))
                    .accessibilityElement(children: .combine)
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.top, 14)
        .padding(.bottom, 18)
        .overlay(alignment: .top) { Hairline(color: t.hairline) }
    }

    private static func symbol(_ icon: Dashboard.ToolIcon) -> String {
        switch icon {
        case .read: "folder"
        case .edit: "pencil"
        case .bash: "terminal"
        case .search: "magnifyingglass"
        case .web: "globe"
        }
    }

    // MARK: no snapshot yet

    private func placeholder(_ t: Theme) -> some View {
        let err = data.error?.describe(deviceName: nil, hasSnapshot: false)
        let connecting = data.refreshing || (data.error == nil && data.fetchedAt == nil)
        let title = connecting ? "Connecting…" : data.error?.kind == .empty ? "Waiting for the main Mac" : (err?.short ?? "No data yet")
        let text = connecting ? "Fetching the latest snapshot." : (err?.long ?? "Nothing has arrived yet.")
        return VStack(spacing: 8) {
            if connecting {
                ProgressView().controlSize(.large).padding(.bottom, 4)
            } else {
                Image(systemName: "icloud.and.arrow.down").font(.system(size: 30)).foregroundStyle(t.inkMute)
            }
            Text(title).font(.system(size: 18, weight: .semibold)).foregroundStyle(t.ink)
            Text(text).font(.system(size: 15)).lineSpacing(3).multilineTextAlignment(.center).frame(maxWidth: 300)
            if !connecting {
                Button("Try again") { Task { await actions.refresh(true) } }
                    .buttonStyle(.glass)
                    .controlSize(.large)
                    .padding(.top, 10)
            }
        }
        .foregroundStyle(t.inkSoft)
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 28)
        .padding(.top, 64)
        .padding(.bottom, 72)
    }
}
