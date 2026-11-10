// The app's wallpaper: a sea of colour that drifts on its own, makes waves
// under the finger and sloshes when the phone is shaken.
//
// Colours: a 4×4 palette (Claude orange top left, Codex blue on the right,
// violet bottom left by day; one orange glow in deep wine, violet and blue by
// night) whose control points wander slowly. On top of that the wallpaper is
// a small water surface (`WaveField`): a touch or a shake pushes it, the wave
// spreads, bounces off the screen edges and dies down. The surface bends the
// colours (like looking through water) and lights its crests.
//
// Drawn as one MeshGradient on a fine, regular grid that never moves; each
// frame only computes the colour of every node. So nothing can fold, however
// wild the water gets.
//
// Still when Reduce Motion is on and while the app is not in front.
import CoreMotion
import Observation
import SwiftUI
import UIKit
import UIKit.UIGestureRecognizerSubclass

struct Wallpaper: View {
    let dark: Bool

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.scenePhase) private var phase

    var body: some View {
        let field = WaveField.shared
        // Only the gradient is redrawn per frame, not the screen above it:
        // 30 fps for the slow drift, every frame while there are waves.
        TimelineView(.animation(minimumInterval: field.lively ? nil : 1.0 / 30.0, paused: reduceMotion || phase != .active)) { timeline in
            let colors = WallpaperFrame.colors(time: timeline.date.timeIntervalSinceReferenceDate, dark: dark, field: field)
            ZStack {
                MeshGradient(
                    width: WaveField.cols,
                    height: WaveField.rows,
                    points: WallpaperFrame.grid,
                    colors: colors,
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
}

// MARK: palettes

enum WallpaperPalette {
    // Row by row, top to bottom. The orange is saturated rather than light
    // (more chroma, not more white).
    static let dayHex = [
        "#F5691F", "#EE7436", "#D9866E", "#8090D8",
        "#EE7A3C", "#DC8468", "#A88FC0", "#6292D6",
        "#B88CB4", "#A58CC4", "#8F97D4", "#7E9BDA",
        "#8C78D6", "#8F84D8", "#8C9BD6", "#8FA3DC",
    ]

    // Night. Warm colours turn brown when they are dimmed, cool ones stay deep,
    // so the dark palette is not a dimmed copy of the day one: a single small
    // glow of vivid orange in the top-left corner, and everything dark around
    // it in wine, violet and blue (the fall-off runs orange -> wine -> violet
    // -> blue, never through brown). Third round of candidates; the user
    // rejected a uniformly dimmed orange as ugly.
    static let nightHex = [
        "#F5822F", "#A2443E", "#522A68", "#2A2A78",
        "#8A3248", "#582A6C", "#34307E", "#24408E",
        "#2E1E5A", "#26266E", "#1E3684", "#1A4690",
        "#0E1030", "#0E1638", "#0E1E44", "#0C2650",
    ]

    static func rgb(_ hex: String) -> SIMD3<Float> {
        let n = UInt32(hex.dropFirst(hex.hasPrefix("#") ? 1 : 0), radix: 16) ?? 0
        return SIMD3(Float((n >> 16) & 0xFF), Float((n >> 8) & 0xFF), Float(n & 0xFF)) / 255
    }

    /** The built-in palette, or in a Debug build on the Simulator
     *  `TC_PREVIEW_DAY` / `TC_PREVIEW_NIGHT` (16 comma-separated #RRGGBB
     *  values) to try another without a rebuild. */
    private static func load(_ hexes: [String], _ variable: String) -> [SIMD3<Float>] {
        var hexes = hexes
        #if DEBUG && targetEnvironment(simulator)
        let override = (ProcessInfo.processInfo.environment[variable] ?? "").split(separator: ",").map(String.init)
        if override.count == 16 { hexes = override }
        #endif
        return hexes.map(rgb)
    }

    static let day = load(dayHex, "TC_PREVIEW_DAY")
    static let night = load(nightHex, "TC_PREVIEW_NIGHT")
}

// MARK: the 4×4 lattice

enum Lattice {
    /** Where the 16 control points sit when nothing moves, row by row. */
    static let rest: [SIMD2<Float>] = (0..<16).map { [Float($0 % 4) / 3, Float($0 / 4) / 3] }

    /** The 16 control points at time `t` (seconds). Corners stay put and
     *  border points only slide along their edge; inner points roam. Every
     *  point has its own loop (about 4–6 s; at 10–15 s the user took the
     *  wallpaper for a still image). */
    static func points(_ t: Float) -> [SIMD2<Float>] {
        var out = rest
        for row in 0..<4 {
            for col in 0..<4 {
                let i = Float(row * 4 + col)
                if col != 0 && col != 3 {
                    out[row * 4 + col].x += 0.14 * sin(t * (1.25 + 0.25 * sin(i * 1.7)) + i * 2.1)
                }
                if row != 0 && row != 3 {
                    out[row * 4 + col].y += 0.14 * cos(t * (1.10 + 0.25 * cos(i * 2.3)) + i * 1.3)
                }
            }
        }
        return out
    }

    /** How far each control point is from its rest position at time `t`. */
    static func drift(_ t: Float) -> [SIMD2<Float>] {
        zip(points(t), rest).map { $0 - $1 }
    }

    /** Catmull-Rom through four values; `f` in 0…1 runs from `b` to `c`. */
    private static func spline<V: SIMD>(_ a: V, _ b: V, _ c: V, _ d: V, _ f: Float) -> V where V.Scalar == Float {
        let f2 = f * f
        let f3 = f2 * f
        let p1 = (c - a) * f
        let p2 = (a * 2 - b * 5 + c * 4 - d) * f2
        let p3 = (b * 3 - a - c * 3 + d) * f3
        return (b * 2 + p1 + p2 + p3) * 0.5
    }

    /** A smooth sample of a 4×4 lattice at (u, v), each clamped to 0…1;
     *  exact at the lattice's own nodes. */
    static func sample<V: SIMD>(_ lattice: [V], u: Float, v: Float) -> V where V.Scalar == Float {
        func split(_ x: Float) -> (Int, Float) {
            let g = min(max(x, 0), 1) * 3
            let i = min(Int(g), 2)
            return (i, g - Float(i))
        }
        let (ci, cf) = split(u)
        let (ri, rf) = split(v)
        func row(_ r: Int) -> V {
            let base = min(max(r, 0), 3) * 4
            return spline(lattice[base + max(ci - 1, 0)], lattice[base + ci], lattice[base + ci + 1], lattice[base + min(ci + 2, 3)], cf)
        }
        return spline(row(ri - 1), row(ri), row(ri + 1), row(ri + 2), rf)
    }
}

// MARK: one frame

@MainActor
enum WallpaperFrame {
    /** The mesh: a regular grid, row by row. Only its colours change. */
    static let grid: [SIMD2<Float>] = (0..<WaveField.rows).flatMap { r in
        (0..<WaveField.cols).map { c in SIMD2(Float(c) / Float(WaveField.cols - 1), Float(r) / Float(WaveField.rows - 1)) }
    }

    // How strongly the water shows: how far its slope bends the picture
    // (screen widths per unit of slope, and at most), how much a crest is
    // lit (towards white, and brighter) and a trough shaded.
    static let bend: Float = Tuning.wave("TC_PREVIEW_BEND", 0.035)
    static let maxBend: Float = 0.12
    // The sea has two colours: a crest takes the colour from further towards
    // the orange corner (top left), a trough from further towards the blue
    // one, by this much of the screen per unit of height. This is what makes
    // a wave read as orange and blue water rather than as a light patch.
    static let tide: Float = Tuning.wave("TC_PREVIEW_TIDE", 0.22)
    static let gain: Float = Tuning.wave("TC_PREVIEW_GAIN", 2.2)
    static let crestGlow: Float = Tuning.wave("TC_PREVIEW_GLOW", 0.22)
    static let crestLift: Float = Tuning.wave("TC_PREVIEW_LIFT", 0.08)
    static let troughShade: Float = Tuning.wave("TC_PREVIEW_SHADE", 0.18)

    /** The colour of every grid node at `time`, after moving the water on. */
    static func colors(time: TimeInterval, dark: Bool, field: WaveField) -> [Color] {
        field.step(now: time)
        let palette = dark ? WallpaperPalette.night : WallpaperPalette.day
        let drift = Lattice.drift(Float(time.truncatingRemainder(dividingBy: 3600)))
        let waves = field.lively
        var colors: [Color] = []
        colors.reserveCapacity(grid.count)
        for r in 0..<WaveField.rows {
            for c in 0..<WaveField.cols {
                let at = grid[r * WaveField.cols + c]
                // The colour that has wandered here: look where it came from.
                var from = at - Lattice.sample(drift, u: at.x, v: at.y)
                var height: Float = 0
                if waves {
                    // Like looking through water: the picture is shifted
                    // along the surface's slope.
                    var shift = field.slope(c, r) * bend
                    let length = (shift * shift).sum().squareRoot()
                    if length > maxBend { shift *= maxBend / length }
                    // What is shown is not the height itself: a ring that has
                    // spread out is a fraction of the splash it came from, and
                    // would be invisible next to it. So small waves are shown
                    // large and big ones level off (-1…1).
                    height = tanh(field.height(c, r) * gain)
                    from += SIMD2(shift.x, shift.y / field.aspect) - SIMD2(repeating: tide * height)
                }
                var rgb = Lattice.sample(palette, u: from.x, v: from.y)
                if height > 0 {
                    rgb += (SIMD3(repeating: 1) - rgb) * (crestGlow * height)
                    rgb *= 1 + crestLift * height
                } else if height < 0 {
                    rgb *= 1 + troughShade * height
                }
                let k = rgb.clamped(lowerBound: .zero, upperBound: .one)
                colors.append(Color(.sRGB, red: Double(k.x), green: Double(k.y), blue: Double(k.z)))
            }
        }
        return colors
    }
}

extension Tuning {
    /** A wave constant, or its override from the environment in a Debug
     *  build on the Simulator. */
    static func wave(_ name: String, _ fallback: Float) -> Float {
        #if DEBUG && targetEnvironment(simulator)
        if let v = ProcessInfo.processInfo.environment[name].flatMap(Float.init) { return v }
        #endif
        return fallback
    }
}

// MARK: the water surface

/** A height field on the grid that obeys the wave equation: what a touch or
 *  a shake stirs up spreads, reflects off the screen edges and fades.
 *  Positions are screen fractions (0…1 across, 0…1 down); distances are in
 *  screen widths. */
@MainActor
@Observable
final class WaveField {
    static let shared = WaveField()

    nonisolated static let cols = 10
    nonisolated static let rows = 22

    /** True while there are waves (the wallpaper then redraws every frame). */
    private(set) var lively = false

    /** Screen height / width, so a wave is round on a tall screen. */
    @ObservationIgnored var aspect: Float = 2.17

    @ObservationIgnored private var h = [Float](repeating: 0, count: cols * rows)
    @ObservationIgnored private var v = [Float](repeating: 0, count: cols * rows)
    @ObservationIgnored private var lastStep: TimeInterval?
    @ObservationIgnored private var fingers: [ObjectIdentifier: (p: SIMD2<Float>, t: TimeInterval)] = [:]
    @ObservationIgnored private var agitation: Float = 0
    @ObservationIgnored private var dice: UInt64 = 0x9E37_79B9_7F4A_7C15
    @ObservationIgnored private var settling = false

    // The water: wave speed (screen widths per second), how fast waves die
    // and how fast a raised surface sinks back.
    static let speed: Float = Tuning.wave("TC_PREVIEW_SPEED", 0.75)
    static let damping: Float = Tuning.wave("TC_PREVIEW_DAMPING", 0.8)
    static let sink: Float = 0.25
    // A finger: its reach, how hard a tap and a swipe push.
    static let reach: Float = Tuning.wave("TC_PREVIEW_REACH", 0.22)
    static let tapPush: Float = Tuning.wave("TC_PREVIEW_TAP", 14)
    static let swipePush: Float = Tuning.wave("TC_PREVIEW_SWIPE", 40)
    static let maxFingerSpeed: Float = 4
    // The phone moving (forces in g): below `stillness` nothing happens (a
    // hand is never quite still), above `maxForce` no more; how hard the
    // water leans, and how much shaking makes one splash of what strength.
    static let stillness: Float = 0.06
    static let maxForce: Float = 2.5
    static let leanPush: Float = Tuning.wave("TC_PREVIEW_LEAN", 50)
    static let splashEvery: Float = Tuning.wave("TC_PREVIEW_SPLASH_EVERY", 0.10)
    static let splashPush: Float = Tuning.wave("TC_PREVIEW_SPLASH", 9)

    func height(_ c: Int, _ r: Int) -> Float { h[r * Self.cols + c] }

    /** The highest crest or deepest trough right now. */
    var peak: Float { h.reduce(0) { max($0, abs($1)) } }
    /** The average height (stays zero: water is moved, never added). */
    var mean: Float { h.reduce(0, +) / Float(h.count) }
    var isStill: Bool { !h.contains { $0 != 0 } && !v.contains { $0 != 0 } }
    var isFinite: Bool { h.allSatisfy(\.isFinite) && v.allSatisfy(\.isFinite) }

    /** The surface's slope at a node, in height per screen width. */
    func slope(_ c: Int, _ r: Int) -> SIMD2<Float> {
        let cols = Self.cols
        let rows = Self.rows
        let left = h[r * cols + max(c - 1, 0)]
        let right = h[r * cols + min(c + 1, cols - 1)]
        let up = h[max(r - 1, 0) * cols + c]
        let down = h[min(r + 1, rows - 1) * cols + c]
        return SIMD2((right - left) * Float(cols - 1) / 2, (down - up) * Float(rows - 1) / 2 / aspect)
    }

    // MARK: what stirs it

    /** A finger went down, moved or lifted at `p`. `time` is the touch's own
     *  timestamp (seconds). */
    func touch(_ finger: ObjectIdentifier, at p: SIMD2<Float>, phase: UITouch.Phase, time: TimeInterval) {
        switch phase {
        case .began:
            fingers[finger] = (p, time)
            push(at: p, strength: Self.tapPush)
        case .moved, .stationary:
            if let last = fingers[finger] {
                let dt = Float(max(time - last.t, 1.0 / 240))
                var d = p - last.p
                d.y *= aspect
                let speed = min((d * d).sum().squareRoot() / dt, Self.maxFingerSpeed)
                // A slow drag barely stirs the water; a swipe throws a wave.
                push(at: p, strength: Self.swipePush * speed * min(dt, 0.05))
            }
            fingers[finger] = (p, time)
        default:
            fingers[finger] = nil
        }
    }

    /** Every finger is off the screen. */
    func liftAll() { fingers.removeAll() }

    /** Pushes the water up around `p` (a smooth bump of radius `reach`; a
     *  negative strength pushes it down). The water comes from everywhere
     *  else, so the average height stays the same. */
    func push(at p: SIMD2<Float>, strength: Float) {
        guard strength.isFinite, strength != 0, p.x.isFinite, p.y.isFinite else { return }
        var bump = [Float](repeating: 0, count: h.count)
        var total: Float = 0
        for r in 0..<Self.rows {
            for c in 0..<Self.cols {
                let dx = Float(c) / Float(Self.cols - 1) - p.x
                let dy = (Float(r) / Float(Self.rows - 1) - p.y) * aspect
                let d = (dx * dx + dy * dy).squareRoot()
                guard d < Self.reach else { continue }
                let g = cos(d / Self.reach * .pi / 2)
                bump[r * Self.cols + c] = g * g
                total += g * g
            }
        }
        guard total > 0 else { return }
        let mean = total / Float(bump.count)
        for i in bump.indices {
            v[i] += strength * (bump[i] - mean)
        }
        if !lively { lively = true }
    }

    /** The phone itself moved for `dt` seconds. `force` is what the water
     *  feels, in g: x to the right and y down along the screen, z out of it.
     *  The water leans the way it is pushed (and sloshes back), and hard
     *  shaking in any direction makes it choppy: a splash somewhere for
     *  every `splashEvery` g·seconds of shaking. */
    func shake(force: SIMD3<Float>, dt: Float) {
        let size = (force * force).sum().squareRoot()
        guard size.isFinite, dt.isFinite, dt > 0, size > Self.stillness else { return }
        let felt = min(size, Self.maxForce) - Self.stillness
        let f = force * (felt / size)
        let time = min(dt, 0.05)
        for r in 0..<Self.rows {
            for c in 0..<Self.cols {
                let x = Float(c) / Float(Self.cols - 1) - 0.5
                let y = Float(r) / Float(Self.rows - 1) - 0.5
                v[r * Self.cols + c] += Self.leanPush * (f.x * x + f.y * y) * time
            }
        }
        if !lively { lively = true }
        agitation += felt * time
        while agitation >= Self.splashEvery {
            agitation -= Self.splashEvery
            let p: SIMD2<Float> = [roll(), roll()]
            push(at: p, strength: roll() < 0.5 ? -Self.splashPush : Self.splashPush)
        }
    }

    /** The next number in 0..<1 of a fixed sequence (SplitMix64): where the
     *  splashes of a shake land. Fixed, so tests see the same sea every run. */
    private func roll() -> Float {
        dice &+= 0x9E37_79B9_7F4A_7C15
        var z = dice
        z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
        z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
        z ^= z >> 31
        return Float(z >> 40) / Float(1 << 24)
    }

    // MARK: moving on

    /** Advances the surface to `now` (seconds). Costs nothing while flat. */
    func step(now: TimeInterval) {
        defer { lastStep = now }
        guard lively, let last = lastStep else { return }
        var remaining = Float(min(max(now - last, 0), 0.1))
        while remaining > 1e-4 {
            let dt = min(remaining, 1.0 / 90)
            advance(dt)
            remaining -= dt
        }
        // Flat again, and nobody touching: stop redrawing every frame. The
        // flag is read by the view being drawn right now, so it changes
        // after this frame.
        if fingers.isEmpty, !h.contains(where: { abs($0) > 0.01 }), !v.contains(where: { abs($0) > 0.05 }) {
            flatten()
            if !settling {
                settling = true
                Task { @MainActor in
                    self.settling = false
                    if self.isStill, self.lively { self.lively = false }
                }
            }
        }
    }

    /** Flat and at rest, at once. */
    func calm() {
        flatten()
        fingers.removeAll()
        agitation = 0
        if lively { lively = false }
    }

    private func flatten() {
        for i in h.indices {
            h[i] = 0
            v[i] = 0
        }
    }

    private func advance(_ dt: Float) {
        let cols = Self.cols
        let rows = Self.rows
        let ix = Float(cols - 1)
        let iy = Float(rows - 1) / aspect
        let c2 = Self.speed * Self.speed
        let keep = exp(-Self.damping * dt)
        let stay = exp(-Self.sink * dt)
        // The edges reflect: a missing neighbour counts as the node itself.
        for r in 0..<rows {
            for c in 0..<cols {
                let i = r * cols + c
                let here = h[i]
                let lapX = (h[r * cols + max(c - 1, 0)] + h[r * cols + min(c + 1, cols - 1)] - 2 * here) * ix * ix
                let lapY = (h[max(r - 1, 0) * cols + c] + h[min(r + 1, rows - 1) * cols + c] - 2 * here) * iy * iy
                v[i] = (v[i] + c2 * (lapX + lapY) * dt) * keep
            }
        }
        for i in h.indices {
            h[i] = min(max((h[i] + v[i] * dt) * stay, -3), 3)
        }
    }
}

// MARK: the phone's own movement

/** Turns what the motion sensors report into the force the water feels. */
struct ShakeFilter {
    /** How long a new way of holding the phone takes to feel level again. */
    static let settle: Float = 0.45

    private var level: SIMD3<Float>?

    /** `gravity` and `hand` (the acceleration the hand gives the phone) are
     *  in g in the device's frame: x right, y to the top of the phone, z out
     *  of the screen. Returns the force on the water: x right, y down the
     *  screen, z out of it.
     *
     *  Gravity only counts while it changes: tilting the phone makes the
     *  water run to the low side and slosh, but holding it tilted does not
     *  keep the water leaning. And water is left behind when the phone
     *  speeds up, so the hand's acceleration pushes it the other way. */
    mutating func force(gravity: SIMD3<Float>, hand: SIMD3<Float>, dt: Float) -> SIMD3<Float> {
        var now = level ?? gravity
        now += (gravity - now) * (1 - exp(-max(dt, 0) / Self.settle))
        level = now
        let f = (gravity - now) - hand
        return [f.x, -f.y, f.z]
    }
}

/** Feeds the phone's movement to the water while the app is in front. The
 *  accelerometer and gyroscope need no permission. */
@MainActor
final class MotionWaves {
    static let shared = MotionWaves()

    private let manager = CMMotionManager()
    private var filter = ShakeFilter()
    private var last: TimeInterval?

    func start() {
        guard manager.isDeviceMotionAvailable, !manager.isDeviceMotionActive else { return }
        filter = ShakeFilter()
        last = nil
        manager.deviceMotionUpdateInterval = 1.0 / 60
        manager.startDeviceMotionUpdates(to: .main) { motion, _ in
            guard let motion else { return }
            let gravity = SIMD3(Float(motion.gravity.x), Float(motion.gravity.y), Float(motion.gravity.z))
            let hand = SIMD3(Float(motion.userAcceleration.x), Float(motion.userAcceleration.y), Float(motion.userAcceleration.z))
            let time = motion.timestamp
            // Delivered on the main queue.
            MainActor.assumeIsolated {
                MotionWaves.shared.feed(gravity: gravity, hand: hand, time: time)
            }
        }
    }

    func stop() {
        if manager.isDeviceMotionActive { manager.stopDeviceMotionUpdates() }
    }

    private func feed(gravity: SIMD3<Float>, hand: SIMD3<Float>, time: TimeInterval) {
        let dt = Float(min(max(time - (last ?? time), 0), 0.1))
        last = time
        let force = filter.force(gravity: gravity, hand: hand, dt: dt)
        WaveField.shared.shake(force: force, dt: dt)
    }
}

// MARK: listening to touches

/** Reports every touch in the window to the water surface without taking
 *  part in gesture recognition: scrolling, buttons and sheets work as before. */
final class WaveTouchRecognizer: UIGestureRecognizer {
    init() {
        super.init(target: nil, action: nil)
        cancelsTouchesInView = false
        delaysTouchesBegan = false
        delaysTouchesEnded = false
    }

    override func canPrevent(_ other: UIGestureRecognizer) -> Bool { false }
    override func canBePrevented(by other: UIGestureRecognizer) -> Bool { false }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) { forward(touches) }
    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent) { forward(touches) }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent) {
        forward(touches)
        finishIfLifted(event)
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent) {
        forward(touches)
        finishIfLifted(event)
    }

    override func reset() {
        super.reset()
        WaveField.shared.liftAll()
    }

    private func finishIfLifted(_ event: UIEvent) {
        let down = event.allTouches?.contains { $0.phase != .ended && $0.phase != .cancelled } ?? false
        if !down { state = .failed }
    }

    private func forward(_ touches: Set<UITouch>) {
        guard let view, view.bounds.width > 0, view.bounds.height > 0 else { return }
        let field = WaveField.shared
        let still = UIAccessibility.isReduceMotionEnabled
        field.aspect = Float(view.bounds.height / view.bounds.width)
        for touch in touches {
            let p = touch.location(in: view)
            field.touch(
                ObjectIdentifier(touch),
                at: [Float(p.x / view.bounds.width), Float(p.y / view.bounds.height)],
                phase: still ? .ended : touch.phase,
                time: touch.timestamp
            )
        }
    }
}

/** Puts the touch listener on the window the app is shown in. */
struct WaveTouchInstaller: UIViewRepresentable {
    func makeUIView(context: Context) -> Probe { Probe() }
    func updateUIView(_ view: Probe, context: Context) {}

    final class Probe: UIView {
        private var recognizer: WaveTouchRecognizer?

        override func didMoveToWindow() {
            super.didMoveToWindow()
            if let old = recognizer {
                old.view?.removeGestureRecognizer(old)
                recognizer = nil
            }
            guard let window else { return }
            if window.bounds.width > 0 {
                WaveField.shared.aspect = Float(window.bounds.height / window.bounds.width)
            }
            let r = WaveTouchRecognizer()
            window.addGestureRecognizer(r)
            recognizer = r
        }
    }
}

/** Everything that stirs the water: fingers anywhere in the window, and the
 *  phone's own movement while the app is in front. Invisible; put it behind
 *  the app's content. */
struct WaveSources: View {
    @Environment(\.scenePhase) private var phase
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        WaveTouchInstaller()
            .allowsHitTesting(false)
            .accessibilityHidden(true)
            .onChange(of: phase == .active && !reduceMotion, initial: true) { _, on in
                if on {
                    MotionWaves.shared.start()
                } else {
                    MotionWaves.shared.stop()
                    WaveField.shared.calm()
                }
            }
        #if DEBUG && targetEnvironment(simulator)
            .task { await DebugWaves.runIfAsked() }
        #endif
    }
}

#if DEBUG && targetEnvironment(simulator)
/** The Simulator has no motion sensors and screenshots can't follow a
 *  finger: `TC_PREVIEW_WAVES=swipe|tap|shake|tilt` stirs the water by itself;
 *  `watch` does not. With `TC_PREVIEW_WAVES_LOG=1` the highest wave is printed
 *  ten times a second. */
@MainActor
enum DebugWaves {
    static func runIfAsked() async {
        guard let mode = ProcessInfo.processInfo.environment["TC_PREVIEW_WAVES"] else { return }
        let field = WaveField.shared
        let finger = ObjectIdentifier(DebugWaves.self)
        var filter = ShakeFilter()
        var t: Float = 0
        var lastTap: Float = -10
        while !Task.isCancelled {
            try? await Task.sleep(for: .milliseconds(16))
            let dt: Float = 0.016
            t += dt
            if ProcessInfo.processInfo.environment["TC_PREVIEW_WAVES_LOG"] != nil, Int(t / dt) % 6 == 0 {
                print(String(format: "wave t=%.2f peak=%.3f lively=%d", t, field.peak, field.lively ? 1 : 0))
            }
            switch mode {
            case "watch": // nothing: real touches only (with TC_PREVIEW_WAVES_LOG)
                break
            case "shake": // 1.2 s of hard shaking every 6 s
                if t.truncatingRemainder(dividingBy: 6) < 1.2 {
                    field.shake(force: [1.6 * sin(t * 22), 0.7 * sin(t * 19 + 1), 0.3 * sin(t * 25)], dt: dt)
                }
            case "tilt": // the phone rocks 25° to the right and back, every 6 s
                let s = t.truncatingRemainder(dividingBy: 6)
                let angle: Float = s < 0.4 ? s / 0.4 * 0.44 : s < 3 ? 0.44 : s < 3.4 ? (3.4 - s) / 0.4 * 0.44 : 0
                field.shake(force: filter.force(gravity: [sin(angle), -cos(angle), 0], hand: .zero, dt: dt), dt: dt)
            case "tap": // one tap in the middle every 4 s
                if t - lastTap > 4 {
                    lastTap = t
                    field.touch(finger, at: [0.5, 0.45], phase: .began, time: Double(t))
                    field.touch(finger, at: [0.5, 0.45], phase: .ended, time: Double(t))
                }
            default: // a finger swiping up and down the screen
                let s = t.truncatingRemainder(dividingBy: 5)
                if s < 0.5 {
                    let p: SIMD2<Float> = [0.35 + 0.3 * s / 0.5, 0.75 - 0.5 * s / 0.5]
                    field.touch(finger, at: p, phase: s < dt * 1.5 ? .began : .moved, time: Double(t))
                } else {
                    field.touch(finger, at: .zero, phase: .ended, time: Double(t))
                }
            }
        }
    }
}
#endif
