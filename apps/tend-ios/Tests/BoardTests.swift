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
        // tendd's counts, shown as given; the client's own count agrees.
        let counts = try XCTUnwrap(board.counts)
        XCTAssertEqual(counts.yourTurn, 2)
        XCTAssertEqual(counts.actionable, 2)
        XCTAssertEqual(counts.cantAnswer, 0)
        XCTAssertEqual(counts.inProgress, 2)
        XCTAssertEqual(counts.finished, 2)
        XCTAssertEqual(counts.reviewed, 0)
        XCTAssertEqual(counts.coverage, "complete")
        XCTAssertTrue(board.reviewedMarks)
        // Finished's header counts its main list: this recorded board is
        // from a tendd without `finished_shown`, so the app counts it, and
        // the empty session folded away is not in it.
        XCTAssertEqual(Lane.board.map(board.count), [2, 2, 1])
        XCTAssertEqual(Lane.board.map(board.listedCount), [2, 2, 2])
        XCTAssertEqual(board.cantAnswerCount, 0)
        XCTAssertFalse(board.yourTurnMayBeMore)
    }

    /// COUNTS.md: the numbers are tendd's when it sends them, even where
    /// the listed cards differ (a host's exact count, a local cut).
    func testTendsCountsAreShownAsGiven() throws {
        let unit = { (id: String, lane: String, flags: String) in
            #"{"kind": "unit", "id": "\#(id)", "lane": "\#(lane)", "title": "\#(id)", \#(flags)}"#
        }
        let lanes = #""lanes": {"needs_you": {"groups": [{"items": [\#(unit("w", "needs_you", #""recent": true"#))]}]}, "finished": {"groups": [{"items": [\#(unit("a", "finished", #""reviewed": true"#)), \#(unit("b", "finished", #""recent": true"#))]}]}}"#
        let counts = #""counts": {"your_turn": 5, "actionable": 1, "cant_answer": 3, "in_progress": 7, "finished": 4, "finished_shown": 4, "reviewed": 6, "coverage": "partial"}"#
        let b = try board(#"{\#(lanes), \#(counts), "reviewed_marks": true, "decision_hosts": [{"alias": "a", "local": true}]}"#)
        XCTAssertEqual(Lane.board.map(b.count), [5, 7, 4])
        XCTAssertEqual(b.cantAnswerCount, 3)
        XCTAssertEqual(b.reviewedCount(.finished), 6)
        XCTAssertTrue(b.yourTurnMayBeMore, "partial coverage: maybe more")
        // Without the marks, "Show N reviewed" is what is listed.
        let unmarked = try board(#"{\#(lanes), \#(counts), "decision_hosts": [{"alias": "a", "local": true}]}"#)
        XCTAssertFalse(unmarked.reviewedMarks)
        XCTAssertEqual(unmarked.reviewedCount(.finished), 1)
        // A counts object this app cannot read in part: those fields fall back.
        // Finished's header is `finished_shown`; without it, the main list
        // counted here.
        let odd = try board(#"{\#(lanes), "counts": {"your_turn": "many", "finished": 9}, "decision_hosts": [{"alias": "a", "local": true}]}"#)
        XCTAssertEqual(Lane.board.map(odd.count), [1, 0, 1])
        XCTAssertFalse(odd.yourTurnMayBeMore)
        // Not an object at all: the board still reads, counted here.
        let older = try board(#"{\#(lanes), "counts": [1, 2], "decision_hosts": [{"alias": "a", "local": true}]}"#)
        XCTAssertNil(older.counts)
        XCTAssertEqual(Lane.board.map(older.count), [1, 0, 1])
    }

    /// Finished folds by tendd's `age_group`; a held Looks Good mark moves
    /// a unit out of the row it was counted in.
    func testFinishedFoldsByAgeGroupAndAHeldMarkLeavesItsRow() throws {
        let unit = { (id: String, flags: String) in
            #"{"kind": "unit", "id": "\#(id)", "lane": "finished", "title": "\#(id)", "review_key": "k-\#(id)", \#(flags)}"#
        }
        let items = [
            unit("now", #""recent": true, "age_group": "recent""#),
            unit("days", #""recent": false, "age_group": "older""#),
            unit("weeks", #""recent": false, "age_group": "stale""#),
        ].joined(separator: ",")
        let counts = #""counts": {"finished": 3, "finished_shown": 1, "finished_older": 1, "finished_stale": 1, "reviewed": 0}"#
        let b = try board(#"{"lanes": {"needs_you": {"groups": []}, "finished": {"groups": [{"items": [\#(items)]}]}}, \#(counts), "reviewed_marks": true}"#)
        let folded = FoldedLane(b, lane: .finished)
        XCTAssertEqual(folded.groups.flatMap(\.items).map(\.id), ["u:now"])
        XCTAssertEqual(folded.older.map(\.id), ["days"])
        XCTAssertEqual(folded.stale.map(\.id), ["weeks"])
        XCTAssertEqual(b.count(.finished), 1, "the header counts the main list")
        let held = b.holding(["now": "k-now", "weeks": "k-weeks"])
        XCTAssertEqual(held.count(.finished), 0)
        XCTAssertEqual(held.counts?.finishedStale, 0)
        XCTAssertEqual(held.counts?.finishedOlder, 1)
        XCTAssertEqual(held.counts?.reviewed, 2)
    }

    /// An older tendd sends no counts: the same rules, counted here. Your
    /// turn is every listed card; In progress counts its folded units;
    /// Finished counts its main list only (not older, not reviewed).
    func testWithoutCountsTheLanesAreCountedByTheSameRules() throws {
        let unit = { (id: String, lane: String, flags: String) in
            #"{"kind": "unit", "id": "\#(id)", "lane": "\#(lane)", "title": "\#(id)", \#(flags)}"#
        }
        let running = [
            unit("shown", "running", #""recent": true"#),
            unit("helper", "running", #""probable_helper": true"#),
            unit("empty", "running", #""empty_session": true"#),
            unit("old", "running", #""recent": false"#),
        ].joined(separator: ",")
        let finished = [
            unit("done", "finished", #""recent": true"#),
            unit("older", "finished", #""recent": false"#),
            unit("seen", "finished", #""reviewed": true"#),
        ].joined(separator: ",")
        let question = #"{"kind": "human_request", "request_id": "r1", "body": "Ship?", "from_session_id": "s1", "from_pane": "%1", "created_at": "2026-11-16T09:59:14Z", "host": "a", "actionable": false}"#
        let b = try board(#"{"decision_hosts": [{"alias": "a", "local": true, "truncated": true}], "lanes": {"needs_you": {"groups": [{"items": [\#(question), \#(unit("w", "needs_you", #""recent": true"#))]}]}, "running": {"groups": [{"items": [\#(running)]}]}, "finished": {"groups": [{"items": [\#(finished)]}]}}}"#)
        XCTAssertNil(b.counts)
        XCTAssertEqual(Lane.board.map(b.count), [2, 4, 1])
        XCTAssertEqual(b.reviewedCount(.finished), 1)
        XCTAssertTrue(b.yourTurnMayBeMore, "a cut queue: maybe more")
    }

    /// All clear needs nothing listed, a zero count and complete coverage
    /// by both tendd's word and this app's reading; either one's partial
    /// makes Your turn "maybe more".
    func testAllClearNeedsBothCoveragesAndAZeroCount() throws {
        let empty = #""lanes": {"needs_you": {"groups": []}}"#
        let host = #""decision_hosts": [{"alias": "a", "local": true}]"#
        let complete = #""counts": {"your_turn": 0, "coverage": "complete"}"#
        XCTAssertTrue(try board(#"{\#(empty), \#(host), \#(complete)}"#).allClear)
        // tendd says partial: never all clear, maybe more.
        let partial = try board(#"{\#(empty), \#(host), "counts": {"your_turn": 0, "coverage": "partial"}}"#)
        XCTAssertFalse(partial.allClear)
        XCTAssertTrue(partial.yourTurnMayBeMore)
        XCTAssertTrue(partial.serverPartial)
        // Partial with no host section at all.
        let bare = try board(#"{\#(empty), "decision_hosts": [], "counts": {"your_turn": 0, "coverage": "partial"}}"#)
        XCTAssertFalse(bare.allClear)
        XCTAssertTrue(bare.yourTurnMayBeMore)
        // An empty host list is unknown even without counts.
        XCTAssertFalse(try board(#"{\#(empty), "decision_hosts": []}"#).allClear)
        // tendd counts one this app did not list: not all clear.
        let counted = try board(#"{\#(empty), \#(host), "counts": {"your_turn": 1, "coverage": "complete"}}"#)
        XCTAssertFalse(counted.allClear)
        XCTAssertEqual(counted.count(.yourTurn), 1)
        // tendd says complete, but this app could not read an entry or a
        // host was cut: still maybe more, never all clear.
        let undecoded = try board(#"{"lanes": {"needs_you": {"groups": [{"items": [{"kind": "something_new"}]}]}}, \#(host), "counts": {"your_turn": 1, "coverage": "complete"}}"#)
        XCTAssertEqual(undecoded.undecoded[.yourTurn], 1)
        XCTAssertTrue(undecoded.yourTurnMayBeMore)
        XCTAssertFalse(undecoded.allClear)
        let cut = try board(#"{\#(empty), "decision_hosts": [{"alias": "a", "truncated": true}], \#(complete)}"#)
        XCTAssertTrue(cut.yourTurnMayBeMore)
        XCTAssertFalse(cut.allClear)
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

    func testACardHidesOnlyAHeadlineThatRepeatsItsLanesState() throws {
        func unit(_ lane: String, _ status: String) throws -> BoardUnit {
            let b = try board(#"{"lanes": {"needs_you": {"groups": [\#(lane == "needs_you" ? item(lane, status) : "")]}, "running": {"groups": [\#(lane == "running" ? item(lane, status) : "")]}}}"#)
            let mapped: Lane = lane == "running" ? .inProgress : .yourTurn
            guard case .unit(let u)? = b.items(mapped).first else { throw XCTSkip("no unit") }
            return u
        }
        func item(_ lane: String, _ status: String) -> String {
            #"{"items": [{"kind": "unit", "id": "w", "lane": "\#(lane)", "title": "t", "headline": {"key": "status", "status": "\#(status)"}, "focus": {"session_id": "s", "host": "h", "status": {"key": "status", "status": "\#(status)"}}}]}"#
        }
        // "Working" over "Working": hidden.
        XCTAssertNil(WorkCard.shownHeadline(try unit("running", "working")))
        // An error or a waiting question on Your turn: the row says only
        // "Your turn", so the headline stays.
        XCTAssertNotNil(WorkCard.shownHeadline(try unit("needs_you", "error")))
        XCTAssertNotNil(WorkCard.shownHeadline(try unit("needs_you", "waiting_input")))
        XCTAssertNotNil(WorkCard.shownHeadline(try unit("running", "error")))
    }
}
