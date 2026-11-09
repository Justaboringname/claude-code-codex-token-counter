import SwiftUI

/** Spend per bucket: Claude over Codex on one shared scale (Sparkline.tsx). */
struct SparklineView: View {
    var claude: [Double]
    var codex: [Double]?
    var claudeColor: Color
    var codexColor: Color
    var height: CGFloat = 72

    var body: some View {
        GeometryReader { geo in
            let size = geo.size
            let top = max(((codex ?? []) + claude).max() ?? 0, 0.0001)
            ZStack {
                if let codex { series(codex, codexColor, size, top) }
                series(claude, claudeColor, size, top)
            }
        }
        .frame(height: height)
        .accessibilityHidden(true)
    }

    private func points(_ data: [Double], _ size: CGSize, _ top: Double) -> [CGPoint] {
        let pad: CGFloat = 2
        let step = data.count > 1 ? (size.width - pad * 2) / CGFloat(data.count - 1) : 0
        return data.enumerated().map { i, v in
            CGPoint(x: pad + CGFloat(i) * step, y: size.height - pad - CGFloat(max(0, v) / top) * (size.height - pad * 2))
        }
    }

    @ViewBuilder
    private func series(_ data: [Double], _ color: Color, _ size: CGSize, _ top: Double) -> some View {
        let pts = points(data, size, top)
        if let first = pts.first, let last = pts.last {
            let line = Path { p in
                p.move(to: first)
                for pt in pts.dropFirst() { p.addLine(to: pt) }
            }
            let area = Path { p in
                p.addPath(line)
                p.addLine(to: CGPoint(x: last.x, y: size.height - 2))
                p.addLine(to: CGPoint(x: first.x, y: size.height - 2))
                p.closeSubpath()
            }
            let peak = pts.map(\.y).min() ?? 0
            area.fill(
                LinearGradient(
                    colors: [color.opacity(0.25), color.opacity(0)],
                    startPoint: UnitPoint(x: 0.5, y: peak / max(size.height, 1)),
                    endPoint: UnitPoint(x: 0.5, y: 1)
                )
            )
            line.stroke(color, style: StrokeStyle(lineWidth: 1.6, lineCap: .round, lineJoin: .round))
            Circle().fill(color.opacity(0.2)).frame(width: 10, height: 10).position(last)
            Circle().fill(color).frame(width: 6, height: 6).position(last)
        }
    }
}

/** A capsule track with a filled share (0–100; overflow is clipped). */
struct BarTrack: View {
    var pct: Double
    var color: Color
    var track: Color
    var height: CGFloat = 6

    var body: some View {
        GeometryReader { geo in
            ZStack(alignment: .leading) {
                Capsule().fill(track)
                Capsule().fill(color).frame(width: geo.size.width * CGFloat(min(100, max(0, pct.isFinite ? pct : 0)) / 100))
            }
        }
        .frame(height: height)
        .clipShape(Capsule())
    }
}

/** "BY MODEL"-style section heading with an optional right-hand note. */
struct SectionLabel<Trailing: View>: View {
    var title: String
    var color: Color
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(spacing: 8) {
            Text(title.uppercased()).tracking(0.96).foregroundStyle(color)
            Spacer(minLength: 0)
            trailing
        }
        .font(.system(size: 12, weight: .semibold))
        .accessibilityAddTraits(.isHeader)
    }
}

struct Hairline: View {
    var color: Color
    var body: some View { Rectangle().fill(color).frame(height: 0.5) }
}

/** Round glass button on the wallpaper (38 pt, like the web chips). */
struct ChipButton<Label: View>: View {
    var label: String
    var action: () -> Void
    @ViewBuilder var icon: Label

    var body: some View {
        Button(action: action) {
            icon.frame(width: 40, height: 40).contentShape(Circle())
        }
        .buttonStyle(.plain)
        .glassEffect(.regular.interactive(), in: .circle)
        .accessibilityLabel(label)
    }
}
