// Colours of the phone layout, the same values as the web app's webTheme
// (src/theme.ts), light and dark.
import SwiftUI

extension Color {
    /** "#RRGGBB". */
    init(hex: String, opacity: Double = 1) {
        let n = UInt32(hex.dropFirst(hex.hasPrefix("#") ? 1 : 0), radix: 16) ?? 0
        self.init(
            .sRGB,
            red: Double((n >> 16) & 0xFF) / 255,
            green: Double((n >> 8) & 0xFF) / 255,
            blue: Double(n & 0xFF) / 255,
            opacity: opacity
        )
    }

    static func rgba(_ r: Double, _ g: Double, _ b: Double, _ a: Double) -> Color {
        Color(.sRGB, red: r / 255, green: g / 255, blue: b / 255, opacity: a)
    }
}

/** Two numbers the look of the day appearance hangs on. */
enum Tuning {
    /** How white the card's glass is by day (0 = clear glass, 1 = solid white).
     *  Low on purpose: the wallpaper's orange and blue should show through the
     *  card (at 0.5 the user found it too white and too solid). */
    static let dayCardTint: Double = value("TC_PREVIEW_CARD", 0.12)
    /** The dusk veil over the day wallpaper (keeps the white header text readable). */
    static let dayVeil: Double = value("TC_PREVIEW_VEIL", 0.10)

    private static func value(_ name: String, _ fallback: Double) -> Double {
        #if DEBUG && targetEnvironment(simulator)
        if let v = ProcessInfo.processInfo.environment[name].flatMap(Double.init) { return v }
        #endif
        return fallback
    }
}

struct Theme {
    let dark: Bool
    init(_ scheme: ColorScheme) { dark = scheme == .dark }

    private func pick(_ light: Color, _ darkColor: Color) -> Color { dark ? darkColor : light }

    // On the card.
    var ink: Color { pick(.black.opacity(0.88), .white.opacity(0.92)) }
    var inkStrong: Color { pick(.black.opacity(0.8), .white.opacity(0.85)) }
    var inkSoft: Color { pick(.black.opacity(0.7), .white.opacity(0.68)) }
    var inkMute: Color { pick(.black.opacity(0.64), .white.opacity(0.6)) }
    var warn: Color { pick(Color(hex: "#7A3B00"), Color(hex: "#FF9F0A")) }
    var accent: Color { pick(Color(hex: "#B4532F"), Color(hex: "#B9583A")) }
    var danger: Color { pick(Color(hex: "#B3261E"), Color(hex: "#FF9A92")) }
    var hairline: Color { pick(.black.opacity(0.08), .white.opacity(0.09)) }
    var track: Color { .rgba(120, 120, 128, dark ? 0.3 : 0.2) }
    var field: Color { .rgba(120, 120, 128, dark ? 0.24 : 0.1) }
    var cardTint: Color { pick(.white.opacity(Tuning.dayCardTint), .rgba(28, 28, 30, 0.35)) }
    var tileTop: Color { .white.opacity(dark ? 0.11 : 0.5) }
    var tileBottom: Color { .white.opacity(dark ? 0.055 : 0.3) }
    var tileBorder: Color { .white.opacity(dark ? 0.14 : 0.65) }
    var deltaUp: Color { pick(Color(hex: "#11572B"), Color(hex: "#34C759")) }
    var deltaDown: Color { pick(Color(hex: "#7A3B00"), Color(hex: "#FF9F0A")) }

    // On the wallpaper.
    var wallInk: Color { pick(.white, .white.opacity(0.95)) }
    var wallInkSoft: Color { .white.opacity(dark ? 0.66 : 0.94) }
    var wallWarn: Color { pick(Color(hex: "#FFE0AD"), Color(hex: "#FFC46B")) }
    var wallWarnChip: Color { pick(.rgba(72, 28, 8, 0.34), .rgba(255, 159, 10, 0.14)) }

    /** A tile inside the card: a corner that sits in a card corner follows it
     *  (concentric), the others stay at 14 pt. */
    static let tileShape = ConcentricRectangle(corners: .concentric(minimum: 14))

    // Series: Claude warm, Codex cool.
    var claude: Color { pick(Color(hex: "#B8532F"), Color(hex: "#E29A78")) }
    var codex: Color { pick(Color(hex: "#437FBD"), Color(hex: "#7FB3E3")) }
}

/** The app's wallpaper: the web app's colours (warm orange top left, blue on
 *  the right, violet bottom left), but alive: a 4×4 MeshGradient whose points
 *  wander slowly, so the fields drift into each other. A veil keeps the white
 *  header text readable. Still when Reduce Motion is on, and while the app is
 *  not in front. */
struct Wallpaper: View {
    let dark: Bool

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.scenePhase) private var phase

    var body: some View {
        // Only the gradient is redrawn per frame, not the screen above it.
        TimelineView(.animation(minimumInterval: 1.0 / 30.0, paused: reduceMotion || phase != .active)) { timeline in
            let t = Float(timeline.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: 3600))
            ZStack {
                MeshGradient(
                    width: 4,
                    height: 4,
                    points: Self.points(t),
                    colors: dark ? Self.nightColors : Self.dayColors,
                    smoothsColors: true
                )
                LinearGradient(
                    colors: dark
                        ? [.black.opacity(0.08), .clear]
                        // Thin, so the orange stays bright; the white header text
                        // has a soft shadow of its own (DashboardView).
                        : [.rgba(38, 14, 30, Tuning.dayVeil), .rgba(38, 14, 30, Tuning.dayVeil * 0.9)],
                    startPoint: .top,
                    endPoint: .bottom
                )
            }
        }
        .ignoresSafeArea()
        .accessibilityHidden(true)
    }

    /** The mesh at time `t` (seconds). Corners stay put and border points only
     *  slide along their edge, so the mesh always covers the screen; inner
     *  points roam. Every point has its own loop (about 4–6 s; at 10–15 s the
     *  user took the wallpaper for a still image), small enough that
     *  neighbours never cross. */
    static func points(_ t: Float) -> [SIMD2<Float>] {
        let grid: [Float] = [0, 1.0 / 3, 2.0 / 3, 1]
        var out: [SIMD2<Float>] = []
        out.reserveCapacity(16)
        for row in 0..<4 {
            for col in 0..<4 {
                let i = Float(row * 4 + col)
                var x = grid[col]
                var y = grid[row]
                if col != 0 && col != 3 {
                    x += 0.14 * sin(t * (1.25 + 0.25 * sin(i * 1.7)) + i * 2.1)
                }
                if row != 0 && row != 3 {
                    y += 0.14 * cos(t * (1.10 + 0.25 * cos(i * 2.3)) + i * 1.3)
                }
                out.append([x, y])
            }
        }
        return out
    }

    // Row by row, top to bottom. The Claude orange and the Codex blue, as in
    // the web app's wallpaper. (A pink/violet palette was tried and the user
    // preferred this one.)
    // The orange is saturated rather than light (more chroma, not more white).
    private static let day: [Color] = [
        "#F5691F", "#EE7436", "#D9866E", "#8090D8",
        "#EE7A3C", "#DC8468", "#A88FC0", "#6292D6",
        "#B88CB4", "#A58CC4", "#8F97D4", "#7E9BDA",
        "#8C78D6", "#8F84D8", "#8C9BD6", "#8FA3DC",
    ].map { Color(hex: $0) }

    #if DEBUG && targetEnvironment(simulator)
    /** Simulator only: `TC_PREVIEW_NIGHT` / `TC_PREVIEW_DAY` = 16 comma-separated
     *  #RRGGBB values try another palette without a rebuild. */
    private static func preview(_ name: String, _ fallback: [Color]) -> [Color] {
        let hexes = (ProcessInfo.processInfo.environment[name] ?? "").split(separator: ",").map(String.init)
        return hexes.count == 16 ? hexes.map { Color(hex: $0) } : fallback
    }
    private static let nightColors = preview("TC_PREVIEW_NIGHT", night)
    private static let dayColors = preview("TC_PREVIEW_DAY", day)
    #else
    private static var nightColors: [Color] { night }
    private static var dayColors: [Color] { day }
    #endif

    private static let night: [Color] = [
        "#F5822F", "#A2443E", "#522A68", "#2A2A78",
        "#8A3248", "#582A6C", "#34307E", "#24408E",
        "#2E1E5A", "#26266E", "#1E3684", "#1A4690",
        "#0E1030", "#0E1638", "#0E1E44", "#0C2650",
    ].map { Color(hex: $0) }
}

/** The app icon's rings, drawn live (the pairing screen's mark). */
struct AppMark: View {
    var size: CGFloat
    var dark: Bool

    var body: some View {
        let u = size / 1024
        let arc = dark ? Color(hex: "#EE8A5C") : Color(hex: "#E2632E")
        let inner = dark ? Color(hex: "#9ACBF3") : Color(hex: "#2462C8")
        let track = dark ? Color.white.opacity(0.16) : Color.black.opacity(0.07)
        ZStack {
            RoundedRectangle(cornerRadius: size * 0.225, style: .continuous)
                .fill(
                    dark
                        ? AnyShapeStyle(LinearGradient(colors: [Color(hex: "#26272B"), Color(hex: "#111113")], startPoint: .top, endPoint: .bottom))
                        : AnyShapeStyle(Color.white)
                )
            Group {
                ring(from: 150, sweep: 240, radius: 296, width: 122, track, u)
                ring(from: 150, sweep: 240, radius: 156, width: 96, track, u)
                ring(from: 150, sweep: 172.83, radius: 296, width: 122, arc, u)
                ring(from: 258, sweep: 132, radius: 156, width: 96, inner, u)
                Circle().fill(arc).frame(width: 116 * u, height: 116 * u)
            }
            .offset(y: 64 * u)
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }

    /** An arc starting `from` degrees clockwise from 3 o'clock. */
    private func ring(from: Double, sweep: Double, radius: CGFloat, width: CGFloat, _ c: Color, _ u: CGFloat) -> some View {
        Circle()
            .trim(from: 0, to: sweep / 360)
            .stroke(c, style: StrokeStyle(lineWidth: width * u, lineCap: .round))
            .rotationEffect(.degrees(from))
            .frame(width: radius * 2 * u, height: radius * 2 * u)
    }
}
