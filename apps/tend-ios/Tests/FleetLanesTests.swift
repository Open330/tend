import XCTest
@testable import Tend

/// The device Fleet snapshot as the daemon sends it, and the lane rules the
/// web Home shares (crates/tend/src/dashboard/web/home-model.mjs).
final class FleetLanesTests: XCTestCase {
    private func fixture() throws -> FleetSnapshot {
        let url = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "device-fleet", withExtension: "json"))
        return try JSONDecoder().decode(FleetSnapshot.self, from: Data(contentsOf: url))
    }

    /// "Now" for the fixture: just after its newest agent changed.
    private func now(_ fleet: FleetSnapshot) -> Date {
        fleet.hosts.flatMap(\.agents).compactMap(\.since).max()!.addingTimeInterval(60)
    }

    func testTheRecordedSnapshotDecodes() throws {
        let fleet = try fixture()
        let host = try XCTUnwrap(fleet.hosts.first)
        XCTAssertTrue(host.local)
        XCTAssertTrue(host.online)
        XCTAssertTrue(host.control)
        XCTAssertFalse(host.agents.isEmpty)
        let cache = try XCTUnwrap(host.agents.first { $0.sessionID.hasPrefix("ios-cache-") })
        XCTAssertEqual(cache.state, "idle")
        XCTAssertEqual(cache.repo, "billing-api")
        XCTAssertEqual(cache.maker, "claude")
        XCTAssertEqual(cache.status, .idle)
        XCTAssertTrue(cache.lastResponse?.contains("41 s to 17 s") ?? false)
    }

    func testInProgressIsWhatIsWorkingAndFinishedIsWhatWentIdle() throws {
        let fleet = try fixture()
        let at = now(fleet)
        let running = FleetLanes.inProgress(fleet).map(\.agent.sessionID)
        let finished = FleetLanes.finished(fleet, now: at).map(\.agent.sessionID)
        XCTAssertTrue(running.contains { $0.hasPrefix("ios-api-") })
        XCTAssertTrue(running.contains { $0.hasPrefix("ios-web-") })
        XCTAssertFalse(running.contains { $0.hasPrefix("ios-cache-") })
        XCTAssertEqual(Set(finished.map { String($0.split(separator: "-")[1]) }), ["cache", "i18n"])
        XCTAssertTrue(Set(running).isDisjoint(with: finished))
        // A day later, nothing has "just" finished.
        XCTAssertTrue(FleetLanes.finished(fleet, now: at.addingTimeInterval(25 * 3600)).isEmpty)
    }

    func testAgentsWaitingOnThePersonAreNeverInProgressOrFinished() throws {
        let json = #"""
        {"hosts":[{"alias":"mini","local":false,"mode":"control","state":"online","remote":{"agents":[
          {"kind":"codex","agent_session_id":"w","pane":"%1","state":"waiting_input","state_entered_at":"2026-11-16T07:00:00Z"},
          {"kind":"codex","agent_session_id":"a","pane":"%2","state":"idle","awaiting_reply":{"since":"x"},"last_response":"?","state_entered_at":"2026-11-16T07:00:00Z"},
          {"kind":"codex","agent_session_id":"h","state":"working"},
          {"kind":"codex","agent_session_id":"s","pane":"%3","state":"stopped"},
          {"not":"an agent"}
        ]}}]}
        """#
        let fleet = try JSONDecoder().decode(FleetSnapshot.self, from: Data(json.utf8))
        let at = Timestamp.parse("2026-11-16T07:01:00Z")!
        XCTAssertEqual(fleet.hosts[0].agents.count, 4, "the unreadable record is dropped, not the host")
        XCTAssertTrue(FleetLanes.inProgress(fleet).isEmpty, "waiting, headless and stopped agents are not in progress")
        XCTAssertTrue(FleetLanes.finished(fleet, now: at).isEmpty, "an agent that asked in its pane is not finished")
        XCTAssertEqual(fleet.hosts[0].agents.first { $0.sessionID == "a" }?.status, .asked)
    }

    func testHostsAreOrderedThisOneFirst() throws {
        let json = #"{"hosts":[{"alias":"zeta","state":"offline"},{"alias":"here","local":true},{"alias":"alpha","state":"online"}]}"#
        let fleet = try JSONDecoder().decode(FleetSnapshot.self, from: Data(json.utf8))
        XCTAssertEqual(fleet.ordered.map(\.alias), ["here", "alpha", "zeta"])
        XCTAssertFalse(fleet.hosts[0].online)
    }

    func testCardHeadlinesShowMarkdownNotItsMarkers() {
        let line = TendMarkdown.inline("Warm-up: **41 s to 17 s** via `cache::warm`")
        XCTAssertEqual(String(line.characters), "Warm-up: 41 s to 17 s via cache::warm")
        let block = TendMarkdown.attributed("Done.\n\n- one\n- two")
        XCTAssertEqual(String(block.characters), "Done.\n•  one\n•  two")
    }

    func testAPhase1PairingKeepsDecidingOnly() throws {
        let stored = #"{"origin":{"url":"https://tend.test"},"device":{"id":"dev_1","name":"phone"},"token":"tdv_x"}"#
        let pairing = try JSONDecoder().decode(Pairing.self, from: Data(stored.utf8))
        XCTAssertEqual(pairing.scope, .decide)
        XCTAssertEqual(DeviceScope(names: ["decisions"]), .decide)
        XCTAssertEqual(DeviceScope(names: ["decisions", "operate"]), .operate)
        XCTAssertEqual(DeviceScope(names: nil), .decide)
    }
}
