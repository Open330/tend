import AppKit
import Combine
import SwiftUI

/// Everything Home's board draws from the app model, published only when
/// what Home shows changes. Home observes this instead of the whole
/// `AppModel`, whose other publishes (the Inbox's progress, Ask, a telemetry
/// field of a remote agent, the workspace revision) used to re-evaluate the
/// entire board.
@MainActor
final class FleetHomeBoard: ObservableObject, TendWindowVisibilityTracking {
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
    /// clock stops, Home pauses its backlog polls and the decisions are
    /// read at the rail's slower pace; showing a window again applies what
    /// arrived and reloads once. Menu-bar,
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

    /// The daemon's decision queue Home shows, kept for the app's life so
    /// the rail's Home badge counts the same decisions as Your turn when no
    /// Home is open. Only `decisionPoller` reads it on a schedule: quickly
    /// while a Home is on screen, slowly while only the rail is, not at all
    /// while neither is.
    let decisionFeed: FleetDecisionFeed
    private(set) var decisionPoller: FleetDecisionPoller?
    /// Whether a workbench window, and so the rail's badge, is on screen.
    private var railVisible = false

    /// Whether a Home is mounted in any window (shown or not).
    var hasHome: Bool { !readers.isEmpty }

    /// How often the decisions are read now.
    var decisionPace: FleetDecisionSchedule.Pace {
        if hasHome && isVisible { return .home }
        return railVisible ? .background : .paused
    }

    var localHostAlias: String {
        hosts.first(where: \.local)?.alias ?? "local"
    }

    private var subscriptions: [AnyCancellable] = []
    private var perfSubscription: AnyCancellable?

    init() {
        decisionFeed = FleetDecisionFeed()
        perfSubscription = TendPerf.observe(self, "board.publish")
    }

    /// Follows the model's inputs Home reads. `@Published` sends the new
    /// value before it is stored, so each input is mapped from the value
    /// itself.
    func follow(_ model: AppModel) {
        let feed = decisionFeed
        decisionPoller = FleetDecisionPoller { [weak self, weak model] read in
            guard let model else { return }
            switch read {
            case .local: await feed.load(model.client, works: self?.latestWorks ?? [])
            case .remote: await feed.loadRemote(model.client, hosts: model.fleetHosts)
            }
        }
        railVisible = model.chrome.isVisible
        subscriptions = [
            model.chrome.visibilityChanged
                .sink { [weak self] in self?.setRailVisible($0) },
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
        decisionPoller?.setPace(decisionPace)
    }

    private func setRailVisible(_ visible: Bool) {
        railVisible = visible
        decisionPoller?.setPace(decisionPace)
    }

    /// Asks for a read of the decisions soon (an event said they changed),
    /// coalesced with any other ask and after a read in flight.
    func requestDecisions(_ reads: [FleetDecisionSchedule.Read] = [.local, .remote]) {
        reads.forEach { decisionPoller?.request($0) }
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
        decisionPoller?.setPace(decisionPace)
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
    let board: any TendWindowVisibilityTracking

    func makeNSView(context: Context) -> ReaderView {
        let view = ReaderView()
        view.changed = Self.report(to: board, from: view)
        return view
    }

    func updateNSView(_ view: ReaderView, context: Context) {
        view.changed = Self.report(to: board, from: view)
    }

    /// Each reader is one entry in the board's set of Home windows.
    private static func report(to board: any TendWindowVisibilityTracking, from view: ReaderView) -> (Bool?) -> Void {
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
            // Out of the view update that attached it, unless it was
            // detached or moved again by then.
            attachment &+= 1
            let current = attachment
            DispatchQueue.main.async { [weak self] in
                guard let self, self.attachment == current, self.isAttached else { return }
                self.report()
            }
        }

        /// Bumped on each attachment and detach, so a queued first report
        /// from an earlier one is dropped.
        private var attachment: UInt64 = 0
        private var isAttached: Bool { !observers.isEmpty }

        func detach() {
            stopObserving()
            attachment &+= 1
            reported = nil
            changed?(nil)
        }

        func stopObserving() {
            for observer in observers { NotificationCenter.default.removeObserver(observer) }
            observers.removeAll()
        }

        private func report() {
            guard isAttached, let window else { return }
            let visible = window.isVisible && !window.isMiniaturized && !NSApp.isHidden
                && window.occlusionState.contains(.visible)
            guard visible != reported else { return }
            reported = visible
            changed?(visible)
        }
    }
}

// MARK: - Reading the decisions

/// When the shared decision feed is read, for every Home and the rail's
/// badge at once: one schedule, so several Home windows never poll side by
/// side and an event that arrives during a read is followed by one more.
/// Pure, so the cadence can be tested without a clock.
struct FleetDecisionSchedule: Equatable, Sendable {
    /// How often the feed is read.
    enum Pace: Equatable, Sendable {
        /// Nothing that shows decisions is on screen.
        case paused
        /// The rail's badge is on screen, no Home is.
        case background
        /// A Home is on screen.
        case home
    }

    enum Read: Hashable, Sendable { case local, remote }

    /// The cadence of each read at a pace; nil when paused.
    static func period(_ read: Read, at pace: Pace) -> TimeInterval? {
        switch (pace, read) {
        case (.paused, _): nil
        case (.home, .local): 5
        case (.home, .remote): 15
        case (.background, .local): 30
        case (.background, .remote): 60
        }
    }

    /// The least time between two reads of one kind, however many events
    /// ask for one: a read asked for sooner waits (and still happens).
    static let minimumGap: TimeInterval = 2

    var pace = Pace.paused
    private(set) var lastStart: [Read: Date] = [:]
    private(set) var inFlight: Set<Read> = []
    /// Asked for since the last read began: the next one comes after
    /// `minimumGap`, not a whole period.
    private(set) var requested: Set<Read> = []

    /// When `read` is due next; nil while one is in flight or paused.
    func due(_ read: Read) -> Date? {
        guard !inFlight.contains(read), let period = Self.period(read, at: pace) else { return nil }
        let last = lastStart[read] ?? .distantPast
        return last.addingTimeInterval(requested.contains(read) ? Self.minimumGap : period)
    }

    /// The reads due at `now`, marked in flight, and when to look again.
    mutating func start(at now: Date) -> (reads: [Read], next: Date?) {
        var reads: [Read] = []
        for read in [Read.local, .remote] {
            guard let due = due(read), due <= now else { continue }
            reads.append(read)
            inFlight.insert(read)
            requested.remove(read)
            lastStart[read] = now
        }
        let next = [Read.local, .remote].compactMap(due).min()
        return (reads, next)
    }

    mutating func finish(_ read: Read) { inFlight.remove(read) }

    /// An event says the decisions changed: read again soon, after the
    /// read in flight if there is one.
    mutating func request(_ read: Read) { requested.insert(read) }

    /// Another reader just read the same queue, so the next read can wait a
    /// period from now.
    mutating func noteRead(_ read: Read, at now: Date) {
        lastStart[read] = now
        requested.remove(read)
    }
}

/// Reads `FleetDecisionFeed` on `FleetDecisionSchedule`: the feed's one
/// refresh owner. Reads run in tasks of their own, so rescheduling never
/// cancels one halfway.
@MainActor
final class FleetDecisionPoller {
    private(set) var schedule = FleetDecisionSchedule()
    private let read: (FleetDecisionSchedule.Read) async -> Void
    private let now: () -> Date
    private var timer: Task<Void, Never>?

    init(now: @escaping () -> Date = Date.init, read: @escaping (FleetDecisionSchedule.Read) async -> Void) {
        self.now = now
        self.read = read
    }

    func setPace(_ pace: FleetDecisionSchedule.Pace) {
        guard schedule.pace != pace else { return }
        schedule.pace = pace
        run()
    }

    func request(_ read: FleetDecisionSchedule.Read) {
        schedule.request(read)
        run()
    }

    func noteRead(_ read: FleetDecisionSchedule.Read) {
        schedule.noteRead(read, at: now())
        run()
    }

    /// Starts what is due and sleeps until the next one is.
    private func run() {
        timer?.cancel()
        timer = nil
        let (reads, next) = schedule.start(at: now())
        for kind in reads {
            Task { [weak self, read] in
                await read(kind)
                self?.schedule.finish(kind)
                self?.run()
            }
        }
        guard let next else { return }
        let delay = max(0.05, next.timeIntervalSince(now()))
        timer = Task { [weak self] in
            try? await Task.sleep(for: .seconds(delay))
            guard !Task.isCancelled else { return }
            self?.run()
        }
    }
}
