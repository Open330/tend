// The app's only connection: one tend dashboard origin, over HTTPS (plain
// HTTP only to a loopback address), with a paired device's token.
//
// - The origin is a bare scheme://host[:port]; user info, a path, a query
//   or a fragment is refused.
// - No redirect is followed, for pairing and for every authenticated call:
//   the token goes to the origin it was paired with and nowhere else.
// - No cookies: an ephemeral session without a cookie store, so nothing the
//   dashboard's browser sign-in sets can be picked up or sent.
import Foundation

/// A dashboard address the app may talk to.
struct ServerOrigin: Hashable, Sendable, Codable {
    let url: URL

    enum Problem: Error, Equatable {
        case empty, notAnAddress, scheme, plainHTTP, notBare
    }

    /// Accepts `https://host[:port]` (a bare host gets `https://`), and
    /// `http://` only for `127.0.0.1`, `localhost` or `[::1]`.
    init(parsing input: String) throws(Problem) {
        var text = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { throw .empty }
        if !text.contains("://") { text = "https://" + text }
        while text.hasSuffix("/") { text.removeLast() }
        guard let components = URLComponents(string: text), let host = components.host, !host.isEmpty else {
            throw .notAnAddress
        }
        guard components.user == nil, components.password == nil,
              components.path.isEmpty, components.query == nil, components.fragment == nil
        else { throw .notBare }
        let scheme = components.scheme?.lowercased()
        switch scheme {
        case "https": break
        case "http":
            guard ["127.0.0.1", "localhost", "::1", "[::1]"].contains(host.lowercased()) else { throw .plainHTTP }
        default: throw .scheme
        }
        var canonical = URLComponents()
        canonical.scheme = scheme
        canonical.host = host.lowercased()
        canonical.port = components.port
        guard let url = canonical.url else { throw .notAnAddress }
        self.url = url
    }

    var display: String { url.absoluteString }

    func endpoint(_ path: String) -> URL {
        URL(string: url.absoluteString + path)!
    }
}

extension ServerOrigin.Problem: LocalizedError {
    var errorDescription: String? {
        switch self {
        case .empty: String(localized: "Enter your dashboard's address.")
        case .notAnAddress: String(localized: "That is not a web address.")
        case .scheme: String(localized: "Use an https:// address.")
        case .plainHTTP: String(localized: "Use https://. Plain http:// works only for this device itself (127.0.0.1).")
        case .notBare: String(localized: "Enter only the address, like https://tend.example.com, without a path.")
        }
    }
}

/// What a dashboard call can end in, worded for the person.
enum DashboardError: Error, Equatable, LocalizedError {
    /// The device token was refused: removed, expired, or another daemon.
    case signedOut
    /// The dashboard has no device pairing (an older tend, or no token set).
    case pairingUnavailable
    case redirected
    /// The server refused with this message (and status).
    case server(Int, String)
    case transport(String)
    case undecodable(String)
    /// This device's scope does not cover it.
    case notAllowed
    /// The dashboard does not allow starting work (`allow_work_start`).
    case workStartOff

    var errorDescription: String? {
        switch self {
        case .signedOut: String(localized: "This device is no longer paired. Pair it again.")
        case .pairingUnavailable: String(localized: "This dashboard cannot pair devices. It needs a newer tend with an access token set.")
        case .redirected: String(localized: "The dashboard answered with a redirect. Tend only talks to the exact address you paired with.")
        case .server(_, let message): message
        case .transport(let message): message
        case .undecodable(let message): String(localized: "Unexpected answer from the dashboard: \(message)")
        case .notAllowed: String(localized: "This device may only decide. Pair it again with hosts and new work to see this.")
        case .workStartOff: String(localized: "Starting work is turned off on this dashboard ([dashboard] allow_work_start).")
        }
    }
}

/// `POST /auth/devices/request`.
struct PairingRequest: Decodable, Sendable {
    let deviceCode: String
    let userCode: String
    let expiresIn: Int
    let interval: Int

    enum CodingKeys: String, CodingKey {
        case deviceCode = "device_code", userCode = "user_code", expiresIn = "expires_in", interval
    }
}

struct PairedDevice: Codable, Hashable, Sendable {
    let id: String
    let name: String
}

enum PairingPoll: Sendable {
    case pending
    case issued(PairedDevice, token: String, scope: DeviceScope)
}

/// What a paired device may do. An older dashboard names no scope: decide.
enum DeviceScope: String, Codable, Sendable, Hashable {
    /// Read and answer what needs the person.
    case decide
    /// Also see every host and agent, read recent output, start new work.
    case operate

    init(names: [String]?) {
        self = names?.contains("operate") == true ? .operate : .decide
    }
}

/// `GET /api/device`.
struct DeviceInfo: Decodable, Sendable {
    let device: PairedDevice
    let scope: DeviceScope
    /// What this dashboard serves an `operate` device beyond the hosts:
    /// `capture` (recent output), `work` (start work). Absent on a dashboard
    /// that has neither yet.
    let features: Set<String>
    /// Whether this dashboard sends notifications; nil from a tend that
    /// cannot.
    let push: PushSupport?

    struct PushSupport: Decodable, Sendable, Equatable {
        let enabled: Bool
        let contentAllowed: Bool
        let environments: [String]

        enum CodingKeys: String, CodingKey {
            case enabled, environments
            case contentAllowed = "content_allowed"
        }

        init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            enabled = (try? values.decode(Bool.self, forKey: .enabled)) ?? false
            contentAllowed = (try? values.decode(Bool.self, forKey: .contentAllowed)) ?? false
            environments = (try? values.decode([String].self, forKey: .environments)) ?? []
        }

        init(enabled: Bool, contentAllowed: Bool, environments: [String]) {
            self.enabled = enabled
            self.contentAllowed = contentAllowed
            self.environments = environments
        }
    }

    enum CodingKeys: String, CodingKey { case device, scope, features, push }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        device = try values.decode(PairedDevice.self, forKey: .device)
        scope = DeviceScope(names: try? values.decodeIfPresent([String].self, forKey: .scope))
        features = Set(((try? values.decodeIfPresent([String].self, forKey: .features)) ?? nil) ?? [])
        push = (try? values.decodeIfPresent(PushSupport.self, forKey: .push)) ?? nil
    }
}

/// What `POST /api/device/push` answered.
enum PushRegistration: Equatable, Sendable {
    /// The host does not send to this phone (push off, or this build's
    /// environment is not one its key serves).
    case off(reason: String)
    case on(PushBinding)
}

/// What a host offers new work from the phone (`GET
/// /api/device/work-options`): whether it is allowed, the pipelines (teams)
/// and the project folders.
struct WorkOptions: Decodable, Sendable, Equatable {
    struct Pipeline: Decodable, Sendable, Equatable, Hashable, Identifiable {
        let name: String
        let description: String?
        let agents: Int
        var id: String { name }

        enum CodingKeys: String, CodingKey { case name, description, agents }

        init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            name = try values.decode(String.self, forKey: .name)
            description = (try? values.decodeIfPresent(String.self, forKey: .description)) ?? nil
            struct Any_: Decodable, Sendable {}
            agents = ((try? values.decodeIfPresent([TendLenient<Any_>].self, forKey: .agents)) ?? nil)?.count ?? 0
        }

        init(name: String, description: String? = nil, agents: Int = 1) {
            self.name = name
            self.description = description
            self.agents = agents
        }
    }

    let pipelines: [Pipeline]
    /// The folders new work may start in there: folders its agents use.
    let projects: [String]
    /// Whether the dashboard allows starting work at all.
    let allowed: Bool

    enum CodingKeys: String, CodingKey {
        case pipelines, projects
        case allowed = "allow_work_start"
    }

    init(pipelines: [Pipeline], projects: [String], allowed: Bool = true) {
        self.pipelines = pipelines
        self.projects = projects
        self.allowed = allowed
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        let listed = ((try? values.decodeIfPresent([TendLenient<Pipeline>].self, forKey: .pipelines)) ?? nil) ?? []
        pipelines = listed.compactMap(\.value)
        projects = ((try? values.decodeIfPresent([String].self, forKey: .projects)) ?? nil) ?? []
        allowed = ((try? values.decodeIfPresent(Bool.self, forKey: .allowed)) ?? nil) ?? false
    }
}

/// `POST /api/device/work`, as the phone sends it: what to do, where and
/// with which team. The dashboard mints the Work id.
struct StartWorkRequest: Sendable, Equatable {
    /// One per Start; a retry sends the same key.
    var idempotencyKey: String
    var host: String
    var project: String
    var pipeline: String
    var request: String

    var bodyJSON: [String: Any] {
        ["idempotency_key": idempotencyKey, "host": host, "project": project, "pipeline": pipeline, "request": request]
    }
}

struct StartedWork: Decodable, Sendable {
    /// `started`, `failed`, `unknown` (it may have started; check the host).
    let state: String
    let work: String?
}

/// One host's part of `GET /api/decisions`.
struct HostQueue: Decodable, Hashable, Sendable, Identifiable {
    let alias: String
    let local: Bool
    let control: Bool
    let decidesReviews: Bool
    let covers: [String]
    let unavailable: [String]
    let queue: TendPendingQueue
    /// Questions nobody can answer can be kept here (`subject_check_v1`).
    var subjectCheck = false
    /// And closed, while the host's `[review] close_unanswerable` is on.
    var closeUnanswerable = false

    var id: String { alias }

    enum CodingKeys: String, CodingKey {
        case alias, local, control, covers, unavailable, queue
        case decidesReviews = "decides_reviews"
        case subjectCheck = "subject_check"
        case closeUnanswerable = "close_unanswerable"
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        alias = try values.decode(String.self, forKey: .alias)
        local = (try? values.decodeIfPresent(Bool.self, forKey: .local)) ?? false
        control = (try? values.decodeIfPresent(Bool.self, forKey: .control)) ?? false
        decidesReviews = (try? values.decodeIfPresent(Bool.self, forKey: .decidesReviews)) ?? local
        covers = (try? values.decodeIfPresent([String].self, forKey: .covers)) ?? []
        unavailable = (try? values.decodeIfPresent([String].self, forKey: .unavailable)) ?? []
        // A section whose queue cannot be read is an error, never an empty
        // queue: "nothing needs you" must mean that.
        queue = try values.decode(TendPendingQueue.self, forKey: .queue)
        subjectCheck = (try? values.decodeIfPresent(Bool.self, forKey: .subjectCheck)) ?? nil ?? false
        closeUnanswerable = (try? values.decodeIfPresent(Bool.self, forKey: .closeUnanswerable)) ?? nil ?? false
    }
}

/// One host's part of `GET /api/decisions/history`.
struct HostHistory: Decodable, Hashable, Sendable, Identifiable {
    let alias: String
    let local: Bool
    let unavailable: [String]
    let history: TendDecisionHistory

    var id: String { alias }

    enum CodingKeys: String, CodingKey { case alias, local, unavailable, history }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        alias = try values.decode(String.self, forKey: .alias)
        local = (try? values.decodeIfPresent(Bool.self, forKey: .local)) ?? false
        unavailable = (try? values.decodeIfPresent([String].self, forKey: .unavailable)) ?? []
        history = try values.decode(TendDecisionHistory.self, forKey: .history)
    }
}

private struct Hosts<Section: Decodable>: Decodable {
    let hosts: [Section]
}

/// Refuses every redirect: the request fails instead of following it.
private final class NoRedirects: NSObject, URLSessionTaskDelegate, Sendable {
    func urlSession(
        _ session: URLSession,
        task: URLSessionTask,
        willPerformHTTPRedirection response: HTTPURLResponse,
        newRequest request: URLRequest
    ) async -> URLRequest? {
        nil
    }
}

struct DashboardClient: Sendable {
    /// Unit tests answer requests with a `URLProtocol` stub; set before the
    /// first request. Never set by the app.
    nonisolated(unsafe) static var testProtocolClasses: [AnyClass]?

    let origin: ServerOrigin
    /// The device token; nil before pairing.
    var token: String?

    private static let session: URLSession = {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.httpCookieStorage = nil
        configuration.httpShouldSetCookies = false
        configuration.httpCookieAcceptPolicy = .never
        configuration.urlCache = nil
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        configuration.timeoutIntervalForRequest = 30
        if let stub = testProtocolClasses { configuration.protocolClasses = stub }
        return URLSession(configuration: configuration, delegate: NoRedirects(), delegateQueue: nil)
    }()

    private func send(_ method: String, _ path: String, body: [String: Any]? = nil, authorized: Bool, via: String? = nil) async throws -> (Int, Data) {
        var request = URLRequest(url: origin.endpoint(path))
        request.httpMethod = method
        // Recorded in the decision's audit line on the host.
        if let via { request.setValue(via, forHTTPHeaderField: "X-Tend-Via") }
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        if let body {
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.httpBody = try JSONSerialization.data(withJSONObject: body)
        }
        if authorized {
            guard let token else { throw DashboardError.signedOut }
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await Self.session.data(for: request)
        } catch let error as URLError where error.code == .cancelled {
            throw CancellationError()
        } catch is CancellationError {
            throw CancellationError()
        } catch {
            throw DashboardError.transport(error.localizedDescription)
        }
        guard let http = response as? HTTPURLResponse else { throw DashboardError.transport("no HTTP response") }
        if (300..<400).contains(http.statusCode) { throw DashboardError.redirected }
        return (http.statusCode, data)
    }

    private static func message(_ data: Data, _ status: Int) -> String {
        if let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
           let error = object["error"] as? String {
            return error
        }
        return HTTPURLResponse.localizedString(forStatusCode: status)
    }

    private func decode<T: Decodable>(_ type: T.Type, _ data: Data) throws -> T {
        do {
            return try JSONDecoder().decode(type, from: data)
        } catch {
            throw DashboardError.undecodable(String(describing: error))
        }
    }

    /// For an authenticated call: 401 means this device is gone.
    private func check(_ status: Int, _ data: Data) throws {
        switch status {
        case 200..<300: return
        case 401: throw DashboardError.signedOut
        default: throw DashboardError.server(status, Self.message(data, status))
        }
    }

    // MARK: Pairing

    func requestPairing(name: String, platform: String, scope: DeviceScope) async throws -> PairingRequest {
        let (status, data) = try await send("POST", "/auth/devices/request", body: ["name": name, "platform": platform, "scope": scope.rawValue], authorized: false)
        switch status {
        case 201: return try decode(PairingRequest.self, data)
        case 404, 405: throw DashboardError.pairingUnavailable
        default: throw DashboardError.server(status, Self.message(data, status))
        }
    }

    func pollPairing(deviceCode: String) async throws -> PairingPoll {
        let (status, data) = try await send("POST", "/auth/devices/token", body: ["device_code": deviceCode], authorized: false)
        switch status {
        case 202: return .pending
        case 201:
            struct Issued: Decodable { let device: PairedDevice; let token: String; let scope: [String]? }
            let issued = try decode(Issued.self, data)
            return .issued(issued.device, token: issued.token, scope: DeviceScope(names: issued.scope))
        case 404, 405: throw DashboardError.pairingUnavailable
        default: throw DashboardError.server(status, Self.message(data, status))
        }
    }

    /// Unpair this device on the dashboard (Sign out).
    func forget() async throws {
        let (status, data) = try await send("POST", "/api/device/forget", authorized: true)
        try check(status, data)
    }

    // MARK: Decisions

    func queue(limit: Int = 100) async throws -> [HostQueue] {
        let (status, data) = try await send("GET", "/api/decisions?limit=\(limit)", authorized: true)
        try check(status, data)
        return try decode(Hosts<HostQueue>.self, data).hosts
    }

    func history(limit: Int = 100) async throws -> [HostHistory] {
        // tend's own closures too, with why (an older dashboard ignores it).
        let (status, data) = try await send("GET", "/api/decisions/history?limit=\(limit)&closures=true", authorized: true)
        try check(status, data)
        return try decode(Hosts<HostHistory>.self, data).hosts
    }

    /// Answer (or decline) a question addressed to the person.
    func answer(requestID: String, answer: String, declined: Bool, host: String?, via: String? = nil) async throws {
        var body: [String: Any] = ["answer": answer, "declined": declined]
        if let host { body["host"] = host }
        let (status, data) = try await send("POST", "/api/decisions/requests/\(Self.segment(requestID))", body: body, authorized: true, via: via)
        try check(status, data)
    }

    /// `POST /api/decisions/keep`: keep a question nobody can answer in
    /// Your turn through its current episode (this host only).
    func keep(requestID: String, episode: String, host: String? = nil) async throws {
        var body: [String: Any] = ["request_id": requestID, "episode": episode]
        if let host { body["host"] = host }
        let (status, data) = try await send("POST", "/api/decisions/keep", body: body, authorized: true)
        try check(status, data)
    }

    /// `POST /api/decisions/close-unanswerable`: close exactly these pairs
    /// on one host (`host`, or the dashboard's own when nil); what was
    /// skipped comes back.
    func closeUnanswerable(_ items: [(requestID: String, episode: String)], host: String? = nil) async throws -> TendCloseOutcome {
        var body: [String: Any] = [
            "items": items.map { ["request_id": $0.requestID, "episode": $0.episode] },
            "by_label": "ios",
        ]
        if let host { body["host"] = host }
        let (status, data) = try await send("POST", "/api/decisions/close-unanswerable", body: body, authorized: true)
        try check(status, data)
        return try decode(TendCloseOutcome.self, data)
    }

    /// Decide a result: accept (optionally with one of its options), send it
    /// back with a note, or withdraw it.
    func review(id: String, disposition: TendReviewDisposition, note: String?, option: String?, host: String?, via: String? = nil) async throws {
        var body: [String: Any] = ["disposition": disposition.rawValue]
        if let note, !note.isEmpty { body["note"] = note }
        if let option { body["option"] = option }
        if let host { body["host"] = host }
        let (status, data) = try await send("POST", "/api/decisions/reviews/\(Self.segment(id))", body: body, authorized: true, via: via)
        try check(status, data)
    }

    // MARK: The device

    /// `GET /api/device`: who this device is and what it may do.
    func device() async throws -> DeviceInfo {
        let (status, data) = try await send("GET", "/api/device", authorized: true)
        try check(status, data)
        return try decode(DeviceInfo.self, data)
    }

    // MARK: Notifications

    /// `POST /api/device/push`: this phone's APNs token, for this build's
    /// environment. 404 from a tend without push.
    func registerPush(token: String, environment: String, content: Bool) async throws -> PushRegistration {
        let (status, data) = try await send("POST", "/api/device/push", body: ["token": token, "environment": environment, "content": content], authorized: true)
        if status == 404 { return .off(reason: "unsupported") }
        try check(status, data)
        struct Answer: Decodable {
            let enabled: Bool
            let reason: String?
            let content: Bool?
            let channel: String?
            let refKey: String?
            enum CodingKeys: String, CodingKey { case enabled, reason, content, channel, refKey = "ref_key" }
        }
        let answer = try decode(Answer.self, data)
        guard answer.enabled else { return .off(reason: answer.reason ?? "off") }
        guard let channel = answer.channel, !channel.isEmpty,
              let key = answer.refKey.flatMap({ Data(base64Encoded: $0) }), key.count >= 16
        else { throw DashboardError.server(status, "the dashboard's push answer is incomplete") }
        return .on(PushBinding(channel: channel, refKey: key, content: answer.content ?? false))
    }

    /// `POST /api/device/push/remove`: stop notifying this phone.
    func removePush() async throws {
        let (status, data) = try await send("POST", "/api/device/push/remove", authorized: true)
        if status == 404 { return }
        try check(status, data)
    }

    // MARK: Hosts, agents, work (the `operate` scope)

    /// `GET /api/device/fleet`: every host and its agents, cut down for a
    /// device. 403 means this device may only decide (never "signed out").
    func fleet() async throws -> FleetSnapshot {
        let (status, data) = try await send("GET", "/api/device/fleet", authorized: true)
        if status == 403 { throw DashboardError.notAllowed }
        try check(status, data)
        return try decode(FleetSnapshot.self, data)
    }

    /// `GET /api/device/home`: Home's board as tendd computes it (Work
    /// units, decisions, folds, backlog). 403 means this device may only
    /// decide.
    func home() async throws -> DeviceHome {
        let (status, data) = try await send("GET", "/api/device/home", authorized: true)
        if status == 403 { throw DashboardError.notAllowed }
        try check(status, data)
        return try decode(DeviceHome.self, data)
    }

    /// `POST /api/device/capture`: the recent output of one agent's pane,
    /// named by its session id.
    func capture(host: String, sessionID: String) async throws -> String {
        let body: [String: Any] = ["host": host, "session_id": sessionID]
        let (status, data) = try await send("POST", "/api/device/capture", body: body, authorized: true)
        if status == 403 { throw DashboardError.notAllowed }
        try check(status, data)
        struct Capture: Decodable { let capture: String? }
        return try decode(Capture.self, data).capture ?? ""
    }

    /// `GET /api/device/work-options`: the projects and pipelines new work
    /// can start with on that host.
    func workOptions(host: String) async throws -> WorkOptions {
        let query = host.addingPercentEncoding(withAllowedCharacters: .alphanumerics) ?? host
        let (status, data) = try await send("GET", "/api/device/work-options?host=\(query)", authorized: true)
        if status == 403 { throw DashboardError.notAllowed }
        try check(status, data)
        return try decode(WorkOptions.self, data)
    }

    /// `POST /api/device/work`: start new work. 501 means the dashboard
    /// does not allow starting work.
    func startWork(_ request: StartWorkRequest) async throws -> StartedWork {
        let (status, data) = try await send("POST", "/api/device/work", body: request.bodyJSON, authorized: true)
        switch status {
        case 403: throw DashboardError.notAllowed
        case 501: throw DashboardError.workStartOff
        default: try check(status, data)
        }
        return try decode(StartedWork.self, data)
    }

    private static func segment(_ id: String) -> String {
        id.addingPercentEncoding(withAllowedCharacters: .alphanumerics.union(CharacterSet(charactersIn: "-_.~"))) ?? id
    }
}
