import XCTest
@testable import Tend

/// Home's board (`GET /api/device/home`) as the app reads it: the recorded
/// fixture (`apps/shared/Fixtures/device-home.json`, the projection of a
/// real tendd's board), and the Mac's folds on hand-made boards.
final class BoardTests: XCTestCase {
    private func board(_ json: String) throws -> DeviceHome {
        try JSONDecoder().decode(DeviceHome.self, from: Data(json.utf8))
    }

    func testTheRecordedBoardDecodesLaneByLane() throws {
        let url = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "device-home", withExtension: "json"))
        let board = try JSONDecoder().decode(DeviceHome.self, from: Data(contentsOf: url))
        XCTAssertEqual(board.undecoded, [:])
        // Your turn: the review and the question, decidable here, on this host.
        let decisions = board.decisions
        XCTAssertEqual(decisions.count, 2)
        XCTAssertTrue(decisions.allSatisfy { $0.decidable && $0.hostIsLocal && $0.host == "local" })
        XCTAssertEqual(decisions.filter(\.isQuestion).count, 1)
        XCTAssertFalse(board.allClear)
        // In progress: two units at work, the title as given.
        let running = board.items(.inProgress).compactMap { if case .unit(let u) = $0 { u } else { nil } }
        XCTAssertEqual(running.map(\.lane), [.inProgress, .inProgress])
        XCTAssertEqual(running.first?.titleText, "Port the retry path to the Windows named pipe")
        XCTAssertEqual(running.first?.headlineText, "Working")
        XCTAssertEqual(running.first?.stages.map(\.state), [.running])
        XCTAssertEqual(running.first?.focus?.sessionID, "web-worker-1794823154")
        XCTAssertEqual(running.first?.hosts, ["local"])
        // Finished: an empty session folds away; a keyed title is worded.
        let folded = FoldedLane(board, lane: .finished)
        XCTAssertEqual(folded.empty.count, 1)
        XCTAssertEqual(folded.empty.first?.titleText, "claude on local")
        XCTAssertEqual(folded.count, 2)
        XCTAssertEqual(folded.groups.flatMap(\.items).count, 1)
        // The backlog, without its ticket reference.
        XCTAssertEqual(board.backlog.map(\.titleText), ["Document the retry path"])
        XCTAssertEqual(board.backlog.first?.priority, 1)
    }

    func testFoldsFollowTheMac() throws {
        let unit = { (id: String, flags: String) in
            #"{"kind": "unit", "id": "\#(id)", "lane": "running", "title": "\#(id)", \#(flags)}"#
        }
        let items = [
            unit("shown", #""recent": true"#),
            unit("helper", #""probable_helper": true"#),
            unit("empty", #""empty_session": true"#),
            unit("old", #""recent": false"#),
            unit("seen", #""reviewed": true, "probable_helper": true"#),
        ].joined(separator: ",")
        let b = try board(#"{"lanes": {"needs_you": {"groups": []}, "running": {"groups": [{"project": "tend", "items": [\#(items)]}]}}}"#)
        let folded = FoldedLane(b, lane: .inProgress)
        XCTAssertEqual(folded.groups.flatMap(\.items).map(\.id), ["u:shown"])
        XCTAssertEqual(folded.helpers.map(\.id), ["helper"])
        XCTAssertEqual(folded.empty.map(\.id), ["empty"])
        XCTAssertEqual(folded.older.map(\.id), ["old"])
        // Reviewed wins over every other fold, and is not counted.
        XCTAssertEqual(folded.reviewed.map(\.id), ["seen"])
        XCTAssertEqual(folded.count, 4)
    }

    func testYourTurnNeverFoldsAndIsAllClearOnlyWhenFullyRead() throws {
        let waiting = #"{"kind": "unit", "id": "w", "lane": "needs_you", "title": "t", "probable_helper": true}"#
        let b = try board(#"{"lanes": {"needs_you": {"groups": [{"project": null, "items": [\#(waiting)]}]}}}"#)
        XCTAssertEqual(FoldedLane(b, lane: .yourTurn).groups.flatMap(\.items).count, 1)
        XCTAssertFalse(b.allClear)

        let empty = #""lanes": {"needs_you": {"groups": []}}"#
        XCTAssertTrue(try board(#"{\#(empty), "decision_hosts": [{"alias": "a", "local": true}]}"#).allClear)
        XCTAssertFalse(try board(#"{\#(empty), "decision_hosts": [{"alias": "a", "truncated": true}]}"#).allClear)
        XCTAssertFalse(try board(#"{\#(empty), "decision_hosts": [{"alias": "a", "unavailable": ["reviews"]}]}"#).allClear)
        // An item this app cannot read is counted, never all clear.
        let unknown = try board(#"{"lanes": {"needs_you": {"groups": [{"items": [{"kind": "something_new"}]}]}}}"#)
        XCTAssertEqual(unknown.undecoded[.yourTurn], 1)
        XCTAssertFalse(unknown.allClear)
    }

    func testADecisionOnAnObserveOnlyHostIsShownNotDecidable() throws {
        let entry = #"{"kind": "human_request", "request_id": "r1", "body": "Ship?", "from_session_id": "s1", "from_pane": "%1", "created_at": "2026-11-16T09:59:14Z", "host": "mbp", "actionable": false}"#
        let b = try board(#"{"hosts": [{"alias": "mbp", "local": false, "state": "online"}], "lanes": {"needs_you": {"groups": [{"items": [\#(entry)]}]}}}"#)
        let decision = try XCTUnwrap(b.decisions.first)
        XCTAssertFalse(decision.decidable)
        XCTAssertFalse(decision.hostIsLocal)
        XCTAssertEqual(decision.host, "mbp")
    }

    func testBoardTextsAreWorded() throws {
        func text(_ json: String) throws -> String {
            try JSONDecoder().decode(BoardText.self, from: Data(json.utf8)).words
        }
        XCTAssertEqual(try text(#""plain""#), "plain")
        XCTAssertEqual(try text(#"{"key": "status", "status": "asked"}"#), "Asked you")
        XCTAssertEqual(try text(#"{"key": "owner_says", "owner": "impl", "text": {"key": "waits_for_answer"}}"#),
                       "impl: Waits for your answer. Open to see the question.")
        XCTAssertEqual(try text(#"{"key": "stage_is", "stage": "review", "status": "failed"}"#), "Stage review is Error")
        XCTAssertEqual(try text(#"{"key": "not_yet_known"}"#), "")
    }

    func testStagesShowWhereTheWorkIs() throws {
        let b = try board(#"{"lanes": {"needs_you": {"groups": []}, "running": {"groups": [{"items": [{"kind": "unit", "id": "w", "lane": "running", "title": {"key": "kind_on_host", "kind": "codex", "host": "mini"}, "stages": [{"name": "plan", "state": "done"}, {"name": "build", "state": "running"}, {"name": "review", "state": "waiting"}]}]}]}}}"#)
        guard case .unit(let unit)? = b.items(.inProgress).first else { return XCTFail("no unit") }
        XCTAssertEqual(unit.currentStage, 1)
        XCTAssertEqual(unit.doneStages, 1)
        XCTAssertEqual(unit.titleText, "codex on mini")
    }

    func testABoardWithoutCoverageIsNeverAllClear() throws {
        // No Your turn lane at all: not a board.
        XCTAssertThrowsError(try board(#"{"lanes": {}}"#))
        let empty = #""lanes": {"needs_you": {"groups": []}}"#
        // No coverage sent, or a host record this app cannot read.
        XCTAssertFalse(try board(#"{\#(empty)}"#).allClear)
        let unreadable = try board(#"{\#(empty), "decision_hosts": [{"alias": 42, "truncated": true}]}"#)
        XCTAssertFalse(unreadable.coverageKnown)
        XCTAssertFalse(unreadable.allClear)
    }
}
