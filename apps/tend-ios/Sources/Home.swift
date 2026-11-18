// Home: Tend for Mac's three lanes (Your turn, In progress, Finished).
//
// iPad shows them side by side as the Mac does. iPhone shows a lane strip
// with all three lanes and their counts, always on screen, over the list of
// the chosen lane; tap a lane or swipe sideways to switch. The counts are the
// at-a-glance part (what needs me, how much is running, what just finished);
// stacking three lists would push Finished off the screen, and a tab per
// lane would hide the other two counts.
import SwiftUI

struct HomeView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.horizontalSizeClass) private var sizeClass
    @State private var lane: Lane = .yourTurn
    @State private var startingWork = false

    var body: some View {
        // The Mac's columns need room: below 900 pt (iPad portrait with the
        // sidebar, Split View) the lane strip is the better fit.
        GeometryReader { proxy in
            if sizeClass == .regular, proxy.size.width >= 900 {
                board.background(TendSurface.workspace)
            } else {
                strip.background(TendSurface.panel)
            }
        }
        .navigationTitle(String(localized: "Home"))
        // Inline on iPhone too: the lane strip, not a large title, is what
        // the top of the screen is for.
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if sizeClass == .regular {
                ToolbarItem(placement: .topBarLeading) { HostsSummary() }
            }
            ToolbarItem(placement: .topBarTrailing) {
                if model.canStartWork {
                Button {
                    startingWork = true
                } label: {
                    Label(String(localized: "Start Work"), systemImage: "play.fill")
                }
                .accessibilityIdentifier("startWork")
                }
            }
        }
        .sheet(isPresented: $startingWork) {
            StartWorkView()
        }
    }

    // MARK: iPhone

    private var strip: some View {
        VStack(spacing: 0) {
            LaneStrip(selection: $lane)
                .padding(.horizontal, 16)
                .padding(.bottom, 10)
            Divider()
            TabView(selection: $lane) {
                ForEach(model.lanes) { lane in
                    ScrollView {
                        LaneContent(lane: lane)
                            .padding(16)
                    }
                    .refreshable { await model.refresh(); await model.refreshHistory() }
                    .tag(lane)
                }
            }
            .tabViewStyle(.page(indexDisplayMode: .never))
        }
        // The backlog emptied while it was shown.
        .onChange(of: model.lanes) { if !model.lanes.contains(lane) { lane = .yourTurn } }
    }

    // MARK: iPad

    private var board: some View {
        HStack(alignment: .top, spacing: 12) {
            ForEach(model.lanes) { lane in
                LaneColumn(lane: lane)
            }
        }
        .padding(16)
    }
}

/// The three lanes with their counts (iPhone).
struct LaneStrip: View {
    @Binding var selection: Lane
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(spacing: model.lanes.count > 3 ? 6 : 8) {
            ForEach(model.lanes) { lane in
                let selected = lane == selection
                let count = model.count(lane)
                Button {
                    withAnimation(.snappy) { selection = lane }
                } label: {
                    VStack(alignment: .leading, spacing: 2) {
                        // A decide key cannot see agents: no count, not a 0.
                        if lane == .inProgress, !model.operates {
                            Image(systemName: "lock")
                                .font(.title2.weight(.semibold))
                                .foregroundStyle(.secondary)
                                .accessibilityLabel(String(localized: "Not shown on this device"))
                        } else {
                            Text(verbatim: "\(count)")
                                .font(.title2.weight(.bold))
                                .monospacedDigit()
                                .foregroundStyle(lane == .yourTurn && count > 0 ? lane.tint : Color.primary)
                        }
                        HStack(spacing: 5) {
                            // Four lanes leave no room for the dot.
                            if model.lanes.count <= 3 {
                                Circle().fill(lane.tint).frame(width: 7, height: 7)
                            }
                            Text(lane.title)
                                .font(TendType.meta.weight(.semibold))
                                .lineLimit(1)
                                .minimumScaleFactor(0.75)
                        }
                        .foregroundStyle(selected ? Color.primary : Color.secondary)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, model.lanes.count > 3 ? 9 : 12)
                    .padding(.vertical, 10)
                    .background(
                        selected ? lane.tint.opacity(0.12) : TendSurface.workspace,
                        in: RoundedRectangle(cornerRadius: TendSurface.cardRadius, style: .continuous)
                    )
                    .overlay(
                        RoundedRectangle(cornerRadius: TendSurface.cardRadius, style: .continuous)
                            .strokeBorder(selected ? lane.tint.opacity(0.6) : TendSurface.border, lineWidth: selected ? 1.5 : 1)
                    )
                }
                .buttonStyle(.plain)
                .accessibilityLabel(Text("\(lane.title), \(count)"))
                .accessibilityAddTraits(selected ? .isSelected : [])
                .accessibilityIdentifier("lane-\(lane.rawValue)")
            }
        }
    }
}

/// One lane as a column (iPad): the Mac's lane header and cards.
struct LaneColumn: View {
    let lane: Lane
    @Environment(AppModel.self) private var model

    var body: some View {
        let count = model.count(lane)
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text(lane.title)
                        .font(TendType.laneTitle)
                        .foregroundStyle(lane == .yourTurn && count > 0 ? lane.tint : Color.primary)
                    if lane == .inProgress, !model.operates {
                        Image(systemName: "lock").font(TendType.meta).foregroundStyle(.secondary)
                    } else {
                        Text(verbatim: "\(count)")
                            .font(TendType.meta.weight(.semibold))
                            .monospacedDigit()
                            .foregroundStyle(.secondary)
                    }
                    Spacer(minLength: 4)
                    Text(lane.subtitle).font(TendType.meta).foregroundStyle(.tertiary).lineLimit(1)
                }
            }
            .padding(.horizontal, 14)
            .padding(.top, 12)
            .padding(.bottom, 10)
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isHeader)
            .accessibilityIdentifier("laneHeader-\(lane.rawValue)")
            ScrollView {
                LaneContent(lane: lane, showsSubtitle: false)
                    .padding(.horizontal, 10)
                    .padding(.bottom, 12)
            }
            .refreshable { await model.refresh(); await model.refreshHistory() }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(TendSurface.panel, in: RoundedRectangle(cornerRadius: TendSurface.laneRadius, style: .continuous))
    }
}

/// A lane's cards.
struct LaneContent: View {
    let lane: Lane
    var showsSubtitle = true
    @Environment(AppModel.self) private var model
    @State private var showsOlder = false

    var body: some View {
        LazyVStack(alignment: .leading, spacing: 10) {
            if showsSubtitle {
                HStack {
                    Text(lane.subtitle)
                        .font(TendType.meta)
                        .foregroundStyle(.secondary)
                    Spacer()
                    HostsSummary().foregroundStyle(.secondary)
                }
            }
            notices
            if let board = model.visibleBoard {
                switch lane {
                case .backlog: backlog(board)
                case .yourTurn: BoardYourTurn(board: board)
                case .inProgress: BoardLaneItems(board: board, lane: .inProgress)
                case .finished:
                    BoardLaneItems(board: board, lane: .finished)
                    decidedItems(showsEmpty: FoldedLane(board, lane: .finished).count == 0)
                }
            } else {
                switch lane {
                case .backlog: EmptyView()
                case .yourTurn: yourTurn
                case .inProgress: inProgress
                case .finished: finished
                }
            }
        }
        .frame(maxWidth: 760, alignment: .leading)
        .frame(maxWidth: .infinity)
    }

    @ViewBuilder
    private var notices: some View {
        if lane == .yourTurn, let outcome = model.outcome {
            Label(outcome, systemImage: "checkmark.circle.fill")
                .font(TendType.body.weight(.semibold))
                .foregroundStyle(TendPalette.green)
                .accessibilityIdentifier("outcome")
        }
        if let problem = model.problem {
            Label(problem, systemImage: "exclamationmark.triangle")
                .font(TendType.meta)
                .foregroundStyle(TendPalette.red)
                .accessibilityIdentifier("problem")
        }
    }

    // MARK: Backlog

    @ViewBuilder
    private func backlog(_ board: DeviceHome) -> some View {
        if board.backlog.isEmpty { Empty(text: lane.empty) }
        ForEach(board.backlog) { BacklogCard(record: $0) }
        if !board.backlog.isEmpty {
            Text("Planned Work starts from Tend for Mac or the dashboard; here you can start new work.")
                .font(TendType.meta)
                .foregroundStyle(.tertiary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    /// Finished's decided items (from the history), after the board's units.
    @ViewBuilder
    private func decidedItems(showsEmpty: Bool) -> some View {
        let decided = model.past
        let recent = model.recentPast
        let older = decided.count - recent.count
        if !model.historyLoaded {
            ProgressView().frame(maxWidth: .infinity).padding(.top, 16)
        } else if showsEmpty, recent.isEmpty, older == 0 {
            Empty(text: lane.empty)
        }
        ForEach(model.history.filter { !$0.history.isComplete || !$0.unavailable.isEmpty }) { section in
            Label(historyGap(section), systemImage: "exclamationmark.triangle")
                .font(TendType.meta)
                .foregroundStyle(.secondary)
        }
        if !recent.isEmpty {
            DecidedHeader(count: recent.count)
            ForEach(recent) { DecidedCard(past: $0) }
        }
        if older > 0 {
            Button(showsOlder ? String(localized: "Hide older") : String(localized: "\(older) older decided")) {
                withAnimation { showsOlder.toggle() }
            }
            .font(TendType.meta.weight(.medium))
            .buttonStyle(.borderless)
            .accessibilityIdentifier("showOlderDecided")
            if showsOlder {
                ForEach(decided.filter { !recent.contains($0) }) { DecidedCard(past: $0) }
            }
        }
    }

    // MARK: Your turn

    @ViewBuilder
    private var yourTurn: some View {
        if !model.loaded {
            ProgressView().frame(maxWidth: .infinity).padding(.top, 32)
        } else if AppModel.allClear(model.hosts) {
            Empty(text: lane.empty, mark: true)
                .accessibilityIdentifier("emptyQueue")
        }
        ForEach(model.hosts) { section in
            HostSectionView(section: section)
        }
    }

    // MARK: In progress

    @ViewBuilder
    private var inProgress: some View {
        if !model.operates {
            DecideOnlyNotice()
        } else if model.visibleFleet == nil {
            if let problem = model.fleetProblem {
                Label(problem, systemImage: "exclamationmark.triangle").font(TendType.meta).foregroundStyle(.secondary)
            } else {
                ProgressView().frame(maxWidth: .infinity).padding(.top, 32)
            }
        } else {
            let agents = model.inProgress
            if agents.isEmpty { Empty(text: lane.empty) }
            let groups = Self.byProject(agents)
            ForEach(groups, id: \.name) { group in
                if groups.count > 1 { ProjectHeader(name: group.name, count: group.agents.count) }
                ForEach(group.agents) { agent in
                    NavigationLink(value: agent) { AgentCard(agent: agent, lane: .inProgress) }
                        .buttonStyle(.plain)
                }
            }
        }
    }

    /// Cards grouped by project in order of first appearance, as the Mac
    /// lanes group them.
    static func byProject(_ agents: [LaneAgent]) -> [(name: String, agents: [LaneAgent])] {
        var order: [String] = []
        var groups: [String: [LaneAgent]] = [:]
        for agent in agents {
            let name = agent.agent.repo ?? String(localized: "Other")
            if groups[name] == nil { order.append(name) }
            groups[name, default: []].append(agent)
        }
        return order.map { ($0, groups[$0]!) }
    }

    fileprivate static func finishedGroups(_ items: [FinishedItem]) -> [(name: String, decided: Bool, items: [FinishedItem])] {
        let decidedKey = "\u{0}decided"
        var order: [String] = []
        var groups: [String: [FinishedItem]] = [:]
        for item in items {
            let key: String = switch item {
            case .decided: decidedKey
            case .agent(let agent): agent.agent.repo ?? String(localized: "Other")
            }
            if groups[key] == nil { order.append(key) }
            groups[key, default: []].append(item)
        }
        return order.map { ($0, $0 == decidedKey, groups[$0]!) }
    }

    // MARK: Finished


    @ViewBuilder
    private var finished: some View {
        let decided = model.past
        let recent = model.recentPast
        let older = decided.count - recent.count
        let agents = model.finishedAgents
        let items: [FinishedItem] = (recent.map(FinishedItem.decided) + agents.map(FinishedItem.agent))
            .sorted { ($0.at ?? .distantPast) > ($1.at ?? .distantPast) }
        if !model.historyLoaded {
            ProgressView().frame(maxWidth: .infinity).padding(.top, 32)
        } else if items.isEmpty, older == 0 {
            Empty(text: lane.empty)
        }
        ForEach(model.history.filter { !$0.history.isComplete || !$0.unavailable.isEmpty }) { section in
            Label(historyGap(section), systemImage: "exclamationmark.triangle")
                .font(TendType.meta)
                .foregroundStyle(.secondary)
        }
        // As on the Mac: by project, in order of the newest item in each;
        // decisions taken form their own group.
        let groups = Self.finishedGroups(items)
        ForEach(groups, id: \.name) { group in
            if groups.count > 1 {
                if group.decided {
                    DecidedHeader(count: group.items.count)
                } else {
                    ProjectHeader(name: group.name, count: group.items.count)
                }
            }
            ForEach(group.items) { item in
                switch item {
                case .decided(let past): DecidedCard(past: past)
                case .agent(let agent):
                    NavigationLink(value: agent) { AgentCard(agent: agent, lane: .finished) }
                        .buttonStyle(.plain)
                }
            }
        }
        if older > 0 {
            Button(showsOlder ? String(localized: "Hide older") : String(localized: "\(older) older")) {
                withAnimation { showsOlder.toggle() }
            }
            .font(TendType.meta.weight(.medium))
            .buttonStyle(.borderless)
            .accessibilityIdentifier("showOlder")
            if showsOlder {
                ForEach(decided.filter { !recent.contains($0) }) { DecidedCard(past: $0) }
            }
        }
        if !model.operates {
            DecideOnlyNotice(compact: true)
        }
    }
}

private enum FinishedItem: Identifiable {
    case decided(PastDecision)
    case agent(LaneAgent)

    var id: String {
        switch self {
        case .decided(let past): "d:" + past.id
        case .agent(let agent): "a:" + agent.id
        }
    }

    var at: Date? {
        switch self {
        case .decided(let past): past.at
        case .agent(let agent): agent.agent.since
        }
    }
}

/// A lane with nothing in it (the Mac's tertiary line).
struct Empty: View {
    let text: String
    var mark = false

    var body: some View {
        VStack(spacing: 12) {
            if mark {
                TendMark(waiting: false, error: false, working: true, done: true)
                    .frame(width: 44, height: 44)
            }
            Text(text)
                .font(TendType.detail)
                .foregroundStyle(.tertiary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 28)
    }
}

/// What a decide-only device cannot show, why, and the one button that
/// asks the dashboard for more.
struct DecideOnlyNotice: View {
    var compact = false
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Label(String(localized: "This device decides only"), systemImage: "lock")
                .font(TendType.body.weight(.semibold))
            if !compact || model.upgrade != nil {
                Text(explanation)
                    .font(TendType.meta)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HostsAndWorkRequest(prominent: !compact)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .tendCard()
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("decideOnly")
    }

    private var explanation: String {
        switch model.scopeNote {
        case .grantedLess:
            String(localized: "You asked for hosts and new work, and the operator allowed deciding only. Your turn works as before. To ask again, an operator has to approve it on the dashboard.")
        case .limited:
            String(localized: "An operator limited this device to deciding. Your turn works as before. To see hosts and agents again, ask again; an operator approves it on the dashboard.")
        case nil:
            String(localized: "Your turn works here. To also see what is in progress and finished, every host and its agents, and to start new work, ask the dashboard for hosts and new work. An operator approves it there.")
        }
    }
}

/// "Pair again with hosts and new work…": asks the dashboard while this
/// device keeps its decide key, then shows the code an operator approves.
struct HostsAndWorkRequest: View {
    var prominent = true
    @Environment(AppModel.self) private var model
    @State private var confirming = false

    var body: some View {
        if let upgrade = model.upgrade {
            VStack(alignment: .leading, spacing: 8) {
                Text("On a computer signed in to \(upgrade.origin.display), open **access ▸ Devices**, enter this code, and tick **Also let this device see every host and agent…**:")
                    .font(TendType.meta)
                    .fixedSize(horizontal: false, vertical: true)
                Text(verbatim: upgrade.userCode)
                    .font(.system(.title, design: .monospaced).weight(.bold))
                    .kerning(2)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 10)
                    .background(TendColor.waiting.opacity(0.14), in: RoundedRectangle(cornerRadius: 10))
                    .accessibilityIdentifier("upgradeCode")
                HStack(spacing: 8) {
                    ProgressView()
                    Text("Waiting for approval · expires \(upgrade.expires, style: .relative)")
                        .font(TendType.meta)
                        .foregroundStyle(.secondary)
                    Spacer(minLength: 4)
                    Button(String(localized: "Cancel"), role: .cancel) { model.cancelUpgrade() }
                        .font(TendType.meta.weight(.medium))
                        .accessibilityIdentifier("cancelUpgrade")
                }
                Text("Until then this device keeps deciding with its current key.")
                    .font(TendType.meta)
                    .foregroundStyle(.tertiary)
            }
        } else if model.pairing != nil {
            VStack(alignment: .leading, spacing: 6) {
                button
                if let problem = model.upgradeProblem {
                    Label(problem, systemImage: "exclamationmark.circle")
                        .font(TendType.meta)
                        .foregroundStyle(TendPalette.red)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityIdentifier("upgradeProblem")
                }
            }
            .confirmationDialog(String(localized: "Ask for hosts and new work?"), isPresented: $confirming, titleVisibility: .visible) {
                Button(String(localized: "Ask the dashboard")) { model.requestHostsAndWork() }
            } message: {
                Text("This device shows a code for an operator to approve on the dashboard. It keeps deciding meanwhile; once approved, the new key replaces this one. The operator may still allow deciding only.")
            }
        }
    }

    @ViewBuilder
    private var button: some View {
        let label = String(localized: "Pair again with hosts and new work…")
        if prominent {
            Button(label) { confirming = true }
                .buttonStyle(.borderedProminent)
                .accessibilityIdentifier("repair")
        } else {
            Button(label) { confirming = true }
                .font(TendType.meta.weight(.medium))
                .buttonStyle(.borderless)
                .accessibilityIdentifier("repair")
        }
    }
}

/// "Hosts n/m" with a dot: green when every host is online (the Mac
/// header's hosts button).
struct HostsSummary: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if let fleet = model.visibleFleet {
            let online = fleet.hosts.filter(\.online).count
            HStack(spacing: 5) {
                Circle().fill(online == fleet.hosts.count ? TendPalette.green : TendPalette.orange).frame(width: 7, height: 7)
                Text("Hosts \(online)/\(fleet.hosts.count)")
                    .font(TendType.meta.weight(.medium))
                    .monospacedDigit()
                    .lineLimit(1)
                    .fixedSize()
            }
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("hostsSummary")
        }
    }
}

func historyGap(_ section: HostHistory) -> String {
    var parts = section.unavailable + section.history.unavailable
    if section.history.truncated { parts.append(String(localized: "only the latest are shown")) }
    if section.history.undecoded > 0 {
        parts.append(String(localized: "\(section.history.undecoded) this app cannot show"))
    }
    return String(localized: "\(section.alias): \(parts.joined(separator: "; "))")
}

/// The heading over decisions taken, in Finished.
struct DecidedHeader: View {
    let count: Int

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "checkmark.seal").imageScale(.small)
            Text("Decided").font(TendType.meta.weight(.semibold))
            Text(verbatim: "\(count)").font(TendType.meta).monospacedDigit().foregroundStyle(.tertiary)
        }
        .foregroundStyle(.secondary)
        .padding(.top, 4)
        .accessibilityAddTraits(.isHeader)
    }
}
