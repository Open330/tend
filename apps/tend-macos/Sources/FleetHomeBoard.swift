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
    /// Whether Home's window is on screen (not hidden, minimized, fully
    /// covered, or on another Space), as `TendWindowVisibilityReader` in
    /// Home reports it. While it is not, the board holds
    /// new units back, the clock stops, and Home pauses its decision and
    /// backlog polls; showing the window again applies what arrived and
    /// reloads once. Menu-bar, notification and attention inputs come from
    /// the model and keep running.
    @Published private(set) var isVisible = true
    /// Units that arrived while the window was off screen.
    private var heldWorks: [FleetWork]?
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

    func setVisible(_ visible: Bool) {
        guard visible != isVisible else { return }
        TendPerf.count(visible ? "board.shown" : "board.hidden")
        if visible, let heldWorks {
            self.heldWorks = nil
            works = heldWorks
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
        guard isVisible else {
            TendPerf.count("board.held")
            heldWorks = works
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
/// hidden. Reports on attaching to a window and on each change.
struct TendWindowVisibilityReader: NSViewRepresentable {
    let changed: (Bool) -> Void

    func makeNSView(context: Context) -> ReaderView {
        let view = ReaderView()
        view.changed = changed
        return view
    }

    func updateNSView(_ view: ReaderView, context: Context) {
        view.changed = changed
    }

    static func dismantleNSView(_ view: ReaderView, coordinator: ()) {
        view.stopObserving()
    }

    final class ReaderView: NSView {
        var changed: ((Bool) -> Void)?
        private var observers: [NSObjectProtocol] = []
        private var reported: Bool?

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            stopObserving()
            guard let window else { return }
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
