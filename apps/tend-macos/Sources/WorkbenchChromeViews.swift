// The workbench frame: editor tabs and the status bar.
import GhosttyTerminal
import SwiftUI
import UniformTypeIdentifiers

struct WorkbenchWindowPresenter: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView {
        let view = NSView(frame: .zero)
        revealWindow(for: view, remainingAttempts: 30)
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {}

    private func revealWindow(for view: NSView, remainingAttempts: Int) {
        // A test host draws windows off-screen and never brings one forward.
        guard !AppModel.isRunningTests() else { return }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { [weak view] in
            guard let view else { return }
            if let window = view.window {
                window.identifier = NSUserInterfaceItemIdentifier("tend.main-workbench")
                window.isRestorable = false
                window.makeKeyAndOrderFront(nil)
                NSApp.activate(ignoringOtherApps: true)
            } else if remainingAttempts > 0 {
                revealWindow(for: view, remainingAttempts: remainingAttempts - 1)
            }
        }
    }
}

struct WorkspaceTabBar: View {
    /// Read, not observed: `labels` publishes when a tab's title, state dot
    /// or unread pane changes (`TendTabStripInputs`).
    let model: AppModel
    @ObservedObject var labels: TendChromeSlice<TendTabStripInputs>
    @ObservedObject var tabs: TendWorkbenchTabs
    let groupID: UUID
    /// The title bar row's height: the strip is the window's title bar.
    let height: CGFloat
    /// The window's actions, carried by the last group's strip only.
    let accessory: AnyView?
    var leadingInset: CGFloat = 0
    @Environment(\.openWindow) private var openWindow
    @ObservedObject private var dragSession = TendTabDragSession.shared
    /// Each tab's frame in the strip, for placing a dragged tab.
    @State private var tabFrames: [TendSidebarSelection: CGRect] = [:]

    private var group: TendWorkbenchTabs.Group? {
        tabs.group(id: groupID)
    }

    private var stripSpace: String { "tab-strip-\(groupID)" }

    /// Aligned with the group's tabs; nil for one not laid out yet.
    private var orderedTabFrames: [CGRect?] {
        (group?.tabs ?? []).map { tabFrames[$0] }
    }

    /// Where the insertion bar sits while a dragged tab hovers this strip.
    private var insertionX: CGFloat? {
        guard case .strip(groupID, let index)? = dragSession.target else { return nil }
        let frames = orderedTabFrames
        if frames.indices.contains(index), let frame = frames[index] { return frame.minX }
        return frames.compactMap { $0 }.last?.maxX ?? 0
    }

    var body: some View {
        HStack(spacing: 0) {
            if leadingInset > 0 {
                WindowDragArea().frame(width: leadingInset)
            }
            GeometryReader { strip in
                ScrollViewReader { proxy in
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 0) {
                        tabItems
                        // The strip is also the window's title bar: the space
                        // after the last tab moves the window. It lives inside
                        // the scroll view, which would otherwise take the click.
                        WindowDragArea()
                            .frame(minWidth: 24, maxWidth: .infinity, maxHeight: .infinity)
                    }
                    .frame(minWidth: strip.size.width, maxHeight: .infinity, alignment: .leading)
                    .coordinateSpace(name: stripSpace)
                    .onPreferenceChange(TabFramesKey.self) { tabFrames = $0 }
                    .overlay(alignment: .leading) {
                        if let insertionX {
                            TabInsertionIndicator(x: insertionX)
                        }
                    }
                    .onDrop(
                        of: [TendTabDragSession.tabType],
                        delegate: TabStripDropDelegate(
                            groupID: groupID,
                            orderedFrames: orderedTabFrames,
                            tabs: tabs,
                            session: dragSession,
                            didDrop: { selection, _ in model.activateEditor(selection) }
                        )
                    )
                }
                .onChange(of: group?.active) { active in
                    if let active { proxy.scrollTo(active, anchor: .trailing) }
                }
                .onAppear { if let active = group?.active { proxy.scrollTo(active, anchor: .trailing) } }
                }
            }

            if let active = group?.active {
                HStack(spacing: 1) {
                    if group?.preview == active {
                        Button {
                            tabs.pin(active, groupID: groupID)
                        } label: {
                            Image(systemName: "pin")
                        }
                        .help("Pin preview tab")
                    }

                    Button {
                        splitRight(active)
                    } label: {
                        Image(systemName: "rectangle.split.2x1")
                    }
                    .help("Open to the Side")

                    if let route = active.moduleRoute {
                        Button {
                            openWindow(value: route)
                        } label: {
                            Image(systemName: "macwindow.badge.plus")
                        }
                        .help("Open in New Window")
                    }
                }
                .buttonStyle(.tendIcon)
                .padding(.leading, 6)
            }

            if let accessory {
                TendTheme.border(colorScheme)
                    .frame(width: 1, height: 16)
                    .padding(.horizontal, 6)
                accessory
            }
        }
        .padding(.trailing, 6)
        .font(.system(size: 12))
        .foregroundStyle(.secondary)
        .frame(height: height)
        // The strip's bottom border sits under the tabs, so the active tab,
        // filled with the editor's color, reads as joined to the editor.
        .background(alignment: .bottom) {
            ZStack(alignment: .bottom) {
                TendTheme.tabStrip(colorScheme)
                WindowDragArea()
                TendTheme.border(colorScheme).frame(height: 1)
                    .allowsHitTesting(false)
            }
        }
    }

    @ViewBuilder
    private var tabItems: some View {
        ForEach(group?.tabs ?? [], id: \.self) { selection in
            let label = labels.value[selection] ?? TendTabLabel(selection, model: model)
            EditorTab(
                selection: selection,
                title: label.title,
                systemImage: tabIcon(for: selection),
                stateColor: label.status?.color,
                unreadPane: label.unreadPane, // WS-A
                active: group?.active == selection,
                preview: group?.preview == selection,
                activate: { activate(selection) },
                pin: { tabs.pin(selection, groupID: groupID) },
                close: { close(selection) },
                closeOthers: {
                    tabs.closeOthers(keeping: selection, groupID: groupID)
                    model.activateEditor(selection)
                },
                splitRight: { splitRight(selection) },
                moveToWindow: selection.moduleRoute.map { route in
                    { openWindow(value: route) }
                }
            )
            .id(selection)
            .background {
                GeometryReader { proxy in
                    Color.clear.preference(
                        key: TabFramesKey.self,
                        value: [selection: proxy.frame(in: .named(stripSpace))]
                    )
                }
            }
            // The tab being dragged dims in place, as in VS Code.
            .opacity(dragSession.drag?.selection == selection && dragSession.drag?.sourceGroupID == groupID ? 0.45 : 1)
            .onDrag { dragSession.begin(selection, from: groupID, in: tabs) }
        }
    }

    @Environment(\.colorScheme) private var colorScheme

    private func activate(_ selection: TendSidebarSelection) {
        tabs.activate(selection, groupID: groupID)
        model.activateEditor(selection)
    }

    private func close(_ selection: TendSidebarSelection) {
        let next = tabs.close(selection, groupID: groupID)
        model.activateEditor(next)
    }

    private func splitRight(_ selection: TendSidebarSelection) {
        let target = tabs.splitRight(selection: selection, from: groupID)
        tabs.activate(selection, groupID: target)
        model.activateEditor(selection)
    }

    static func title(for selection: TendSidebarSelection, model: AppModel) -> String {
        switch selection {
        case .file(let location):
            location.name
        case .workBoard:
            String(localized: "Home")
        case .watch:
            String(localized: "Live Watch")
        case .inbox:
            String(localized: "Inbox")
        case .ask:
            String(localized: "Ask")
        case .work(let identity):
            model.workGroups.first { $0.identity == identity }?.title ?? identity.workID
        case .agent(let id):
            model.hostedAgent(id: id).map {
                $0.agent.aiTitle
                    ?? $0.pane?.agentAlias.map { "@\($0)" }
                    ?? $0.agent.kind.replacingOccurrences(of: "_", with: " ")
            } ?? String(localized: "Agent")
        case .host(let id):
            model.fleetHosts.first { $0.id == id }?.alias ?? String(localized: "Host")
        case .fleetSession(let id):
            model.executionSnapshot.watchSession(id: id).map {
                "\($0.hostAlias) · \($0.name.isEmpty ? $0.sessionID : $0.name)"
            } ?? String(localized: "Session")
        case .fleetWindow(let id):
            model.executionSnapshot.watchWindow(id: id).map {
                $0.name.isEmpty ? $0.windowID : $0.name
            } ?? String(localized: "Window")
        case .shell(let id):
            model.sessions.first { $0.id == id }.map { $0.displayName ?? $0.id }
                ?? String(localized: "Shell")
        case .pane(let id):
            model.executionSnapshot.watchPane(id: id).map {
                "\($0.host.alias) · \($0.pane.windowName.isEmpty ? $0.pane.paneID : $0.pane.windowName)"
            } ?? String(localized: "Pane")
        }
    }

    /// The live state of the agent a tab shows, for the dot on its icon
    /// (Orca marks agent tabs the same way).
    static func agentStatus(for selection: TendSidebarSelection, model: AppModel) -> TendAgentStatus? {
        switch selection {
        case .pane(let id): model.executionSnapshot.watchPane(id: id)?.agent?.status
        case .agent(let id): model.hostedAgent(id: id)?.agent.status
        default: nil
        }
    }

    // WS-A: the pane whose unread dot a tab carries.
    static func unreadPane(for selection: TendSidebarSelection, model: AppModel) -> TendWatchPaneIdentity? {
        switch selection {
        case .pane(let id): id
        case .agent(let id): model.hostedAgent(id: id).flatMap(TendAgentAttentionCenter.paneIdentity)
        default: nil
        }
    }

    private func tabIcon(for selection: TendSidebarSelection) -> String {
        switch selection {
        case .file: "doc"
        case .workBoard: "rectangle.3.group"
        case .watch: "waveform.path.ecg.rectangle"
        case .inbox: "tray.full"
        case .ask: "sparkles"
        case .work: "square.stack.3d.up"
        case .agent: "person.crop.circle"
        case .host: "network"
        case .fleetSession: "square.3.layers.3d"
        case .fleetWindow: "macwindow"
        case .shell: "terminal"
        case .pane: "terminal.fill"
        }
    }
}

private struct EditorTab: View {
    let selection: TendSidebarSelection
    let title: String
    let systemImage: String
    var stateColor: Color? = nil
    var unreadPane: TendWatchPaneIdentity? = nil // WS-A
    let active: Bool
    let preview: Bool
    let activate: () -> Void
    let pin: () -> Void
    let close: () -> Void
    let closeOthers: () -> Void
    let splitRight: () -> Void
    let moveToWindow: (() -> Void)?
    @State private var hovering = false

    var body: some View {
        HStack(spacing: 4) {
            Button(action: activate) {
                HStack(spacing: 6) {
                    Image(systemName: systemImage)
                        .font(.system(size: 12))
                        .foregroundStyle(active ? TendInk.label(colorScheme) : TendInk.secondary(colorScheme))
                        .overlay(alignment: .bottomTrailing) {
                            if let stateColor {
                                Circle()
                                    .fill(stateColor)
                                    .frame(width: 6, height: 6)
                                    .overlay(Circle().stroke(TendTheme.tabStrip(colorScheme), lineWidth: 1))
                                    .offset(x: 3, y: 2)
                            }
                        }
                    Text(title)
                        .fontWeight(active ? .medium : .regular)
                        .italic(preview)
                        .lineLimit(1)
                }
                .padding(.leading, 12)
                .frame(maxHeight: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityAddTraits(active ? .isSelected : [])
            .simultaneousGesture(
                TapGesture(count: 2).onEnded {
                    activate()
                    pin()
                }
            )

            Spacer(minLength: 0)

            // The close button's slot is always reserved so tabs never
            // change width as the pointer moves across them.
            Button(action: close) {
                Image(systemName: "xmark")
                    .font(.system(size: 9, weight: .bold))
            }
            .buttonStyle(.tendIcon(size: 18))
            .opacity(active || hovering ? 1 : 0)
            // WS-A: an unseen agent shows a dot in the close slot, like an
            // editor's unsaved-changes dot, until the tab is hovered.
            .overlay {
                if let unreadPane, !active, !hovering {
                    TendUnreadDot(pane: unreadPane, size: 7).allowsHitTesting(false)
                }
            }
            .accessibilityLabel("Close \(title)")
            .help("Close \(title)")
            .padding(.trailing, 6)
        }
        .frame(minWidth: 120, maxWidth: 220, maxHeight: .infinity)
        .foregroundStyle(active || hovering ? TendInk.label(colorScheme) : TendInk.secondary(colorScheme))
        // A view, not a ShapeStyle: a style background would spread into the
        // title bar's safe area above the tab. The showing tab is the
        // content's own colour, joined to it with rounded top corners; no
        // coloured rule (docs/proposals/MAC_DESIGN.md). Drawn as background
        // only, so the strip's layout does not move.
        .background {
            // Only the showing tab has a fill: a hovered one lifts its text
            // instead, so it never looks selected.
            UnevenRoundedRectangle(topLeadingRadius: 7, topTrailingRadius: 7, style: .continuous)
                .fill(active ? TendTheme.editor(colorScheme) : Color.clear)
                .padding(.top, 4)
        }
        .overlay(alignment: .trailing) {
            TendTheme.border(colorScheme).frame(width: 1)
        }
        .onHover { hovering = $0 }
        .help(preview ? Text("Preview — double-click to keep open") : Text(title))
        .contextMenu {
            if preview { Button("Keep Open", action: pin) }
            Button("Close", action: close)
            Button("Close Others", action: closeOthers)
            Divider()
            Button("Open to the Side", action: splitRight)
            if let moveToWindow {
                Button("Open in New Window", action: moveToWindow)
            }
        }
    }

    @Environment(\.colorScheme) private var colorScheme
}

struct WorkbenchStatusBar: View {
    /// For its actions; what it shows comes from `inputs`, published only
    /// when that changes (`TendStatusBarInputs`).
    let model: AppModel
    @ObservedObject var inputs: TendChromeSlice<TendStatusBarInputs>
    let openPane: (TendWatchPaneIdentity) -> Void

    @Environment(\.colorScheme) private var colorScheme

    private var shown: TendStatusBarInputs { inputs.value }

    /// Healthy, the bar is quiet: tendd's state is a dot and a word, like the
    /// rest of the chrome (docs/proposals/MAC_DESIGN.md). Connecting is quiet
    /// too. A problem turns the whole bar a fill that white text reads on, so
    /// it can't be missed.
    private var healthy: Bool {
        if case .connected = shown.connectionState { return true }
        return false
    }

    /// Upgrade required or disconnected: the bar is a coloured fill.
    private var alarming: Bool {
        switch shown.connectionState {
        case .upgradeRequired, .failed: true
        case .connected, .connecting: false
        }
    }

    var body: some View {
        let _ = TendPerf.count("statusbar.body")
        HStack(spacing: 0) {
            connectionItems
                .padding(.horizontal, 10)
                .frame(maxHeight: .infinity)
                .foregroundStyle(alarming ? Color.white : TendInk.secondary(colorScheme))

            Spacer(minLength: 12)

            HStack(spacing: 14) {
                // WS-C usage: hidden while unhealthy, when the figures are stale.
                if let error = shown.agentActionError {
                    Button {
                        model.agentActionError = nil
                    } label: {
                        Label(error, systemImage: "exclamationmark.triangle.fill")
                            .foregroundStyle(alarming ? Color.white : TendInk.state(.orange, colorScheme))
                    }
                    .buttonStyle(.plain)
                    .help("Click to dismiss")
                } else if let notice = shown.agentActionNotice {
                    Label(notice, systemImage: "checkmark.circle.fill")
                        .foregroundStyle(alarming ? Color.white : TendInk.state(.green, colorScheme))
                        .lineLimit(1)
                }
                if healthy {
                    TendSwapStatusItem(swaps: model.swaps, openPane: openPane)
                    TendUsageStatusItems(
                        items: shown.usage, more: shown.moreUsage, moreLevel: shown.moreUsageLevel,
                        openPane: openPane
                    )
                }
                Label("\(shown.hostCount) hosts", systemImage: "server.rack")
                Label("\(shown.agentCount) agents", systemImage: "person.2")
                Label("\(shown.shellCount) shells", systemImage: "terminal")
            }
            .labelStyle(StatusBarLabelStyle())
            .padding(.trailing, 10)
            .foregroundStyle(alarming ? Color.white : TendInk.secondary(colorScheme))
        }
        .font(TendType.tabularMeta)
        .lineLimit(1)
        .frame(maxWidth: .infinity, minHeight: TendTheme.statusBarHeight, maxHeight: TendTheme.statusBarHeight)
        .background(alarming ? statusColor : TendTheme.sideBar(colorScheme))
        .overlay(alignment: .top) {
            TendTheme.border(colorScheme).frame(height: 1)
        }
    }

    @ViewBuilder
    private var connectionItems: some View {
        HStack(spacing: 8) {
            switch shown.connectionState {
            case .connecting:
                ProgressView()
                    .controlSize(.mini)
                Text("Connecting to tendd…")
            case .connected:
                HStack(spacing: 5) {
                    Circle().fill(TendInk.state(.green, colorScheme)).frame(width: 6, height: 6)
                    Text(verbatim: "tendd")
                }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(String(localized: "tendd connected"))
            case .upgradeRequired(let message):
                Label("tendd upgrade required", systemImage: "arrow.triangle.2.circlepath")
                Text(message).lineLimit(1)
                Button("Use Bundled tendd") {
                    model.isConfirmingDaemonReplacement = true
                }
                .buttonStyle(.plain)
                .underline()
            case .failed(let message):
                Label("Disconnected", systemImage: "exclamationmark.triangle.fill")
                Text(message).lineLimit(1)
                Button("Retry", action: model.retryConnection)
                    .buttonStyle(.plain)
                    .underline()
            }
        }
    }

    /// Fills white reads on at 4.8:1 or more (system orange and red were
    /// 2.2 and 3.6:1).
    private var statusColor: Color {
        switch shown.connectionState {
        case .connected, .connecting: TendTheme.sideBar(colorScheme)
        case .upgradeRequired: TendTheme.token(TendDesignTokens.Mac.stateInkOrange, .light)
        case .failed: TendInk.dangerFill(colorScheme)
        }
    }
}

private struct StatusBarLabelStyle: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 4) {
            configuration.icon.font(.system(size: 11))
            configuration.title
        }
    }
}

private struct SidebarConnectionStatus: View {
    let state: AppModel.ConnectionState
    let retry: () -> Void
    let useBundledDaemon: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            switch state {
            case .connecting:
                Label {
                    Text("Connecting to tendd…")
                } icon: {
                    ProgressView().controlSize(.small)
                }
            case .connected:
                Label("tendd connected", systemImage: "circle.fill")
                    .symbolRenderingMode(.palette)
                    .foregroundStyle(.green, .secondary)
            case .upgradeRequired(let message):
                statusMessage(
                    title: "tendd upgrade required",
                    message: message,
                    systemImage: "arrow.triangle.2.circlepath.circle.fill"
                )
                Button("Use Bundled tendd", action: useBundledDaemon)
                    .buttonStyle(.tendPrimary)
                    .controlSize(.small)
            case .failed(let message):
                statusMessage(
                    title: "Connection failed",
                    message: message,
                    systemImage: "exclamationmark.triangle.fill"
                )
                Button("Retry", action: retry)
                    .controlSize(.small)
            }
        }
        .font(TendType.detail)
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder
    private func statusMessage(
        title: LocalizedStringKey,
        message: String,
        systemImage: String
    ) -> some View {
        Label(title, systemImage: systemImage)
            .foregroundStyle(.orange)
        Text(message)
            .font(TendType.meta)
            .foregroundStyle(.secondary)
            .lineLimit(3)
            .fixedSize(horizontal: false, vertical: true)
    }
}
