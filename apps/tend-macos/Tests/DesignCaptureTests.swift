import AppKit
import Foundation
import SwiftUI
import Testing
@testable import Tend

// Screens of the whole app, drawn off-screen against a fixture tendd, for
// design reviews: Home (both card layouts, the Review and Start Work
// sheets), Explore, Inbox, every Settings tab, the menu bar popover and the
// Welcome guide, light and dark. Runs only where the capture workflow
// (.github/workflows/mac-shots.yml, on a CI runner) sets TEND_CAPTURE_SOCKET
// to the demo fixture's sandboxed tendd and TEND_RENDER_DIR to where the
// PNGs go; the language is the test run's (`xcodebuild -testLanguage`).
//
// It only ever talks to that fixture: the socket must be the sandbox's own
// and already answering, and the model never starts or replaces a daemon.
// Nothing is shown on screen (a test host presents no window), and the
// preferences it changes are put back.

private let environment = ProcessInfo.processInfo.environment

@MainActor
@Suite(.serialized, .enabled(if: environment["TEND_CAPTURE_SOCKET"] != nil && environment["TEND_RENDER_DIR"] != nil))
struct DesignCaptureTests {
    private let directory = URL(fileURLWithPath: environment["TEND_RENDER_DIR"] ?? NSTemporaryDirectory())
    private let language = Locale.preferredLanguages.first.map { String($0.prefix(2)) } ?? "en"
    private let schemes: [ColorScheme] = [.light, .dark]

    /// The demo fixture's sandbox socket (scripts/tend-sandbox.sh): nothing else.
    static let fixtureSocket = "/tmp/tend-demo-sandbox/tendd.sock"

    @Test func everyScreen() async throws {
        let socket = try #require(environment["TEND_CAPTURE_SOCKET"])
        guard socket == Self.fixtureSocket,
              let type = try? FileManager.default.attributesOfItem(atPath: socket)[.type] as? FileAttributeType,
              type == .typeSocket else {
            Issue.record("TEND_CAPTURE_SOCKET must be the running fixture's \(Self.fixtureSocket), not \(socket)")
            return
        }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let saved = savePreferences()
        defer { restorePreferences(saved) }
        let model = AppModel(client: TendIPCClient(socketPath: socket), launchesDaemon: false)
        model.retryConnection()
        try await until("connected with agents", timeout: 60) {
            model.connectionState == .connected && !model.fleetHosts.isEmpty
        }
        // Let the board, the decision queue and the pane previews arrive.
        try await Task.sleep(for: .seconds(6))

        for scheme in schemes {
            try await workbench(model, scheme: scheme)
            try await settings(model, scheme: scheme)
            // The popover lists the agents the menu bar controller picked.
            let menuBar = TendMenuBarState()
            menuBar.agents = model.executionSnapshot.hostedAgents.prefix(8).map { TendMenuBarAgent(hosted: $0, unread: false) }
            try await capture(TendMenuBarContent(
                model: model, state: menuBar, attention: model.attention,
                dismiss: {}, select: { _ in }, selectScope: { _ in }
            ).environmentObject(model), name: "menubar", scheme: scheme, size: CGSize(width: 380, height: 560), fit: true)
            try await capture(OnboardingView(model: model).environmentObject(model),
                              name: "onboarding", scheme: scheme, size: CGSize(width: 760, height: 600))
            try await capture(WhatsNewView(), name: "whats-new", scheme: scheme, size: CGSize(width: 560, height: 520), fit: true)
        }

        // Before tendd answers: what a window shows while it connects.
        let unconnected = AppModel(client: TendIPCClient(socketPath: "/tmp/tend-capture-\(UUID().uuidString).sock"), launchesDaemon: false)
        for scheme in schemes {
            try await capture(ContentView(model: unconnected).environmentObject(unconnected),
                              name: "home-unconnected", scheme: scheme, size: CGSize(width: 1280, height: 800))
        }
    }

    // MARK: - Screens

    private func workbench(_ model: AppModel, scheme: ColorScheme) async throws {
        let window = OffscreenWindow(ContentView(model: model).environmentObject(model), scheme: scheme, size: CGSize(width: 1440, height: 900))
        defer { window.close() }
        for (layout, name) in [(FleetCardLayout.emphasis, "home"), (.compact, "home-compact")] {
            UserDefaults.standard.set(layout.rawValue, forKey: TendPreferences.homeCardLayoutKey)
            model.select(.workBoard)
            try await window.settle(seconds: 2)
            try write(window.image(), name: name, scheme: scheme)
        }
        UserDefaults.standard.set(FleetCardLayout.emphasis.rawValue, forKey: TendPreferences.homeCardLayoutKey)
        try await window.settle(seconds: 1)

        // The Review sheet, opened the way the person does: its toolbar button.
        if window.press(label: String(localized: "Review")) {
            try await window.settle(seconds: 2)
            if let sheet = window.sheetImage() { try write(sheet, name: "home-review", scheme: scheme) }
            window.closeSheet()
            try await window.settle(seconds: 1)
        }
        model.isPresentingWorkStart = true
        try await window.settle(seconds: 2)
        if let sheet = window.sheetImage() { try write(sheet, name: "home-start-work", scheme: scheme) }
        model.isPresentingWorkStart = false
        window.closeSheet()

        for (selection, name) in [(TendSidebarSelection.watch, "explore"), (.inbox, "inbox"), (.ask, "ask")] {
            model.select(selection)
            try await window.settle(seconds: 3)
            try write(window.image(), name: name, scheme: scheme)
        }
        model.select(.workBoard)
    }

    private func settings(_ model: AppModel, scheme: ColorScheme) async throws {
        for tab in TendSettingsTab.allCases {
            TendSettingsOpener.select(tab)
            try await capture(TendSettingsView(model: model).environmentObject(model),
                              name: "settings-\(tab.rawValue)", scheme: scheme, size: CGSize(width: 820, height: 680))
        }
        TendSettingsOpener.select(.general)
    }

    // MARK: - Drawing

    private func capture<V: View>(_ view: V, name: String, scheme: ColorScheme, size: CGSize, fit: Bool = false) async throws {
        let window = OffscreenWindow(view, scheme: scheme, size: size, fit: fit)
        defer { window.close() }
        try await window.settle(seconds: 1.5)
        try write(window.image(), name: name, scheme: scheme)
    }

    private func write(_ rep: NSBitmapImageRep?, name: String, scheme: ColorScheme) throws {
        let rep = try #require(rep, "\(name) did not render")
        let png = try #require(rep.representation(using: .png, properties: [:]))
        let file = directory.appendingPathComponent("\(name)-\(scheme == .dark ? "dark" : "light")-\(language).png")
        try png.write(to: file)
    }

    private struct TimedOut: Error, CustomStringConvertible {
        let what: String
        var description: String { "timed out waiting: \(what)" }
    }

    /// Fails the capture (nothing is drawn from a fixture that never came up).
    private func until(_ what: String, timeout: TimeInterval, _ condition: () -> Bool) async throws {
        let deadline = Date().addingTimeInterval(timeout)
        while !condition() {
            guard Date() < deadline else { throw TimedOut(what: what) }
            try await Task.sleep(for: .milliseconds(200))
        }
    }

    private static let preferenceKeys = [TendPreferences.homeCardLayoutKey, TendPreferences.settingsTabKey]

    private func savePreferences() -> [String: Any?] {
        Dictionary(uniqueKeysWithValues: Self.preferenceKeys.map { ($0, UserDefaults.standard.object(forKey: $0)) })
    }

    private func restorePreferences(_ saved: [String: Any?]) {
        for (key, value) in saved {
            if let value { UserDefaults.standard.set(value, forKey: key) } else { UserDefaults.standard.removeObject(forKey: key) }
        }
    }
}

/// A borderless window far off every display and ordered behind the others,
/// so SwiftUI lays out scroll views, lazy stacks and sheets as it does on
/// screen without anything coming to the front.
@MainActor
private final class OffscreenWindow {
    let window: NSWindow
    let host: NSView

    init<V: View>(_ view: V, scheme: ColorScheme, size: CGSize, fit: Bool = false) {
        let root = view
            .environment(\.colorScheme, scheme)
            .frame(width: fit ? nil : size.width, height: fit ? nil : size.height)
            .frame(maxWidth: fit ? size.width : nil)
            .background(TendSurfacePalette.workspace(for: scheme))
        let hosting = NSHostingView(rootView: root)
        if fit {
            hosting.frame = CGRect(origin: .zero, size: hosting.fittingSize.width > 0 ? hosting.fittingSize : size)
        } else {
            hosting.frame = CGRect(origin: .zero, size: size)
        }
        host = hosting
        window = NSWindow(contentRect: hosting.frame, styleMask: [.borderless], backing: .buffered, defer: false)
        window.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        window.setFrameOrigin(NSPoint(x: -20_000, y: -20_000))
        window.orderBack(nil)
    }

    func settle(seconds: TimeInterval) async throws {
        let deadline = Date().addingTimeInterval(seconds)
        repeat {
            host.layoutSubtreeIfNeeded()
            try await Task.sleep(for: .milliseconds(100))
        } while Date() < deadline
    }

    func image() -> NSBitmapImageRep? {
        Self.draw(host)
    }

    func sheetImage() -> NSBitmapImageRep? {
        guard let content = window.attachedSheet?.contentView else { return nil }
        content.layoutSubtreeIfNeeded()
        return Self.draw(content)
    }

    func closeSheet() {
        if let sheet = window.attachedSheet { window.endSheet(sheet) }
    }

    /// Presses the first button whose accessibility label or title starts
    /// with `label`, through the accessibility tree as VoiceOver would.
    func press(label: String) -> Bool {
        func get(_ object: NSObject, _ key: String) -> Any? {
            object.responds(to: Selector(key)) ? object.value(forKey: key) : nil
        }
        func find(_ element: Any) -> NSObject? {
            guard let element = element as? NSObject else { return nil }
            let name = (get(element, "accessibilityLabel") as? String) ?? (get(element, "accessibilityTitle") as? String) ?? ""
            if (get(element, "accessibilityRole") as? String) == NSAccessibility.Role.button.rawValue, name.hasPrefix(label) {
                return element
            }
            for child in (get(element, "accessibilityChildren") as? [Any]) ?? [] {
                if let hit = find(child) { return hit }
            }
            return nil
        }
        guard let button = find(host), button.responds(to: Selector("accessibilityPerformPress")) else { return false }
        button.perform(Selector("accessibilityPerformPress"))
        return true
    }

    func close() {
        closeSheet()
        window.orderOut(nil)
        window.close()
    }

    private static func draw(_ view: NSView) -> NSBitmapImageRep? {
        guard let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { return nil }
        view.cacheDisplay(in: view.bounds, to: rep)
        return rep
    }
}
