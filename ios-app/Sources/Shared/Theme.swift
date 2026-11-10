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
