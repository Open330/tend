import Foundation
import Testing
@testable import Tend

// Questions nobody can answer (docs/proposals/STALE_QUESTIONS.md): the
// wire fields decode, the folded items leave Your turn and every count,
// Keep and Close all send the exact pairs, Close all needs the daemon's
// switch, history asks for closures, and an older daemon reads as before.

private let local = TendFleetHostIdentity(alias: "mac", local: true, state: "online", mode: "local")

private let foldedQueueJSON = """
{"items":[
  {"kind":"human_request","request_id":"rq_live","body":"Merge #12?","from_session_id":"s1","from_pane":"%3",
   "created_at":"2026-11-19T11:00:00Z"},
  {"kind":"human_request","request_id":"rq_a","body":"Deploy?","from_session_id":"s2","from_pane":"%42",
   "created_at":"2026-11-10T11:00:00Z",
   "unanswerable":{"cause":"pane_gone","episode":"ep_a","since":"2026-11-16T00:00:00Z","pane":"%42"}},
  {"kind":"human_request","request_id":"rq_b","body":"Rotate keys?","from_session_id":"s3","from_pane":"%43",
   "created_at":"2026-11-09T11:00:00Z",
   "unanswerable":{"cause":"something_new","episode":"ep_b","since":"2026-11-16T00:00:00Z"}},
  {"kind":"human_request","request_id":"rq_c","body":"Odd?","from_session_id":"s4","from_pane":"%44",
   "created_at":"2026-11-09T11:00:00Z","unanswerable":{"cause":"ended"}}
],"truncated":true,"unanswerable_count":5}
"""

private func hello(_ capabilities: [String]) throws -> Data {
    try JSONSerialization.data(withJSONObject: [
        "ok": true, "min_protocol": 1, "max_protocol": 6,
        "capabilities": ["session_bytes_v1", "session_attachment_identity_v1", "work_control_v1"] + capabilities,
    ])
}

private final class Sent: @unchecked Sendable {
    private let lock = NSLock()
    private var objects: [[String: Any]] = []
    func add(_ object: [String: Any]) { lock.lock(); objects.append(object); lock.unlock() }
    var all: [[String: Any]] { lock.lock(); defer { lock.unlock() }; return objects }
}

private func message(id: String) throws -> TendOperatorMessage {
    let json = """
    {"id":"\(id)","kind":"question","body":"Deploy?","expects_reply":true,"work_mode":"read_only",
     "status":"queued","created_at":"2026-11-10T11:00:00Z",
     "from":{"agent_kind":"codex","agent_session_id":"s-\(id)","pane":"%9","room":{"host":"tmux","window_id":"@1"}},
     "to":{"agent_kind":"unknown","agent_session_id":"__tend_console__","pane":"console","room":{"host":"tmux","window_id":"@1"},"console":true}}
    """
    let request = try JSONDecoder().decode(TendCollaborationRequest.self, from: Data(json.utf8))
    return TendOperatorMessage(host: local, routePane: .operatorRoute, request: request)
}

@Suite struct CantAnswerTests {
    @Test func theNewFieldsDecodeAndAMalformedOneLeavesTheItemLive() throws {
        let queue = try JSONDecoder().decode(TendPendingQueue.self, from: Data(foldedQueueJSON.utf8))
        #expect(queue.items.count == 4)
        #expect(queue.unanswerableCount == 5)
        let entries: [TendHumanRequestEntry] = queue.items.compactMap { if case .humanRequest(let e) = $0 { e } else { nil } }
        #expect(entries[0].unanswerable == nil)
        #expect(entries[1].unanswerable?.cause == .paneGone)
        #expect(entries[1].unanswerable?.episode == "ep_a")
        #expect(entries[1].unanswerable?.pane == "%42")
        #expect(entries[2].unanswerable?.cause == .other("something_new"), "a cause this app does not know still folds")
        #expect(entries[3].unanswerable == nil, "no episode: nothing to keep or close, so it stays live")
        // An older daemon's queue: no count, nothing folded.
        let old = try JSONDecoder().decode(TendPendingQueue.self, from: Data(#"{"items":[],"truncated":false}"#.utf8))
        #expect(old.unanswerableCount == 0)
    }

    @Test func causesReadAsPeopleSayThem() {
        #expect(TendUnanswerable.Cause("ended").text == "Session ended")
        #expect(TendUnanswerable.Cause("replaced").text == "Replaced by another agent")
        #expect(TendUnanswerable.Cause("pane_gone").text == "Pane gone")
        #expect(TendUnanswerable.Cause("agent_exited").text == "Agent exited")
        let since = TendUnanswerable(cause: .paneGone, episode: "e", since: "2026-11-16T00:00:00Z")
        let line = since.reasonLine(relativeTo: try! Date("2026-11-19T00:00:00Z", strategy: .iso8601))
        #expect(line.hasPrefix("Pane gone · since "))
        #expect(line.contains("3 days ago"))
    }

    @Test func foldedQuestionsComeLastAndAreKnownToTheBadges() throws {
        let queue = try JSONDecoder().decode(TendPendingQueue.self, from: Data(foldedQueueJSON.utf8))
        let merged = FleetDecisions.merge(queue: queue, reviews: [], messages: [], localHost: local)
        #expect(merged.map(\.isUnanswerable) == [false, false, true, true])
        #expect(merged.last?.unanswerable?.episode == "ep_b")
        let folded = FleetDecisions.foldedQuestions(queue, localHost: "mac")
        #expect(folded.folded == ["mac:rq_a", "mac:rq_b"])

        let summary = TendAttentionSummary.make(
            agents: [], messages: [try message(id: "rq_a"), try message(id: "rq_live")], unread: [],
            folded: folded, paneKey: { $0.id }
        )
        #expect(summary.decisions == 1, "the folded question is not counted")
        #expect(summary.dock == 1)
        let before = TendAttentionSummary.make(
            agents: [], messages: [try message(id: "rq_a"), try message(id: "rq_live")], unread: [], paneKey: { $0.id }
        )
        #expect(before.decisions == 2, "an older daemon folds nothing")
    }

    /// A cut page of a folding queue: a local question it left out (a folded
    /// one past the cut) never comes back from the mailbox as live, in Your
    /// turn or on the badges; another host's question still does. Without
    /// folded questions, a cut page is filled in from the mailbox as before.
    @Test func aTruncatedFoldingQueueNeverBringsAnOmittedQuestionBackAsLive() throws {
        let queue = try JSONDecoder().decode(TendPendingQueue.self, from: Data(foldedQueueJSON.utf8))
        #expect(queue.truncated && queue.unanswerableCount == 5)
        #expect(FleetDecisions.omitsFoldedRows(queue))
        // rq_omitted is folded on the daemon but past the 200-row page.
        let omitted = try message(id: "rq_omitted")
        let remote = TendFleetHostIdentity(alias: "mini", local: false, state: "online", mode: "control")
        let theirs = TendOperatorMessage(host: remote, routePane: .operatorRoute, request: try message(id: "rq_r").request)
        let merged = FleetDecisions.merge(queue: queue, reviews: [], messages: [omitted, theirs], localHost: local)
        #expect(!merged.contains { $0.id == "human:mac:rq_omitted" }, "not live in Your turn")
        #expect(merged.contains { $0.id == "human:mini:rq_r" })
        #expect(merged.filter { !$0.isUnanswerable }.map(\.id) == ["human:mac:rq_live", "human:mac:rq_c", "human:mini:rq_r"])

        let badge = TendAttentionSummary.make(
            agents: [], messages: [omitted, try message(id: "rq_live"), theirs], unread: [],
            folded: FleetDecisions.foldedQuestions(queue, localHost: "mac"), paneKey: { $0.id }
        )
        #expect(badge.decisions == 2, "rq_live and the other host's question; not the omitted one")

        // No folded questions: what the cut page left out is live.
        let plain = TendPendingQueue(items: queue.items.filter {
            if case .humanRequest(let entry) = $0 { return entry.unanswerable == nil }
            return true
        }, truncated: true)
        #expect(!FleetDecisions.omitsFoldedRows(plain))
        let filled = FleetDecisions.merge(queue: plain, reviews: [], messages: [omitted], localHost: local)
        #expect(filled.contains { $0.id == "human:mac:rq_omitted" })
        #expect(FleetDecisions.foldedQuestions(plain, localHost: "mac").live == nil)
    }

    /// Another host's folded question opens on the dashboard's group to be
    /// kept or closed there; this Mac's never does, and neither does one
    /// when tendd names no dashboard (docs/proposals/FLEET_UNANSWERABLE.md).
    @Test func anotherHostsQuestionOpensOnTheDashboard() throws {
        let queue = try JSONDecoder().decode(TendPendingQueue.self, from: Data(foldedQueueJSON.utf8))
        let mini = TendFleetHostIdentity(alias: "jiun-mini", local: false, state: "online", mode: "control")
        let folded = FleetDecisions.merge(queue: queue, reviews: [], messages: [], localHost: local).filter(\.isUnanswerable)
        let theirs = FleetDecisions.merge(queue: queue, reviews: [], messages: [], localHost: mini).filter(\.isUnanswerable)
        let dashboard = try #require(URL(string: "https://tend.example"))
        #expect(FleetCantAnswer.dashboardLink(folded[0], on: dashboard) == nil, "this Mac's is kept here")
        #expect(FleetCantAnswer.dashboardLink(theirs[0], on: dashboard)?.absoluteString
            == "https://tend.example/needs#item=rq_a&host=jiun-mini")
        #expect(FleetCantAnswer.dashboardLink(theirs[0], on: nil) == nil)
    }

    @Test func theCloseAllReportListsEverySkippedQuestion() {
        let skipped = (1...8).map { TendCloseOutcome.Skipped(requestID: "rq_\($0)", reason: "kept") }
        let text = FleetDecisions.closeAllText(TendCloseOutcome(closed: ["rq_0"], skipped: skipped))
        #expect(text.hasPrefix("Closed 1 question. Skipped 8: "))
        #expect((1...8).allSatisfy { text.contains("rq_\($0) (kept)") })
    }

    @Test func theGroupCountsExactlyUnderTruncation() {
        #expect(FleetCantAnswer.count(listed: 2, exact: 5, filtered: false) == 5)
        #expect(FleetCantAnswer.count(listed: 2, exact: 0, filtered: false) == 2)
        #expect(FleetCantAnswer.count(listed: 2, exact: 5, filtered: true) == 2)
        #expect(FleetCantAnswer.confirmTitle(3) == "Close 3 questions?")
        #expect(FleetCantAnswer.confirmTitle(1) == "Close 1 question?")
        #expect(FleetCantAnswer.confirmMessage == "Each asking agent gets a reply that you dismissed it. This can't be undone.")
    }

    @Test func anUnlistedAskerIsNoLongerFlaggedOnceFolded() throws {
        let queue = try JSONDecoder().decode(TendPendingQueue.self, from: Data(foldedQueueJSON.utf8))
        var decision = try #require(FleetDecisions.merge(queue: queue, reviews: [], messages: [], localHost: local)
            .first { $0.isUnanswerable })
        decision.askerUnlisted = true
        #expect(!decision.asksUnlisted(at: Date()), "the group says why; the hint is for live questions")
        decision.unanswerable = nil
        #expect(decision.asksUnlisted(at: Date()))
    }

    @Test func keepAndCloseAllSendExactPairsAndCloseAllNeedsTheSwitch() async throws {
        let sent = Sent()
        let client = TendIPCClient(socketPath: "/tmp/tend-cant-answer.sock") { _, payload in
            let object = try #require(JSONSerialization.jsonObject(with: payload) as? [String: Any])
            sent.add(object)
            switch object["kind"] as? String {
            case "hello":
                return try hello(["decision_queue_v1", "subject_check_v1", "close_unanswerable_v1", "decision_history_closures_v1"])
            case "decision_close_unanswerable":
                return try JSONSerialization.data(withJSONObject: ["ok": true, "decision_close_unanswerable": [
                    "closed": ["rq_a"], "skipped": [["request_id": "rq_b", "reason": "kept"]],
                ]])
            case "decision_history":
                return try JSONSerialization.data(withJSONObject: ["ok": true, "decision_history": ["items": [
                    ["kind": "closed_by_tend", "request_id": "rq_sys", "body": "Recover?", "from_session_id": "tend:peer-recovery",
                     "from_pane": "", "created_at": "2026-11-17T00:00:00Z", "system": true, "reason": "stale",
                     "reason_text": "stale: nobody left to act on it", "closed_by": "daemon", "closed_at": "2026-11-19T10:00:00Z"],
                    ["kind": "human_answer", "request_id": "rq_a", "body": "Deploy?", "from_session_id": "s", "from_pane": "%4",
                     "created_at": "2026-11-10T00:00:00Z", "status": "declined", "answer": "dismissed",
                     "answered_at": "2026-11-19T11:00:00Z",
                     "closure": ["reason": "subject_gone", "cause": "pane_gone", "episode": "ep_a", "by": "person",
                                 "surface": "mac", "at": "2026-11-19T11:00:00Z"]],
                ]]])
            default:
                return try JSONSerialization.data(withJSONObject: ["ok": true])
            }
        }
        try await client.hello()
        try await client.decisionKeep(requestID: "rq_a", episode: "ep_a")
        let keep = try #require(sent.all.last)
        #expect(keep["kind"] as? String == "decision_keep")
        #expect(keep["request_id"] as? String == "rq_a" && keep["episode"] as? String == "ep_a")
        #expect((keep["origin"] as? [String: Any])?["console"] as? Bool == true)

        let outcome = try await client.decisionCloseUnanswerable([("rq_a", "ep_a"), ("rq_b", "ep_b")])
        let close = try #require(sent.all.last)
        #expect(close["kind"] as? String == "decision_close_unanswerable")
        #expect(close["by_label"] as? String == "mac")
        let items = try #require(close["items"] as? [[String: String]])
        #expect(items == [["request_id": "rq_a", "episode": "ep_a"], ["request_id": "rq_b", "episode": "ep_b"]])
        #expect(outcome.closed == ["rq_a"])
        #expect(FleetDecisions.closeAllText(outcome) == "Closed 1 question. Skipped 1: rq_b (kept)")

        let history = try #require(try await client.decisionHistory())
        #expect(sent.all.last?["include_closures"] as? Bool == true)
        let rows = history.items.map(FleetDecidedRow.init)
        #expect(rows.map(\.label) == ["Closed by tend", "Closed"])
        #expect(rows[0].said == "Closed by tend: nobody is left to act on it")
        #expect(rows[1].said == "Closed, asker gone: Pane gone")

        // Without the switch the daemon does not advertise Close all, and
        // the client never sends it; an older daemon is not asked for
        // closures either.
        let older = TendIPCClient(socketPath: "/tmp/tend-cant-answer-old.sock") { _, payload in
            let object = try #require(JSONSerialization.jsonObject(with: payload) as? [String: Any])
            sent.add(object)
            if object["kind"] as? String == "hello" { return try hello(["decision_queue_v1", "subject_check_v1"]) }
            return try JSONSerialization.data(withJSONObject: ["ok": true, "decision_history": ["items": []]])
        }
        try await older.hello()
        let count = sent.all.count
        await #expect(throws: (any Error).self) { _ = try await older.decisionCloseUnanswerable([("rq_a", "ep_a")]) }
        #expect(sent.all.count == count, "nothing was sent")
        _ = try await older.decisionHistory()
        #expect(sent.all.last?["include_closures"] == nil)
    }
}
