import XCTest
@testable import Tend

/// Questions nobody can answer (docs/proposals/STALE_QUESTIONS.md) on the
/// phone: folded out of Your turn and the badge on the board and on the
/// queue, counted exactly, Keep and Close all sending the listed pairs,
/// Close all only where the paired host allows it, history with closures,
/// and an older dashboard read as before.
@MainActor
final class CantAnswerTests: XCTestCase {
    override class func setUp() {
        DashboardClient.testProtocolClasses = [StubProtocol.self]
    }

    override func setUp() {
        StubProtocol.reset()
        CredentialStore.clear()
    }

    override func tearDown() {
        CredentialStore.clear()
    }

    private static func question(_ id: String, unanswerable: String? = nil) -> String {
        let folded = unanswerable.map { #","unanswerable":\#($0)"# } ?? ""
        return #"{"kind":"human_request","request_id":"\#(id)","body":"\#(id)?","from_session_id":"s_\#(id)","from_pane":"%1","created_at":"2026-11-10T10:00:00Z","host":"here","actionable":true\#(folded)}"#
    }

    private static func board(close: Bool, flags: Bool = true) -> String {
        let hostFlags = flags ? #","subject_check":true,"close_unanswerable":\#(close),"unanswerable_count":4"# : ""
        let items = [
            question("live"),
            question("gone_a", unanswerable: #"{"cause":"pane_gone","episode":"ep_a","since":"2026-11-16T00:00:00Z"}"#),
            question("gone_b", unanswerable: #"{"cause":"replaced","episode":"ep_b","since":"2026-11-16T00:00:00Z"}"#),
        ].joined(separator: ",")
        return #"{"hosts":[{"alias":"here","local":true}],"decision_hosts":[{"alias":"here","local":true,"control":true,"truncated":true\#(hostFlags)}],"lanes":{"needs_you":{"groups":[{"items":[\#(items)]}]}}}"#
    }

    private static let homeDevice = #"{"device":{"id":"dev_1","name":"phone"},"scope":["operate"],"features":["home"]}"#
    private static let fleetJSON = #"{"hosts":[{"alias":"here","local":true,"state":"online","remote":{"agents":[]}}]}"#

    private func operateModel() -> AppModel {
        let origin = try! ServerOrigin(parsing: "https://tend.test")
        CredentialStore.save(Pairing(origin: origin, device: PairedDevice(id: "dev_1", name: "phone"), token: "tdv_x", scope: .operate))
        return AppModel()
    }

    private func decideModel() -> AppModel {
        let origin = try! ServerOrigin(parsing: "https://tend.test")
        CredentialStore.save(Pairing(origin: origin, device: PairedDevice(id: "dev_1", name: "phone"), token: "tdv_x"))
        return AppModel()
    }

    func testTheBoardFoldsThemOutOfYourTurnAndTheBadge() throws {
        let board = try JSONDecoder().decode(DeviceHome.self, from: Data(Self.board(close: false).utf8))
        XCTAssertEqual(board.decisions.map(\.title), ["live?"])
        XCTAssertEqual(board.cantAnswer.map(\.title), ["gone_a?", "gone_b?"])
        XCTAssertEqual(board.cantAnswer.first?.unanswerable?.cause, .paneGone)
        XCTAssertEqual(board.cantAnswerCount, 4, "the exact count, though only two are listed")
        XCTAssertEqual(FoldedLane(board, lane: .yourTurn).count, 1)
        // An older dashboard: no host flags, no count; what is listed still folds.
        let older = try JSONDecoder().decode(DeviceHome.self, from: Data(Self.board(close: false, flags: false).utf8))
        XCTAssertEqual(older.cantAnswerCount, 2)
        XCTAssertFalse(older.decisionHosts[0].subjectCheck)
    }

    func testAnOlderDaemonsQueueReadsAsBefore() throws {
        let json = #"{"hosts":[{"alias":"here","local":true,"control":true,"queue":{"items":[\#(Self.question("live")),\#(Self.question("old"))]}}]}"#
        struct Hosts: Decodable { let hosts: [HostQueue] }
        let hosts = try JSONDecoder().decode(Hosts.self, from: Data(json.utf8)).hosts
        XCTAssertEqual(PendingDecision.from(hosts[0]).filter { $0.unanswerable == nil }.count, 2)
        XCTAssertFalse(hosts[0].subjectCheck)
        XCTAssertEqual(hosts[0].queue.unanswerableCount, 0)
        // Only folded questions left: all clear, the group says the rest.
        let folded = #"{"hosts":[{"alias":"here","local":true,"control":true,"subject_check":true,"queue":{"items":[\#(Self.question("g", unanswerable: #"{"cause":"ended","episode":"e","since":"2026-11-16T00:00:00Z"}"#))],"unanswerable_count":1}}]}"#
        XCTAssertTrue(AppModel.allClear(try JSONDecoder().decode(Hosts.self, from: Data(folded.utf8)).hosts))
    }

    func testKeepSendsTheEpisodeAndCloseAllNeedsTheSwitch() async throws {
        StubProtocol.on("/api/device", .init(status: 200, body: Self.homeDevice))
        StubProtocol.on("/api/device/fleet", .init(status: 200, body: Self.fleetJSON))
        StubProtocol.on("/api/device/home", .init(status: 200, body: Self.board(close: false)), .init(status: 200, body: Self.board(close: true)))
        StubProtocol.on("/api/decisions/history", .init(status: 200, body: #"{"hosts":[]}"#))
        StubProtocol.on("/api/decisions/keep", .init(status: 200, body: #"{"request_id":"gone_a","episode":"ep_a"}"#))
        StubProtocol.on("/api/decisions/close-unanswerable", .init(status: 200, body: #"{"closed":["gone_a"],"skipped":[{"request_id":"gone_b","reason":"kept"}]}"#))
        let model = operateModel()
        await model.refreshDevice()
        await model.refresh()
        XCTAssertEqual(model.actionableCount, 1, "the badge leaves them out")
        XCTAssertEqual(model.cantAnswerCount, 4)
        XCTAssertTrue(model.closable.isEmpty, "Close all hidden while the switch is off")
        let gone = try XCTUnwrap(model.cantAnswer.first)
        XCTAssertTrue(model.canKeep(gone))
        let kept = await model.keep(gone)
        XCTAssertTrue(kept)
        let keepBody = try XCTUnwrap(StubProtocol.bodies["/api/decisions/keep"])
        XCTAssertEqual(try JSONSerialization.jsonObject(with: keepBody) as? [String: String], ["request_id": "gone_a", "episode": "ep_a"])

        // The board now says the switch is on.
        XCTAssertEqual(model.closable.count, 2)
        await model.closeAll(model.closable)
        let closeBody = try XCTUnwrap(StubProtocol.bodies["/api/decisions/close-unanswerable"])
        let sent = try XCTUnwrap(JSONSerialization.jsonObject(with: closeBody) as? [String: Any])
        XCTAssertEqual(sent["by_label"] as? String, "ios")
        XCTAssertEqual(sent["items"] as? [[String: String]], [["request_id": "gone_a", "episode": "ep_a"], ["request_id": "gone_b", "episode": "ep_b"]])
        XCTAssertEqual(model.closeAllResult?.ok, false)
        XCTAssertEqual(model.closeAllResult?.text, "Closed 1 question. Skipped 1: gone_b (kept)")
        XCTAssertTrue(StubProtocol.queries.contains { $0.hasPrefix("/api/decisions/history?") && $0.contains("closures=true") })
    }

    func testTheQueueFoldsTooForADeviceThatOnlyDecides() async {
        let queue = #"{"hosts":[{"alias":"here","local":true,"control":true,"subject_check":true,"close_unanswerable":false,"queue":{"items":[\#(Self.question("live")),\#(Self.question("g", unanswerable: #"{"cause":"agent_exited","episode":"e","since":"2026-11-16T00:00:00Z"}"#))],"truncated":true,"unanswerable_count":3}}]}"#
        StubProtocol.on("/api/decisions", .init(status: 200, body: queue))
        let model = decideModel()
        await model.refresh()
        XCTAssertEqual(model.pending.map(\.title), ["live?"])
        XCTAssertEqual(model.actionableCount, 1)
        XCTAssertEqual(model.cantAnswer.map(\.title), ["g?"])
        XCTAssertEqual(model.cantAnswerCount, 3)
        XCTAssertTrue(model.canKeep(model.cantAnswer[0]))
        XCTAssertTrue(model.closable.isEmpty)
    }

    func testHistoryShowsClosuresWithWhy() throws {
        let json = #"""
        {"hosts":[{"alias":"here","local":true,"history":{"items":[
          {"kind":"closed_by_tend","request_id":"sys","body":"Recover?","from_session_id":"tend:peer-recovery","from_pane":"","created_at":"2026-11-17T00:00:00Z","system":true,"reason":"parent_missing","reason_text":"original request gone","closed_by":"daemon","closed_at":"2026-11-19T10:00:00Z"},
          {"kind":"human_answer","request_id":"g","body":"Deploy?","from_session_id":"s","from_pane":"%4","created_at":"2026-11-10T00:00:00Z","status":"declined","answer":"dismissed","answered_at":"2026-11-19T11:00:00Z","closure":{"reason":"subject_gone","cause":"agent_exited","episode":"e","by":"person","surface":"ios","at":"2026-11-19T11:00:00Z"}}
        ]}}]}
        """#
        struct Hosts: Decodable { let hosts: [HostHistory] }
        let section = try JSONDecoder().decode(Hosts.self, from: Data(json.utf8)).hosts[0]
        XCTAssertEqual(section.history.undecoded, 0)
        let past = section.history.items.map { PastDecision(item: $0, host: "here", hostIsLocal: true) }
        XCTAssertEqual(past.map(\.outcome), ["Closed by tend", "Closed"])
        XCTAssertEqual(past[0].text, "Closed by tend: the original request is gone")
        XCTAssertEqual(past[1].text, "Closed, asker gone: Agent exited")
        XCTAssertNotNil(past[0].at)
    }
}
