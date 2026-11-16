// Home's board as tendd computes it (`GET /api/device/home`): the Mac's
// lanes of Work units and decisions, grouped by project, with each unit's
// fold flags, plus the backlog. The daemon decides what is in which lane
// (home_board.rs, FleetBoard on the Mac); this file only reads it and
// applies the Mac's folds, as the web Home does (home-board.mjs,
// `fromServerBoard`). Decoding is lenient per item: one entry this app
// cannot read is counted, never fatal.
import Foundation

/// A text from the board: plain, or `{key, …}` for words the app owns.
indirect enum BoardText: Decodable, Hashable, Sendable {
    case plain(String)
    case keyed(String, [String: BoardText])

    private struct Key: CodingKey {
        let stringValue: String
        var intValue: Int? { nil }
        init(stringValue: String) { self.stringValue = stringValue }
        init?(intValue: Int) { nil }
    }

    init(from decoder: Decoder) throws {
        if let single = try? decoder.singleValueContainer() {
            if let text = try? single.decode(String.self) { self = .plain(text); return }
            if let number = try? single.decode(Int.self) { self = .plain(String(number)); return }
            if let flag = try? single.decode(Bool.self) { self = .plain(String(flag)); return }
        }
        let values = try decoder.container(keyedBy: Key.self)
        let key = try values.decode(String.self, forKey: Key(stringValue: "key"))
        var args: [String: BoardText] = [:]
        for name in values.allKeys where name.stringValue != "key" {
            if let arg = try? values.decode(BoardText.self, forKey: name) { args[name.stringValue] = arg }
        }
        self = .keyed(key, args)
    }

    /// The words, as the web Home and the Mac say them; "" for a key this
    /// app does not know (the caller falls back).
    var words: String {
        switch self {
        case .plain(let text): return text
        case .keyed(let key, let args):
            func arg(_ name: String) -> String { args[name]?.words ?? "" }
            switch key {
            case "status": return Self.status(arg("status"))
            case "background_task": return String(localized: "⟳ Background task")
            case "reply_ready": return String(localized: "Reply from \(arg("who")) is ready")
            case "kind_on_host": return String(localized: "\(arg("kind")) on \(arg("host"))")
            case "asks_to_choose": return String(localized: "Asks you to choose. Open to see the question.")
            case "waits_for_answer": return String(localized: "Waits for your answer. Open to see the question.")
            case "stage_is":
                return arg("status") == "failed"
                    ? String(localized: "Stage \(arg("stage")) is Error")
                    : String(localized: "Stage \(arg("stage")) is Blocked")
            case "waiting_for_agents": return String(localized: "Waiting for its agents to start")
            case "owner_says": return "\(arg("owner")): \(arg("text"))"
            case "waiting_on": return String(localized: "Waiting on \(arg("owner")): \(arg("text"))")
            default: return ""
            }
        }
    }

    private static func status(_ status: String) -> String {
        switch status {
        case "limited": String(localized: "Limited")
        case "asked": String(localized: "Asked you")
        case "working": String(localized: "Working")
        case "starting": String(localized: "Starting")
        case "idle": String(localized: "Idle")
        case "error": String(localized: "Error")
        case "waiting_input": String(localized: "Waiting for input")
        case "waiting_choice": String(localized: "Waiting for choice")
        case "stopped": String(localized: "Stopped")
        default: status
        }
    }

    /// The `status` inside a `{key: "status"}` text, for its colour.
    var statusName: String? {
        if case .keyed("status", let args) = self, case .plain(let name)? = args["status"] { return name }
        return nil
    }
}

/// An agent a unit names (no pane or socket: a capture names it by host
/// and session).
struct BoardAgent: Decodable, Hashable, Sendable, Identifiable {
    let host: String
    let local: Bool
    let sessionID: String?
    let kind: String?
    let name: String?
    let owner: String?
    let state: String?
    let status: BoardText?
    let needsAttention: Bool
    let since: Date?

    var id: String { "\(host):\(sessionID ?? name ?? "")" }

    enum CodingKeys: String, CodingKey {
        case host, local, kind, name, owner, state, status
        case sessionID = "session_id"
        case needsAttention = "needs_attention"
        case since = "state_entered_at"
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        func text(_ key: CodingKeys) -> String? {
            ((try? values.decodeIfPresent(String.self, forKey: key)) ?? nil).flatMap { $0.isEmpty ? nil : $0 }
        }
        host = text(.host) ?? ""
        local = (try? values.decodeIfPresent(Bool.self, forKey: .local)) ?? false
        sessionID = text(.sessionID)
        kind = text(.kind)
        name = text(.name)
        owner = text(.owner)
        state = text(.state)
        status = (try? values.decodeIfPresent(BoardText.self, forKey: .status)) ?? nil
        needsAttention = (try? values.decodeIfPresent(Bool.self, forKey: .needsAttention)) ?? false
        since = text(.since).flatMap(Timestamp.parse)
    }

    var agentStatus: AgentStatus { AgentStatus(state: status?.statusName == "asked" ? "waiting_choice" : (state ?? "")) }
    var label: String { owner ?? name ?? kind ?? String(localized: "Agent") }
}

/// One stage of a Work, as its stage bar draws it.
struct BoardStage: Decodable, Hashable, Sendable {
    enum State: String, Sendable { case needsYou = "needs_you", running, done, waiting }

    let name: String
    let owner: String?
    let kind: String?
    let state: State

    enum CodingKeys: String, CodingKey { case name, owner, kind, state }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        name = (try? values.decodeIfPresent(String.self, forKey: .name)) ?? ""
        owner = (try? values.decodeIfPresent(String.self, forKey: .owner)) ?? nil
        kind = (try? values.decodeIfPresent(String.self, forKey: .kind)) ?? nil
        state = ((try? values.decodeIfPresent(String.self, forKey: .state)) ?? nil).flatMap(State.init(rawValue:)) ?? .waiting
    }
}

/// A unit of work: one Work with its stages, or one agent on its own.
struct BoardUnit: Decodable, Hashable, Sendable, Identifiable {
    let id: String
    let lane: Lane
    let title: BoardText
    let headline: BoardText?
    let stages: [BoardStage]
    let repo: String?
    let hostAliases: [String]
    let agentKinds: [String]
    let agents: [BoardAgent]
    let helpers: [BoardAgent]
    /// Whom the person would talk to; nil when nobody is on the stage.
    let focus: BoardAgent?
    let since: Date?
    let shownTime: Date?
    let handledLimit: Bool
    let orphanHelper: Bool
    let probableHelper: Bool
    let emptySession: Bool
    let recent: Bool
    let reviewed: Bool
    let requestedByPerson: Bool

    enum CodingKeys: String, CodingKey {
        case id, lane, title, headline, stages, repo, agents, helpers, focus, since, recent, reviewed
        case hostAliases = "host_aliases"
        case agentKinds = "agent_kinds"
        case shownTime = "shown_time"
        case handledLimit = "handled_limit"
        case orphanHelper = "orphan_helper"
        case probableHelper = "probable_helper"
        case emptySession = "empty_session"
        case requestedByPerson = "requested_by_person"
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        func flag(_ key: CodingKeys, _ otherwise: Bool = false) -> Bool {
            (try? values.decodeIfPresent(Bool.self, forKey: key)) ?? nil ?? otherwise
        }
        func time(_ key: CodingKeys) -> Date? {
            ((try? values.decodeIfPresent(String.self, forKey: key)) ?? nil).flatMap(Timestamp.parse)
        }
        func refs(_ key: CodingKeys) -> [BoardAgent] {
            ((try? values.decodeIfPresent([TendLenient<BoardAgent>].self, forKey: key)) ?? nil)?.compactMap(\.value) ?? []
        }
        id = try values.decode(String.self, forKey: .id)
        lane = switch (try? values.decodeIfPresent(String.self, forKey: .lane)) ?? nil {
        case "needs_you": .yourTurn
        case "finished": .finished
        default: .inProgress
        }
        title = (try? values.decodeIfPresent(BoardText.self, forKey: .title)) ?? nil ?? .plain("")
        headline = (try? values.decodeIfPresent(BoardText.self, forKey: .headline)) ?? nil
        stages = ((try? values.decodeIfPresent([TendLenient<BoardStage>].self, forKey: .stages)) ?? nil)?.compactMap(\.value) ?? []
        repo = ((try? values.decodeIfPresent(String.self, forKey: .repo)) ?? nil).flatMap { $0.isEmpty ? nil : $0 }
        hostAliases = (try? values.decodeIfPresent([String].self, forKey: .hostAliases)) ?? nil ?? []
        agentKinds = (try? values.decodeIfPresent([String].self, forKey: .agentKinds)) ?? nil ?? []
        agents = refs(.agents)
        helpers = refs(.helpers)
        focus = (try? values.decodeIfPresent(BoardAgent.self, forKey: .focus)) ?? nil
        since = time(.since)
        shownTime = time(.shownTime)
        handledLimit = flag(.handledLimit)
        orphanHelper = flag(.orphanHelper)
        probableHelper = flag(.probableHelper)
        emptySession = flag(.emptySession)
        recent = flag(.recent, true)
        reviewed = flag(.reviewed)
        requestedByPerson = flag(.requestedByPerson)
    }

    /// The title in words; a key this app does not know falls back to the
    /// focus agent's name.
    var titleText: String {
        title.words.nonEmpty ?? focus?.label ?? String(localized: "Work")
    }

    var headlineText: String? { headline?.words.nonEmpty }

    var time: Date? { shownTime ?? since }

    /// The hosts it runs on, first the focus agent's.
    var hosts: [String] {
        var out: [String] = []
        for alias in [focus?.host].compactMap({ $0 }) + hostAliases + agents.map(\.host) where !alias.isEmpty && !out.contains(alias) {
            out.append(alias)
        }
        return out
    }

    /// The stage on now: the first not done (the Mac's `currentStageIndex`).
    var currentStage: Int? {
        stages.firstIndex { $0.state != .done } ?? (stages.isEmpty ? nil : stages.count - 1)
    }

    var doneStages: Int { stages.filter { $0.state == .done }.count }
}

/// A host the board covers.
struct BoardHost: Decodable, Hashable, Sendable {
    let alias: String
    let local: Bool
    let state: String

    enum CodingKeys: String, CodingKey { case alias, local, state }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        alias = try values.decode(String.self, forKey: .alias)
        local = (try? values.decodeIfPresent(Bool.self, forKey: .local)) ?? nil ?? false
        state = (try? values.decodeIfPresent(String.self, forKey: .state)) ?? nil ?? "unknown"
    }
}

/// What one host's decision queue covers and could not read: an empty
/// Your turn is all clear only when every host's queue was read in full.
struct BoardDecisionHost: Decodable, Hashable, Sendable {
    let alias: String
    let local: Bool
    let control: Bool
    let truncated: Bool
    let unavailable: [String]

    enum CodingKeys: String, CodingKey { case alias, local, control, truncated, unavailable }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        alias = try values.decode(String.self, forKey: .alias)
        local = (try? values.decodeIfPresent(Bool.self, forKey: .local)) ?? nil ?? false
        control = (try? values.decodeIfPresent(Bool.self, forKey: .control)) ?? nil ?? local
        truncated = (try? values.decodeIfPresent(Bool.self, forKey: .truncated)) ?? nil ?? false
        unavailable = (try? values.decodeIfPresent([String].self, forKey: .unavailable)) ?? nil ?? []
    }
}

/// Planned Work, not started (`work_backlog`): shown, not started from
/// here (a device starts new work only).
struct BacklogRecord: Decodable, Hashable, Sendable, Identifiable {
    let workspace: String?
    let work: String
    let title: String?
    let line: String?
    let priority: Int?
    let pipeline: String?
    let project: String?

    var id: String { "\(workspace ?? "")/\(work)" }
    var titleText: String { title?.nonEmpty ?? work }
}

/// One item in a lane.
enum BoardItem: Hashable, Sendable, Identifiable {
    case unit(BoardUnit)
    case decision(PendingDecision)

    var id: String {
        switch self {
        case .unit(let unit): "u:" + unit.id
        case .decision(let decision): "d:" + decision.id
        }
    }
}

struct BoardGroup: Hashable, Sendable, Identifiable {
    let project: String?
    let system: Bool
    let items: [BoardItem]

    var id: String { system ? "\u{0}system" : project ?? "" }
}

/// `GET /api/device/home`.
struct DeviceHome: Decodable, Hashable, Sendable {
    let hosts: [BoardHost]
    let decisionHosts: [BoardDecisionHost]
    /// Each lane's groups, in the daemon's order.
    let lanes: [Lane: [BoardGroup]]
    let backlog: [BacklogRecord]
    /// Board entries this app could not read, per lane.
    let undecoded: [Lane: Int]
    /// Whether every host's queue coverage was read: false when the board
    /// sent none, or a record this app could not read.
    let coverageKnown: Bool

    enum CodingKeys: String, CodingKey {
        case hosts, lanes, backlog
        case decisionHosts = "decision_hosts"
    }

    private struct RawLane: Decodable { let groups: [RawGroup]? }
    private struct RawGroup: Decodable {
        let project: String?
        let system: Bool?
        let items: [RawItem]?
    }

    /// A unit, or a decision exactly as the queue lists it plus its host
    /// and whether it can be decided from here.
    private struct RawItem: Decodable {
        let item: Parsed?

        enum Parsed {
            case unit(BoardUnit)
            case decision(TendPendingItem, host: String, actionable: Bool)
        }

        private enum Keys: String, CodingKey { case kind, host, actionable }

        init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: Keys.self)
            let kind = (try? values.decodeIfPresent(String.self, forKey: .kind)) ?? nil
            if kind == "unit" {
                item = (try? BoardUnit(from: decoder)).map(Parsed.unit)
            } else if let entry = try? TendPendingItem(from: decoder),
                      let host = (try? values.decodeIfPresent(String.self, forKey: .host)) ?? nil {
                let actionable = (try? values.decodeIfPresent(Bool.self, forKey: .actionable)) ?? nil ?? false
                item = .decision(entry, host: host, actionable: actionable)
            } else {
                item = nil
            }
        }
    }

    private struct RawLanes: Decodable {
        // Your turn is required: without it nothing can be said about
        // what needs the person.
        let needsYou: RawLane
        let running: RawLane?
        let finished: RawLane?
        enum CodingKeys: String, CodingKey { case running, finished, needsYou = "needs_you" }
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        hosts = ((try? values.decodeIfPresent([TendLenient<BoardHost>].self, forKey: .hosts)) ?? nil)?.compactMap(\.value) ?? []
        let coverage = (try? values.decodeIfPresent([TendLenient<BoardDecisionHost>].self, forKey: .decisionHosts)) ?? nil
        let readHosts = coverage?.compactMap(\.value) ?? []
        decisionHosts = readHosts
        coverageKnown = coverage.map { $0.count == readHosts.count } ?? false
        backlog = ((try? values.decodeIfPresent([TendLenient<BacklogRecord>].self, forKey: .backlog)) ?? nil)?.compactMap(\.value) ?? []
        // The lanes are the board: without them this is not a board.
        let raw = try values.decode(RawLanes.self, forKey: .lanes)
        let local = Dictionary(decisionHosts.map { ($0.alias, $0.local) }, uniquingKeysWith: { first, _ in first })
            .merging(hosts.map { ($0.alias, $0.local) }, uniquingKeysWith: { first, _ in first })
        var lanes: [Lane: [BoardGroup]] = [:]
        var undecoded: [Lane: Int] = [:]
        for (lane, rawLane) in [(Lane.yourTurn, raw.needsYou), (.inProgress, raw.running), (.finished, raw.finished)] {
            var groups: [BoardGroup] = []
            for group in rawLane?.groups ?? [] {
                var items: [BoardItem] = []
                for entry in group.items ?? [] {
                    switch entry.item {
                    case .unit(let unit):
                        items.append(.unit(unit))
                    case .decision(let pending, let host, let actionable):
                        items.append(.decision(PendingDecision(pending, host: host, hostIsLocal: local[host] ?? false, decidable: actionable)))
                    case nil:
                        undecoded[lane, default: 0] += 1
                    }
                }
                groups.append(BoardGroup(project: group.project, system: group.system ?? false, items: items))
            }
            lanes[lane] = groups
        }
        self.lanes = lanes
        self.undecoded = undecoded
    }

    /// Every item in a lane, in order.
    func items(_ lane: Lane) -> [BoardItem] { (lanes[lane] ?? []).flatMap(\.items) }

    var decisions: [PendingDecision] {
        items(.yourTurn).compactMap { if case .decision(let d) = $0 { d } else { nil } }
    }

    /// "Nothing needs you" only when every host's queue was read in full,
    /// every entry decoded, and none is waiting.
    var allClear: Bool {
        coverageKnown && items(.yourTurn).isEmpty && (undecoded[.yourTurn] ?? 0) == 0
            && decisionHosts.allSatisfy { !$0.truncated && $0.unavailable.isEmpty }
    }
}

/// A lane after the Mac's folds (outside Your turn): possible helpers,
/// empty sessions and older units each behind their own button, reviewed
/// results behind "Show N reviewed".
struct FoldedLane: Sendable {
    var groups: [BoardGroup] = []
    var helpers: [BoardUnit] = []
    var empty: [BoardUnit] = []
    var older: [BoardUnit] = []
    var reviewed: [BoardUnit] = []
    /// What the lane counts: everything but the reviewed.
    var count = 0

    init(_ board: DeviceHome, lane: Lane) {
        for group in board.lanes[lane] ?? [] {
            var shown: [BoardItem] = []
            for item in group.items {
                guard lane != .yourTurn, case .unit(let unit) = item else {
                    shown.append(item)
                    continue
                }
                if unit.reviewed { reviewed.append(unit) }
                else if unit.probableHelper { helpers.append(unit) }
                else if unit.emptySession { empty.append(unit) }
                else if !unit.recent { older.append(unit) }
                else { shown.append(item) }
            }
            if !shown.isEmpty { groups.append(BoardGroup(project: group.project, system: group.system, items: shown)) }
        }
        count = board.items(lane).count - reviewed.count
    }
}
