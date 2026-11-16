// Draws an app bundle's icon the way Finder and the Dock do (NSWorkspace),
// so icon previews show macOS's own rendering of an Icon Composer bundle.
// Used by scripts/brand/tend_icons.py; macOS only. The icon style (default,
// dark, clear, tinted) follows the system setting; tend_icons.py uses ictool
// for the other styles.
//
//   render-app-icon <out.png> <pixel size> <path/to/App.app>
import AppKit

let args = CommandLine.arguments
guard args.count == 4, let size = Int(args[2]) else {
    FileHandle.standardError.write("usage: render-app-icon <out.png> <size> <app>\n".data(using: .utf8)!)
    exit(2)
}
let image = NSWorkspace.shared.icon(forFile: args[3])
let rep = NSBitmapImageRep(
    bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size, bitsPerSample: 8, samplesPerPixel: 4,
    hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
rep.size = NSSize(width: size, height: size)
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
image.draw(in: NSRect(x: 0, y: 0, width: size, height: size), from: .zero, operation: .copy, fraction: 1)
NSGraphicsContext.restoreGraphicsState()
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: args[1]))
