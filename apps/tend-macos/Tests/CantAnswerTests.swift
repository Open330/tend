import Combine
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
        #expect(FleetDecisions.foldedQuestions(plain, localHost: "mac").live.isEmpty)
    }

    /// Another host's queue, read over Fleet, folds its questions here as
    /// it does there: through `loadRemote` and `withRemote` the folded
    /// entry replaces that host's mailbox copy (live in Your turn before),
    /// comes last, opens on the dashboard and is left off the badges; that
    /// host's review items are listed and counted (as tendd counts them)
    /// but, on a host Home does not decide them for, not acted on.
    /// This Mac's own folded question never opens there
    /// (docs/proposals/FLEET_UNANSWERABLE.md).
    @MainActor
    @Test func anotherHostsFoldedQuestionFoldsHereAndOpensOnTheDashboard() async throws {
        let remoteQueue = """
        {"items":[
          {"kind":"human_request","request_id":"rq_r","body":"Deploy there?","from_session_id":"s9","from_pane":"%9",
           "created_at":"2026-11-10T11:00:00Z",
           "unanswerable":{"cause":"ended","episode":"ep_r","since":"2026-11-16T00:00:00Z"}},
          {"kind":"review","id":"rv_1","target":{"kind":"session","provider":"claude","session_id":"s"},
           "generation":1,"summary":"Check it","requested_by":{"kind":"agent","label":"impl"}}
        ],"unanswerable_count":1}
        """
        let client = TendIPCClient(socketPath: "/tmp/tend-cant-answer-remote.sock") { _, payload in
            let object = try #require(JSONSerialization.jsonObject(with: payload) as? [String: Any])
            switch object["kind"] as? String {
            case "hello":
                var hello = try #require(JSONSerialization.jsonObject(with: try hello(["fleet_v1", "subject_check_v1"])) as? [String: Any])
                hello["dashboard_url"] = "https://tend.example/home"
                return try JSONSerialization.data(withJSONObject: hello)
            case "fleet_command":
                let operation = try #require(object["operation"] as? [String: Any])
                #expect(operation["kind"] as? String == "decision_queue")
                #expect(object["host"] as? String == "jiun-mini")
                let queueObject = try JSONSerialization.jsonObject(with: Data(remoteQueue.utf8))
                return try JSONSerialization.data(withJSONObject: ["ok": true, "fleet_result": [
                    "accepted": true, "decision_queue": queueObject,
                ]])
            default:
                return try JSONSerialization.data(withJSONObject: ["ok": true])
            }
        }
        try await client.hello()
        // An observe-only host that sends its queue: its folded questions
        // are read, its review items listed as observe-only.
        let mini = try JSONDecoder().decode(TendFleetHost.self, from: JSONSerialization.data(withJSONObject: [
            "alias": "jiun-mini", "local": false, "mode": "observe", "state": "online",
            "capabilities": ["fleet_decision_queue_v1"],
        ]))
        #expect(FleetDecisions.readsQueue(mini) && !FleetDecisions.decidesReviews(mini))
        let feed = FleetDecisionFeed()
        await feed.loadRemote(client, hosts: [mini])
        #expect(feed.remoteQueues.map(\.host.alias) == ["jiun-mini"])
        #expect(feed.reviewHosts.isEmpty)
        let dashboard = try #require(feed.dashboard)
        // Home polls this every 15 s: the same read again publishes nothing,
        // so it neither redraws Home nor rebuilds the merged decisions.
        var publishes = 0
        let watch = feed.objectWillChange.sink { publishes += 1 }
        await feed.loadRemote(client, hosts: [mini])
        #expect(publishes == 0)
        watch.cancel()

        // Its mailbox copy, which listed it as live before, and a live one.
        let copy = TendOperatorMessage(host: mini.identity, routePane: .operatorRoute, request: try message(id: "rq_r").request)
        let liveThere = TendOperatorMessage(host: mini.identity, routePane: .operatorRoute, request: try message(id: "rq_live2").request)
        let before = FleetDecisions.merge(queue: nil, reviews: [], messages: [copy, liveThere], localHost: local)
        #expect(before.filter { !$0.isUnanswerable }.map(\.id) == ["human:jiun-mini:rq_r", "human:jiun-mini:rq_live2"])
        let decisions = FleetDecisions.withRemote(before, queues: feed.remoteQueues, deciding: feed.reviewHosts)
        #expect(decisions.map(\.id) == ["human:jiun-mini:rq_live2", "review:jiun-mini:rv_1", "human:jiun-mini:rq_r"])
        #expect(decisions[1].observeOnly && decisions[1].decidedOnDashboard)
        let folded = try #require(decisions.last)
        #expect(folded.isUnanswerable && folded.unanswerable?.episode == "ep_r")
        #expect(FleetCantAnswer.dashboardLink(folded, on: dashboard)?.absoluteString
            == "https://tend.example/needs#item=rq_r&host=jiun-mini")
        #expect(FleetCantAnswer.dashboardLink(folded, on: nil) == nil)
        // Where Home decides its review items, they are listed too.
        let deciding = FleetDecisions.withRemote(before, queues: feed.remoteQueues, deciding: ["jiun-mini"])
        #expect(deciding.map(\.id) == ["human:jiun-mini:rq_live2", "review:jiun-mini:rv_1", "human:jiun-mini:rq_r"])
        #expect(!deciding[1].observeOnly)

        // The badges leave it out, as AppModel.refreshFoldedQuestions reads it.
        var foldedIDs = FleetDecisions.foldedQuestions(nil, localHost: "mac")
        foldedIDs.merge(FleetDecisions.foldedQuestions(feed.remoteQueues[0].queue, localHost: "jiun-mini"))
        let badge = TendAttentionSummary.make(agents: [], messages: [copy, liveThere], unread: [], folded: foldedIDs, paneKey: { $0.id })
        #expect(badge.decisions == 1)

        // The group's count: each host's exact count, never less than it
        // lists. A remote page cut before any folded row still counts them
        // (and its omitted mailbox copies stay out of Your turn); with this
        // Mac's, the hosts add up; a host filter keeps that host's.
        let cutJSON = #"{"items":[{"kind":"human_request","request_id":"rq_x","body":"?","from_session_id":"s","from_pane":"%1","created_at":"2026-11-10T11:00:00Z"}],"truncated":true,"unanswerable_count":2}"#
        let cut = try JSONDecoder().decode(TendPendingQueue.self, from: Data(cutJSON.utf8))
        let omittedCopy = TendOperatorMessage(host: mini.identity, routePane: .operatorRoute, request: try message(id: "rq_gone").request)
        let cutDecisions = FleetDecisions.withRemote(
            FleetDecisions.merge(queue: nil, reviews: [], messages: [omittedCopy], localHost: local),
            queues: [(mini.identity, cut)], deciding: []
        )
        #expect(cutDecisions.map(\.id) == ["human:jiun-mini:rq_x"],
                "the omitted copy may be folded there, so not live here; what the page lists live counts once")
        let exact = ["mac": 5, "jiun-mini": 2]
        #expect(FleetCantAnswer.count(listed: [], exact: exact, host: "jiun-mini", filtered: false) == 2)
        let localQueue = try JSONDecoder().decode(TendPendingQueue.self, from: Data(foldedQueueJSON.utf8))
        let ourFolded = FleetDecisions.merge(queue: localQueue, reviews: [], messages: [], localHost: local).filter(\.isUnanswerable)
        let mixed = Array(ourFolded.prefix(1)) + decisions.filter(\.isUnanswerable)
        #expect(FleetCantAnswer.count(listed: Array(mixed), exact: exact, host: nil, filtered: false) == 7)
        #expect(FleetCantAnswer.count(listed: Array(mixed), exact: exact, host: "mac", filtered: false) == 5)
        // Listed rows stand in for a reader without exact counts; a project
        // filter counts only what is listed.
        #expect(FleetCantAnswer.count(listed: Array(mixed), exact: [:], host: nil, filtered: false) == 2)
        #expect(FleetCantAnswer.count(listed: Array(mixed), exact: exact, host: nil, filtered: true) == 2)

        // This Mac's own folded question is kept here, never opened there.
        let queue = try JSONDecoder().decode(TendPendingQueue.self, from: Data(foldedQueueJSON.utf8))
        let ours = FleetDecisions.merge(queue: queue, reviews: [], messages: [], localHost: local).filter(\.isUnanswerable)
        #expect(FleetCantAnswer.dashboardLink(ours[0], on: dashboard) == nil)
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

// Home's numbers never pass a queue not read whole off as everything
// (apps/shared/Design/COUNTS.md `coverage`), and the one schedule that
// reads the shared decision feed coalesces and never lets a stale read win.

private func fleetHost(_ alias: String, state: String = "online", mode: String = "control", capabilities: [String]) throws -> TendFleetHost {
    try JSONDecoder().decode(TendFleetHost.self, from: JSONSerialization.data(withJSONObject: [
        "alias": alias, "local": false, "mode": mode, "state": state, "capabilities": capabilities,
    ]))
}

private func pendingQueue(_ json: String) throws -> TendPendingQueue {
    try JSONDecoder().decode(TendPendingQueue.self, from: Data(json.utf8))
}

@Suite struct DecisionCoverageTests {
    @MainActor
    @Test func aStaleRemoteReadNeverReplacesANewerOne() throws {
        let mini = try fleetHost("jiun-mini", capabilities: ["fleet_decision_queue_v1"])
        let newer = try pendingQueue(#"{"items":[{"kind":"review","id":"rv_new","target":{"kind":"session","provider":"claude","session_id":"s"},"generation":1,"summary":"New","requested_by":{"kind":"agent","label":"impl"}}]}"#)
        let older = try pendingQueue(#"{"items":[],"truncated":true}"#)
        let feed = FleetDecisionFeed()
        let first = feed.beginRemote(), second = feed.beginRemote()
        #expect(feed.apply(hosts: [mini], read: [mini.identity], found: [0: newer], generation: second))
        #expect(!feed.apply(hosts: [mini], read: [mini.identity], found: [0: older], generation: first),
                "the read that started first finished last: dropped")
        #expect(feed.remoteQueues.first?.queue == newer)
        #expect(feed.partialHosts.isEmpty)
    }

    @MainActor
    @Test func aHostWhoseQueueWasNotReadWholeIsPartial() throws {
        let read = try fleetHost("read", capabilities: ["fleet_decision_queue_v1"])
        let failed = try fleetHost("failed", capabilities: ["fleet_decision_queue_v1"])
        let cut = try fleetHost("cut", capabilities: ["fleet_decision_queue_v1"])
        let offline = try fleetHost("offline", state: "offline", capabilities: ["fleet_decision_queue_v1"])
        let older = try fleetHost("older", capabilities: [])
        let hosts = [read, failed, cut, offline, older]
        let queued = hosts.filter(FleetDecisions.readsQueue)
        #expect(queued.map(\.alias) == ["read", "failed", "cut"])
        let feed = FleetDecisionFeed()
        feed.apply(
            hosts: hosts, read: queued.map(\.identity),
            found: [0: try pendingQueue(#"{"items":[]}"#), 2: try pendingQueue(#"{"items":[],"truncated":true}"#)],
            generation: feed.beginRemote()
        )
        #expect(feed.partialHosts == ["failed", "cut", "offline", "older"])
        #expect(feed.isPartial(host: nil, localHost: "mac"))
        #expect(!feed.isPartial(host: "read", localHost: "mac"))
        #expect(feed.isPartial(host: "offline", localHost: "mac"))
        #expect(!feed.isPartial(host: "mac", localHost: "mac"))
        // Every host read whole: complete again.
        feed.apply(hosts: [read], read: [read.identity], found: [0: try pendingQueue(#"{"items":[]}"#)], generation: feed.beginRemote())
        #expect(!feed.isPartial(host: nil, localHost: "mac"))
    }

    @MainActor
    @Test func thisMacsQueueNotReadWholeStaysPartial() async throws {
        let client = TendIPCClient(socketPath: "/tmp/tend-coverage.sock") { _, payload in
            let object = try #require(JSONSerialization.jsonObject(with: payload) as? [String: Any])
            if object["kind"] as? String == "hello" { return try hello(["decision_queue_v1", "subject_check_v1"]) }
            return try JSONSerialization.data(withJSONObject: ["ok": true])
        }
        try await client.hello()
        let feed = FleetDecisionFeed()
        // Cut with no folded rows: the mailbox fills in (no warning), but
        // the number may still leave some out.
        await feed.ingestLocal(try pendingQueue(#"{"items":[],"truncated":true}"#), client: client, generation: feed.beginLocal())
        #expect(feed.problem == nil)
        #expect(feed.localPartial)
        var undecoded = try pendingQueue(#"{"items":[]}"#)
        undecoded.undecoded = 1
        await feed.ingestLocal(undecoded, client: client, generation: feed.beginLocal())
        #expect(feed.localPartial, "an entry this app could not read")
        let stale = feed.beginLocal(), fresh = feed.beginLocal()
        await feed.ingestLocal(try pendingQueue(#"{"items":[]}"#), client: client, generation: fresh)
        #expect(!feed.localPartial)
        await feed.ingestLocal(try pendingQueue(#"{"items":[],"truncated":true}"#), client: client, generation: stale)
        #expect(!feed.localPartial, "a read that started earlier never replaces a later one")
    }

    @Test func theScheduleReadsOnEachPaceAndOnceMoreAfterAnEvent() {
        let start = Date(timeIntervalSince1970: 1_000_000)
        var schedule = FleetDecisionSchedule()
        #expect(schedule.start(at: start).reads.isEmpty, "paused: nothing on screen reads")
        schedule.request(.local)
        #expect(schedule.start(at: start).reads.isEmpty, "an event while paused waits for the screen")
        schedule.pace = .background
        let first = schedule.start(at: start)
        #expect(first.reads == [.local, .remote], "shown: one read of each at once")
        #expect(first.next == nil, "both in flight")
        // Events during the read: one more read after it, not one each.
        schedule.request(.local)
        schedule.request(.local)
        #expect(schedule.start(at: start.addingTimeInterval(1)).reads.isEmpty)
        schedule.finish(.local)
        schedule.finish(.remote)
        #expect(schedule.due(.local) == start.addingTimeInterval(FleetDecisionSchedule.minimumGap))
        #expect(schedule.due(.remote) == start.addingTimeInterval(60))
        let trailing = schedule.start(at: start.addingTimeInterval(2))
        #expect(trailing.reads == [.local])
        schedule.finish(.local)
        #expect(schedule.due(.local) == start.addingTimeInterval(32), "then the background period")
        // A Home on screen reads faster; another reader's read counts.
        schedule.pace = .home
        #expect(schedule.due(.local) == start.addingTimeInterval(7))
        schedule.noteRead(.remote, at: start.addingTimeInterval(10))
        #expect(schedule.due(.remote) == start.addingTimeInterval(25))
    }
}
