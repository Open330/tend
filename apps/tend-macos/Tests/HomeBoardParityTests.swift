import Foundation
import Testing
@testable import Tend

// The parity contract with tendd: both board builders read
// apps/shared/Fixtures/home-board-parity-input.json and must produce
// home-board-parity.json, every unit each lane holds, in order. tendd's half
// is crates/tend/src/home_board/parity.rs, which also regenerates the
// summary (UPDATE_FIXTURES=1). A rule changed here or there fails one side
// until the other follows. The units come from Home's own path: the agents
// Home lists, the Work groups from the snapshot and the pipeline runs,
// FleetBoard.works, Your turn's order and the project groups. The counts
// come from FleetBoard.counts, the function Home's lane headers and the
// rail's badge show (apps/shared/Design/COUNTS.md); the cases hold no
// decisions, so Your turn is its units.

private let fixtures = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .appendingPathComponent("../../shared/Fixtures")
    .standardizedFileURL

private func json(_ name: String) throws -> [String: Any] {
    let data = try Data(contentsOf: fixtures.appendingPathComponent(name))
    return try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
}

private func date(_ text: String) throws -> Date {
    try #require(ISO8601DateFormatter().date(from: text))
}

private func stamp(_ date: Date?) -> Any {
    guard let date else { return NSNull() }
    let format = ISO8601DateFormatter()
    format.timeZone = TimeZone(identifier: "UTC")
    return format.string(from: date)
}

private func agentID(_ hosted: TendHostedAgent) -> String {
    "\(hosted.host.alias):\(hosted.agent.agentSessionID)"
}

private func stageState(_ state: FleetWorkStage.State) -> String {
    switch state {
    case .done: "done"
    case .running: "running"
    case .needsYou: "needs_you"
    case .waiting: "waiting"
    }
}

private func laneName(_ lane: FleetLane) -> String {
    switch lane {
    case .needsYou: "needs_you"
    case .running: "running"
    case .finished: "finished"
    }
}

/// One unit in the contract's terms (parity.rs `summary`).
private func summary(_ work: FleetWork, reviewed: Set<String>, now: Date) -> [String: Any] {
    let source: [String: Any]
    switch work.source {
    case .work(let identity):
        source = ["kind": "work", "workspace": identity.workspaceID, "work": identity.workID]
    case .agent:
        let focus = work.focus ?? work.agents.first
        source = ["kind": "agent", "host": focus?.host.alias ?? "", "session_id": focus?.agent.agentSessionID ?? ""]
    }
    var marks = FleetReviewMarks()
    marks.recorded = reviewed
    let focus: Any = work.focus.map { ["agent": agentID($0), "state": $0.agent.state] as [String: Any] } ?? NSNull()
    return [
        "id": work.id,
        "lane": laneName(work.lane),
        "source": source,
        "title": work.title,
        "repo": work.repo.map { $0 as Any } ?? NSNull(),
        "agents": work.agents.map(agentID),
        "helpers": work.helpers.map(agentID),
        "focus": focus,
        "stages": work.stages.map { ["name": $0.name, "state": stageState($0.state)] },
        "since": stamp(work.since),
        "revision": work.revision,
        "review_key": FleetBoard.reviewKey(work),
        "reviewed": work.lane == .finished && marks.hides(work),
        "requested_by_person": FleetBoard.isRequestedByPerson(work),
        "handled_limit": work.handledLimit,
        "probable_helper": work.isProbableHelper,
        "orphan_helper": work.isOrphanHelper,
        "empty_session": work.isEmptySession,
        "age_group": work.lane == .finished ? ageGroupName(FleetBoard.ageGroup(work, now: now)) : NSNull(),
    ]
}

private func ageGroupName(_ group: FleetAgeGroup) -> String {
    switch group {
    case .recent: "recent"
    case .older: "older"
    case .stale: "stale"
    }
}

/// Each lane's units as Home lists them: Your turn by how long each waits,
/// every lane in project groups.
private func lanes(_ works: [FleetWork], reviewed: Set<String>, now: Date) -> [String: Any] {
    var result: [String: Any] = [:]
    for lane in FleetLane.allCases {
        let mine = works.filter { $0.lane == lane }
        let ordered: [FleetWork]
        if lane == .needsYou {
            ordered = FleetBoard.byProject(FleetBoard.yourTurn(works: mine, decisions: [])).flatMap(\.items).compactMap { item in
                if case .work(let work) = item { return work }
                return nil
            }
        } else {
            ordered = FleetBoard.byProject(mine).flatMap(\.works)
        }
        result[laneName(lane)] = ordered.map { summary($0, reviewed: reviewed, now: now) }
    }
    return result
}

/// Home's numbers in the contract's terms (`counts_json`, less what the
/// cases do not cover: decisions and coverage).
private func counts(_ works: [FleetWork], reviewed: Set<String>, now: Date) -> [String: Any] {
    var marks = FleetReviewMarks()
    marks.recorded = reviewed
    let counts = FleetBoard.counts(works: works, decisions: [], marks: marks, now: now)
    return [
        "your_turn": counts.yourTurn,
        "in_progress": counts.inProgress,
        "finished": counts.finished,
        "finished_shown": counts.finishedShown,
        "finished_older": counts.finishedOlder,
        "finished_stale": counts.finishedStale,
        "reviewed": counts.reviewed,
    ]
}

/// JSON values compared as the parsed fixture holds them.
private func normalized(_ value: Any) throws -> NSObject {
    let data = try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
    return try #require(JSONSerialization.jsonObject(with: data) as? NSObject)
}

@Suite struct HomeBoardParityTests {
    @Test func homeMeetsTheParityContractTenddAlsoMeets() throws {
        let input = try json("home-board-parity-input.json")
        let recorded = try json("home-board-parity.json")
        #expect(input["schema"] as? Int == 1)
        let now = try date(try #require(input["now"] as? String))
        let cases = try #require(input["cases"] as? [[String: Any]])
        let expected = try #require(recorded["cases"] as? [[String: Any]])
        #expect(cases.count == expected.count)
        #expect(!cases.isEmpty)
        for (index, item) in cases.enumerated() {
            let name = try #require(item["name"] as? String)
            let hosts = try JSONDecoder().decode(
                [TendFleetHost].self,
                from: JSONSerialization.data(withJSONObject: item["hosts"] ?? [])
            )
            let runs = try JSONDecoder().decode(
                [TendPipelineRun].self,
                from: JSONSerialization.data(withJSONObject: item["runs"] ?? [])
            )
            let reviewed = Set(item["reviewed"] as? [String] ?? [])
            let snapshot = TendExecutionSnapshot(hosts: hosts)
            let agents = FleetBoard.homeAgents(snapshot, now: now)
            let works = FleetBoard.works(groups: snapshot.workGroups(pipelineRuns: runs), agents: agents, now: now)
            let built: [String: Any] = [
                "name": name,
                "lanes": lanes(works, reviewed: reviewed, now: now),
                "counts": counts(works, reviewed: reviewed, now: now),
            ]
            let want = expected[index]
            #expect(want["name"] as? String == name)
            let builtJSON = try normalized(built)
            let wantJSON = try normalized(want)
            if builtJSON != wantJSON {
                let mine = String(decoding: try JSONSerialization.data(withJSONObject: built, options: [.sortedKeys, .prettyPrinted]), as: UTF8.self)
                Issue.record("case \"\(name)\": Home's board differs from tendd's.\nHome built:\n\(mine)")
            }
        }
    }
}
