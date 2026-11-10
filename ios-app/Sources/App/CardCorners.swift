// Card corners that are concentric with the screen's rounded corners.
//
// A card sits `Card.margin` from the screen edges, so its radius should be the
// screen's corner radius minus that margin (the same curve, inset). SwiftUI's
// ConcentricRectangle resolves that per position (a card that happens to be
// away from the corners falls back to its minimum, so the radius would change
// between screens), and it cannot be a `containerShape` for what is inside.
// UIKit can report the number: a probe view the size of the screen, inset by
// the margin, asks for a container-concentric corner and reads its effective
// radius. The cards then use that one fixed radius everywhere, and declare
// themselves as the container so the tiles inside can follow their corners.
import SwiftUI
import UIKit

enum Card {
    /** Distance between a card and the screen's side edges. */
    static let margin: CGFloat = 12
    /** On a screen with square corners, and until the probe has reported. */
    static let minimumRadius: CGFloat = 26
}

extension EnvironmentValues {
    /** The corner radius of the app's cards (see CardCorners.swift). */
    @Entry var cardRadius: CGFloat = Card.minimumRadius
}

extension View {
    /** A card: Liquid Glass in the screen-concentric shape, and the container
     *  its content's concentric shapes resolve against. */
    func cardSurface(tint: Color, radius: CGFloat) -> some View {
        let shape = RoundedRectangle(cornerRadius: radius, style: .continuous)
        return containerShape(shape).glassEffect(.regular.tint(tint), in: shape)
    }
}

/** Publishes `cardRadius` to everything inside. */
struct CardRadiusProvider<Content: View>: View {
    @ViewBuilder var content: Content
    @State private var radius = Card.minimumRadius

    var body: some View {
        content
            .environment(\.cardRadius, radius)
            .background {
                ConcentricRadiusProbe(minimum: Card.minimumRadius) { radius = $0 }
                    .padding(Card.margin)
                    .ignoresSafeArea()
                    .allowsHitTesting(false)
                    .accessibilityHidden(true)
            }
    }
}

private struct ConcentricRadiusProbe: UIViewRepresentable {
    var minimum: CGFloat
    var onChange: (CGFloat) -> Void

    func makeUIView(context: Context) -> ProbeView {
        let v = ProbeView()
        v.isUserInteractionEnabled = false
        v.backgroundColor = .clear
        v.cornerConfiguration = .corners(radius: .containerConcentric(minimum: minimum))
        v.onChange = onChange
        return v
    }

    func updateUIView(_ v: ProbeView, context: Context) {
        v.onChange = onChange
    }

    final class ProbeView: UIView {
        var onChange: ((CGFloat) -> Void)?
        private var reported: CGFloat = -1

        override func layoutSubviews() {
            super.layoutSubviews()
            guard window != nil else { return }
            let r = effectiveRadius(corner: .bottomLeft)
            guard r.isFinite, abs(r - reported) > 0.5 else { return }
            reported = r
            // Not during layout: this changes SwiftUI state.
            DispatchQueue.main.async { [weak self] in self?.onChange?(r) }
        }

        override func didMoveToWindow() {
            super.didMoveToWindow()
            setNeedsLayout()
        }
    }
}
