import Foundation

/// At most one projection is built at a time. A burst replaces the pending
/// input, and an outdated result is never published. All callers await the
/// latest result, including requests that arrived during the current build.
@MainActor
final class TendCoalescedProjection<Value: Sendable> {
    private struct Request {
        let build: @MainActor () async -> Value
        let publish: @MainActor (Value) -> Void
    }
    private var pending: Request?
    private var task: Task<Void, Never>?

    func refresh(build: @escaping @MainActor () async -> Value,
                 publish: @escaping @MainActor (Value) -> Void) async {
        pending = Request(build: build, publish: publish)
        if let task { await task.value; return }
        let worker = Task { @MainActor [self] in
            while let request = pending {
                pending = nil
                let value = await request.build()
                if pending == nil { request.publish(value) }
            }
            task = nil
        }
        task = worker
        await worker.value
    }
}

/// The agents Home and the tabs look up on every redraw, by id and by the
/// host and session that asked a decision.
struct TendHostedAgentIndex: Sendable {
    private(set) var byID: [String: TendHostedAgent] = [:]
    private(set) var bySession: [String: TendHostedAgent] = [:]
    /// What a decision takes from its asking agent (id and checkout), by
    /// host and session: the decisions change only when this does.
    private(set) var askers: [String: Asker] = [:]
    /// The hosts represented in this index (at least one agent listed).
    /// Only on these can a session be reported as not listed; absence is
    /// an observation, not proof that it exited.
    private(set) var hosts: Set<String> = []

    struct Asker: Hashable, Sendable {
        let id: String
        let cwd: String?
    }

    init(_ agents: [TendHostedAgent] = []) {
        for hosted in agents {
            hosts.insert(hosted.host.alias)
            if byID[hosted.id] == nil { byID[hosted.id] = hosted }
            let key = Self.key(host: hosted.host.alias, session: hosted.agent.agentSessionID)
            if bySession[key] == nil {
                bySession[key] = hosted
                askers[key] = Asker(id: hosted.id, cwd: hosted.agent.cwd)
            }
        }
    }

    static func key(host: String, session: String) -> String { "\(host)\u{0}\(session)" }

    func agent(host: String, session: String) -> TendHostedAgent? {
        bySession[Self.key(host: host, session: session)]
    }
}

struct TendWorkspaceProjection: Sendable {
    let agents: [TendHostedAgent]
    var index = TendHostedAgentIndex()
    let groups: [TendWorkGroup]
    let works: [FleetWork]
    let builtAt: Date
    let connection: UInt64
    /// Which parts differ from what was published when the build started;
    /// only those are published, so an unchanged rebuild (the periodic one,
    /// or a snapshot whose change projects to nothing) redraws nothing.
    var changed = Changes.all

    struct Changes: OptionSet, Sendable {
        let rawValue: UInt8
        static let agents = Changes(rawValue: 1 << 0)
        static let groups = Changes(rawValue: 1 << 1)
        static let works = Changes(rawValue: 1 << 2)
        /// Home's units changed in a way Home shows.
        static let board = Changes(rawValue: 1 << 3)
        /// An asking agent's id or checkout changed.
        static let askers = Changes(rawValue: 1 << 4)
        static let all: Changes = [.agents, .groups, .works, .board, .askers]
    }

    /// What differs between `self` and the published `agents`, `groups` and
    /// `works`.
    func changes(
        fromAgents agents: [TendHostedAgent], groups: [TendWorkGroup], works: [FleetWork],
        board: [FleetWork]? = nil, askers: [String: TendHostedAgentIndex.Asker]? = nil
    ) -> Changes {
        var changed: Changes = []
        if let board, !TendProjectionEquality.same(self.works, board, by: TendProjectionEquality.sameOnHome) {
            changed.insert(.board)
        }
        if let askers, index.askers != askers { changed.insert(.askers) }
        if !TendProjectionEquality.same(self.agents, agents, by: TendProjectionEquality.same) { changed.insert(.agents) }
        if !TendProjectionEquality.same(self.groups, groups, by: TendProjectionEquality.same) { changed.insert(.groups) }
        if !TendProjectionEquality.same(self.works, works, by: TendProjectionEquality.same) { changed.insert(.works) }
        return changed
    }
}

/// Content equality for the projected lists, used only off the main thread
/// to decide what to publish. The types are deliberately not `Equatable`:
/// SwiftUI compares an `Equatable` view input with `==` on every update,
/// and a deep compare of every card's agents (long responses included)
/// cost more than it saved.
enum TendProjectionEquality {
    static func same<Element>(_ lhs: [Element], _ rhs: [Element], by equal: (Element, Element) -> Bool) -> Bool {
        lhs.count == rhs.count && zip(lhs, rhs).allSatisfy(equal)
    }

    static func same(_ lhs: TendHostedAgent, _ rhs: TendHostedAgent) -> Bool {
        lhs.host == rhs.host && lhs.pane == rhs.pane && lhs.agent == rhs.agent
    }

    static func same(_ lhs: TendHostedAgent?, _ rhs: TendHostedAgent?) -> Bool {
        switch (lhs, rhs) {
        case (nil, nil): true
        case let (lhs?, rhs?): same(lhs, rhs)
        default: false
        }
    }

    static func same(_ lhs: TendWorkGroup, _ rhs: TendWorkGroup) -> Bool {
        lhs.identity == rhs.identity && lhs.pipelineRun == rhs.pipelineRun
            && same(lhs.participants, rhs.participants, by: same)
    }

    /// Every stored property of `FleetWork`; a test counts them, so a new
    /// one cannot be left out unnoticed.
    static let fleetWorkFields = 16

    static func same(_ lhs: FleetWork, _ rhs: FleetWork) -> Bool {
        same(lhs, rhs, agents: same)
    }

    /// The same, as far as Home shows: an agent's `lastActivityAt` is only
    /// a fallback for `stateEnteredAt` there, and its workload is not shown,
    /// so a heartbeat that moves only those does not redraw the board.
    static func sameOnHome(_ lhs: FleetWork, _ rhs: FleetWork) -> Bool {
        same(lhs, rhs, agents: sameOnHome)
    }

    static func sameOnHome(_ lhs: TendHostedAgent, _ rhs: TendHostedAgent) -> Bool {
        guard lhs.host == rhs.host, lhs.pane == rhs.pane else { return false }
        return lhs.agent == rhs.agent || lhs.agent.shownOnHome == rhs.agent.shownOnHome
    }

    private static func same(
        _ lhs: FleetWork, _ rhs: FleetWork, agents same: (TendHostedAgent, TendHostedAgent) -> Bool
    ) -> Bool {
        func sameFocus(_ lhs: TendHostedAgent?, _ rhs: TendHostedAgent?) -> Bool {
            switch (lhs, rhs) {
            case (nil, nil): true
            case let (lhs?, rhs?): same(lhs, rhs)
            default: false
            }
        }
        return lhs.source == rhs.source && lhs.title == rhs.title && lhs.repo == rhs.repo
            && lhs.lane == rhs.lane && lhs.stages == rhs.stages && lhs.headline == rhs.headline
            && lhs.since == rhs.since && lhs.handledLimit == rhs.handledLimit
            && lhs.revision == rhs.revision && lhs.lastChange == rhs.lastChange && lhs.lastTurn == rhs.lastTurn
            && lhs.isOrphanHelper == rhs.isOrphanHelper && lhs.isProbableHelper == rhs.isProbableHelper
            && lhs.isEmptySession == rhs.isEmptySession
            && sameFocus(lhs.focus, rhs.focus)
            && Self.same(lhs.agents, rhs.agents, by: same) && Self.same(lhs.helpers, rhs.helpers, by: same)
    }
}

extension TendAgent {
    /// This agent with what Home never shows cleared: the workload, and
    /// `lastActivityAt` when `stateEnteredAt` (which Home reads first) is
    /// set. Every field is passed, so a new one cannot be forgotten here.
    var shownOnHome: TendAgent {
        TendAgent(
            kind: kind, agentSessionID: agentSessionID, pane: pane, tmuxSocket: tmuxSocket,
            tmuxSession: tmuxSession, cwd: cwd, state: state, lastPrompt: lastPrompt,
            lastPromptOrigin: lastPromptOrigin, lastPromptAt: lastPromptAt, lastResponse: lastResponse, lastResponseAt: lastResponseAt,
            recap: recap, aiTitle: aiTitle, lastNotification: lastNotification, model: model,
            contextUsedPercent: contextUsedPercent, costUSD: costUSD,
            rateLimit5hPercent: rateLimit5hPercent, rateLimit5hResetsAt: rateLimit5hResetsAt,
            rateLimit7dPercent: rateLimit7dPercent, rateLimit7dResetsAt: rateLimit7dResetsAt,
            rateLimitedUntil: rateLimitedUntil, rateLimitScope: rateLimitScope, rateLimitSource: rateLimitSource,
            startedAt: startedAt, lastActivityAt: stateEnteredAt == nil ? lastActivityAt : nil,
            stateEnteredAt: stateEnteredAt, workload: nil, subagents: subagents, awaitingReply: awaitingReply
        )
    }
}
