// Home's lanes drawn from the daemon's board (`DeviceHome`): units and
// decisions in its order and project groups, the Mac's folds, and a unit's
// detail with its stages, agents and helpers.
import SwiftUI

/// A lane's project heading: a project, "No project", or tend's own asks.
struct BoardGroupHeader: View {
    let group: BoardGroup

    var body: some View {
        if group.system {
            HStack(spacing: 6) {
                Image(systemName: "gearshape").imageScale(.small)
                Text("From tend itself").font(TendType.meta.weight(.semibold))
                Text(verbatim: "\(group.items.count)").font(TendType.meta).monospacedDigit().foregroundStyle(TendInk.tertiary)
            }
            .foregroundStyle(TendInk.secondary)
            .padding(.top, 4)
            .accessibilityAddTraits(.isHeader)
        } else {
            ProjectHeader(name: group.project ?? String(localized: "No project"), count: group.items.count)
        }
    }
}

/// One item of a lane: a decision card, or a unit that opens its detail.
struct BoardItemView: View {
    let item: BoardItem
    /// The project group it is listed in, for a decision's meta row.
    var project: String?

    var body: some View {
        switch item {
        case .decision(let decision):
            DecisionCard(decision: decision, project: project)
        case .unit(let unit):
            UnitLink(unit: unit)
        }
    }
}

/// A unit's card, opening what it is: one agent on its own opens that
/// agent (its latest result and recent output), as the Mac's inspector
/// does; a Work with stages or helpers opens the unit.
struct UnitLink: View {
    let unit: BoardUnit
    @Environment(AppModel.self) private var model

    var body: some View {
        Group {
            if unit.agents.count == 1, unit.helpers.isEmpty, unit.stages.count <= 1,
               let agent = model.laneAgent(unit.agents[0]) {
                NavigationLink(value: agent) { WorkCard(unit: unit) }
            } else {
                NavigationLink(value: unit) { WorkCard(unit: unit) }
            }
        }
        .buttonStyle(.plain)
    }
}

/// Your turn from the board: what each host's queue could not cover, then
/// decisions and units, the longest wait first, by project.
struct BoardYourTurn: View {
    let board: DeviceHome
    @Environment(AppModel.self) private var model

    var body: some View {
        if board.allClear {
            Empty(text: Lane.yourTurn.empty, mark: true)
                .accessibilityIdentifier("emptyQueue")
        }
        ForEach(board.decisionHosts, id: \.alias) { section in
            ForEach(section.unavailable, id: \.self) { reason in
                Label(String(localized: "\(section.alias): not everything could be read: \(reason)"), systemImage: "exclamationmark.triangle")
                    .font(TendType.meta)
                    .foregroundStyle(TendInk.secondary)
            }
            if section.truncated {
                Text("\(section.alias): showing the first ones; more are waiting.")
                    .font(TendType.meta)
                    .foregroundStyle(TendInk.secondary)
            }
        }
        if !board.coverageKnown || board.serverPartial {
            Label(String(localized: "Not every host's queue could be read, so more may need you."), systemImage: "exclamationmark.triangle")
                .font(TendType.meta)
                .foregroundStyle(TendPalette.orange)
                .accessibilityIdentifier("coverageUnknown")
        }
        if let undecoded = board.undecoded[.yourTurn], undecoded > 0 {
            Label(String(localized: "\(undecoded) more need you in a form this app cannot show yet. Open the dashboard in a browser, or update the app."), systemImage: "exclamationmark.triangle")
                .font(TendType.meta)
                .foregroundStyle(TendPalette.orange)
                .accessibilityIdentifier("undecoded")
        }
        let groups = board.lanes[.yourTurn] ?? []
        ForEach(groups) { group in
            VStack(alignment: .leading, spacing: 10) {
                if groups.count > 1 { BoardGroupHeader(group: group) }
                ForEach(group.items) { BoardItemView(item: $0, project: group.project) }
            }
        }
        CantAnswerGroup()
    }
}

/// In progress or Finished from the board, with the Mac's folds: possible
/// helpers, empty sessions and older units each behind a button, reviewed
/// results behind "Show N reviewed".
struct BoardLaneItems: View {
    let board: DeviceHome
    let lane: Lane
    @State private var showsHelpers = false
    @State private var showsEmpty = false
    @State private var showsOlder = false
    @State private var showsReviewed = false

    var body: some View {
        let folded = FoldedLane(board, lane: lane)
        if folded.groups.isEmpty, lane == .inProgress, folded.helpers.isEmpty, folded.empty.isEmpty, folded.older.isEmpty {
            Empty(text: lane.empty)
        }
        ForEach(folded.groups) { group in
            VStack(alignment: .leading, spacing: 10) {
                if folded.groups.count > 1 { BoardGroupHeader(group: group) }
                ForEach(group.items) { BoardItemView(item: $0, project: group.project) }
            }
        }
        fold(folded.helpers, shown: $showsHelpers, show: { String(localized: "\($0) possible helper sessions") }, hide: String(localized: "Hide possible helpers"), id: "foldHelpers")
        fold(folded.empty, shown: $showsEmpty, show: { String(localized: "\($0) empty sessions") }, hide: String(localized: "Hide empty sessions"), id: "foldEmpty")
        fold(folded.older, shown: $showsOlder, show: { String(localized: "\($0) older") }, hide: String(localized: "Hide older"), id: "foldOlder")
        // The number tendd counted when it read the Looks Good marks.
        let reviewed = board.reviewedCount(lane)
        fold(folded.reviewed, shown: $showsReviewed, show: { _ in String(localized: "Show \(reviewed) reviewed") }, hide: String(localized: "Hide reviewed"), id: "foldReviewed")
        if let undecoded = board.undecoded[lane], undecoded > 0 {
            Label(String(localized: "\(undecoded) this app cannot show"), systemImage: "exclamationmark.triangle")
                .font(TendType.meta)
                .foregroundStyle(TendInk.secondary)
        }
    }

    @ViewBuilder
    private func fold(_ units: [BoardUnit], shown: Binding<Bool>, show: (Int) -> String, hide: String, id: String) -> some View {
        if !units.isEmpty {
            Button(shown.wrappedValue ? hide : show(units.count)) {
                withAnimation { shown.wrappedValue.toggle() }
            }
            .font(TendType.meta.weight(.medium))
            .buttonStyle(.borderless)
            .accessibilityIdentifier(id)
            if shown.wrappedValue {
                ForEach(units) { UnitLink(unit: $0) }
            }
        }
    }
}

/// One unit: what it is on, its stages, then the agents on it and the
/// helper sessions they started (the Mac's inspector, read-only here). An
/// agent opens its own detail: latest result, recent output.
struct WorkDetailView: View {
    let unit: BoardUnit
    @Environment(AppModel.self) private var model

    /// The unit as the latest board has it; the snapshot it was opened
    /// with only while this device may still read the board.
    private var current: BoardUnit? {
        guard let board = model.visibleBoard else { return nil }
        for lane in Lane.board {
            for case .unit(let found) in board.items(lane) where found.id == unit.id { return found }
        }
        return unit
    }

    var body: some View {
        if let u = current {
            detail(u)
        } else {
            ScrollView {
                if model.operates {
                    Empty(text: String(localized: "This work is not on the board any more."))
                } else {
                    DecideOnlyNotice()
                }
            }
            .padding(16)
            .background(TendSurface.workspace)
        }
    }

    private func detail(_ u: BoardUnit) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                VStack(alignment: .leading, spacing: 6) {
                    Text(verbatim: u.titleText)
                        .font(.title2.weight(.semibold))
                        .fixedSize(horizontal: false, vertical: true)
                    HStack(spacing: 8) {
                        Label(u.lane.statusLabel, systemImage: u.lane.statusImage)
                            .font(TendType.meta.weight(.semibold))
                            .foregroundStyle(u.lane.tint)
                        if let repo = u.repo {
                            Text(verbatim: repo).font(TendType.meta).foregroundStyle(TendInk.secondary)
                        }
                        Spacer(minLength: 4)
                        if let time = u.time {
                            Text(time, format: .relative(presentation: .numeric, unitsStyle: .abbreviated))
                                .font(TendType.meta).monospacedDigit().foregroundStyle(TendInk.tertiary)
                        }
                    }
                }
                if let headline = u.headlineText {
                    InsetBox {
                        Text(TendMarkdown.attributed(headline))
                            .font(TendType.body)
                            .textSelection(.enabled)
                            .accessibilityIdentifier("workHeadline")
                    }
                }
                if u.stages.count > 1 {
                    DetailSection(title: String(localized: "Stages")) {
                        VStack(alignment: .leading, spacing: 8) {
                            StageBar(unit: u)
                            ForEach(Array(u.stages.enumerated()), id: \.offset) { _, stage in
                                HStack(spacing: 8) {
                                    Circle().fill(stage.state.color).frame(width: 8, height: 8)
                                    Text(verbatim: stage.name).font(TendType.body.weight(.medium))
                                    if let owner = stage.owner {
                                        Text(verbatim: owner).font(TendType.meta).foregroundStyle(TendInk.secondary).lineLimit(1)
                                    }
                                    Spacer(minLength: 4)
                                    Text(stage.state.label).font(TendType.meta).foregroundStyle(TendInk.secondary)
                                }
                                .accessibilityElement(children: .combine)
                            }
                        }
                    }
                }
                agents(String(localized: "Agents"), u.agents, focus: u.focus)
                if !u.helpers.isEmpty {
                    agents(String(localized: "Helpers"), u.helpers, focus: nil)
                    Text("Sessions other agents started to help with this work.")
                        .font(TendType.meta)
                        .foregroundStyle(TendInk.tertiary)
                }
            }
            .padding(16)
            .frame(maxWidth: 760, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .background(TendSurface.workspace)
        .navigationTitle(u.titleText)
        .navigationBarTitleDisplayMode(.inline)
        .refreshable { await model.refresh() }
    }

    @ViewBuilder
    private func agents(_ title: String, _ refs: [BoardAgent], focus: BoardAgent?) -> some View {
        if !refs.isEmpty {
            DetailSection(title: title) {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(refs) { ref in
                        if let lane = model.laneAgent(ref) {
                            NavigationLink(value: lane) {
                                AgentRefRow(ref: ref, isFocus: ref.id == focus?.id, opens: true)
                            }
                            .buttonStyle(.plain)
                        } else {
                            AgentRefRow(ref: ref, isFocus: ref.id == focus?.id, opens: false)
                        }
                    }
                }
            }
        }
    }
}

/// An agent a unit names: its mark, who it is, where, and its status.
struct AgentRefRow: View {
    let ref: BoardAgent
    let isFocus: Bool
    let opens: Bool

    var body: some View {
        HStack(spacing: 10) {
            AgentLogo(kind: ref.kind ?? "", size: 18)
            VStack(alignment: .leading, spacing: 2) {
                Text(verbatim: ref.label).font(TendType.body.weight(isFocus ? .semibold : .regular)).lineLimit(1)
                HStack(spacing: 4) {
                    HostGlyph(alias: ref.host, size: 13)
                    Text(verbatim: ref.host)
                    if let status = ref.status?.words.nonEmpty {
                        Text(verbatim: "· \(status)")
                    }
                }
                .font(TendType.meta)
                .foregroundStyle(TendSurface.metaText)
                .lineLimit(1)
            }
            Spacer(minLength: 4)
            AgentStatusTag(status: ref.agentStatus, showsLabel: false)
            if opens {
                Image(systemName: "chevron.right").font(TendType.meta.weight(.semibold)).foregroundStyle(TendInk.tertiary)
            }
        }
        .tendCard(stroke: isFocus ? ref.agentStatus.color.opacity(0.5) : TendSurface.border)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("agentRef")
    }
}
