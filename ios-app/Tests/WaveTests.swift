// The wallpaper's water: what a finger or a shake stirs up must spread, stay
// bounded and die down, and a phone lying still must leave it flat.
import Foundation
import Testing
import UIKit
@testable import Tokens

/** Lets the water run for `seconds` at 60 frames a second. */
@MainActor private func run(_ field: WaveField, _ t: inout Double, seconds: Double, each: (() -> Void)? = nil) {
    for _ in 0..<Int((seconds * 60).rounded()) {
        t += 1.0 / 60
        field.step(now: t)
        each?()
    }
}

@MainActor private func side(_ field: WaveField, columns: Range<Int>) -> Float {
    var sum: Float = 0
    for r in 0..<WaveField.rows {
        for c in columns { sum += field.height(c, r) }
    }
    return sum / Float(WaveField.rows * columns.count)
}

private let finger = ObjectIdentifier(WaveTests.self)
private let middle = (c: WaveField.cols / 2, r: WaveField.rows / 2)
private let middlePoint: SIMD2<Float> = [Float(middle.c) / Float(WaveField.cols - 1), Float(middle.r) / Float(WaveField.rows - 1)]

@MainActor @Suite struct WaveTests {
    @Test func flatWaterStaysFlat() {
        let field = WaveField()
        var t = 100.0
        run(field, &t, seconds: 2)
        #expect(field.isStill)
        #expect(!field.lively)
        #expect(field.peak == 0)
    }

    @Test func aTapMakesAWaveThatSpreadsAndDies() async throws {
        let field = WaveField()
        var t = 0.0
        field.step(now: t)
        field.touch(finger, at: middlePoint, phase: .began, time: 0)
        field.touch(finger, at: middlePoint, phase: .ended, time: 0.05)
        #expect(field.lively)

        // It rises under the finger first; a third of the screen away nothing yet.
        run(field, &t, seconds: 0.1)
        #expect(field.height(middle.c, middle.r) > 0.4)
        let away = (c: middle.c + 3, r: middle.r)
        #expect(abs(field.height(away.c, away.r)) < 0.1)

        // Then the wave gets there.
        var reached: Float = 0
        run(field, &t, seconds: 1.5) { reached = max(reached, abs(field.height(away.c, away.r))) }
        #expect(reached > 0.1)

        // And the water ends up flat, with the redraw-every-frame flag off.
        run(field, &t, seconds: 30)
        #expect(field.isStill)
        try await Task.sleep(for: .milliseconds(50))
        #expect(!field.lively)
    }

    @Test func waterIsMovedNeverAdded() {
        let field = WaveField()
        var t = 0.0
        field.step(now: t)
        field.push(at: [0.2, 0.15], strength: 12)
        field.push(at: [0.9, 0.8], strength: -7)
        for _ in 0..<6 {
            run(field, &t, seconds: 0.4)
            #expect(abs(field.mean) < 1e-3)
            #expect(field.peak > 0)
        }
    }

    @Test func aSwipeThrowsMoreThanASlowDrag() {
        /** Drags a finger half-way across the screen in `seconds`; the highest
         *  wave in the second half of the drag. */
        func drag(over seconds: Double) -> Float {
            let field = WaveField()
            var t = 0.0
            field.step(now: t)
            var highest: Float = 0
            let steps = Int(seconds * 60)
            for i in 0...steps {
                let f = Float(i) / Float(steps)
                field.touch(finger, at: [0.25 + 0.5 * f, 0.5], phase: i == 0 ? .began : .moved, time: Double(i) / 60)
                run(field, &t, seconds: 1.0 / 60)
                if i * 2 >= steps { highest = max(highest, field.peak) }
            }
            return highest
        }
        // The same path in a quarter of a second and in eight seconds (by
        // then the splash of the finger landing is gone).
        let fast = drag(over: 0.25)
        let slow = drag(over: 8)
        #expect(fast > 0.8)
        #expect(slow < 0.4)
        #expect(slow > 0)
    }

    @Test func wildInputStaysFiniteAndBounded() {
        let field = WaveField()
        var t = 0.0
        field.step(now: t)
        var seed: UInt64 = 42
        func next() -> Float {
            seed = seed &* 6_364_136_223_846_793_005 &+ 1_442_695_040_888_963_407
            return Float(seed >> 40) / Float(1 << 24)
        }
        for i in 0..<3000 {
            field.push(at: [next() * 1.4 - 0.2, next() * 1.4 - 0.2], strength: (next() - 0.5) * 2e4)
            field.shake(force: [(next() - 0.5) * 200, (next() - 0.5) * 200, (next() - 0.5) * 200], dt: 1.0 / 60)
            if i % 500 == 0 {
                field.push(at: [.nan, 0.5], strength: 5)
                field.push(at: [0.5, 0.5], strength: .infinity)
                field.shake(force: [.nan, 0, 0], dt: 1.0 / 60)
                field.shake(force: [1, 1, 1], dt: .nan)
                field.shake(force: [1, 1, 1], dt: -1)
            }
            // Frames arrive unevenly, sometimes after a long pause.
            t += i % 97 == 0 ? 3 : Double(next()) * 0.05
            field.step(now: t)
            #expect(field.isFinite)
        }
        #expect(field.peak <= 3)
        field.calm()
        #expect(field.isStill)
        #expect(!field.lively)
    }

    @Test func aPhoneHeldStillLeavesTheWaterFlat() {
        let field = WaveField()
        var t = 0.0
        field.step(now: t)
        for i in 0..<600 {
            // A hand's tremor: a few hundredths of a g.
            let a = Float(i) * 0.7
            field.shake(force: [0.03 * sin(a), 0.025 * cos(a * 1.3), 0.02 * sin(a * 0.4)], dt: 1.0 / 60)
            run(field, &t, seconds: 1.0 / 60)
        }
        #expect(field.isStill)
        #expect(!field.lively)
    }

    @Test func shakingMakesTheWaterChoppy() {
        let field = WaveField()
        var t = 0.0
        field.step(now: t)
        var highest: Float = 0
        for i in 0..<60 {
            // Back and forth 3.5 times a second, 1.5 g.
            let s = Float(i) / 60
            field.shake(force: [1.5 * sin(s * 22), 0.4 * sin(s * 19 + 1), 0], dt: 1.0 / 60)
            run(field, &t, seconds: 1.0 / 60)
            highest = max(highest, field.peak)
        }
        #expect(field.lively)
        #expect(highest > 0.4)
        #expect(abs(field.mean) < 1e-3)
    }

    @Test func aPushToTheRightPilesTheWaterUpOnTheRight() {
        let field = WaveField()
        var t = 0.0
        field.step(now: t)
        // Too short and gentle for a splash, so only the lean is left.
        for _ in 0..<9 {
            field.shake(force: [0.5, 0, 0], dt: 1.0 / 60)
            run(field, &t, seconds: 1.0 / 60)
        }
        run(field, &t, seconds: 0.2)
        let half = WaveField.cols / 2
        #expect(side(field, columns: half..<WaveField.cols) > 0.05)
        #expect(side(field, columns: 0..<half) < -0.05)
    }
}

@Suite struct ShakeFilterTests {
    private let upright: SIMD3<Float> = [0, -1, 0]

    @Test func aPhoneAtRestFeelsNothing() {
        var filter = ShakeFilter()
        for _ in 0..<120 {
            let f = filter.force(gravity: upright, hand: .zero, dt: 1.0 / 60)
            #expect(f == .zero)
        }
    }

    @Test func theWaterIsLeftBehindWhenTheHandMoves() {
        var filter = ShakeFilter()
        _ = filter.force(gravity: upright, hand: .zero, dt: 1.0 / 60)
        // The phone speeds up to the right: the water goes left.
        #expect(filter.force(gravity: upright, hand: [1, 0, 0], dt: 1.0 / 60).x < -0.9)
        // The phone speeds up towards its top: the water goes down the screen.
        #expect(filter.force(gravity: upright, hand: [0, 1, 0], dt: 1.0 / 60).y > 0.9)
    }

    @Test func tiltingSloshesButHoldingATiltDoesNot() {
        var filter = ShakeFilter()
        _ = filter.force(gravity: upright, hand: .zero, dt: 1.0 / 60)
        // Tipped 25° to the right: the water runs to the right.
        let tipped: SIMD3<Float> = [sin(0.44), -cos(0.44), 0]
        let first = filter.force(gravity: tipped, hand: .zero, dt: 1.0 / 60)
        #expect(first.x > 0.3)
        var last = first
        for _ in 0..<300 { last = filter.force(gravity: tipped, hand: .zero, dt: 1.0 / 60) }
        #expect((last * last).sum().squareRoot() < 0.01)
    }
}

@Suite struct WallpaperLookTests {
    @Test func aSampleAtALatticeNodeIsThatNode() {
        for palette in [WallpaperPalette.day, WallpaperPalette.night] {
            #expect(palette.count == 16)
            for r in 0..<4 {
                for c in 0..<4 {
                    let got = Lattice.sample(palette, u: Float(c) / 3, v: Float(r) / 3)
                    let diff = got - palette[r * 4 + c]
                    #expect((diff * diff).sum().squareRoot() < 1e-4)
                }
            }
        }
        // Outside the screen is the nearest edge.
        #expect(Lattice.sample(WallpaperPalette.day, u: -1, v: 2) == Lattice.sample(WallpaperPalette.day, u: 0, v: 1))
    }

    @Test func thePaletteKeepsItsCorners() {
        // Claude orange top left by day and by night, blue on the right.
        #expect(WallpaperPalette.dayHex.first == "#F5691F")
        #expect(WallpaperPalette.nightHex.first == "#F5822F")
        let blue = WallpaperPalette.day[7]
        #expect(blue.z > blue.x)
    }

    @Test func coloursOnTheEdgeOnlySlideAlongIt() {
        for t in stride(from: Float(0), to: 60, by: 1.7) {
            let drift = Lattice.drift(t)
            for r in 0..<4 {
                for c in 0..<4 {
                    let d = drift[r * 4 + c]
                    if c == 0 || c == 3 { #expect(d.x == 0) }
                    if r == 0 || r == 3 { #expect(d.y == 0) }
                    #expect(abs(d.x) <= 0.1401 && abs(d.y) <= 0.1401)
                }
            }
        }
    }

    @MainActor @Test func everyGridNodeGetsAColour() {
        let grid = WallpaperFrame.grid
        #expect(grid.count == WaveField.cols * WaveField.rows)
        #expect(grid.first == [0, 0])
        #expect(grid.last == [1, 1])
        let field = WaveField()
        #expect(WallpaperFrame.colors(time: 10, dark: false, field: field).count == grid.count)
        field.push(at: [0.5, 0.5], strength: 14)
        #expect(WallpaperFrame.colors(time: 10.1, dark: true, field: field).count == grid.count)
        // Drawing a frame moves the water on.
        #expect(field.peak > 0)
    }
}
