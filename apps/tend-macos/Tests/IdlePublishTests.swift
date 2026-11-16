import Combine
import Foundation
import Testing
@testable import Tend

// What runs on a timer or on every fleet event publishes only what changed:
// an unchanged poll or projection must not redraw Home.

private final class PublishCounter {
    private(set) var count = 0
    private var subscription: AnyCancellable?

    init<Object: ObservableObject>(_ object: Object) where Object.ObjectWillChangePublisher == ObservableObjectPublisher {
        subscription = object.objectWillChange.sink { [weak self] _ in self?.count += 1 }
    }
}

private func daemon(
    capabilities: [String],
    respond: @escaping @Sendable (String) throws -> [String: Any]
) -> TendIPCClient {
    TendIPCClient(socketPath: "/tmp/tend-idle-\(UUID().uuidString).sock") { _, payload in
        let object = try #require(JSONSerialization.jsonObject(with: payload) as? [String: Any])
        let kind = object["kind"] as? String ?? ""
        if kind == "hello" {
            return try JSONSerialization.data(withJSONObject: [
                "ok": true, "min_protocol": 1, "max_protocol": 6,
                "capabilities": ["session_bytes_v1", "session_attachment_identity_v1", "work_control_v1"] + capabilities,
            ])
        }
        return try JSONSerialization.data(withJSONObject: respond(kind))
    }
}

private func partialQueue() -> [String: Any] {
    ["items": [] as [Any], "truncated": false, "unavailable": ["review items: locked"]]
}

@Suite @MainActor struct IdlePublishTests {
    @Test func anUnchangedDecisionPollPublishesNothing() async throws {
        let client = daemon(capabilities: ["decision_queue_v1"]) { kind in
            kind == "decision_queue" ? ["ok": true, "decision_queue": partialQueue()] : ["ok": true]
        }
        try await client.hello()
        let feed = FleetDecisionFeed()
        let counter = PublishCounter(feed)
        await feed.load(client)
        #expect(feed.problem != nil)
        let first = counter.count
        #expect(first > 0)
        await feed.load(client)
        await feed.load(client)
        #expect(counter.count == first, "the same queue and the same problem publish nothing")
    }

    @Test func aDecisionPollThatKeepsFailingPublishesTheProblemOnce() async throws {
        let client = daemon(capabilities: ["decision_queue_v1"]) { _ in
            ["ok": false, "error": "locked"]
        }
        try await client.hello()
        let feed = FleetDecisionFeed()
        let counter = PublishCounter(feed)
        await feed.load(client)
        #expect(feed.problem != nil)
        let first = counter.count
        await feed.load(client)
        #expect(counter.count == first)
    }

    @Test func anUnchangedBacklogPollPublishesNothing() async throws {
        let client = daemon(capabilities: ["work_backlog_v1"]) { kind in
            guard kind == "work_backlog" else { return ["ok": true] }
            return ["ok": true, "work_records": [[
                "identity": ["workspace_id": "tend", "work_id": "export"],
                "metadata": ["title": "CSV export", "stage": "queued", "updated_at": "2026-11-12T01:00:00Z"],
                "created_at": "2026-11-12T01:00:00Z",
            ]]]
        }
        try await client.hello()
        let feed = FleetBacklogFeed()
        let counter = PublishCounter(feed)
        await feed.load(client)
        #expect(feed.records?.count == 1)
        let first = counter.count
        await feed.load(client)
        #expect(counter.count == first)
    }

    @Test func typingAnAnswerRedrawsOnlyItsOwnCard() {
        let feed = FleetDecisionFeed()
        let a = feed.drafts.row(for: "a"), b = feed.drafts.row(for: "b")
        #expect(feed.drafts.row(for: "a") === a, "a card keeps its row across redraws")
        let feedCounter = PublishCounter(feed), aCounter = PublishCounter(a), bCounter = PublishCounter(b)
        a.update("")
        a.update("M")
        a.update("Me")
        #expect(a.value == "Me")
        #expect(feedCounter.count == 0, "the feed, which Home watches, does not publish a keystroke")
        #expect(bCounter.count == 0)
        let typed = aCounter.count
        a.update("Me")
        #expect(aCounter.count == typed, "the same text publishes nothing")
        b.update("")
        #expect(a.value == nil && b.value == "")
        #expect(feedCounter.count == 0)
    }

    @Test func pruningKeepsTabsWithoutPublishingOrPersisting() {
        let defaults = UserDefaults(suiteName: "tend.idle.tabs.\(UUID().uuidString)")!
        let tabs = TendWorkbenchTabs(initial: .workBoard, persistenceKey: "tabs", defaults: defaults)
        tabs.openPinned(.inbox)
        tabs.openPinned(.agent("a"))
        let counter = PublishCounter(tabs)
        let stored = defaults.data(forKey: "tabs")
        tabs.prune { _ in true }
        #expect(counter.count == 0, "nothing went away")
        #expect(defaults.data(forKey: "tabs") == stored)
        tabs.prune { $0 != .agent("a") }
        #expect(counter.count == 1, "one publish for the whole prune")
        #expect(!tabs.groups.flatMap(\.tabs).contains(.agent("a")))
        #expect(tabs.groups.flatMap(\.tabs).contains(.inbox))
    }

    @Test func aProjectionNamesOnlyThePartsThatChanged() throws {
        let host = TendFleetHostIdentity(alias: "mac", local: true, state: "online", mode: "local")
        let agent = try JSONDecoder().decode(TendAgent.self, from: Data("""
        {"kind":"claude","agent_session_id":"s-1","state":"working","last_activity_at":"2026-11-12T01:00:00Z"}
        """.utf8))
        let hosted = TendHostedAgent(host: host, agent: agent, pane: nil)
        let projection = TendWorkspaceProjection(agents: [hosted], groups: [], works: [], builtAt: .now, connection: 1)
        #expect(projection.changes(fromAgents: [hosted], groups: [], works: []).isEmpty)
        #expect(projection.changes(fromAgents: [], groups: [], works: []) == .agents)

        let works = FleetBoard.works(groups: [], agents: [hosted], now: .now)
        let work = try #require(works.first)
        #expect(TendProjectionEquality.same(work, work))
        var moved = work
        moved.headline += "!"
        #expect(!TendProjectionEquality.same(work, moved))
        #expect(
            Mirror(reflecting: work).children.count == TendProjectionEquality.fleetWorkFields,
            "a new FleetWork property must be compared in TendProjectionEquality.same"
        )
    }

    @Test func aWorkloadsProcessPreviewIsNotANewSnapshot() throws {
        func workload(_ pid: Int) throws -> TendAgentWorkload {
            try JSONDecoder().decode(TendAgentWorkload.self, from: Data("""
            {"process_count":2,"shell_count":1,"preview":[{"pid":\(pid),"parent_pid":1,"depth":1,"kind":"tool","command":"rg"}]}
            """.utf8))
        }
        #expect(try workload(10) == workload(11), "no view shows the preview")
        let fewer = try JSONDecoder().decode(TendAgentWorkload.self, from: Data(#"{"process_count":1,"shell_count":1}"#.utf8))
        #expect(try workload(10) != fewer, "a count is shown, so it is a change")
    }
}
