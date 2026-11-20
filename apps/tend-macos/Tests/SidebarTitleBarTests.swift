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
                let (rep, top) = try await render(model, scheme: scheme, size: CGSize(width: width, height: 580))
                // The material itself: mounted only when asked for, and
                // only below the title bar row. The system's Reduce
                // Transparency or Increase Contrast keeps it off.
                let workspace = NSWorkspace.shared
                if material, !workspace.accessibilityDisplayShouldReduceTransparency, !workspace.accessibilityDisplayShouldIncreaseContrast {
                    let materialTop = try #require(top, "material on, \(collapsed ? "folded" : "list shown"), \(scheme): no sidebar material mounted")
                    #expect(materialTop >= chrome.rowHeight - 0.5, "the sidebar material starts at \(materialTop), inside the \(chrome.rowHeight)pt title bar row")
                } else if material {
                    print("SidebarTitleBarTests: the system's accessibility settings keep the material off; checked the opaque fallback only")
                } else {
                    #expect(top == nil, "material off, but a sidebar material is mounted")
                }
                // Under the traffic lights: the whole rail when folded, the
                // space before the section title when the list shows.
                let span = collapsed ? width - 2 : chrome.leadingInset - 20
                let expected = TendDesignTokens.Mac.surfaceSidebar
                let hex = scheme == .dark ? expected.dark : expected.light
                for x in stride(from: CGFloat(1), through: span, by: 4) {
                    for y in [CGFloat(2), chrome.rowHeight / 2, chrome.rowHeight - 3] {
                        let pixel = try #require(Self.pixel(rep, x: x, y: y, width: width))
                        #expect(Self.matches(pixel, hex),
                                "material \(material), \(collapsed ? "folded" : "list shown"), \(scheme): (\(Int(x)), \(Int(y))) is \(Self.describe(pixel)), not the strip's #\(String(hex, radix: 16))")
                    }
                }
            }
        }
    }

    /// The drawn side bar, and where its sidebar material starts (from the
    /// top), if one is mounted.
    private func render(_ model: AppModel, scheme: ColorScheme, size: CGSize) async throws -> (NSBitmapImageRep, CGFloat?) {
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
        let drawn = try #require(hosting.bitmapImageRepForCachingDisplay(in: hosting.bounds))
        hosting.cacheDisplay(in: hosting.bounds, to: drawn)
        // Bytes in sRGB: `colorAt` hands back a Generic RGB colour whose
        // conversion shifts the values, so read the samples themselves.
        let rep = try #require(drawn.converting(to: .sRGB, renderingIntent: .default))
        return (rep, Self.materialTop(in: hosting))
    }

    private static func materialTop(in hosting: NSView) -> CGFloat? {
        func effects(_ view: NSView) -> [NSVisualEffectView] {
            ((view as? NSVisualEffectView).map { [$0] } ?? []) + view.subviews.flatMap(effects)
        }
        return effects(hosting).filter { $0.material == .sidebar }.map { effect in
            let frame = effect.convert(effect.bounds, to: hosting)
            return hosting.isFlipped ? frame.minY : hosting.bounds.height - frame.maxY
        }.min()
    }

    /// The 8-bit RGBA bytes at a point (top-left origin).
    private static func pixel(_ rep: NSBitmapImageRep, x: CGFloat, y: CGFloat, width: CGFloat) -> [Int]? {
        guard rep.bitsPerSample == 8, rep.samplesPerPixel == 4, !rep.isPlanar else { return nil }
        let scale = CGFloat(rep.pixelsWide) / width
        var samples = [Int](repeating: 0, count: 4)
        rep.getPixel(&samples, atX: Int(x * scale), y: Int(y * scale))
        // Opaque pixels, so premultiplication does not change them.
        return rep.bitmapFormat.contains(.alphaFirst) ? Array(samples[1...3]) + [samples[0]] : samples
    }

    /// Exact to one count: the inactive material is within three of the
    /// light strip.
    private static func matches(_ pixel: [Int], _ hex: UInt32) -> Bool {
        let want = [(hex >> 16) & 0xFF, (hex >> 8) & 0xFF, hex & 0xFF].map(Int.init)
        return pixel[3] == 255 && zip(want, pixel.prefix(3)).allSatisfy { abs($0 - $1) <= 1 }
    }

    private static func describe(_ pixel: [Int]) -> String {
        String(format: "#%02X%02X%02X α%d", pixel[0], pixel[1], pixel[2], pixel[3])
    }
}
