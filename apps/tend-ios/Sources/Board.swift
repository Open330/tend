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
    /// Set aside with Looks Good; also set here for a mark this device
    /// made that the board has not confirmed yet (`DeviceHome.holding`).
    var reviewed: Bool
    let requestedByPerson: Bool
    /// The result on screen, for Looks Good (finished units only): tendd
    /// refuses a mark whose key no longer names the unit's result.
    let reviewKey: String?

    enum CodingKeys: String, CodingKey {
        case id, lane, title, headline, stages, repo, agents, helpers, focus, since, recent, reviewed
        case reviewKey = "review_key"
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
        reviewKey = ((try? values.decodeIfPresent(String.self, forKey: .reviewKey)) ?? nil).flatMap { $0.isEmpty ? nil : $0 }
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
    /// `subject_check_v1`: questions nobody can answer can be kept here,
    /// closed while `close_unanswerable` is on, and how many there are,
    /// exact even when the list was cut.
    var subjectCheck = false
    var closeUnanswerable = false
    var unanswerableCount = 0

    enum CodingKeys: String, CodingKey {
        case alias, local, control, truncated, unavailable
        case subjectCheck = "subject_check"
        case closeUnanswerable = "close_unanswerable"
        case unanswerableCount = "unanswerable_count"
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        alias = try values.decode(String.self, forKey: .alias)
        local = (try? values.decodeIfPresent(Bool.self, forKey: .local)) ?? nil ?? false
        control = (try? values.decodeIfPresent(Bool.self, forKey: .control)) ?? nil ?? local
        truncated = (try? values.decodeIfPresent(Bool.self, forKey: .truncated)) ?? nil ?? false
        unavailable = (try? values.decodeIfPresent([String].self, forKey: .unavailable)) ?? nil ?? []
        subjectCheck = (try? values.decodeIfPresent(Bool.self, forKey: .subjectCheck)) ?? nil ?? false
        closeUnanswerable = (try? values.decodeIfPresent(Bool.self, forKey: .closeUnanswerable)) ?? nil ?? false
        unanswerableCount = (try? values.decodeIfPresent(Int.self, forKey: .unanswerableCount)) ?? nil ?? 0
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

/// Home's numbers as tendd counts them (`counts`, apps/shared/Design/
/// COUNTS.md): shown as given. Read field by field, so a field this app
/// cannot read falls back to its own count of what is listed.
struct BoardCounts: Decodable, Hashable, Sendable {
    let yourTurn: Int?
    let actionable: Int?
    let cantAnswer: Int?
    let inProgress: Int?
    var finished: Int?
    var reviewed: Int?
    /// `complete` or `partial`, about the decision queues only.
    let coverage: String?

    enum CodingKeys: String, CodingKey {
        case actionable, finished, reviewed, coverage
        case yourTurn = "your_turn"
        case cantAnswer = "cant_answer"
        case inProgress = "in_progress"
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        func number(_ key: CodingKeys) -> Int? {
            ((try? values.decodeIfPresent(Int.self, forKey: key)) ?? nil).flatMap { $0 >= 0 ? $0 : nil }
        }
        yourTurn = number(.yourTurn)
        actionable = number(.actionable)
        cantAnswer = number(.cantAnswer)
        inProgress = number(.inProgress)
        finished = number(.finished)
        reviewed = number(.reviewed)
        coverage = (try? values.decodeIfPresent(String.self, forKey: .coverage)) ?? nil
    }
}

/// `GET /api/device/home`.
struct DeviceHome: Decodable, Hashable, Sendable {
    let hosts: [BoardHost]
    let decisionHosts: [BoardDecisionHost]
    /// Each lane's groups, in the daemon's order.
    private(set) var lanes: [Lane: [BoardGroup]]
    let backlog: [BacklogRecord]
    /// Board entries this app could not read, per lane.
    let undecoded: [Lane: Int]
    /// Whether every host's queue coverage was read: false when the board
    /// sent none, or a record this app could not read.
    let coverageKnown: Bool
    /// tendd's own counts; nil from an older tendd, which leaves the
    /// counting to this app by the same rules.
    private(set) var counts: BoardCounts?
    /// Whether Looks Good marks were read: only then is `reviewed` told.
    let reviewedMarks: Bool

    enum CodingKeys: String, CodingKey {
        case hosts, lanes, backlog, counts
        case decisionHosts = "decision_hosts"
        case reviewedMarks = "reviewed_marks"
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
        // No host's queue at all says nothing about what is waiting.
        coverageKnown = coverage.map { !$0.isEmpty && $0.count == readHosts.count } ?? false
        backlog = ((try? values.decodeIfPresent([TendLenient<BacklogRecord>].self, forKey: .backlog)) ?? nil)?.compactMap(\.value) ?? []
        counts = (try? values.decodeIfPresent(BoardCounts.self, forKey: .counts)) ?? nil
        reviewedMarks = (try? values.decodeIfPresent(Bool.self, forKey: .reviewedMarks)) ?? nil ?? false
        // The lanes are the board: without them this is not a board.
        let raw = try values.decode(RawLanes.self, forKey: .lanes)
        let local = Dictionary(decisionHosts.map { ($0.alias, $0.local) }, uniquingKeysWith: { first, _ in first })
            .merging(hosts.map { ($0.alias, $0.local) }, uniquingKeysWith: { first, _ in first })
        var lanes: [Lane: [BoardGroup]] = [:]
        var undecoded: [Lane: Int] = [:]
        var cantAnswer: [PendingDecision] = []
        for (lane, rawLane) in [(Lane.yourTurn, raw.needsYou), (.inProgress, raw.running), (.finished, raw.finished)] {
            var groups: [BoardGroup] = []
            for group in rawLane?.groups ?? [] {
                var items: [BoardItem] = []
                for entry in group.items ?? [] {
                    switch entry.item {
                    case .unit(let unit):
                        items.append(.unit(unit))
                    case .decision(let pending, let host, let actionable):
                        let decision = PendingDecision(pending, host: host, hostIsLocal: local[host] ?? false, decidable: actionable)
                        // Questions nobody can answer leave Your turn for
                        // their own group.
                        if lane == .yourTurn, decision.unanswerable != nil {
                            cantAnswer.append(decision)
                        } else {
                            items.append(.decision(decision))
                        }
                    case nil:
                        undecoded[lane, default: 0] += 1
                    }
                }
                if !items.isEmpty || group.items?.isEmpty != false {
                    groups.append(BoardGroup(project: group.project, system: group.system ?? false, items: items))
                }
            }
            lanes[lane] = groups
        }
        self.lanes = lanes
        self.undecoded = undecoded
        self.cantAnswer = cantAnswer
    }

    /// Your turn's Can't be answered group, in the queue's order.
    let cantAnswer: [PendingDecision]

    /// Its count: tendd's, or every host's exact count, never less than
    /// listed.
    var cantAnswerCount: Int {
        counts?.cantAnswer ?? Self.cantAnswerFloor(
            listed: cantAnswer.map(\.host),
            exact: decisionHosts.map { ($0.alias, $0.unanswerableCount) }
        )
    }

    /// Per host, its exact count but never less than it lists; summed. A
    /// host's exact count speaks only for that host.
    static func cantAnswerFloor(listed: [String], exact: [(String, Int)]) -> Int {
        var listedBy: [String: Int] = [:]
        for host in listed { listedBy[host, default: 0] += 1 }
        var exactBy: [String: Int] = [:]
        for (host, count) in exact { exactBy[host] = max(exactBy[host] ?? 0, count) }
        return Set(listedBy.keys).union(exactBy.keys).reduce(0) { sum, host in
            sum + max(listedBy[host] ?? 0, exactBy[host] ?? 0)
        }
    }

    /// A lane's number (COUNTS.md): tendd's `counts` as given; without
    /// them, what the lane lists, folded units included and reviewed ones
    /// left out. Decided history is never counted.
    func count(_ lane: Lane) -> Int {
        switch lane {
        case .backlog: backlog.count
        case .yourTurn: counts?.yourTurn ?? listedCount(.yourTurn)
        case .inProgress: counts?.inProgress ?? listedCount(.inProgress)
        case .finished: counts?.finished ?? listedCount(.finished)
        }
    }

    /// What a lane lists, the folded units included, the reviewed left out
    /// (Your turn never folds). Can't be answered is never in it.
    func listedCount(_ lane: Lane) -> Int {
        items(lane).filter { item in
            if lane != .yourTurn, case .unit(let unit) = item { return !unit.reviewed }
            return true
        }.count
    }

    /// "Show N reviewed": tendd's count when it read the Looks Good marks,
    /// otherwise the reviewed units listed.
    func reviewedCount(_ lane: Lane) -> Int {
        let listed = items(lane).filter { if case .unit(let unit) = $0 { unit.reviewed } else { false } }.count
        guard lane == .finished, reviewedMarks, let told = counts?.reviewed else { return listed }
        return told
    }

    /// Whether more may need the person than Your turn's number says:
    /// partial when either tendd (`coverage`) or this app says so. This
    /// app says so for a queue not read whole or an entry it could not
    /// read, whatever tendd's coverage is.
    var yourTurnMayBeMore: Bool {
        let client = !coverageKnown || (undecoded[.yourTurn] ?? 0) > 0
            || decisionHosts.contains { $0.truncated || !$0.unavailable.isEmpty }
        return serverPartial || client
    }

    /// tendd said its decision queues were not all read whole.
    var serverPartial: Bool { counts?.coverage.map { $0 != "complete" } ?? false }

    /// Whether this board still shows a unit in Finished, not set aside,
    /// with this review key: a mark made here for that result is not
    /// confirmed yet. Reviewed, another key or the unit gone ends it.
    func awaitsMark(id: String, key: String) -> Bool {
        items(.finished).contains { item in
            if case .unit(let unit) = item { return unit.id == id && unit.reviewKey == key && !unit.reviewed }
            return false
        }
    }

    /// The board with this device's confirmed Looks Good marks (unit id
    /// to review key) held on it until the board itself says so: each
    /// such unit reads as reviewed, and tendd's counts move it from
    /// Finished to reviewed.
    func holding(_ marks: [String: String]) -> DeviceHome {
        guard !marks.isEmpty else { return self }
        var copy = self
        var moved = 0
        copy.lanes[.finished] = (lanes[.finished] ?? []).map { group in
            BoardGroup(project: group.project, system: group.system, items: group.items.map { item in
                guard case .unit(var unit) = item, !unit.reviewed, let key = unit.reviewKey, marks[unit.id] == key else { return item }
                unit.reviewed = true
                moved += 1
                return .unit(unit)
            })
        }
        if moved > 0, var counts = copy.counts {
            counts.finished = counts.finished.map { max(0, $0 - moved) }
            counts.reviewed = counts.reviewed.map { $0 + moved }
            copy.counts = counts
        }
        return copy
    }

    /// Every item in a lane, in order.
    func items(_ lane: Lane) -> [BoardItem] { (lanes[lane] ?? []).flatMap(\.items) }

    var decisions: [PendingDecision] {
        items(.yourTurn).compactMap { if case .decision(let d) = $0 { d } else { nil } }
    }

    /// "Nothing needs you" only when nothing is listed, Your turn's
    /// number is 0, and coverage is complete by tendd's word and this
    /// app's own reading: every host's queue read in full, every entry
    /// decoded.
    var allClear: Bool {
        items(.yourTurn).isEmpty && count(.yourTurn) == 0 && !yourTurnMayBeMore
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
    /// What the lane lists: everything but the reviewed, folded units
    /// included (`DeviceHome.listedCount`).
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
        count = board.listedCount(lane)
    }
}
