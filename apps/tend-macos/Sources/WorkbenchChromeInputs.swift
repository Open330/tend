import Combine
import SwiftUI

// What the workbench window's own chrome (the root view, the side bar, the
// tab strips and the status bar) draws from the app model, as small values
// published only when they change. The chrome used to observe the whole
// `AppModel`, so every fleet heartbeat (a snapshot, its projections, the
// workspace revision) redrew all of it, with the window hidden too.

/// Something told by each `TendWindowVisibilityReader` whether its window
/// is on screen.
@MainActor
protocol TendWindowVisibilityTracking: AnyObject {
    func setVisible(_ visible: Bool, reader: ObjectIdentifier)
    func removeReader(_ reader: ObjectIdentifier)
}

/// One part of the chrome's inputs. Views observe this instead of the model.
@MainActor
final class TendChromeSlice<Value: Equatable>: ObservableObject {
    @Published private(set) var value: Value
    private var perfSubscription: AnyCancellable?

    init(_ value: Value, perfName: String) {
        self.value = value
        perfSubscription = TendPerf.observe(self, perfName)
    }

    func set(_ value: Value) {
        if value != self.value { self.value = value }
    }
}

/// What `ContentView`'s own body reads: the side bar's fold, the window's
/// sheets and alert, the window actions' enabled states, and the selection
/// it follows. None of it moves with a heartbeat.
struct TendContentInputs: Equatable {
    var isSidebarListCollapsed = false
    var sidebarSelection: TendSidebarSelection?
    var isConfirmingDaemonReplacement = false
    var isPresentingFleetDispatch = false
    var isPresentingWorkStart = false
    var isPresentingHostRegistration = false
    var isPresentingTemplates = false
    var sessionSnapshotSheet: TendSessionSnapshotSheet?
    var pipelineEditorTarget: TendPipelineEditorTarget?
    var isConnected = false
    var isStartingWork = false
    var isCreatingSession = false
    var isStartingAgent = false
    var isTerminatingSession = false
    var selectedSessionID: String?
    var hasUnread = false

    init() {}

    @MainActor
    init(_ model: AppModel) {
        isSidebarListCollapsed = model.isSidebarListCollapsed
        sidebarSelection = model.sidebarSelection
        isConfirmingDaemonReplacement = model.isConfirmingDaemonReplacement
        isPresentingFleetDispatch = model.isPresentingFleetDispatch
        isPresentingWorkStart = model.isPresentingWorkStart
        isPresentingHostRegistration = model.isPresentingHostRegistration
        isPresentingTemplates = model.isPresentingTemplates
        sessionSnapshotSheet = model.sessionSnapshotSheet
        pipelineEditorTarget = model.pipelineEditorTarget
        isConnected = model.isConnected
        isStartingWork = model.isStartingWork
        isCreatingSession = model.isCreatingSession
        isStartingAgent = model.isStartingAgent
        isTerminatingSession = model.isTerminatingSession
        selectedSessionID = model.selectedSessionID
        hasUnread = model.attention.hasUnread
    }
}

/// What the side bar shows: the activity rail's badges always, and the
/// list's data only for the list showing. With the list folded away (Home
/// in front) only the rail is drawn, so nothing else counts.
struct TendSidebarInputs: Equatable {
    enum Rows: Equatable {
        case folded
        case files
        case work([TendWorkGroup])
        case watch(hosts: [TendWatchHost], pipelineRuns: [TendPipelineRun])
        case inbox(messages: Int, attention: [TendHostedAgent])
        case ask(conversations: [TendAskConversation], entries: [TendAskEntry], agent: String, active: String?)
        case shells(sessions: [TendSession], remoteHosts: [String])
    }

    var isListCollapsed = false
    var mode: TendSidebarMode = .watch
    var homeNeedsYou = 0
    var workAttention = 0
    var askRunning = 0
    var attention = TendAttentionSummary()
    var selection: TendSidebarSelection?
    var watchSelection: TendWatchPaneIdentity?
    var isConnected = false
    var isCreatingSession = false
    var rows = Rows.folded

    init() {}

    @MainActor
    init(_ model: AppModel) {
        isListCollapsed = model.isSidebarListCollapsed
        mode = model.sidebarMode
        homeNeedsYou = model.homeWorks.lazy.filter { $0.lane == .needsYou }.count
        workAttention = model.workGroups.lazy.filter { $0.attentionCount > 0 }.count
        askRunning = model.askEntries.lazy.filter { $0.status == "running" }.count
        attention = model.attention.summary
        guard !isListCollapsed else { return }
        selection = model.sidebarSelection
        watchSelection = model.watchSelection
        isConnected = model.isConnected
        isCreatingSession = model.isCreatingSession
        switch mode {
        case .files: rows = .files
        case .work: rows = .work(model.workGroups)
        case .watch:
            rows = .watch(hosts: model.executionSnapshot.watchHosts, pipelineRuns: model.pipelineRuns)
        case .inbox:
            rows = .inbox(
                messages: model.operatorMessages.count,
                attention: model.hostedAgents.filter { $0.agent.status.needsAttention }
            )
        case .ask:
            rows = .ask(
                conversations: model.askConversations, entries: model.askEntries,
                agent: model.askAgent, active: model.activeAskConversationID
            )
        case .shells:
            rows = .shells(sessions: model.sessions, remoteHosts: model.remoteShellHosts.map(\.alias))
        }
    }
}

/// The status bar's figures. Usage is kept as the text it shows, so a
/// heartbeat that only moves an agent's activity time changes nothing.
struct TendStatusBarInputs: Equatable {
    struct UsageItem: Equatable {
        let id: String
        let text: String
        let level: TendUsageLevel
    }

    var connectionState = AppModel.ConnectionState.connecting
    var agentActionError: String?
    var agentActionNotice: String?
    var hostCount = 0
    var agentCount = 0
    var shellCount = 0
    /// The first `TendUsageStatusItems.inlineLimit` accounts, worst first.
    var usage: [UsageItem] = []
    /// How many more accounts fold into `+N`, and the worst of them.
    var moreUsage = 0
    var moreUsageLevel: TendUsageLevel?

    init() {}

    @MainActor
    init(_ model: AppModel, now: Date) {
        connectionState = model.connectionState
        agentActionError = model.agentActionError
        agentActionNotice = model.agentActionNotice
        hostCount = model.fleetHosts.count
        agentCount = model.hostedAgents.count
        shellCount = model.sessions.lazy.filter { !$0.exited }.count
        let groups = TendUsageGroup.groups(from: model.hostedAgents, now: now)
        let limit = TendUsageStatusItems.inlineLimit
        usage = groups.prefix(limit).map {
            UsageItem(id: $0.id, text: TendUsageFormat.statusText($0, now: now), level: $0.level)
        }
        moreUsage = max(0, groups.count - limit)
        moreUsageLevel = groups.dropFirst(limit).map(\.level).max()
    }
}

/// One editor tab's label: its title, its agent's state dot, and the pane
/// whose unread dot it carries.
struct TendTabLabel: Equatable {
    let title: String
    let status: TendAgentStatus?
    let unreadPane: TendWatchPaneIdentity?

    @MainActor
    init(_ selection: TendSidebarSelection, model: AppModel) {
        title = WorkspaceTabBar.title(for: selection, model: model)
        status = WorkspaceTabBar.agentStatus(for: selection, model: model)
        unreadPane = WorkspaceTabBar.unreadPane(for: selection, model: model)
    }
}

/// The labels of every open tab, by tab.
typealias TendTabStripInputs = [TendSidebarSelection: TendTabLabel]

/// Keeps the chrome's slices up to date with the model. The root view's
/// inputs follow every change; the side bar, tab strips and status bar
/// follow only while a workbench window is on screen (not hidden,
/// minimized, fully covered or on another Space), and catch up at once when
/// one is shown again.
@MainActor
final class TendWindowChrome: TendWindowVisibilityTracking {
    let content = TendChromeSlice(TendContentInputs(), perfName: "chrome.content.publish")
    let sidebar = TendChromeSlice(TendSidebarInputs(), perfName: "chrome.sidebar.publish")
    let statusBar = TendChromeSlice(TendStatusBarInputs(), perfName: "chrome.statusbar.publish")
    let tabStrip = TendChromeSlice(TendTabStripInputs(), perfName: "chrome.tabs.publish")
    /// Sent after each workspace revision, once the model holds it: the
    /// root view prunes its tabs then, without redrawing for it.
    let workspaceRevised = PassthroughSubject<Void, Never>()

    /// Whether any workbench window is on screen.
    private(set) var isVisible = false
    /// The time-dependent part of the status bar (a cap lifting, a window
    /// rolling over, "resets at") is read again this often while shown.
    static let clockPeriod: TimeInterval = 30

    private weak var model: AppModel?
    private var readers: [ObjectIdentifier: Bool] = [:]
    /// Each window's open tabs, by its tab model; the strips are labelled
    /// for all of them.
    private var tabsByWindow: [ObjectIdentifier: [TendSidebarSelection]] = [:]
    private var openTabs: [TendSidebarSelection] = []
    private var refreshQueued = false
    /// A change arrived while no window was on screen.
    private var held = false
    private let now: () -> Date
    private var subscriptions: [AnyCancellable] = []

    init(now: @escaping () -> Date = Date.init) {
        self.now = now
    }

    /// Follows the model. Its publishes come before the new value is
    /// stored, so the slices are read once per run loop turn, after them.
    func follow(_ model: AppModel) {
        self.model = model
        subscriptions = [
            model.objectWillChange.sink { [weak self] _ in self?.queueRefresh() },
            model.attention.objectWillChange.sink { [weak self] _ in self?.queueRefresh() },
            model.$workspaceRevision.dropFirst().receive(on: DispatchQueue.main)
                .sink { [weak self] _ in self?.workspaceRevised.send() },
            Timer.publish(every: Self.clockPeriod, tolerance: 5, on: .main, in: .common).autoconnect()
                .sink { [weak self] _ in self?.tick() },
        ]
        refreshAll()
    }

    /// The tabs open in one window (named by its tab model), whose labels
    /// its tab strips show.
    func setOpenTabs(_ tabs: [TendSidebarSelection], window: ObjectIdentifier) {
        guard tabsByWindow[window] != tabs else { return }
        tabsByWindow[window] = tabs
        openTabsChanged()
    }

    /// That window is gone.
    func removeOpenTabs(window: ObjectIdentifier) {
        guard tabsByWindow.removeValue(forKey: window) != nil else { return }
        openTabsChanged()
    }

    private func openTabsChanged() {
        var seen = Set<TendSidebarSelection>()
        let tabs = tabsByWindow.values.joined().filter { seen.insert($0).inserted }
        guard tabs != openTabs else { return }
        openTabs = tabs
        guard let model else { return }
        if isVisible { tabStrip.set(Self.labels(for: tabs, model: model)) } else { held = true }
    }

    func setVisible(_ visible: Bool, reader: ObjectIdentifier) {
        readers[reader] = visible
        updateVisibility()
    }

    func removeReader(_ reader: ObjectIdentifier) {
        guard readers.removeValue(forKey: reader) != nil else { return }
        updateVisibility()
    }

    private func updateVisibility() {
        let visible = readers.values.contains(true)
        guard visible != isVisible else { return }
        isVisible = visible
        TendPerf.count(visible ? "chrome.shown" : "chrome.hidden")
        // Shown again: catch up at once, time-only changes (a cap lifting,
        // a reset passing) included.
        if visible { refreshAll() }
    }

    private func queueRefresh() {
        guard !refreshQueued else { return }
        refreshQueued = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.refreshQueued = false
            self.refresh()
        }
    }

    func refresh() {
        guard let model else { return }
        TendPerf.count("chrome.refresh")
        content.set(TendContentInputs(model))
        guard isVisible else {
            held = true
            return
        }
        refreshShown(model)
    }

    private func refreshAll() {
        guard let model else { return }
        content.set(TendContentInputs(model))
        refreshShown(model)
    }

    private func refreshShown(_ model: AppModel) {
        held = false
        sidebar.set(TendSidebarInputs(model))
        statusBar.set(TendStatusBarInputs(model, now: now()))
        tabStrip.set(Self.labels(for: openTabs, model: model))
    }

    private func tick() {
        guard isVisible, let model else { return }
        statusBar.set(TendStatusBarInputs(model, now: now()))
    }

    private static func labels(for tabs: [TendSidebarSelection], model: AppModel) -> TendTabStripInputs {
        var labels = TendTabStripInputs(minimumCapacity: tabs.count)
        for tab in tabs where labels[tab] == nil { labels[tab] = TendTabLabel(tab, model: model) }
        return labels
    }
}
