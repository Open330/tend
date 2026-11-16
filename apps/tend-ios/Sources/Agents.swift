// Hosts and agents: every Fleet host with its state, the agents on each,
// and one agent's latest result and recent output (the Mac app's host tree,
// `FleetQuickLook` and `FleetAgentDetailView`). Needs the `operate` scope.
import SwiftUI

extension View {
    /// Agent cards and rows push the agent's detail.
    func withAgentDestinations() -> some View {
        navigationDestination(for: LaneAgent.self) { AgentDetailView(agent: $0) }
            .navigationDestination(for: BoardUnit.self) { WorkDetailView(unit: $0) }
    }
}

/// A host in a list: badge with state dot, alias, agents (the Mac's host
/// row in Explore).
struct HostRow: View {
    let host: FleetHost

    var body: some View {
        let live = host.liveAgents
        let attention = live.filter { $0.status.needsAttention || $0.awaitingReply }.count
        HStack(spacing: 10) {
            HostBadge(alias: host.alias, local: host.local, state: host.local ? "online" : host.state, size: 28)
            VStack(alignment: .leading, spacing: 1) {
                Text(verbatim: host.alias).font(TendType.body.weight(.semibold))
                Text(host.online ? Self.summary(live) : HostState.label(host.state))
                    .font(TendType.meta)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 4)
            if attention > 0 {
                Text(verbatim: "\(attention)")
                    .font(TendType.meta.weight(.bold))
                    .monospacedDigit()
                    .foregroundStyle(.white)
                    .padding(.horizontal, 7)
                    .padding(.vertical, 2)
                    .background(TendPalette.orange, in: Capsule())
                    .accessibilityLabel(Text("\(attention) need you"))
            }
        }
        .padding(.vertical, 2)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("hostRow")
    }
}

extension HostRow {
    /// "12 agents · 7 working · 2 need you".
    static func summary(_ agents: [FleetAgent]) -> String {
        var parts = [String(localized: "\(agents.count) agents")]
        let working = agents.filter(\.running).count
        let waiting = agents.filter { $0.status.needsAttention || $0.awaitingReply }.count
        if working > 0 { parts.append(String(localized: "\(working) working")) }
        if waiting > 0 { parts.append(String(localized: "\(waiting) need you")) }
        return parts.joined(separator: " · ")
    }
}

/// Every host (iPhone's Hosts tab).
struct HostsView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 12) {
                if !model.operates {
                    DecideOnlyNotice()
                } else if let fleet = model.visibleFleet {
                    ForEach(fleet.ordered) { host in
                        NavigationLink(value: host) {
                            HostRow(host: host).tendCard()
                        }
                        .buttonStyle(.plain)
                    }
                } else if let problem = model.fleetProblem {
                    Label(problem, systemImage: "exclamationmark.triangle").font(TendType.meta).foregroundStyle(.secondary)
                } else {
                    ProgressView().frame(maxWidth: .infinity).padding(.top, 32)
                }
            }
            .padding(16)
            .frame(maxWidth: 760)
            .frame(maxWidth: .infinity)
        }
        .background(TendSurface.panel)
        .refreshable { await model.refresh() }
        .navigationTitle(String(localized: "Hosts"))
        .navigationDestination(for: FleetHost.self) { HostDetailView(alias: $0.alias) }
    }
}

/// One host: its state, then its agents, those that need you first.
struct HostDetailView: View {
    let alias: String
    @Environment(AppModel.self) private var model

    private var host: FleetHost? { model.visibleFleet?.hosts.first { $0.alias == alias } }

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 10) {
                if !model.operates {
                    DecideOnlyNotice()
                } else if let host {
                    header(host)
                    let agents = host.liveAgents.sorted(by: Self.order)
                    SectionTitle(text: String(localized: "Agents"), count: agents.count)
                        .padding(.top, 6)
                    if agents.isEmpty {
                        Empty(text: String(localized: "No agent runs here."))
                    }
                    ForEach(agents) { agent in
                        let item = LaneAgent(agent: agent, host: host.alias, hostIsLocal: host.local)
                        NavigationLink(value: item) {
                            AgentCard(agent: item, lane: agent.running ? .inProgress : .finished)
                        }
                        .buttonStyle(.plain)
                    }
                } else {
                    Empty(text: String(localized: "This host is not in the Fleet snapshot."))
                }
            }
            .padding(16)
            .frame(maxWidth: 760)
            .frame(maxWidth: .infinity)
        }
        .background(TendSurface.panel)
        .refreshable { await model.refresh() }
        .navigationTitle(alias)
    }

    private func header(_ host: FleetHost) -> some View {
        HStack(spacing: 12) {
            HostBadge(alias: host.alias, local: host.local, state: host.local ? "online" : host.state, size: 40)
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 6) {
                    Circle().fill(HostState.color(host.local ? "online" : host.state)).frame(width: 8, height: 8)
                    Text(host.local ? String(localized: "This dashboard's host") : HostState.label(host.state))
                        .font(TendType.body.weight(.semibold))
                }
                Text(verbatim: [host.tendVersion.map { "tend \($0)" }, host.control ? String(localized: "control") : String(localized: "observe only")]
                    .compactMap { $0 }.joined(separator: " · "))
                    .font(TendType.meta)
                    .foregroundStyle(.secondary)
                if let error = host.error, !host.online {
                    Text(verbatim: error).font(TendType.meta).foregroundStyle(TendPalette.red).lineLimit(3)
                }
            }
        }
        .tendCard()
    }

    /// Needs you first (the Mac's status priority), then the most recent.
    private static func order(_ left: FleetAgent, _ right: FleetAgent) -> Bool {
        let rank = { (agent: FleetAgent) -> Int in
            switch agent.status {
            case .error: 0
            case .needsInput: 1
            case .asked: 2
            case .limited: 3
            case .working: 4
            case .idle: 5
            case .done: 6
            case .other: 7
            }
        }
        if rank(left) != rank(right) { return rank(left) < rank(right) }
        return (left.since ?? .distantPast) > (right.since ?? .distantPast)
    }
}

/// One agent: what it is on, its latest result as Markdown, what it was
/// asked, and its recent output.
struct AgentDetailView: View {
    let agent: LaneAgent
    @Environment(AppModel.self) private var model

    /// The current record for this agent, refreshed with the snapshot.
    private var current: FleetAgent {
        model.visibleFleet?.hosts.first { $0.alias == agent.host }?.agents.first { $0.id == agent.agent.id } ?? agent.agent
    }

    var body: some View {
        let a = current
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                VStack(alignment: .leading, spacing: 6) {
                    Text(verbatim: a.title)
                        .font(.title2.weight(.semibold))
                        .fixedSize(horizontal: false, vertical: true)
                    HStack(spacing: 8) {
                        AgentStatusTag(status: a.status)
                        MetaRow(kind: a.kind, repo: a.repo, host: agent.host, hostIsLocal: agent.hostIsLocal, since: a.since)
                    }
                }
                NowBox(agent: a)
                if let prompt = a.lastPrompt?.nonEmpty {
                    DetailSection(title: String(localized: "Asked")) {
                        Text(verbatim: String(prompt.prefix(1200)))
                            .font(TendType.body)
                            .textSelection(.enabled)
                    }
                }
                if a.pane != nil, model.canCapture {
                    OutputSection(agent: LaneAgent(agent: a, host: agent.host, hostIsLocal: agent.hostIsLocal))
                }
                DetailSection(title: String(localized: "Where")) {
                    VStack(alignment: .leading, spacing: 4) {
                        row(String(localized: "Host"), agent.host)
                        if let session = a.tmuxSession { row(String(localized: "Session"), session) }
                        if let pane = a.pane { row(String(localized: "Pane"), pane) }
                        if let cwd = a.cwd { row(String(localized: "Folder"), cwd) }
                        if let model = a.model { row(String(localized: "Model"), model) }
                    }
                }
            }
            .padding(16)
            .frame(maxWidth: 760, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .background(TendSurface.workspace)
        .navigationTitle(a.maker)
        .navigationBarTitleDisplayMode(.inline)
        .refreshable { await model.refresh() }
    }

    private func row(_ label: String, _ value: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(label).font(TendType.meta).foregroundStyle(.secondary).frame(width: 64, alignment: .leading)
            Text(verbatim: value).font(TendType.mono).textSelection(.enabled).lineLimit(2).truncationMode(.middle)
        }
    }
}

/// The summary box (`FleetQuickLook`): "Needs you" or "Now" / "Last
/// result", then what the agent said, as Markdown.
struct NowBox: View {
    let agent: FleetAgent

    var body: some View {
        let tint: Color = agent.status.needsAttention ? TendPalette.orange : agent.running ? TendPalette.blue : TendPalette.green
        let label = agent.status.needsAttention ? String(localized: "Needs you")
            : agent.running ? String(localized: "Now") : String(localized: "Last result")
        let text = agent.status.needsAttention
            ? (agent.lastNotification ?? String(localized: "Waits for your answer in its pane."))
            : (agent.running ? (agent.recap ?? agent.lastNotification ?? agent.lastPrompt) : (agent.lastResponse ?? agent.recap))
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                Text(label.uppercased()).font(TendType.section).tracking(0.4).foregroundStyle(tint)
                Spacer()
                if let since = agent.since {
                    Text(since, format: .relative(presentation: .numeric, unitsStyle: .abbreviated))
                        .font(TendType.meta).monospacedDigit().foregroundStyle(.tertiary)
                }
            }
            if let text = text?.nonEmpty {
                Text(TendMarkdown.attributed(String(text.prefix(6000))))
                    .font(TendType.body)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityIdentifier("agentSummary")
            } else {
                Text("Nothing recorded yet.").font(TendType.body).foregroundStyle(.tertiary)
            }
        }
        .padding(14)
        .background(tint.opacity(0.08), in: RoundedRectangle(cornerRadius: TendSurface.laneRadius, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: TendSurface.laneRadius, style: .continuous)
                .strokeBorder(tint.opacity(0.4), lineWidth: 1)
        )
    }
}

struct DetailSection<Content: View>: View {
    let title: String
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            SectionTitle(text: title)
            content
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

/// The last lines of the agent's pane, refreshed every 2 s while shown
/// (the Mac's "Show pane"). Read on request through the dashboard's narrow
/// capture route, under the host's capture policy.
struct OutputSection: View {
    let agent: LaneAgent
    @Environment(AppModel.self) private var model
    @State private var shown = false
    @State private var output: String?
    @State private var problem: String?

    var body: some View {
        DetailSection(title: String(localized: "Recent output")) {
            if shown {
                VStack(alignment: .leading, spacing: 6) {
                    ScrollViewReader { proxy in
                        ScrollView([.vertical, .horizontal]) {
                            Text(verbatim: output ?? "")
                                .font(.system(.caption, design: .monospaced))
                                .foregroundStyle(Color.white.opacity(0.92))
                                .textSelection(.enabled)
                                .padding(10)
                                .id("end")
                        }
                        .frame(height: 280)
                        .background(Color.black.opacity(0.86), in: RoundedRectangle(cornerRadius: TendSurface.controlRadius, style: .continuous))
                        .onChange(of: output) { proxy.scrollTo("end", anchor: .bottom) }
                    }
                    .accessibilityIdentifier("agentOutput")
                    if let problem {
                        Label(problem, systemImage: "exclamationmark.triangle").font(TendType.meta).foregroundStyle(.secondary)
                    }
                }
                .task(id: agent.id) { await poll() }
            } else {
                Button {
                    shown = true
                } label: {
                    Label(String(localized: "Show recent output"), systemImage: "terminal")
                }
                .buttonStyle(.bordered)
                .accessibilityIdentifier("showOutput")
            }
        }
    }

    private func poll() async {
        while !Task.isCancelled {
            do {
                let text = try await model.capture(agent)
                let lines = text.split(separator: "\n", omittingEmptySubsequences: false)
                output = lines.suffix(80).joined(separator: "\n").trimmingCharacters(in: .newlines)
                problem = nil
            } catch is CancellationError {
                return
            } catch {
                problem = error.localizedDescription
            }
            try? await Task.sleep(nanoseconds: 2_000_000_000)
        }
    }
}
