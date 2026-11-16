import Foundation
import Testing
@testable import Tend

// How Home orders its lanes and the cards in each.

private let host = TendFleetHostIdentity(alias: "mini", local: false, state: "online", mode: "control")

private func date(_ text: String) -> Date {
    TendUsageTimestamp.parse(text)!
}

private func unit(_ id: String, _ lane: FleetLane, since: String?, lastChange: String? = nil, repo: String? = nil) -> FleetWork {
    FleetWork(
        source: .agent(id),
        title: id,
        repo: repo,
        lane: lane,
        stages: [],
        headline: "",
        since: since.map(date),
        agents: [],
        focus: nil,
        handledLimit: false,
        lastChange: lastChange.map(date)
    )
}

private func question(_ id: String, at createdAt: String, system: Bool = false) throws -> FleetDecision {
    var object: [String: Any] = [
        "request_id": id, "body": "Which way?", "from_session_id": "s-\(id)", "from_pane": "%1",
        "created_at": createdAt,
    ]
    if system { object["system"] = true }
    let entry = try JSONDecoder().decode(TendHumanRequestEntry.self, from: JSONSerialization.data(withJSONObject: object))
    return FleetDecision(humanRequest: entry, host: host)
}

@Test func lanesAreAlwaysYourTurnThenInProgressThenFinished() {
    // Home draws one column per case in this order at every width; the
    // narrow layout only moves the inspector over the board.
    #expect(FleetLane.allCases == [.needsYou, .running, .finished])
    let shuffled = [
        unit("done", .finished, since: "2026-11-08T05:00:00Z"),
        unit("run", .running, since: "2026-11-08T04:00:00Z"),
        unit("wait", .needsYou, since: "2026-11-08T03:00:00Z"),
    ]
    #expect(shuffled.sorted(by: FleetBoard.order).map(\.lane) == [.needsYou, .running, .finished])
}

@Test func yourTurnWaitsLongestFirst() {
    let works = [
        unit("new", .needsYou, since: "2026-11-08T03:00:00Z"),
        unit("old", .needsYou, since: "2026-11-08T01:00:00Z"),
        unit("mid", .needsYou, since: "2026-11-08T02:00:00Z", lastChange: "2026-11-08T09:00:00Z"),
    ]
    // The wait decides, not a later change of another agent in the unit.
    #expect(works.sorted(by: FleetBoard.order).map(\.title) == ["old", "mid", "new"])
}

@Test func inProgressShowsTheLatestChangeFirst() {
    let works = [
        unit("quiet", .running, since: "2026-11-08T05:00:00Z", lastChange: "2026-11-08T05:00:00Z"),
        // Its focus started long ago, but a helper or second stage moved
        // just now.
        unit("busy", .running, since: "2026-11-08T01:00:00Z", lastChange: "2026-11-08T07:00:00Z"),
        unit("old", .running, since: "2026-11-08T02:00:00Z"),
    ]
    #expect(works.sorted(by: FleetBoard.order).map(\.title) == ["busy", "quiet", "old"])
}

@Test func finishedShowsTheMostRecentlyFinishedFirst() {
    let works = [
        unit("yesterday", .finished, since: "2026-11-07T10:00:00Z", lastChange: "2026-11-07T10:00:00Z"),
        unit("just-now", .finished, since: "2026-11-08T01:00:00Z", lastChange: "2026-11-08T08:00:00Z"),
        unit("unknown", .finished, since: nil),
        unit("this-morning", .finished, since: "2026-11-08T06:00:00Z"),
    ]
    #expect(works.sorted(by: FleetBoard.order).map(\.title) == ["just-now", "this-morning", "yesterday", "unknown"])
}

@Test func decisionsAndWaitingUnitsInterleaveByWait() throws {
    let works = [
        unit("unit-1h", .needsYou, since: "2026-11-08T01:00:00Z"),
        unit("unit-3h", .needsYou, since: "2026-11-08T03:00:00Z"),
    ]
    let decisions = [
        try question("q-2h", at: "2026-11-08T02:00:00Z"),
        try question("q-0h", at: "2026-11-08T00:00:00Z"),
        try question("recovery", at: "2025-01-01T00:00:00Z", system: true),
    ]
    let items = FleetBoard.yourTurn(works: works, decisions: decisions)
    let names = items.map { item -> String in
        switch item {
        case .decision(let decision): decision.id
        case .work(let work): work.title
        }
    }
    // tendd's own item stays last however old it is.
    #expect(names == ["human:mini:q-0h", "unit-1h", "human:mini:q-2h", "unit-3h", "human:mini:recovery"])
}

@Test func onATieADecisionComesFirstAndInputOrderHolds() throws {
    let at = "2026-11-08T01:00:00Z"
    let items = FleetBoard.yourTurn(
        works: [unit("a", .needsYou, since: at), unit("b", .needsYou, since: at)],
        decisions: [try question("q1", at: at), try question("q2", at: at)]
    )
    #expect(items.map(\.id) == ["decision:human:mini:q1", "decision:human:mini:q2", "agent:a", "agent:b"])
}

@Test func yourTurnGroupsByProjectInWaitOrder() throws {
    let items = FleetBoard.yourTurn(
        works: [
            unit("tend-new", .needsYou, since: "2026-11-08T04:00:00Z", repo: "tend"),
            unit("site", .needsYou, since: "2026-11-08T02:00:00Z", repo: "site"),
        ],
        decisions: [try question("q", at: "2026-11-08T03:00:00Z")]
    )
    let groups = FleetBoard.byProject(items)
    // The project whose entry waits longest leads; a decision with no
    // known checkout is in its own group.
    #expect(groups.map(\.project) == ["site", nil, "tend"])
    #expect(groups.map { $0.items.count } == [1, 1, 1])
}
