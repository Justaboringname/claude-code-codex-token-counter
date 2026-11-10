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
    var cardTint: Color { pick(.white.opacity(0.5), .rgba(28, 28, 30, 0.35)) }
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

/** The web app's wallpaper: a vertical base gradient under three soft colour
 *  fields (CSS `radial-gradient(W% H% at X Y)` ellipses), plus a dusk veil in
 *  light mode so white text stays readable. */
struct Wallpaper: View {
    let dark: Bool

    var body: some View {
        GeometryReader { geo in
            let size = geo.size
            ZStack {
                LinearGradient(
                    stops: dark
                        ? [.init(color: Color(hex: "#24171A"), location: 0), .init(color: Color(hex: "#17161F"), location: 0.5),
                           .init(color: Color(hex: "#10131D"), location: 1)]
                        : [.init(color: Color(hex: "#C9775A"), location: 0), .init(color: Color(hex: "#B48AA6"), location: 0.45),
                           .init(color: Color(hex: "#8C9BD6"), location: 1)],
                    startPoint: .top,
                    endPoint: .bottom
                )
                if dark {
                    field(.rgba(98, 78, 170, 0.38), rx: 0.9, ry: 0.6, x: 0.18, y: 1, size)
                    field(.rgba(67, 127, 189, 0.4), rx: 0.7, ry: 0.5, x: 1, y: 0.38, size)
                    field(.rgba(217, 119, 87, 0.42), rx: 0.7, ry: 0.45, x: 0.06, y: 0, size)
                } else {
                    field(.rgba(140, 120, 214, 0.75), rx: 0.9, ry: 0.6, x: 0.2, y: 1, size)
                    field(.rgba(98, 146, 214, 0.85), rx: 0.7, ry: 0.5, x: 1, y: 0.3, size)
                    field(.rgba(214, 112, 76, 0.95), rx: 0.7, ry: 0.45, x: 0.08, y: 0, size)
                    Color.rgba(38, 14, 30, 0.3)
                }
            }
            .frame(width: size.width, height: size.height)
            .clipped()
        }
        .ignoresSafeArea()
        .accessibilityHidden(true)
    }

    private func field(_ c: Color, rx: Double, ry: Double, x: Double, y: Double, _ size: CGSize) -> some View {
        EllipticalGradient(colors: [c, c.opacity(0)], center: .center, startRadiusFraction: 0, endRadiusFraction: 0.5)
            .frame(width: 2 * rx * size.width, height: 2 * ry * size.height)
            .position(x: x * size.width, y: y * size.height)
    }
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
