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

    init(_ agents: [TendHostedAgent] = []) {
        for hosted in agents {
            if byID[hosted.id] == nil { byID[hosted.id] = hosted }
            let key = Self.key(host: hosted.host.alias, session: hosted.agent.agentSessionID)
            if bySession[key] == nil { bySession[key] = hosted }
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
        static let all: Changes = [.agents, .groups, .works]
    }

    /// What differs between `self` and the published `agents`, `groups` and
    /// `works`.
    func changes(fromAgents agents: [TendHostedAgent], groups: [TendWorkGroup], works: [FleetWork]) -> Changes {
        var changed: Changes = []
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
        lhs.source == rhs.source && lhs.title == rhs.title && lhs.repo == rhs.repo
            && lhs.lane == rhs.lane && lhs.stages == rhs.stages && lhs.headline == rhs.headline
            && lhs.since == rhs.since && lhs.handledLimit == rhs.handledLimit
            && lhs.revision == rhs.revision && lhs.lastChange == rhs.lastChange
            && lhs.isOrphanHelper == rhs.isOrphanHelper && lhs.isProbableHelper == rhs.isProbableHelper
            && lhs.isEmptySession == rhs.isEmptySession
            && same(lhs.focus, rhs.focus)
            && same(lhs.agents, rhs.agents, by: same) && same(lhs.helpers, rhs.helpers, by: same)
    }
}
