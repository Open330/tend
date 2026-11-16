import AppKit
import Combine
import Foundation
import Testing
@testable import Tend

// Home observes FleetHomeBoard, which republishes only what Home shows.

private func snapshot(activity: String, state: String = "working", enteredAt: String? = "2026-11-16T01:00:00Z", processes: Int = 1, cwd: String = "/src/tend") throws -> TendExecutionSnapshot {
    var agent: [String: Any] = [
        "cwd": cwd,
        "kind": "codex", "agent_session_id": "coder", "pane": "%1", "state": state,
        "last_activity_at": activity, "last_response": "Working on it.",
        "workload": ["process_count": processes, "preview": [["pid": processes, "parent_pid": 1, "depth": 1, "kind": "tool", "command": "rg"]]],
    ]
    agent["state_entered_at"] = enteredAt
    let pane: [String: Any] = [
        "pane_id": "%1", "title": "coder", "agent_alias": "coder", "session_id": "$1", "session": "demo",
        "window_id": "@1", "window_name": "work", "window_index": "0", "pane_index": "0",
        "current_command": "codex", "current_path": "/tmp", "socket": "default",
    ]
    let host = try JSONDecoder().decode(TendFleetHost.self, from: JSONSerialization.data(withJSONObject: [
        "alias": "local", "local": true, "mode": "control", "state": "online",
        "remote": ["agents": [agent], "panes": [pane]],
    ]))
    return TendExecutionSnapshot(hosts: [host])
}

private func works(_ snapshot: TendExecutionSnapshot) -> [FleetWork] {
    FleetBoard.works(groups: [], agents: snapshot.hostedAgents, now: Date(timeIntervalSince1970: 1_800_000_000))
}

@MainActor
private func waitFor(_ condition: () -> Bool) async throws {
    for _ in 0..<300 where !condition() { try await Task.sleep(for: .milliseconds(10)) }
}

@Suite @MainActor struct HomeBoardTests {
    @Test func aHeartbeatHomeDoesNotShowLeavesTheBoardAlone() throws {
        let base = works(try snapshot(activity: "2026-11-16T01:00:01Z"))
        let heartbeat = works(try snapshot(activity: "2026-11-16T01:00:09Z", processes: 3))
        #expect(!TendProjectionEquality.same(base, heartbeat, by: TendProjectionEquality.same), "the model still sees it")
        #expect(TendProjectionEquality.same(base, heartbeat, by: TendProjectionEquality.sameOnHome))

        let moved = works(try snapshot(activity: "2026-11-16T01:00:09Z", state: "waiting_input"))
        #expect(!TendProjectionEquality.same(base, moved, by: TendProjectionEquality.sameOnHome))

        // Without stateEnteredAt, Home shows lastActivityAt as the unit's age.
        let undated = works(try snapshot(activity: "2026-11-16T01:00:01Z", enteredAt: nil))
        let undatedLater = works(try snapshot(activity: "2026-11-16T01:00:09Z", enteredAt: nil))
        #expect(!TendProjectionEquality.same(undated, undatedLater, by: TendProjectionEquality.sameOnHome))
    }

    @Test func homesBoardSkipsHeartbeatsTheModelPublishes() async throws {
        let model = AppModel()
        var boardPublishes = 0
        let subscription = model.homeBoard.objectWillChange.sink { boardPublishes += 1 }
        defer { subscription.cancel() }

        model.ingestExecutionSnapshotForTesting(try snapshot(activity: "2026-11-16T01:00:01Z"))
        NotificationCenter.default.post(name: TendLimitCoverage.didChange, object: nil)
        try await waitFor { !model.homeBoard.works.isEmpty }
        #expect(model.homeBoard.works.first?.focus?.agent.agentSessionID == "coder")
        let modelWorks = model.homeWorks.first?.focus?.agent.lastActivityAt
        let afterFirst = boardPublishes

        model.ingestExecutionSnapshotForTesting(try snapshot(activity: "2026-11-16T01:00:09Z", processes: 4))
        NotificationCenter.default.post(name: TendLimitCoverage.didChange, object: nil)
        try await waitFor { model.homeWorks.first?.focus?.agent.lastActivityAt != modelWorks }
        #expect(model.homeWorks.first?.focus?.agent.lastActivityAt == "2026-11-16T01:00:09Z")
        #expect(boardPublishes == afterFirst, "a heartbeat redraws nothing on Home")

        model.ingestExecutionSnapshotForTesting(try snapshot(activity: "2026-11-16T01:00:10Z", state: "waiting_input"))
        NotificationCenter.default.post(name: TendLimitCoverage.didChange, object: nil)
        try await waitFor { model.homeBoard.works.first?.lane == .needsYou }
        #expect(model.homeBoard.works.first?.lane == .needsYou)
        #expect(boardPublishes > afterFirst)
    }

    @Test func theBoardFollowsOnlyTheHostFieldsHomeShows() throws {
        let model = AppModel()
        model.ingestExecutionSnapshotForTesting(try snapshot(activity: "2026-11-16T01:00:01Z"))
        #expect(model.homeBoard.hosts.map(\.alias) == ["local"])
        #expect(model.homeBoard.localHostAlias == "local")
        var publishes = 0
        let subscription = model.homeBoard.objectWillChange.sink { publishes += 1 }
        defer { subscription.cancel() }
        model.ingestExecutionSnapshotForTesting(try snapshot(activity: "2026-11-16T01:00:05Z"))
        #expect(publishes == 0, "same hosts, same states")
    }

    @Test func decisionsAreRebuiltWhenAnAskersCheckoutChangesNotOnAHeartbeat() async throws {
        let model = AppModel()
        model.ingestExecutionSnapshotForTesting(try snapshot(activity: "2026-11-16T01:00:01Z"))
        NotificationCenter.default.post(name: TendLimitCoverage.didChange, object: nil)
        try await waitFor { !model.hostedAgents.isEmpty }
        let askers = model.askersRevision, decisions = model.homeBoard.decisionsRevision

        model.ingestExecutionSnapshotForTesting(try snapshot(activity: "2026-11-16T01:00:09Z"))
        NotificationCenter.default.post(name: TendLimitCoverage.didChange, object: nil)
        try await waitFor { model.hostedAgents.first?.agent.lastActivityAt == "2026-11-16T01:00:09Z" }
        #expect(model.askersRevision == askers)
        #expect(model.homeBoard.decisionsRevision == decisions)

        model.ingestExecutionSnapshotForTesting(try snapshot(activity: "2026-11-16T01:00:09Z", cwd: "/src/other"))
        NotificationCenter.default.post(name: TendLimitCoverage.didChange, object: nil)
        try await waitFor { model.askersRevision != askers }
        #expect(model.askersRevision != askers)
        #expect(model.homeBoard.decisionsRevision != decisions)
    }

    @Test func anOffScreenBoardHoldsUnitsAndTheClockUntilShown() throws {
        let board = FleetHomeBoard()
        let owner = NSObject()
        let window = ObjectIdentifier(owner)
        defer { withExtendedLifetime(owner) {} }
        var publishes = 0
        let subscription = board.objectWillChange.sink { publishes += 1 }
        defer { subscription.cancel() }
        let first = works(try snapshot(activity: "2026-11-16T01:00:01Z"))
        board.setWorks(first)
        #expect(board.works.count == 1 && publishes == 1)

        board.setVisible(false, reader: window)
        let hidden = publishes
        board.setWorks(works(try snapshot(activity: "2026-11-16T01:00:02Z", state: "waiting_input")))
        board.setWorks(works(try snapshot(activity: "2026-11-16T01:00:03Z", state: "idle")))
        board.tick()
        board.tick()
        let revision = board.decisionsRevision
        board.decisionsChanged()
        #expect(publishes == hidden, "nothing on Home changes while the window is off screen")
        #expect(board.works.first?.focus?.agent.state == "working")
        let clock = board.clock

        board.setVisible(true, reader: window)
        #expect(board.isVisible)
        #expect(board.works.first?.focus?.agent.state == "idle", "the latest units show at once")
        #expect(board.clock != clock, "relative times catch up at once")
        #expect(board.decisionsRevision != revision, "a held decisions change applies")
        board.setVisible(true, reader: window)
        let shown = publishes
        board.tick()
        #expect(publishes == shown + 1)
    }

    /// What the projection does: publish to the board only when the new
    /// units differ from its baseline.
    private func project(_ works: [FleetWork], to board: FleetHomeBoard) {
        if !TendProjectionEquality.same(works, board.latestWorks, by: TendProjectionEquality.sameOnHome) {
            board.setWorks(works)
        }
    }

    @Test func aHiddenChangeThatReturnsToWhatIsShownLeavesNothingStaleHeld() throws {
        let board = FleetHomeBoard()
        let owner = NSObject()
        let window = ObjectIdentifier(owner)
        defer { withExtendedLifetime(owner) {} }
        let a = works(try snapshot(activity: "2026-11-16T01:00:01Z"))
        let b = works(try snapshot(activity: "2026-11-16T01:00:02Z", state: "waiting_input"))
        project(a, to: board)
        board.setVisible(false, reader: window)
        project(b, to: board)
        project(a, to: board)
        board.setVisible(true, reader: window)
        #expect(board.works.first?.focus?.agent.state == "working", "A, as the model has it now; not the held B")
    }

    @Test func homeStaysLiveWhileAnyOfItsWindowsIsOnScreen() throws {
        let board = FleetHomeBoard()
        let windows = (NSObject(), NSObject())
        let front = ObjectIdentifier(windows.0), back = ObjectIdentifier(windows.1)
        defer { withExtendedLifetime(windows) {} }
        board.setVisible(true, reader: front)
        board.setVisible(false, reader: back)
        #expect(board.isVisible, "a covered second window does not pause the visible one")
        board.setVisible(false, reader: front)
        #expect(!board.isVisible)
        board.removeReader(back)
        #expect(!board.isVisible)
        board.removeReader(front)
        #expect(board.isVisible, "with no Home mounted nothing is held")
        board.setVisible(false, reader: back)
        board.removeReader(back)
        #expect(board.isVisible)
    }
}
