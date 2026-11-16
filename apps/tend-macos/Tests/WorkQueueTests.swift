import Combine
import Foundation
import Testing
@testable import Tend

/// A `work_queue_list` reply as tendd writes it, plus fields, a state, a
/// wait reason, a fail reason and a target kind this app does not know, and
/// one item it cannot read at all.
private let queueReplyJSON = """
{"ok":true,"work_queue":{
  "revision":42,"evaluated_at":"2026-11-16T03:00:05Z","future_field":{"x":1},
  "fences":[{"session_id":"s-1","pane":"%3","item_id":"q-2","attempt":"a1","kind":"unresolved",
             "idle_stamp":{"state_entered_at":"2026-11-16T02:00:00Z"},"since":"2026-11-16T02:59:00Z"}],
  "items":[
    {"id":"q-3","revision":2,"prompt":"Second in line\\nmore","target":{"kind":"agent","session_id":"0123456789abcdef"},
     "created_by":{"kind":"operator"},"created_at":"2026-11-16T02:00:00Z","updated_at":"2026-11-16T02:00:00Z",
     "state":{"state":"waiting"},"retry":{},"position":1,"wait":"behind"},
    {"id":"q-1","revision":5,"title":"Add CSV export","prompt":"Export the review history","created_by":{"kind":"operator"},
     "target":{"kind":"new_work","request":{"work":"csv-export","workspace":"tend","cwd":"/src/tend","pipeline":"implement-review","new_only":true}},
     "created_at":"2026-11-16T01:00:00Z","updated_at":"2026-11-16T01:30:00Z",
     "state":{"state":"waiting"},"retry":{"refusals":1},"position":0,"wait":"no_slot","shiny":true},
    {"id":"q-2","revision":7,"prompt":"Run the tests","target":{"kind":"work","identity":{"workspace_id":"tend","work_id":"ci"},"alias":"impl"},
     "created_by":{"kind":"agent","session_id":"s-9"},"created_at":"2026-11-16T01:00:00Z","updated_at":"2026-11-16T02:59:00Z",
     "state":{"state":"uncertain","attempt":"a1","at":"2026-11-16T02:59:00Z","reason":"the pane closed while typing"},
     "retry":{},"position":2},
    {"id":"q-4","revision":1,"prompt":"Later","target":{"kind":"satellite","orbit":3},
     "created_by":{"kind":"operator"},"created_at":"2026-11-16T01:00:00Z","updated_at":"2026-11-16T01:00:00Z",
     "state":{"state":"hovering","since":"now"},"retry":{},"position":3,"wait":"moon_phase"},
    {"id":"q-5","revision":3,"prompt":"Done already","target":{"kind":"agent","session_id":"s-2"},
     "created_by":{"kind":"operator"},"created_at":"2026-11-16T01:00:00Z","updated_at":"2026-11-16T01:00:00Z",
     "state":{"state":"failed","at":"2026-11-16T01:10:00Z","reason":"target_vanished_in_space","detail":"gone"},"retry":{}},
    {"revision":"not a number"}
  ]}}
"""

private struct Reply: Decodable {
    let work_queue: TendWorkQueue
}

private func decodedQueue() throws -> TendWorkQueue {
    try JSONDecoder().decode(Reply.self, from: Data(queueReplyJSON.utf8)).work_queue
}

private final class QueueDaemon: @unchecked Sendable {
    private let lock = NSLock()
    private var requests: [[String: Any]] = []
    var capabilities = ["session_bytes_v1", "session_attachment_identity_v1", "work_control_v1", "work_queue_v1"]

    var sent: [[String: Any]] { lock.lock(); defer { lock.unlock() }; return requests }

    func handle(_ payload: Data) throws -> Data {
        let object = try #require(JSONSerialization.jsonObject(with: payload) as? [String: Any])
        lock.lock()
        requests.append(object)
        let capabilities = capabilities
        lock.unlock()
        switch object["kind"] as? String {
        case "hello":
            return try JSONSerialization.data(withJSONObject: [
                "ok": true, "min_protocol": 1, "max_protocol": 6, "capabilities": capabilities,
            ])
        case "work_queue_list":
            return Data(queueReplyJSON.utf8)
        case "work_queue_move":
            return try JSONSerialization.data(withJSONObject: ["ok": false, "error": "item q-3 changed since revision 2"])
        default:
            return try JSONSerialization.data(withJSONObject: ["ok": true])
        }
    }
}

@Suite struct WorkQueueTests {
    @Test func aRealisticListDecodesLenientlyAndInOrder() throws {
        let queue = try decodedQueue()
        #expect(queue.revision == 42)
        #expect(queue.broken == nil && !queue.unsaved)
        // The unreadable item is dropped; the rest keep the daemon's order.
        #expect(queue.items.map(\.id) == ["q-3", "q-1", "q-2", "q-4", "q-5"])

        let newWork = try #require(queue.items.first { $0.id == "q-1" })
        #expect(newWork.revision == 5)
        #expect(newWork.displayTitle == "Add CSV export")
        #expect(newWork.target == .newWork(work: "csv-export", workspace: "tend", cwd: "/src/tend", pipeline: "implement-review"))
        #expect(newWork.wait == .noSlot && newWork.position == 0)

        let agent = try #require(queue.items.first { $0.id == "q-3" })
        #expect(agent.displayTitle == "Second in line")
        #expect(agent.target == .agent(sessionID: "0123456789abcdef", label: nil))

        let uncertain = try #require(queue.items.first { $0.id == "q-2" })
        #expect(uncertain.state == .uncertain(reason: "the pane closed while typing"))
        #expect(uncertain.isUncertain)
        #expect(uncertain.target == .work(TendWorkIdentity(workspaceID: "tend", workID: "ci"), alias: "impl"))

        // Unknown kinds, states and reasons read as themselves.
        let future = try #require(queue.items.first { $0.id == "q-4" })
        #expect(future.target == .other("satellite"))
        #expect(future.state == .other("hovering"))
        #expect(future.wait?.rawValue == "moon_phase")
        let failed = try #require(queue.items.first { $0.id == "q-5" })
        #expect(failed.state == .failed(reason: .init(rawValue: "target_vanished_in_space"), detail: "gone"))

        // Home lists the unfinished ones by position.
        #expect(queue.pending.map(\.id) == ["q-1", "q-3", "q-2", "q-4"])
    }

    @Test func statusAndWaitAreWordedForPeople() throws {
        let queue = try decodedQueue()
        let byID = Dictionary(uniqueKeysWithValues: queue.items.map { ($0.id, $0) })
        let newWork = try #require(byID["q-1"])
        #expect(newWork.statusText == "Waiting")
        #expect(newWork.detailText == "Waiting for a free slot: too many agents are busy")
        #expect(newWork.targetText == "New Work · csv-export · tend · implement-review")

        let agent = try #require(byID["q-3"])
        #expect(agent.detailText == "Waiting behind an earlier item for the same agent")
        #expect(agent.targetText == "Agent 01234567")

        let uncertain = try #require(byID["q-2"])
        #expect(uncertain.statusText == "Uncertain")
        #expect(uncertain.detailText == "May already have been sent — check the agent")
        #expect(uncertain.targetText == "tend/ci · impl")

        // An unknown wait still reads as waiting; an unknown fail reason
        // shows tendd's own detail.
        #expect(try #require(byID["q-4"]).detailText == nil)
        let unknownWait = TendQueueItem(id: "w", prompt: "p", target: .agent(sessionID: "s", label: nil), wait: .init(rawValue: "moon_phase"))
        #expect(unknownWait.detailText == "Waiting")
        #expect(try #require(byID["q-4"]).statusText == "Unknown")
        #expect(try #require(byID["q-5"]).detailText == "gone")

        #expect(TendQueueItem.Wait.agentBusy.text == "Waiting for the agent to finish its turn")
        #expect(TendQueueItem.Wait.storeUnsaved.text == "Paused: tendd could not save the queue")
        #expect(TendQueueItem.FailReason.identityInUse.text == "That Work ID is already in use")

        let starting = TendQueueItem(id: "x", prompt: "p", target: .newWork(work: nil, workspace: nil, cwd: nil, pipeline: nil), state: .dispatching)
        #expect(starting.statusText == "Starting")
        let sending = TendQueueItem(id: "y", prompt: "p", target: .agent(sessionID: "s", label: "impl"), state: .dispatching)
        #expect(sending.statusText == "Sending")
        #expect(sending.targetText == "Agent impl")
    }

    @Test func changesSendTheDocumentedRequests() throws {
        let item = TendQueueItem(id: "q-1", revision: 5, prompt: "p", target: .agent(sessionID: "s", label: nil))
        let edit = TendQueueChange.edit(prompt: "new").request(for: item)
        #expect(edit["kind"] as? String == "work_queue_edit")
        #expect(edit["expected_revision"] as? UInt64 == 5)
        #expect((edit["edit"] as? [String: Any])?["prompt"] as? String == "new")
        let move = TendQueueChange.move(to: 2).request(for: item)
        #expect(move["kind"] as? String == "work_queue_move" && move["to"] as? Int == 2)
        let again = TendQueueChange.runNow(allowDuplicate: true).request(for: item)
        #expect(again["kind"] as? String == "work_queue_run_now" && again["allow_duplicate"] as? Bool == true)
        let notSent = TendQueueChange.resolve(sent: false, ackDuplicateRisk: true).request(for: item)
        #expect(notSent["kind"] as? String == "work_queue_resolve")
        #expect(notSent["sent"] as? Bool == false && notSent["ack_duplicate_risk"] as? Bool == true)
        #expect(notSent["expected_revision"] == nil)
        let remove = TendQueueChange.remove(ackDuplicateRisk: false).request(for: item)
        #expect(remove["kind"] as? String == "work_queue_remove" && remove["ack_duplicate_risk"] as? Bool == false)
        #expect(TendQueueChange.cancel.request(for: item)["kind"] as? String == "work_queue_cancel")

        let add = TendQueueNewWork(
            requestKey: "k-1", prompt: "Do it", work: nil, workspace: "", cwd: "/src/tend",
            pipeline: "implement-review", external: " ", skill: nil, context: nil
        ).payload
        #expect(add["request_key"] as? String == "k-1")
        #expect(add["title"] == nil)
        let target = try #require(add["target"] as? [String: Any])
        #expect(target["kind"] as? String == "new_work" && target["cwd"] as? String == "/src/tend")
        #expect(target["pipeline"] as? String == "implement-review")
        // Blank fields are left out, so tendd picks its own Work id.
        #expect(target["work"] == nil && target["workspace"] == nil && target["external"] == nil)
        // A fresh key per click.
        #expect(TendQueueNewWork(prompt: "a", cwd: "/").requestKey != TendQueueNewWork(prompt: "a", cwd: "/").requestKey)
    }

    @MainActor
    @Test func theQueueClientReadsTheListAndSurfacesRefusals() async throws {
        let daemon = QueueDaemon()
        // A daemon without `work_queue_v1` has no queue: Home hides it.
        daemon.capabilities.removeLast()
        let old = TendIPCClient(socketPath: "/tmp/tend-queue-\(UUID().uuidString).sock") { _, payload in
            try daemon.handle(payload)
        }
        try await old.hello()
        #expect(try await old.workQueue() == nil)
        #expect(!daemon.sent.contains { $0["kind"] as? String == "work_queue_list" })

        let feed = FleetQueueFeed(center: NotificationCenter())
        // The queue client has its own connection; the test reads through
        // its seam.
        let queueClient = TendWorkQueueClient(socketPath: "/tmp/tend-queue-test.sock") { _, payload in
            try daemon.handle(payload)
        }
        let queue = try await queueClient.list()
        feed.apply(queue)
        #expect(feed.shown.map(\.id) == ["q-1", "q-3", "q-2", "q-4"])
        // A refused change surfaces tendd's own error text.
        let refused = feed.shown[1]
        do {
            try await queueClient.change(.move(to: 0), on: refused)
            Issue.record("the move should have been refused")
        } catch {
            #expect(error.localizedDescription.contains("changed since revision 2"))
        }
        #expect(daemon.sent.contains { $0["kind"] as? String == "work_queue_list" && $0["include_finished"] as? Bool == false })
    }

    @MainActor
    @Test func theFeedDoesNotRepublishAnIdenticalSnapshot() throws {
        let feed = FleetQueueFeed(center: NotificationCenter())
        var publishes = 0
        let watch = feed.objectWillChange.sink { publishes += 1 }
        defer { watch.cancel() }

        let first = try decodedQueue()
        feed.apply(first)
        let afterFirst = publishes
        #expect(afterFirst > 0)

        // The same queue read again — even with a later `evaluated_at` and
        // other fences — publishes nothing.
        let again = queueReplyJSON
            .replacingOccurrences(of: "2026-11-16T03:00:05Z", with: "2026-11-16T03:00:15Z")
            .replacingOccurrences(of: "\"kind\":\"unresolved\"", with: "\"kind\":\"delivered\"")
        let second = try JSONDecoder().decode(Reply.self, from: Data(again.utf8)).work_queue
        feed.apply(second)
        feed.apply(first)
        #expect(publishes == afterFirst)

        // A real change does.
        let moved = TendWorkQueue(items: Array(first.items.dropFirst()), revision: 43)
        feed.apply(moved)
        #expect(publishes > afterFirst)

        // A daemon without the queue hides it.
        feed.apply(nil)
        #expect(feed.queue == nil && feed.shown.isEmpty)
    }
}

// MARK: - Add to Queue

/// Holds a request open until the test lets it go.
private actor Gate {
    private var continuation: CheckedContinuation<Void, Never>?
    private var opened = false

    func wait() async {
        if opened { return }
        await withCheckedContinuation { continuation = $0 }
    }

    func open() {
        opened = true
        continuation?.resume()
        continuation = nil
    }
}

private func ready(_ plan: WorkQueueForm.Plan) throws -> TendQueueNewWork {
    guard case .ready(let work) = plan else {
        Issue.record("expected a ready plan, got \(plan)")
        throw CancellationError()
    }
    return work
}

private let engineering = TendWorkOptions(
    configured: true,
    routes: [
        TendWorkOptions.Route(match: "^ENG-", workspace: "engineering", pipeline: "special"),
        TendWorkOptions.Route(match: "^WT-", workspace: "trees", worktree: true),
        TendWorkOptions.Route(match: "^TPL-", workspace: "{{work}}", prepare: true),
    ],
    pipelines: [
        TendWorkOptions.Pipeline(name: "default", agents: []),
        TendWorkOptions.Pipeline(name: "special", agents: []),
    ]
)

@Suite struct WorkQueueSubmitTests {
    @Test func aQueuedWorkRoutesLikeThePreview() throws {
        // Blank form fields: the route supplies the pipeline and workspace,
        // and routes match the previewed id, not tendd's reserved one.
        let form = WorkQueueForm(prompt: "Ship it", work: "ENG-42", routeKey: "ENG-42", folder: "/src/eng")
        let work = try ready(form.plan(options: engineering))
        #expect(work.routeKey == "ENG-42" && work.work == "ENG-42")
        #expect(work.pipeline == "special" && work.workspace == "engineering")
        #expect(work.cwd == "/src/eng")
        let target = try #require(work.payload["target"] as? [String: Any])
        #expect(target["route_key"] as? String == "ENG-42")
        #expect(target["pipeline"] as? String == "special")
        #expect(target["workspace"] as? String == "engineering")

        // Explicit choices win over the route's.
        var chosen = form
        chosen.pipeline = "default"
        chosen.workspace = "mine"
        let explicit = try ready(chosen.plan(options: engineering))
        #expect(explicit.pipeline == "default" && explicit.workspace == "mine")

        // A made-up id from the task routes too; no typed id is sent.
        let untyped = try ready(WorkQueueForm(prompt: "p", routeKey: "ENG-made-up", folder: "~/x").plan(options: engineering))
        #expect(untyped.work == nil && untyped.routeKey == "ENG-made-up")
        #expect(untyped.cwd?.hasPrefix("/") == true)
    }

    @Test func aRouteThatProvidesTheFolderSendsNoCwd() throws {
        let worktree = try ready(WorkQueueForm(prompt: "p", routeKey: "WT-1").plan(options: engineering))
        #expect(worktree.cwd == nil && worktree.workspace == "trees")
        let target = try #require(worktree.payload["target"] as? [String: Any])
        #expect(target["cwd"] == nil)

        // A templated workspace is not known here, so a folder is needed.
        #expect(WorkQueueForm(prompt: "p", routeKey: "TPL-1").plan(options: engineering) == .noFolder)
        // ...unless one is typed.
        #expect(try ready(WorkQueueForm(prompt: "p", routeKey: "TPL-1", workspace: "w").plan(options: engineering)).cwd == nil)
        // No folder and no such route: nothing to start in.
        #expect(WorkQueueForm(prompt: "p", routeKey: "ENG-1").plan(options: engineering) == .noFolder)
        #expect(WorkQueueForm(prompt: "p", routeKey: "X", folder: "/a").plan(options: nil) != .noFolder)
        #expect(WorkQueueForm(prompt: "p", routeKey: "X", folder: "rel/dir").plan(options: nil) == .relativeFolder)
        #expect(WorkQueueForm(prompt: " ", routeKey: "X", folder: "/a").plan(options: nil) == .noPrompt)
    }

    @MainActor
    @Test func startWorkWaitsWhileAnAddIsInFlight() async throws {
        let submitter = WorkStartSubmitter()
        let gate = Gate()
        let draft = try ready(WorkQueueForm(prompt: "p", routeKey: "X", folder: "/a").plan(options: nil))
        let queueing = Task { @MainActor in
            await submitter.queue(draft) { _ in await gate.wait() }
        }
        while !submitter.isQueueing { await Task.yield() }

        var launches = 0
        let started = await submitter.start { launches += 1; return true }
        #expect(!started && launches == 0)
        // A second add of the same thing is refused too.
        let again = await submitter.queue(draft) { _ in Issue.record("a second add went out") }
        #expect(!again)

        await gate.open()
        #expect(await queueing.value)
        // A start the app is already running elsewhere blocks a queue.
        let duringStart = await submitter.queue(draft, busy: true) { _ in Issue.record("queued during a start") }
        #expect(!duringStart)
        #expect(await submitter.start { launches += 1; return true })
        #expect(launches == 1)
    }

    @MainActor
    @Test func aRetryAfterALostReplyAddsOnlyOnce() async throws {
        let submitter = WorkStartSubmitter()
        // A daemon that keys adds by request_key, as tendd does, and whose
        // first reply is lost after the add committed.
        var stored: [String: TendQueueNewWork] = [:]
        var dropNextReply = true
        let add: (TendQueueNewWork) async throws -> Void = { request in
            if stored[request.requestKey] == nil { stored[request.requestKey] = request }
            if dropNextReply {
                dropNextReply = false
                throw TendIPCError.posix(operation: "read", code: ECONNRESET)
            }
        }
        let form = WorkQueueForm(prompt: "Ship it", routeKey: "ENG-42", folder: "/src")

        // Each click plans afresh, with a fresh key, as the view does.
        func click(_ form: WorkQueueForm, _ add: (TendQueueNewWork) async throws -> Void) async throws -> Bool {
            let draft = try ready(form.plan(options: engineering))
            return await submitter.queue(draft, add: add)
        }
        #expect(try await click(form, add) == false)
        let error = try #require(submitter.queueError)
        #expect(error.contains("It may have been queued; Add to Queue again will not add it twice."))
        #expect(try await click(form, add))
        #expect(stored.count == 1)
        #expect(submitter.queueError == nil && submitter.pendingAdd == nil)

        // After a success, the same form is a new intent with a new key.
        #expect(try await click(form, add))
        #expect(stored.count == 2)

        // An edit after a failure is a new request too.
        dropNextReply = true
        #expect(try await click(form, add) == false)
        var edited = form
        edited.prompt = "Ship it today"
        #expect(try await click(edited, add))
        #expect(stored.count == 4)

        // tendd's own refusal is final: no "may have been queued".
        let refuse: (TendQueueNewWork) async throws -> Void = { _ in throw TendIPCError.server("cwd must be absolute") }
        #expect(try await click(form, refuse) == false)
        #expect(submitter.queueError == "cwd must be absolute")
    }

    @MainActor
    @Test func queueProblemsShowWithoutRows() async throws {
        struct Down: Error, LocalizedError { var errorDescription: String? { "tendd is not running" } }
        let feed = FleetQueueFeed(center: NotificationCenter())
        await feed.load { throw Down() }
        #expect(feed.queue == nil && feed.shown.isEmpty)
        #expect(feed.problem == "Could not read the queue: tendd is not running")

        await feed.load { TendWorkQueue(items: [], revision: 1) }
        #expect(feed.problem == nil)

        await feed.load { TendWorkQueue(items: [], revision: 2, broken: "bad JSON at line 3") }
        #expect(feed.shown.isEmpty)
        #expect(feed.problem?.contains("bad JSON at line 3") == true)

        await feed.load { TendWorkQueue(items: [], revision: 3, unsaved: true) }
        #expect(feed.problem == "tendd could not save the queue; nothing new goes until it can")

        // A daemon without the queue shows nothing at all.
        await feed.load { nil }
        #expect(feed.problem == nil && feed.queue == nil)
    }
}
