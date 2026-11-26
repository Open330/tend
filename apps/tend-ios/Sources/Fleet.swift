// The Fleet snapshot (`GET /api/fleet`): every host this dashboard covers,
// its state, and the agents on it. In progress and the agents half of
// Finished come from here, by the same rules as the web Home
// (crates/tend/src/dashboard/web/home-model.mjs), which follow the Mac
// app's `FleetLane`. Decoding is lenient per agent: one record this app
// cannot read drops that agent, never the host.
import Foundation

struct FleetSnapshot: Decodable, Sendable, Equatable {
    let hosts: [FleetHost]

    /// This host first, then the others by name, like the decision queue.
    var ordered: [FleetHost] {
        hosts.sorted { left, right in
            left.local != right.local ? left.local : left.alias.localizedStandardCompare(right.alias) == .orderedAscending
        }
    }
}

struct FleetHost: Decodable, Sendable, Equatable, Identifiable, Hashable {
    let alias: String
    let local: Bool
    let state: String
    let mode: String
    let tendVersion: String?
    let error: String?
    let lastSeenAt: Date?
    let agents: [FleetAgent]
    /// Panes without an agent (shells), for the host's count.
    let paneCount: Int

    var id: String { alias }
    var online: Bool { local || state == "online" }
    var control: Bool { mode == "control" }

    enum CodingKeys: String, CodingKey {
        case alias, local, state, mode, error, remote
        case tendVersion = "tend_version"
        case lastSeenAt = "last_seen_at"
    }

    private struct Remote: Decodable {
        let agents: [TendLenient<FleetAgent>]?
        let panes: [TendLenient<Ignored>]?
    }

    private struct Ignored: Decodable, Sendable {}

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        alias = try values.decode(String.self, forKey: .alias)
        local = (try? values.decodeIfPresent(Bool.self, forKey: .local)) ?? false
        state = (try? values.decodeIfPresent(String.self, forKey: .state)) ?? "offline"
        mode = (try? values.decodeIfPresent(String.self, forKey: .mode)) ?? "observe"
        tendVersion = try? values.decodeIfPresent(String.self, forKey: .tendVersion)
        error = try? values.decodeIfPresent(String.self, forKey: .error)
        lastSeenAt = ((try? values.decodeIfPresent(String.self, forKey: .lastSeenAt)) ?? nil).flatMap { Timestamp.parse($0) }
        let remote = try? values.decodeIfPresent(Remote.self, forKey: .remote)
        agents = (remote?.agents ?? []).compactMap(\.value)
        paneCount = remote?.panes?.count ?? 0
    }

    init(alias: String, local: Bool, state: String, mode: String = "control", agents: [FleetAgent]) {
        self.alias = alias
        self.local = local
        self.state = state
        self.mode = mode
        tendVersion = nil
        error = nil
        lastSeenAt = nil
        self.agents = agents
        paneCount = agents.count
    }

    /// Agents with a pane that have not stopped: what the lanes count.
    var liveAgents: [FleetAgent] { agents.filter { $0.pane != nil && $0.state != "stopped" } }
}

struct FleetAgent: Decodable, Sendable, Equatable, Hashable, Identifiable {
    let kind: String
    let sessionID: String
    let pane: String?
    let tmuxSocket: String?
    let tmuxSession: String?
    let cwd: String?
    let state: String
    let awaitingReply: Bool
    let lastPrompt: String?
    let lastResponse: String?
    let lastNotification: String?
    let recap: String?
    let aiTitle: String?
    let model: String?
    let stateEnteredAt: Date?
    let lastActivityAt: Date?
    let lastPromptAt: Date?
    let lastResponseAt: Date?
    let startedAt: Date?

    var id: String { sessionID.isEmpty ? (pane ?? "") : sessionID }

    enum CodingKeys: String, CodingKey {
        case kind, pane, cwd, state, recap, model
        case sessionID = "agent_session_id"
        case tmuxSocket = "tmux_socket"
        case tmuxSession = "tmux_session"
        case awaitingReply = "awaiting_reply"
        case lastPrompt = "last_prompt"
        case lastResponse = "last_response"
        case lastNotification = "last_notification"
        case aiTitle = "ai_title"
        case stateEnteredAt = "state_entered_at"
        case lastActivityAt = "last_activity_at"
        case lastPromptAt = "last_prompt_at"
        case lastResponseAt = "last_response_at"
        case startedAt = "started_at"
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        func text(_ key: CodingKeys) -> String? {
            // An empty string is no value, as the web's truthiness reads it.
            ((try? values.decodeIfPresent(String.self, forKey: key)) ?? nil).flatMap { $0.isEmpty ? nil : $0 }
        }
        kind = text(.kind) ?? "agent"
        sessionID = text(.sessionID) ?? ""
        pane = text(.pane)
        tmuxSocket = text(.tmuxSocket)
        tmuxSession = text(.tmuxSession)
        cwd = text(.cwd)
        state = try values.decode(String.self, forKey: .state)
        // Present (any value but null) means it asked in its pane.
        awaitingReply = values.contains(.awaitingReply) && !((try? values.decodeNil(forKey: .awaitingReply)) ?? true)
        lastPrompt = text(.lastPrompt)
        lastResponse = text(.lastResponse)
        lastNotification = text(.lastNotification)
        recap = text(.recap)
        aiTitle = text(.aiTitle)
        model = text(.model)
        stateEnteredAt = text(.stateEnteredAt).flatMap(Timestamp.parse)
        lastActivityAt = text(.lastActivityAt).flatMap(Timestamp.parse)
        lastPromptAt = text(.lastPromptAt).flatMap(Timestamp.parse)
        lastResponseAt = text(.lastResponseAt).flatMap(Timestamp.parse)
        startedAt = text(.startedAt).flatMap(Timestamp.parse)
    }

    var status: AgentStatus { AgentStatus(state: state, awaitingReply: awaitingReply) }
    var running: Bool { state == "working" || state == "starting" }
    var since: Date? { stateEnteredAt ?? lastActivityAt }
    /// Its last turn: the last prompt or response, whichever is later, else
    /// its start (tendd's `turn_time`). Not `stateEnteredAt`, which hooks
    /// reset on agents idle for days.
    var lastTurn: Date? { [lastPromptAt, lastResponseAt].compactMap { $0 }.max() ?? startedAt ?? since }

    /// What it is on: its title, recap, prompt or notification.
    var title: String {
        for candidate in [aiTitle, recap, lastPrompt, lastNotification] {
            if let line = candidate?.firstLine { return line }
        }
        return running ? String(localized: "Working") : String(localized: "No task recorded")
    }

    /// The headline under the title (`FleetWork.headline`): a waiting
    /// agent's question; otherwise the recap or the latest response.
    var headline: String? {
        if status.needsAttention {
            return lastNotification?.firstLine ?? String(localized: "Waits for your answer. Open to see the question.")
        }
        let line = (running ? (recap?.firstLine ?? lastNotification?.firstLine) : (lastResponse?.firstLine ?? recap?.firstLine))
        guard let line, line != title else { return nil }
        return String(line.prefix(160))
    }

    /// The project: the last part of its working directory.
    var repo: String? {
        cwd?.split(whereSeparator: { $0 == "/" || $0 == "\\" }).last.map(String.init)
    }

    /// The agent's maker, for its label ("claude", "codex").
    var maker: String {
        switch kind {
        case "claude_code": "claude"
        default: kind.replacingOccurrences(of: "_", with: " ")
        }
    }
}

extension String {
    /// The first non-blank line, trimmed.
    var firstLine: String? {
        split(separator: "\n", omittingEmptySubsequences: true)
            .lazy
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .first { !$0.isEmpty }
    }
}

/// One agent placed in a lane, with its host.
struct LaneAgent: Identifiable, Hashable, Sendable {
    let agent: FleetAgent
    let host: String
    let hostIsLocal: Bool

    var id: String { "\(host):\(agent.id)" }
}

/// The lane rules, the same as the web Home's (`home-model.mjs`).
enum FleetLanes {
    /// How long an agent that went idle still counts as recently finished.
    static let finishedWindow: TimeInterval = 24 * 3600

    /// In progress: agents working or starting, newest first; one waiting
    /// on the person is in Your turn, never here.
    static func inProgress(_ fleet: FleetSnapshot) -> [LaneAgent] {
        fleet.ordered.flatMap { host in
            host.liveAgents
                .filter { $0.running && !$0.status.needsAttention && !$0.awaitingReply }
                .sorted(by: newestFirst)
                .map { LaneAgent(agent: $0, host: host.alias, hostIsLocal: host.local) }
        }
    }

    /// The agents half of Finished: idle agents that did something within
    /// the window, newest first. A clock a little ahead is tolerated.
    static func finished(_ fleet: FleetSnapshot, now: Date = Date(), window: TimeInterval = finishedWindow) -> [LaneAgent] {
        fleet.ordered.flatMap { host in
            host.liveAgents
                .filter { $0.state == "idle" && !$0.awaitingReply && ($0.lastResponse != nil || $0.lastPrompt != nil) }
                .filter { agent in
                    guard let at = agent.lastTurn else { return false }
                    return now.timeIntervalSince(at) <= window && at.timeIntervalSince(now) <= 60
                }
                .sorted(by: newestFirst)
                .map { LaneAgent(agent: $0, host: host.alias, hostIsLocal: host.local) }
        }
    }

    private static func newestFirst(_ left: FleetAgent, _ right: FleetAgent) -> Bool {
        (left.since ?? .distantPast) > (right.since ?? .distantPast)
    }
}
