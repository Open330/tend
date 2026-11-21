// Still images for the demo video's edit (scripts/demo-video/edit.py):
//
//   cards caption OUT.png TEXT            a 1920x1080 transparent frame with
//                                         TEXT in a pill in the bottom strip
//   cards title OUT.png LOGO.png TAGLINE  the opening overlay
//   cards end OUT.png LOGO.png LINE1 LINE2  the closing card
//   cards mask OUT.png W H RADIUS         a white rounded rectangle on black
import AppKit

let a = CommandLine.arguments
func canvas(_ w: Int, _ h: Int, _ draw: () -> Void) -> NSBitmapImageRep {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: w, pixelsHigh: h, bitsPerSample: 8, samplesPerPixel: 4,
                               hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    draw()
    NSGraphicsContext.restoreGraphicsState()
    return rep
}
func save(_ rep: NSBitmapImageRep, _ path: String) {
    try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: path))
}
func text(_ s: String, size: CGFloat, weight: NSFont.Weight, color: NSColor) -> NSAttributedString {
    let p = NSMutableParagraphStyle(); p.alignment = .center
    return NSAttributedString(string: s, attributes: [.font: NSFont.systemFont(ofSize: size, weight: weight),
                                                      .foregroundColor: color, .paragraphStyle: p, .kern: -0.2])
}
func centered(_ t: NSAttributedString, y: CGFloat, width: CGFloat = 1920) {
    let size = t.size()
    t.draw(at: NSPoint(x: (width - size.width) / 2, y: y))
}
let ink = NSColor(srgbRed: 0.086, green: 0.094, blue: 0.114, alpha: 1)

switch a[1] {
case "caption":
    save(canvas(1920, 1080) {
        let t = text(a[3], size: 34, weight: .semibold, color: .white)
        let size = t.size()
        let pill = NSRect(x: (1920 - size.width) / 2 - 32, y: 22, width: size.width + 64, height: size.height + 26)
        ink.withAlphaComponent(0.88).setFill()
        NSBezierPath(roundedRect: pill, xRadius: pill.height / 2, yRadius: pill.height / 2).fill()
        t.draw(at: NSPoint(x: pill.minX + 32, y: pill.minY + 13))
    }, a[2])
case "title":
    let logo = NSImage(contentsOfFile: a[3])!
    save(canvas(1920, 1080) {
        NSColor.white.withAlphaComponent(0.94).setFill()
        let panel = NSRect(x: 560, y: 330, width: 800, height: 420)
        NSBezierPath(roundedRect: panel, xRadius: 36, yRadius: 36).fill()
        let w: CGFloat = 520, h = w * logo.size.height / logo.size.width
        logo.draw(in: NSRect(x: (1920 - w) / 2, y: 470, width: w, height: h))
        centered(text(a[4], size: 30, weight: .medium, color: ink.withAlphaComponent(0.8)), y: 400)
    }, a[2])
case "end":
    let logo = NSImage(contentsOfFile: a[3])!
    save(canvas(1920, 1080) {
        NSColor.white.setFill(); NSRect(x: 0, y: 0, width: 1920, height: 1080).fill()
        let w: CGFloat = 640, h = w * logo.size.height / logo.size.width
        logo.draw(in: NSRect(x: (1920 - w) / 2, y: 520, width: w, height: h))
        centered(text(a[4], size: 40, weight: .semibold, color: ink), y: 420)
        centered(text(a[5], size: 30, weight: .regular, color: ink.withAlphaComponent(0.6)), y: 360)
    }, a[2])
case "mask":
    let w = Int(a[3])!, h = Int(a[4])!, r = CGFloat(Double(a[5])!)
    save(canvas(w, h) {
        NSColor.black.setFill(); NSRect(x: 0, y: 0, width: w, height: h).fill()
        NSColor.white.setFill()
        NSBezierPath(roundedRect: NSRect(x: 0, y: 0, width: w, height: h), xRadius: r, yRadius: r).fill()
    }, a[2])
default:
    exit(2)
}
