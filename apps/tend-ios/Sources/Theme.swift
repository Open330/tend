// The visual language of Tend for Mac and the brand: the four-room mark and
// its state colours (assets/brand/tend/STATES.md), the Home lane words.
import SwiftUI
import UIKit

enum TendColor {
    /// Waiting on you (input, a choice): the top-right room.
    static let waiting = Color(red: 1.0, green: 0xAA / 255, blue: 0x2B / 255)
    /// Error or blocked: the top-left room.
    static let error = Color(red: 1.0, green: 0x5A / 255, blue: 0x3C / 255)
    /// Finished: the bottom-left room.
    static let done = Color(red: 0x3F / 255, green: 0xBF / 255, blue: 0x84 / 255)
    /// Working: ink on light, paper on dark (bottom-right room).
    static let working = Color(uiColor: UIColor { traits in
        traits.userInterfaceStyle == .dark
            ? UIColor(red: 0xF2 / 255, green: 0xF3 / 255, blue: 0xF5 / 255, alpha: 1)
            : UIColor(red: 0x16 / 255, green: 0x18 / 255, blue: 0x1D / 255, alpha: 1)
    })
}

/// The F2 four-room mark, drawn from `mark-rooms.svg` (viewBox 4 4 56 56).
struct TendMark: View {
    var waiting = true
    var error = false
    var working = true
    var done = true

    @Environment(\.colorScheme) private var colorScheme

    private static func room(_ points: [(CGFloat, CGFloat)], curves: [Int: (CGFloat, CGFloat)] = [:]) -> Path {
        // `points` in SVG user units; `curves[i]` is the control point of
        // a quadratic curve that ends at points[i].
        var path = Path()
        for (index, point) in points.enumerated() {
            let p = CGPoint(x: point.0 - 4, y: point.1 - 4)
            if index == 0 {
                path.move(to: p)
            } else if let control = curves[index] {
                path.addQuadCurve(to: p, control: CGPoint(x: control.0 - 4, y: control.1 - 4))
            } else {
                path.addLine(to: p)
            }
        }
        path.closeSubpath()
        return path
    }

    private static let waitingRoom = room([(26.266, 4), (57, 4), (60, 7), (60, 17.25), (27.685, 17.25)], curves: [2: (60, 4)])
    private static let errorRoom = room([(4, 7), (7, 4), (20.734, 4), (24.082, 35.25), (4, 35.25)], curves: [1: (4, 4)])
    private static let workingRoom = room([(28.275, 22.75), (60, 22.75), (60, 57), (57, 60), (32.266, 60)], curves: [3: (60, 60)])
    private static let doneRoom = room([(4, 40.75), (24.672, 40.75), (26.734, 60), (7, 60), (4, 57)], curves: [4: (4, 60)])

    var body: some View {
        let dim = TendColor.working.opacity(colorScheme == .dark ? 0.26 : 0.22)
        Canvas { context, size in
            let scale = min(size.width, size.height) / 56
            context.scaleBy(x: scale, y: scale)
            context.fill(Self.waitingRoom, with: .color(waiting ? TendColor.waiting : dim))
            context.fill(Self.errorRoom, with: .color(error ? TendColor.error : dim))
            context.fill(Self.workingRoom, with: .color(working ? TendColor.working : dim))
            context.fill(Self.doneRoom, with: .color(done ? TendColor.done : dim))
        }
        .aspectRatio(1, contentMode: .fit)
        .accessibilityHidden(true)
    }
}

/// A small coloured capsule label (Question, Review, a host).
struct Chip: View {
    let text: String
    var color: Color = .secondary

    var body: some View {
        Text(verbatim: text)
            .font(.caption.weight(.semibold))
            .padding(.horizontal, 7)
            .padding(.vertical, 2)
            .foregroundStyle(color)
            .background(color.opacity(0.14), in: Capsule())
    }
}
