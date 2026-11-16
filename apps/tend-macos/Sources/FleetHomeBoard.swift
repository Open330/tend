import AppKit
import Combine
import SwiftUI

/// Everything Home's board draws from the app model, published only when
/// what Home shows changes. Home observes this instead of the whole
/// `AppModel`, whose other publishes (the Inbox's progress, Ask, a telemetry
/// field of a remote agent, the workspace revision) used to re-evaluate the
/// entire board.
@MainActor
final class FleetHomeBoard: ObservableObject {
    /// Home's units. Replaced when a unit changes in a way Home can show
    /// (`TendProjectionEquality.sameOnHome`), not for each agent heartbeat.
    @Published private(set) var works: [FleetWork] = []
    /// Every host, as the header's Hosts button and its popover name them.
    @Published private(set) var hosts: [TendFleetHostIdentity] = []
    @Published private(set) var isConnected = false
    @Published private(set) var isStartingWork = false
    /// Bumped when Home's decisions may have changed outside the decision
    /// feed: a new operator message, or an asking agent's id or checkout.
    @Published private(set) var decisionsRevision: UInt64 = 0
    /// Ticks every `clockPeriod` so what Home works out from the clock (the
    /// cards' "5 min ago", Finished units folding away after a day) keeps
    /// moving on a quiet board. Home used to get this for free from the
    /// model's constant publishes.
    @Published private(set) var clock: UInt64 = 0
    static let clockPeriod: TimeInterval = 15
    /// Whether any window showing Home is on screen (not hidden, minimized,
    /// fully covered, or on another Space), as each Home's
    /// `TendWindowVisibilityReader` reports it; true while no Home is
    /// mounted. While it is false, the board holds new units back, the
    /// clock stops, and Home pauses its decision and backlog polls; showing
    /// a window again applies what arrived and reloads once. Menu-bar,
    /// notification and attention inputs come from the model and keep
    /// running.
    @Published private(set) var isVisible = true
    /// The latest units from the projection, published or held. The
    /// projection compares against this, never against `works`, so a hidden
    /// change that returns to what is on screen still replaces what is held.
    private(set) var latestWorks: [FleetWork] = []
    /// `latestWorks` is newer than `works`.
    private var worksHeld = false
    /// Each mounted Home's window, and whether it is on screen.
    private var readers: [ObjectIdentifier: Bool] = [:]
    /// A decisions change that arrived while the window was off screen.
    private var heldDecisions = false

    var localHostAlias: String {
        hosts.first(where: \.local)?.alias ?? "local"
    }

    private var subscriptions: [AnyCancellable] = []
    private var perfSubscription: AnyCancellable?

    init() {
        perfSubscription = TendPerf.observe(self, "board.publish")
    }

    /// Follows the model's inputs Home reads. `@Published` sends the new
    /// value before it is stored, so each input is mapped from the value
    /// itself.
    func follow(_ model: AppModel) {
        subscriptions = [
            model.$connectionState.map { $0 == .connected }.removeDuplicates()
                .sink { [weak self] in self?.setConnected($0) },
            model.$isStartingWork.removeDuplicates()
                .sink { [weak self] in self?.setStartingWork($0) },
            model.$executionSnapshot.map { $0.hosts.map(\.identity) }.removeDuplicates()
                .sink { [weak self] in self?.setHosts($0) },
            model.$operatorMessages.dropFirst()
                .sink { [weak self] _ in self?.decisionsChanged() },
            Timer.publish(every: Self.clockPeriod, tolerance: 2, on: .main, in: .common).autoconnect()
                .sink { [weak self] _ in self?.tick() },
        ]
    }

    /// One Home's window went on or off screen.
    func setVisible(_ visible: Bool, reader: ObjectIdentifier) {
        readers[reader] = visible
        updateVisibility()
    }

    /// That Home is gone.
    func removeReader(_ reader: ObjectIdentifier) {
        guard readers.removeValue(forKey: reader) != nil else { return }
        updateVisibility()
    }

    private func updateVisibility() {
        setVisible(readers.isEmpty || readers.values.contains(true))
    }

    private func setVisible(_ visible: Bool) {
        guard visible != isVisible else { return }
        TendPerf.count(visible ? "board.shown" : "board.hidden")
        if visible, worksHeld {
            worksHeld = false
            works = latestWorks
        }
        if visible, heldDecisions {
            heldDecisions = false
            decisionsRevision &+= 1
        }
        isVisible = visible
        // Whatever the clock moved while hidden shows at once.
        if visible { clock &+= 1 }
    }

    func tick() {
        guard isVisible else { return }
        clock &+= 1
    }

    func setWorks(_ works: [FleetWork]) {
        latestWorks = works
        guard isVisible else {
            TendPerf.count("board.held")
            worksHeld = true
            return
        }
        TendPerf.count("board.works")
        self.works = works
    }

    func setHosts(_ hosts: [TendFleetHostIdentity]) {
        if self.hosts != hosts { self.hosts = hosts }
    }

    func setConnected(_ connected: Bool) {
        if isConnected != connected { isConnected = connected }
    }

    func setStartingWork(_ starting: Bool) {
        if isStartingWork != starting { isStartingWork = starting }
    }

    func decisionsChanged() {
        guard isVisible else { heldDecisions = true; return }
        decisionsRevision &+= 1
    }
}

/// Reports whether the window this view is in is on screen: shown, not
/// minimized, not fully covered or on another Space, and the app not
/// hidden. Reports on attaching to a window and on each change, and nil
/// when it leaves its window or is removed.
struct TendWindowVisibilityReader: NSViewRepresentable {
    let board: FleetHomeBoard

    func makeNSView(context: Context) -> ReaderView {
        let view = ReaderView()
        view.changed = Self.report(to: board, from: view)
        return view
    }

    func updateNSView(_ view: ReaderView, context: Context) {
        view.changed = Self.report(to: board, from: view)
    }

    /// Each reader is one entry in the board's set of Home windows.
    private static func report(to board: FleetHomeBoard, from view: ReaderView) -> (Bool?) -> Void {
        let reader = ObjectIdentifier(view)
        return { [weak board] visible in
            guard let board else { return }
            if let visible { board.setVisible(visible, reader: reader) } else { board.removeReader(reader) }
        }
    }

    static func dismantleNSView(_ view: ReaderView, coordinator: ()) {
        view.detach()
    }

    final class ReaderView: NSView {
        var changed: ((Bool?) -> Void)?
        private var observers: [NSObjectProtocol] = []
        private var reported: Bool?

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            stopObserving()
            // Each attachment states its window afresh.
            reported = nil
            guard let window else {
                changed?(nil)
                return
            }
            let center = NotificationCenter.default
            let windowEvents: [Notification.Name] = [
                NSWindow.didChangeOcclusionStateNotification, NSWindow.didMiniaturizeNotification,
                NSWindow.didDeminiaturizeNotification,
            ]
            let appEvents: [Notification.Name] = [NSApplication.didHideNotification, NSApplication.didUnhideNotification]
            for name in windowEvents {
                observers.append(center.addObserver(forName: name, object: window, queue: .main) { [weak self] _ in
                    MainActor.assumeIsolated { self?.report() }
                })
            }
            for name in appEvents {
                observers.append(center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                    MainActor.assumeIsolated { self?.report() }
                })
            }
            // Out of the view update that attached it.
            DispatchQueue.main.async { [weak self] in self?.report() }
        }

        func detach() {
            stopObserving()
            reported = nil
            changed?(nil)
        }

        func stopObserving() {
            for observer in observers { NotificationCenter.default.removeObserver(observer) }
            observers.removeAll()
        }

        private func report() {
            guard let window else { return }
            let visible = window.isVisible && !window.isMiniaturized && !NSApp.isHidden
                && window.occlusionState.contains(.visible)
            guard visible != reported else { return }
            reported = visible
            changed?(visible)
        }
    }
}
