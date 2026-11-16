// The daemon's Work queue (`work_queue_v1`): prompts and new Work that tendd
// sends on its own, at most once, when their target is ready. The models
// decode leniently, so a newer tendd's fields, states and reasons never fail
// the read; the client speaks the documented requests on its own connection.
import Foundation

/// Daemons that serve the `work_queue_*` requests.
let tendWorkQueueCapability = "work_queue_v1"

/// One queued item as `work_queue_list` returns it (the queue item with its
/// place and wait reason flattened in).
struct TendQueueItem: Decodable, Hashable, Identifiable, Sendable {
    /// Why a waiting item has not gone yet. A string so an unknown reason
    /// from a newer tendd still decodes.
    struct Wait: RawRepresentable, Hashable, Sendable {
        let rawValue: String
        static let agentBusy = Wait(rawValue: "agent_busy")
        static let agentNeedsYou = Wait(rawValue: "agent_needs_you")
        static let agentRateLimited = Wait(rawValue: "agent_rate_limited")
        static let agentError = Wait(rawValue: "agent_error")
        static let agentMissing = Wait(rawValue: "agent_missing")
        static let agentAmbiguous = Wait(rawValue: "agent_ambiguous")
        static let workAmbiguous = Wait(rawValue: "work_ambiguous")
        static let draining = Wait(rawValue: "draining")
        static let reserved = Wait(rawValue: "reserved")
        static let behind = Wait(rawValue: "behind")
        static let noSlot = Wait(rawValue: "no_slot")
        static let launching = Wait(rawValue: "launching")
        static let retryLater = Wait(rawValue: "retry_later")
        static let storeUnsaved = Wait(rawValue: "store_unsaved")
    }

    /// Why an item failed; unknown reasons decode as themselves.
    struct FailReason: RawRepresentable, Hashable, Sendable {
        let rawValue: String
        static let targetGone = FailReason(rawValue: "target_gone")
        static let targetNotWorker = FailReason(rawValue: "target_not_worker")
        static let paneRefusedInput = FailReason(rawValue: "pane_refused_input")
        static let launchRefused = FailReason(rawValue: "launch_refused")
        static let identityInUse = FailReason(rawValue: "identity_in_use")
        static let notLocal = FailReason(rawValue: "not_local")
    }

    enum Target: Hashable, Sendable {
        case agent(sessionID: String, label: String?)
        case work(TendWorkIdentity, alias: String?)
        case newWork(work: String?, workspace: String?, cwd: String?, pipeline: String?)
        /// A target kind this app does not know.
        case other(String)
    }

    enum State: Hashable, Sendable {
        case waiting
        /// Claimed: the send or launch may be under way.
        case dispatching
        case dispatched
        /// The attempt may or may not have reached its target; tendd never
        /// repeats it without a person.
        case uncertain(reason: String)
        case failed(reason: FailReason, detail: String)
        case cancelled
        /// A state this app does not know.
        case other(String)

        var isFinished: Bool {
            switch self {
            case .dispatched, .failed, .cancelled: true
            default: false
            }
        }
    }

    let id: String
    /// Bumped by every change; edits, moves and run-now name it.
    let revision: UInt64
    let title: String?
    let prompt: String
    let target: Target
    let state: State
    let createdAt: String?
    /// 0-based place among the items not finished.
    let position: Int?
    let wait: Wait?

    init(
        id: String,
        revision: UInt64 = 1,
        title: String? = nil,
        prompt: String,
        target: Target,
        state: State = .waiting,
        createdAt: String? = nil,
        position: Int? = nil,
        wait: Wait? = nil
    ) {
        self.id = id
        self.revision = revision
        self.title = title
        self.prompt = prompt
        self.target = target
        self.state = state
        self.createdAt = createdAt
        self.position = position
        self.wait = wait
    }

    private enum CodingKeys: String, CodingKey {
        case id, revision, title, prompt, target, state, position, wait
        case createdAt = "created_at"
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        id = try values.decode(String.self, forKey: .id)
        revision = (try? values.decodeIfPresent(UInt64.self, forKey: .revision)) ?? 0
        title = try? values.decodeIfPresent(String.self, forKey: .title)
        prompt = (try? values.decodeIfPresent(String.self, forKey: .prompt)) ?? ""
        target = (try? values.decodeIfPresent(TargetWire.self, forKey: .target))?.value ?? .other("")
        state = (try? values.decodeIfPresent(StateWire.self, forKey: .state))?.value ?? .other("")
        createdAt = try? values.decodeIfPresent(String.self, forKey: .createdAt)
        position = try? values.decodeIfPresent(Int.self, forKey: .position)
        wait = (try? values.decodeIfPresent(String.self, forKey: .wait)).map(Wait.init(rawValue:))
    }

    /// The title, else the prompt's first line.
    var displayTitle: String {
        if let title = title?.trimmingCharacters(in: .whitespacesAndNewlines), !title.isEmpty { return title }
        return prompt.split(whereSeparator: \.isNewline)
            .lazy.map { $0.trimmingCharacters(in: .whitespaces) }
            .first { !$0.isEmpty } ?? id
    }

    var isUncertain: Bool {
        if case .uncertain = state { return true }
        return false
    }
}

// MARK: - Wire shapes

private struct TargetWire: Decodable {
    let value: TendQueueItem.Target

    private enum CodingKeys: String, CodingKey {
        case kind, label, identity, alias, request
        case sessionID = "session_id"
    }

    private struct Request: Decodable {
        let work: String?
        let workspace: String?
        let cwd: String?
        let pipeline: String?
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        let kind = try values.decode(String.self, forKey: .kind)
        switch kind {
        case "agent":
            value = .agent(
                sessionID: try values.decode(String.self, forKey: .sessionID),
                label: try? values.decodeIfPresent(String.self, forKey: .label)
            )
        case "work":
            value = .work(
                try values.decode(TendWorkIdentity.self, forKey: .identity),
                alias: try? values.decodeIfPresent(String.self, forKey: .alias)
            )
        case "new_work":
            let request = try? values.decodeIfPresent(Request.self, forKey: .request)
            value = .newWork(
                work: request?.work,
                workspace: request?.workspace,
                cwd: request?.cwd,
                pipeline: request?.pipeline
            )
        default:
            value = .other(kind)
        }
    }
}

private struct StateWire: Decodable {
    let value: TendQueueItem.State

    private enum CodingKeys: String, CodingKey {
        case state, reason, detail
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        let state = try values.decode(String.self, forKey: .state)
        switch state {
        case "waiting": value = .waiting
        case "dispatching": value = .dispatching
        case "dispatched": value = .dispatched
        case "uncertain":
            value = .uncertain(reason: (try? values.decodeIfPresent(String.self, forKey: .reason)) ?? "")
        case "failed":
            value = .failed(
                reason: .init(rawValue: (try? values.decodeIfPresent(String.self, forKey: .reason)) ?? ""),
                detail: (try? values.decodeIfPresent(String.self, forKey: .detail)) ?? ""
            )
        case "cancelled": value = .cancelled
        default: value = .other(state)
        }
    }
}

/// `work_queue_list`'s answer. Only what Home shows is kept: `evaluated_at`
/// moves on every dispatcher pass and the fences are tendd's own
/// bookkeeping, so neither may make an unchanged queue look changed.
struct TendWorkQueue: Decodable, Hashable, Sendable {
    let items: [TendQueueItem]
    let revision: UInt64
    /// Set when tendd could not read the queue file; it is read-only then.
    let broken: String?
    /// Set while tendd could not save an outcome; nothing new goes.
    let unsaved: Bool

    init(items: [TendQueueItem], revision: UInt64, broken: String? = nil, unsaved: Bool = false) {
        self.items = items
        self.revision = revision
        self.broken = broken
        self.unsaved = unsaved
    }

    private enum CodingKeys: String, CodingKey {
        case items, revision, broken, unsaved
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        // An item this app cannot read is dropped, not fatal.
        items = ((try? values.decodeIfPresent([TendLenient<TendQueueItem>].self, forKey: .items)) ?? [])
            .compactMap(\.value)
        revision = (try? values.decodeIfPresent(UInt64.self, forKey: .revision)) ?? 0
        broken = try? values.decodeIfPresent(String.self, forKey: .broken)
        unsaved = (try? values.decodeIfPresent(Bool.self, forKey: .unsaved)) ?? false
    }

    /// What Home lists: the items not finished, in queue order.
    var pending: [TendQueueItem] {
        items.enumerated()
            .filter { !$0.element.state.isFinished }
            .sorted { ($0.element.position ?? Int.max, $0.offset) < ($1.element.position ?? Int.max, $1.offset) }
            .map(\.element)
    }
}

/// A new Work to queue (`work_queue_add` with a `new_work` target).
struct TendQueueNewWork: Equatable, Sendable {
    var requestKey = UUID().uuidString
    var title: String?
    var prompt: String
    var work: String?
    var workspace: String?
    /// Absolute; tendd refuses a new Work without one.
    var cwd: String
    var pipeline: String?
    var external: String?
    var skill: String?
    var context: String?

    var payload: [String: Any] {
        var target: [String: Any] = ["kind": "new_work", "cwd": cwd]
        for (key, value) in [
            ("work", work), ("workspace", workspace), ("pipeline", pipeline),
            ("external", external), ("skill", skill), ("context", context),
        ] {
            if let value = value?.trimmingCharacters(in: .whitespacesAndNewlines), !value.isEmpty {
                target[key] = value
            }
        }
        var item: [String: Any] = [
            "request_key": requestKey,
            "prompt": prompt,
            "target": target,
        ]
        if let title = title?.trimmingCharacters(in: .whitespacesAndNewlines), !title.isEmpty {
            item["title"] = title
        }
        return item
    }
}

/// A change to one queued item, as the person makes it from Home.
enum TendQueueChange: Equatable, Sendable {
    case edit(prompt: String)
    case move(to: Int)
    case cancel
    case remove(ackDuplicateRisk: Bool)
    case runNow(allowDuplicate: Bool)
    case resolve(sent: Bool, ackDuplicateRisk: Bool)

    /// The wire request for `item`, made against the revision Home showed.
    func request(for item: TendQueueItem) -> [String: Any] {
        var object: [String: Any] = ["id": item.id]
        let kind: String
        switch self {
        case .edit(let prompt):
            kind = "work_queue_edit"
            object["expected_revision"] = item.revision
            object["edit"] = ["prompt": prompt]
        case .move(let to):
            kind = "work_queue_move"
            object["expected_revision"] = item.revision
            object["to"] = to
        case .cancel:
            kind = "work_queue_cancel"
            object["expected_revision"] = item.revision
        case .remove(let ack):
            kind = "work_queue_remove"
            object["ack_duplicate_risk"] = ack
        case .runNow(let allowDuplicate):
            kind = "work_queue_run_now"
            object["expected_revision"] = item.revision
            object["allow_duplicate"] = allowDuplicate
        case .resolve(let sent, let ack):
            kind = "work_queue_resolve"
            object["sent"] = sent
            object["ack_duplicate_risk"] = ack
        }
        object["protocol"] = TendIPCClient.protocolVersion
        object["kind"] = kind
        return object
    }
}

private struct TendWorkQueueEnvelope: Decodable {
    let ok: Bool?
    let error: String?
    let workQueue: TendLenient<TendWorkQueue>?

    enum CodingKeys: String, CodingKey {
        case ok, error
        case workQueue = "work_queue"
    }
}

/// The queue requests on their own connection: a run-now or add can take a
/// moment while tendd checks the target, and the control connection's
/// polling must not wait behind it.
final class TendWorkQueueClient: Sendable {
    static let requestTimeout: TimeInterval = 20

    let socketPath: String
    private let transport: SerializedIPCTransport

    init(socketPath: String) {
        self.socketPath = socketPath
        transport = SerializedIPCTransport(label: "dev.tend.mac.ipc-work-queue", timedHandler: UnixSocket.request)
    }

    /// Test seam: exchanges go through `request` instead of the socket.
    init(socketPath: String, request: @escaping TendIPCRequestHandler) {
        self.socketPath = socketPath
        transport = SerializedIPCTransport(label: "dev.tend.mac.ipc-work-queue-test", handler: request)
    }

    static var listRequest: [String: Any] {
        [
            "protocol": TendIPCClient.protocolVersion,
            "kind": "work_queue_list",
            "include_finished": false,
        ]
    }

    func list() async throws -> TendWorkQueue {
        let response = try await call(Self.listRequest, timeout: 10)
        guard let queue = response.workQueue?.value else {
            throw TendIPCError.server(String(localized: "tendd sent a queue this app cannot read"))
        }
        return queue
    }

    func add(_ newWork: TendQueueNewWork) async throws {
        _ = try await call([
            "protocol": TendIPCClient.protocolVersion,
            "kind": "work_queue_add",
            "item": newWork.payload,
        ])
    }

    func change(_ change: TendQueueChange, on item: TendQueueItem) async throws {
        _ = try await call(change.request(for: item))
    }

    private func call(_ object: [String: Any], timeout: TimeInterval = requestTimeout) async throws -> TendWorkQueueEnvelope {
        let payload = try JSONSerialization.data(withJSONObject: object)
        let data = try await transport.request(path: socketPath, payload: payload, timeout: timeout)
        let response = try JSONDecoder().decode(TendWorkQueueEnvelope.self, from: data)
        if response.ok == false {
            throw TendIPCError.server(response.error ?? "tendd rejected the request")
        }
        return response
    }
}

extension TendIPCClient {
    nonisolated func makeWorkQueueClient() -> TendWorkQueueClient {
        let socketPath = socketPath
        return workQueueClientCache.value { TendWorkQueueClient(socketPath: socketPath) }
    }

    /// The queue's unfinished items; nil from a daemon without
    /// `work_queue_v1`.
    func workQueue() async throws -> TendWorkQueue? {
        guard supports(tendWorkQueueCapability) else { return nil }
        return try await makeWorkQueueClient().list()
    }
}
