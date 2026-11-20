import AppKit
import Foundation
import SwiftUI
import Testing
@testable import Tend

// The title bar row under the traffic lights is opaque in every sidebar
// state: list shown or folded (Home), sidebar material on or off, light and
// dark. With the material on, the side bar's behind-window effect used to
// reach into that row, so the desktop's tint showed under the traffic lights
// beside the opaque tab strip (0.10.28, most visible with the list folded).
// The window here is off-screen and never key, so the material draws its
// inactive look; the row must not depend on it either way.

@MainActor
@Suite(.serialized)
struct SidebarTitleBarTests {
    /// A real window's title bar: 32pt tall, traffic lights in the first 80pt.
    private let chrome = TitleBarMetrics(height: 32, leadingInset: 80)

    @Test(arguments: [false, true])
    func titleBarRowIsTheStripSurface(material: Bool) async throws {
        let defaults = UserDefaults.standard
        let saved = Bundle.main.bundleIdentifier
            .flatMap(defaults.persistentDomain(forName:))?[TendPreferences.sidebarMaterialKey]
        defer {
            if let saved { defaults.set(saved, forKey: TendPreferences.sidebarMaterialKey) }
            else { defaults.removeObject(forKey: TendPreferences.sidebarMaterialKey) }
        }
        defaults.set(material, forKey: TendPreferences.sidebarMaterialKey)

        let model = AppModel(client: TendIPCClient(socketPath: "/tmp/tend-titlebar-\(UUID().uuidString).sock"), launchesDaemon: false)
        for scheme in [ColorScheme.light, .dark] {
            for collapsed in [true, false] {
                model.select(collapsed ? .workBoard : .watch)
                try #require(model.isSidebarListCollapsed == collapsed, "the sidebar did not \(collapsed ? "fold" : "show") its list")
                let width = collapsed ? TendTheme.activityBarWidth + 1 : 300
                let rep = try await render(model, scheme: scheme, size: CGSize(width: width, height: 580))
                // Under the traffic lights: the whole rail when folded, the
                // space before the section title when the list shows.
                let span = collapsed ? width - 2 : chrome.leadingInset - 20
                let expected = TendDesignTokens.Mac.surfaceSidebar
                let hex = scheme == .dark ? expected.dark : expected.light
                for x in stride(from: CGFloat(1), through: span, by: 4) {
                    for y in [CGFloat(2), chrome.rowHeight / 2, chrome.rowHeight - 3] {
                        let pixel = try #require(color(rep, x: x, y: y, width: width))
                        #expect(Self.matches(pixel, hex),
                                "material \(material), \(collapsed ? "folded" : "list shown"), \(scheme): (\(Int(x)), \(Int(y))) is \(Self.describe(pixel)), not the strip's #\(String(hex, radix: 16))")
                    }
                }
            }
        }
    }

    private func render(_ model: AppModel, scheme: ColorScheme, size: CGSize) async throws -> NSBitmapImageRep {
        let view = TendSidebar(
            model: model, inputs: model.chrome.sidebar, chrome: chrome,
            openPinnedSession: { _ in }, openPinnedPane: { _ in }, openFile: { _, _ in }, closeShell: { _ in },
            focusRequest: UUID(), dismissedShellIDs: .constant([])
        )
        .environmentObject(model)
        .environment(\.colorScheme, scheme)
        .frame(width: size.width, height: size.height)
        let hosting = NSHostingView(rootView: view)
        hosting.frame = CGRect(origin: .zero, size: size)
        let window = NSWindow(contentRect: hosting.frame, styleMask: [.borderless], backing: .buffered, defer: false)
        window.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        window.setFrameOrigin(NSPoint(x: -20_000, y: -20_000))
        window.orderBack(nil)
        defer { window.orderOut(nil); window.close() }
        for _ in 0..<10 {
            hosting.layoutSubtreeIfNeeded()
            try await Task.sleep(for: .milliseconds(50))
        }
        let rep = try #require(hosting.bitmapImageRepForCachingDisplay(in: hosting.bounds))
        hosting.cacheDisplay(in: hosting.bounds, to: rep)
        return rep
    }

    /// The pixel at a point (top-left origin), in sRGB.
    private func color(_ rep: NSBitmapImageRep, x: CGFloat, y: CGFloat, width: CGFloat) -> NSColor? {
        let scale = CGFloat(rep.pixelsWide) / width
        return rep.colorAt(x: Int(x * scale), y: Int(y * scale))?.usingColorSpace(.sRGB)
    }

    private static func matches(_ color: NSColor, _ hex: UInt32) -> Bool {
        let want = [(hex >> 16) & 0xFF, (hex >> 8) & 0xFF, hex & 0xFF].map { CGFloat($0) / 255 }
        let got = [color.redComponent, color.greenComponent, color.blueComponent]
        return color.alphaComponent > 0.99 && zip(want, got).allSatisfy { abs($0 - $1) <= 3 / 255 }
    }

    private static func describe(_ color: NSColor) -> String {
        let bytes = [color.redComponent, color.greenComponent, color.blueComponent].map { Int(($0 * 255).rounded()) }
        return String(format: "#%02X%02X%02X α%.2f", bytes[0], bytes[1], bytes[2], color.alphaComponent)
    }
}
