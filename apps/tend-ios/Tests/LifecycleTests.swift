import XCTest
@testable import Tend

/// Answers the app's requests from a table the test sets, with an optional
/// delay per path.
final class StubProtocol: URLProtocol, @unchecked Sendable {
    struct Reply {
        var status: Int
        var body: String
        var delay: TimeInterval = 0
    }

    nonisolated(unsafe) static var replies: [String: [Reply]] = [:]
    nonisolated(unsafe) static var seen: [String] = []
    /// Each request's path with the bearer token it carried.
    nonisolated(unsafe) static var authorized: [(path: String, token: String?)] = []
    /// The last body sent to each path, and each request's query.
    nonisolated(unsafe) static var bodies: [String: Data] = [:]
    nonisolated(unsafe) static var queries: [String] = []
    static let lock = NSLock()

    static func reset() {
        lock.withLock { replies = [:]; seen = []; authorized = []; bodies = [:]; queries = [] }
    }

    static func on(_ path: String, _ replies: Reply...) {
        lock.withLock { Self.replies[path, default: []].append(contentsOf: replies) }
    }

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        let path = request.url?.path ?? ""
        var body = request.httpBody
        if body == nil, let stream = request.httpBodyStream {
            stream.open()
            var data = Data()
            var buffer = [UInt8](repeating: 0, count: 4096)
            while stream.hasBytesAvailable {
                let read = stream.read(&buffer, maxLength: buffer.count)
                if read <= 0 { break }
                data.append(buffer, count: read)
            }
            stream.close()
            body = data
        }
        let query = request.url?.query ?? ""
        let reply: Reply = Self.lock.withLock {
            Self.seen.append(path)
            let bearer = request.value(forHTTPHeaderField: "Authorization").map { $0.replacingOccurrences(of: "Bearer ", with: "") }
            Self.authorized.append((path, bearer))
            Self.queries.append("\(path)?\(query)")
            if let body, !body.isEmpty { Self.bodies[path] = body }
            guard var queue = Self.replies[path], !queue.isEmpty else {
                return Reply(status: 404, body: #"{"error":"no stub"}"#)
            }
            let next = queue.count > 1 ? queue.removeFirst() : queue[0]
            Self.replies[path] = queue
            return next
        }
        let url = request.url!
        DispatchQueue.global().asyncAfter(deadline: .now() + reply.delay) { [self] in
            let response = HTTPURLResponse(url: url, statusCode: reply.status, httpVersion: nil, headerFields: ["Content-Type": "application/json"])!
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: Data(reply.body.utf8))
            client?.urlProtocolDidFinishLoading(self)
        }
    }

    override func stopLoading() {}
}

@MainActor
final class LifecycleTests: XCTestCase {
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

    private func eventually(_ timeout: TimeInterval = 5, _ condition: () -> Bool) async {
        let end = Date().addingTimeInterval(timeout)
        while !condition(), Date() < end {
            try? await Task.sleep(nanoseconds: 50_000_000)
        }
    }

    private static let emptyQueue = #"{"hosts":[{"alias":"here","local":true,"control":true,"covers":[],"unavailable":[],"queue":{"items":[],"truncated":false,"unavailable":[]}}]}"#

    private func pairedModel() -> AppModel {
        let origin = try! ServerOrigin(parsing: "https://tend.test")
        CredentialStore.save(Pairing(origin: origin, device: PairedDevice(id: "dev_1", name: "phone"), token: "tdv_x"))
        return AppModel()
    }

    func testALateAnswerToAnOldAttemptLeavesTheNewOneAlone() async {
        // The first attempt's request is slow and fails; the second one
        // answers at once. The first must not undo the second.
        StubProtocol.on("/auth/devices/request",
                        .init(status: 500, body: #"{"error":"old attempt"}"#, delay: 0.6),
                        .init(status: 201, body: #"{"device_code":"d2","user_code":"NEWC-ODE2","expires_in":600,"interval":1}"#))
        StubProtocol.on("/auth/devices/token", .init(status: 202, body: #"{"status":"pending"}"#))
        let model = AppModel()
        model.startPairing(address: "https://tend.test", name: "phone")
        try? await Task.sleep(nanoseconds: 100_000_000)
        model.startPairing(address: "https://tend.test", name: "phone")
        try? await Task.sleep(nanoseconds: 1_200_000_000)
        guard case .waitingForApproval(_, let code, _) = model.phase else {
            return XCTFail("phase is \(model.phase)")
        }
        XCTAssertEqual(code, "NEWC-ODE2")
        XCTAssertNil(model.pairingProblem)
        model.cancelPairing()
    }

    func testACancelledAttemptNeverPairs() async {
        StubProtocol.on("/auth/devices/request",
                        .init(status: 201, body: #"{"device_code":"d1","user_code":"AAAA-BBBB","expires_in":600,"interval":1}"#))
        StubProtocol.on("/auth/devices/token",
                        .init(status: 201, body: #"{"device":{"id":"dev_9","name":"phone"},"token":"tdv_late"}"#, delay: 0.3))
        let model = AppModel()
        model.startPairing(address: "https://tend.test", name: "phone")
        await eventually { if case .waitingForApproval = model.phase { true } else { false } }
        // Cancel while the poll that would issue the token is in flight.
        try? await Task.sleep(nanoseconds: 1_100_000_000)
        model.cancelPairing()
        try? await Task.sleep(nanoseconds: 600_000_000)
        XCTAssertEqual(model.phase, .unpaired)
        XCTAssertNil(CredentialStore.load(), "a cancelled attempt kept a key")
    }

    func testSignOutThatCannotReachTheDashboardKeepsTheKey() async {
        StubProtocol.on("/api/device/forget", .init(status: 500, body: #"{"error":"store unavailable"}"#))
        let model = pairedModel()
        await model.signOut()
        XCTAssertNotNil(model.pairing, "the key was dropped although the dashboard still honours it")
        XCTAssertNotNil(model.signOutProblem)
        XCTAssertNotNil(CredentialStore.load())
        // The explicit way out.
        model.forgetLocally()
        XCTAssertNil(model.pairing)
        XCTAssertNil(CredentialStore.load())
    }

    func testSignOutOfAnAlreadyRemovedDeviceSucceeds() async {
        StubProtocol.on("/api/device/forget", .init(status: 401, body: ""))
        let model = pairedModel()
        await model.signOut()
        XCTAssertNil(model.pairing)
        XCTAssertNil(model.signOutProblem)
    }

    func testAReadFromBeforeSignOutDoesNotRepopulate() async {
        StubProtocol.on("/api/decisions", .init(status: 200, body: Self.emptyQueue, delay: 0.5))
        StubProtocol.on("/api/device/forget", .init(status: 200, body: #"{"removed":"dev_1"}"#))
        let model = pairedModel()
        let read = Task { await model.refresh() }
        try? await Task.sleep(nanoseconds: 100_000_000)
        await model.signOut()
        await read.value
        XCTAssertNil(model.pairing)
        XCTAssertTrue(model.hosts.isEmpty)
        XCTAssertFalse(model.loaded)
    }

    func testARemovedDeviceIsUnpairedOnItsNextRead() async {
        StubProtocol.on("/api/decisions", .init(status: 401, body: ""))
        let model = pairedModel()
        await model.refresh()
        XCTAssertNil(model.pairing)
        XCTAssertNotNil(model.pairingProblem)
    }

    private func operateModel() -> AppModel {
        let origin = try! ServerOrigin(parsing: "https://tend.test")
        CredentialStore.save(Pairing(origin: origin, device: PairedDevice(id: "dev_1", name: "phone"), token: "tdv_x", scope: .operate))
        return AppModel()
    }

    private static let fleetJSON = #"{"hosts":[{"alias":"here","local":true,"state":"online","remote":{"agents":[{"kind":"claude_code","agent_session_id":"a","pane":"%1","state":"working"}]}}]}"#
    private static let operateDevice = #"{"device":{"id":"dev_1","name":"phone"},"scope":["decisions","operate"],"features":["capture","work"]}"#

    func testALateOperateAnswerNeverUndoesANarrowing() async {
        StubProtocol.on("/api/device", .init(status: 200, body: Self.operateDevice, delay: 0.5))
        StubProtocol.on("/api/device/fleet", .init(status: 403, body: #"{"error":"decide only"}"#))
        StubProtocol.on("/api/decisions", .init(status: 200, body: Self.emptyQueue))
        let model = operateModel()
        let device = Task { await model.refreshDevice() }
        try? await Task.sleep(nanoseconds: 100_000_000)
        await model.refresh()
        XCTAssertFalse(model.operates, "the 403 narrows the device")
        await device.value
        XCTAssertFalse(model.operates, "an answer from before the narrowing widened it again")
        XCTAssertFalse(model.canStartWork)
        XCTAssertEqual(CredentialStore.load()?.scope, .decide)
    }

    func testAFleetAnswerFromBeforeANarrowingIsDropped() async {
        StubProtocol.on("/api/device/fleet", .init(status: 200, body: Self.fleetJSON, delay: 0.5))
        StubProtocol.on("/api/device", .init(status: 200, body: #"{"device":{"id":"dev_1","name":"phone"},"scope":["decisions"]}"#))
        StubProtocol.on("/api/decisions", .init(status: 200, body: Self.emptyQueue))
        let model = operateModel()
        let read = Task { await model.refresh() }
        try? await Task.sleep(nanoseconds: 100_000_000)
        await model.refreshDevice()
        XCTAssertFalse(model.operates)
        await read.value
        XCTAssertNil(model.fleet, "the late snapshot came back after the narrowing")
        XCTAssertTrue(model.inProgress.isEmpty)
    }

    func testANarrowedDeviceCannotStartWork() async throws {
        StubProtocol.on("/api/device", .init(status: 200, body: #"{"device":{"id":"dev_1","name":"phone"},"scope":["decisions"]}"#))
        let model = operateModel()
        await model.refreshDevice()
        let request = StartWorkRequest(idempotencyKey: UUID().uuidString, host: "here", project: "/w", pipeline: "solo", request: "x")
        do {
            _ = try await model.startWork(request)
            XCTFail("a decide device sent a start")
        } catch {
            XCTAssertEqual(error as? DashboardError, .notAllowed)
        }
        XCTAssertFalse(StubProtocol.seen.contains("/api/device/work"))
    }

    func testAllClearNeedsCompleteQueues() throws {
        func hosts(_ json: String) throws -> [HostQueue] {
            struct Hosts: Decodable { let hosts: [HostQueue] }
            return try JSONDecoder().decode(Hosts.self, from: Data(json.utf8)).hosts
        }
        XCTAssertTrue(AppModel.allClear(try hosts(Self.emptyQueue)))
        // Only an item this app cannot read: not all clear.
        let unknown = #"{"hosts":[{"alias":"here","local":true,"unavailable":[],"queue":{"items":[{"kind":"future_item"}],"truncated":false,"unavailable":[]}}]}"#
        XCTAssertFalse(AppModel.allClear(try hosts(unknown)))
        let offline = #"{"hosts":[{"alias":"mini","local":false,"unavailable":["host is offline"],"queue":{"items":[]}}]}"#
        XCTAssertFalse(AppModel.allClear(try hosts(offline)))
        // A section without a readable queue is an error, not an empty one.
        XCTAssertThrowsError(try hosts(#"{"hosts":[{"alias":"x","queue":"garbage"}]}"#))
    }
}

/// Home on the daemon's board, and leaving it.
extension LifecycleTests {
    private static let boardWithAQuestion = #"{"hosts":[{"alias":"here","local":true}],"decision_hosts":[{"alias":"here","local":true}],"lanes":{"needs_you":{"groups":[{"items":[{"kind":"human_request","request_id":"r1","body":"Ship?","from_session_id":"s","from_pane":"%1","created_at":"2026-11-16T10:00:00Z","host":"here","actionable":true}]}]}}}"#
    private static let homeDevice = #"{"device":{"id":"dev_1","name":"phone"},"scope":["operate"],"features":["home"]}"#

    func testNarrowedOffTheBoardReadsTheQueueBeforeSayingAllClear() async {
        StubProtocol.on("/api/device", .init(status: 200, body: Self.homeDevice))
        StubProtocol.on("/api/device/fleet", .init(status: 200, body: Self.fleetJSON))
        StubProtocol.on("/api/device/home", .init(status: 200, body: Self.boardWithAQuestion), .init(status: 403, body: "{}"))
        StubProtocol.on("/api/decisions", .init(status: 200, body: Self.emptyQueue))
        let model = operateModel()
        await model.refreshDevice()
        await model.refresh()
        XCTAssertEqual(model.pending.count, 1)
        XCTAssertFalse(StubProtocol.seen.contains("/api/decisions"), "the board stands in for the queue")
        await model.refresh()
        XCTAssertFalse(model.operates)
        XCTAssertNil(model.visibleBoard)
        XCTAssertTrue(StubProtocol.seen.contains("/api/decisions"), "the queue is read in the same refresh")
        XCTAssertTrue(model.loaded)
        XCTAssertEqual(model.hosts.map(\.alias), ["here"])
    }

    func testLosingTheBoardFeatureReadsTheQueue() async {
        StubProtocol.on("/api/device",
                        .init(status: 200, body: Self.homeDevice),
                        .init(status: 200, body: #"{"device":{"id":"dev_1","name":"phone"},"scope":["operate"],"features":[]}"#))
        StubProtocol.on("/api/device/fleet", .init(status: 200, body: Self.fleetJSON))
        StubProtocol.on("/api/device/home", .init(status: 200, body: Self.boardWithAQuestion))
        StubProtocol.on("/api/decisions", .init(status: 200, body: Self.emptyQueue))
        let model = operateModel()
        await model.refreshDevice()
        await model.refresh()
        XCTAssertNotNil(model.visibleBoard)
        await model.refreshDevice()
        XCTAssertNil(model.visibleBoard)
        XCTAssertTrue(StubProtocol.seen.contains("/api/decisions"))
        XCTAssertTrue(model.loaded)
        XCTAssertEqual(model.hosts.map(\.alias), ["here"])
    }

    func testFinishedCountsOlderDecisionsAsTheWebDoes() async {
        StubProtocol.on("/api/device", .init(status: 200, body: Self.homeDevice))
        StubProtocol.on("/api/device/fleet", .init(status: 200, body: Self.fleetJSON))
        StubProtocol.on("/api/device/home", .init(status: 200, body: #"{"decision_hosts":[{"alias":"here","local":true}],"lanes":{"needs_you":{"groups":[]},"finished":{"groups":[]}}}"#))
        let history = #"{"hosts":[{"alias":"here","local":true,"history":{"items":[{"kind":"human_answer","request_id":"r1","body":"Ship?","from_session_id":"s","from_pane":"%1","created_at":"2020-01-01T00:00:00Z","status":"completed","answer":"yes","answered_at":"2020-01-02T00:00:00Z"}]}}]}"#
        StubProtocol.on("/api/decisions/history", .init(status: 200, body: history))
        let model = operateModel()
        await model.refreshDevice()
        await model.refresh()
        await model.refreshHistory()
        XCTAssertEqual(model.past.count, 1)
        XCTAssertEqual(model.count(.finished), 1)
    }
}

/// Asking for hosts and new work while the device keeps its decide key.
extension LifecycleTests {
    private static let upgradeRequest = #"{"device_code":"d2","user_code":"UPGR-ADE2","expires_in":600,"interval":1}"#
    private static let issuedOperate = #"{"device":{"id":"dev_2","name":"phone"},"token":"tdv_new","scope":["operate"]}"#

    func testSignOutDuringAnUpgradeNeverKeepsTheNewKey() async {
        // The old key's forget is slow; the upgrade's code is approved
        // meanwhile. Sign out must win: no key is left at all.
        StubProtocol.on("/auth/devices/request", .init(status: 201, body: Self.upgradeRequest))
        StubProtocol.on("/auth/devices/token", .init(status: 201, body: Self.issuedOperate, delay: 0.2))
        StubProtocol.on("/api/device/forget", .init(status: 200, body: #"{"removed":"dev_1"}"#, delay: 1.5))
        let model = pairedModel()
        model.requestHostsAndWork()
        await eventually { model.upgrade != nil }
        XCTAssertNotNil(model.upgrade)
        await model.signOut()
        try? await Task.sleep(nanoseconds: 1_500_000_000)
        XCTAssertNil(model.pairing, "signed out, yet a key is in use")
        XCTAssertNil(CredentialStore.load(), "signed out, yet a key is kept")
        XCTAssertNil(model.upgrade)
    }

    func testAnUpgradeThatCannotBeKeptLeavesTheOldKeyWorking() async {
        StubProtocol.on("/auth/devices/request", .init(status: 201, body: Self.upgradeRequest))
        StubProtocol.on("/auth/devices/token", .init(status: 201, body: Self.issuedOperate))
        StubProtocol.on("/api/device/forget", .init(status: 200, body: #"{"removed":"dev_2"}"#))
        let model = pairedModel()
        CredentialStore.failSaves = true
        defer { CredentialStore.failSaves = false }
        model.requestHostsAndWork()
        await eventually(4) { model.upgradeProblem != nil }
        XCTAssertNotNil(model.upgradeProblem)
        XCTAssertEqual(model.pairing?.token, "tdv_x", "the old key was dropped")
        XCTAssertEqual(CredentialStore.load()?.token, "tdv_x")
        XCTAssertFalse(model.operates)
        // Only the new key, never kept, is given back: one forget, with it.
        // It is sent just after the failure is reported.
        await eventually { StubProtocol.authorized.contains { $0.path == "/api/device/forget" } }
        XCTAssertEqual(StubProtocol.authorized.filter { $0.path == "/api/device/forget" }.map(\.token), ["tdv_new"])
        XCTAssertNil(model.upgrade)
    }

    func testAnApprovedUpgradeReplacesTheOldKey() async {
        StubProtocol.on("/auth/devices/request", .init(status: 201, body: Self.upgradeRequest))
        StubProtocol.on("/auth/devices/token", .init(status: 201, body: Self.issuedOperate))
        StubProtocol.on("/api/device/forget", .init(status: 200, body: #"{"removed":"dev_1"}"#))
        let model = pairedModel()
        model.requestHostsAndWork()
        await eventually(4) { model.operates }
        XCTAssertEqual(model.pairing?.token, "tdv_new")
        XCTAssertEqual(CredentialStore.load()?.token, "tdv_new")
        XCTAssertNil(model.scopeNote)
        // The old key is the one given back.
        await eventually { StubProtocol.authorized.contains { $0.path == "/api/device/forget" } }
        XCTAssertEqual(StubProtocol.authorized.filter { $0.path == "/api/device/forget" }.map(\.token), ["tdv_x"])
    }

    func testADecideHostLineNeverSaysNothingForAPartialRead() throws {
        func host(_ json: String) throws -> HostQueue {
            struct Hosts: Decodable { let hosts: [HostQueue] }
            return try JSONDecoder().decode(Hosts.self, from: Data(json.utf8)).hosts[0]
        }
        XCTAssertEqual(HostsView.waitingLine(try host(Self.emptyQueue)), "Nothing needs you")
        let offline = try host(#"{"hosts":[{"alias":"mini","local":false,"unavailable":["host is offline"],"queue":{"items":[]}}]}"#)
        XCTAssertEqual(HostsView.waitingLine(offline), "Could not be read in full: host is offline")
        XCTAssertFalse(HostsView.complete(offline))
        let unknown = try host(#"{"hosts":[{"alias":"here","local":true,"unavailable":[],"queue":{"items":[{"kind":"future_item"}],"truncated":false,"unavailable":[]}}]}"#)
        XCTAssertEqual(HostsView.waitingLine(unknown), "Could not be read in full")
        let truncated = try host(#"{"hosts":[{"alias":"here","local":true,"unavailable":[],"queue":{"items":[{"kind":"human_request","request_id":"r1","body":"Ship?","from_session_id":"s","from_pane":"%1","created_at":"2026-11-16T10:00:00Z"}],"truncated":true,"unavailable":[]}}]}"#)
        XCTAssertEqual(HostsView.waitingLine(truncated), "1 need you, maybe more")
    }
}

/// Notifications: the host's registration follows the person's last wish,
/// and a notification's button decides only on what the host says now.
extension LifecycleTests {
    private static let pushDevice = #"{"device":{"id":"dev_1","name":"phone"},"scope":["decide"],"push":{"enabled":true,"content_allowed":true,"environments":["development"]}}"#
    private static let pushKey = Data(repeating: 7, count: 32)
    private static var pushAnswer: String {
        #"{"enabled":true,"content":true,"channel":"ch_new","ref_key":"\#(pushKey.base64EncodedString())"}"#
    }
    private static let oneQuestion = #"{"hosts":[{"alias":"here","local":true,"unavailable":[],"queue":{"items":[{"kind":"human_request","request_id":"r1","body":"Ship?","from_session_id":"s1","from_pane":"%1","created_at":"2026-11-19T10:00:00Z"}],"truncated":false,"unavailable":[]}}]}"#

    /// Runs `body` with the notification preferences set, restoring them.
    private func withPreferences(wanted: Bool, content: Bool, _ body: () async throws -> Void) async rethrows {
        let defaults = UserDefaults.standard
        let saved = (defaults.object(forKey: AppModel.notificationsKey), defaults.object(forKey: AppModel.notificationContentKey), defaults.object(forKey: "pushRemovalOwed"))
        defaults.set(wanted, forKey: AppModel.notificationsKey)
        defaults.set(content, forKey: AppModel.notificationContentKey)
        defaults.removeObject(forKey: "pushRemovalOwed")
        defer {
            defaults.set(saved.0, forKey: AppModel.notificationsKey)
            defaults.set(saved.1, forKey: AppModel.notificationContentKey)
            defaults.set(saved.2, forKey: "pushRemovalOwed")
        }
        try await body()
    }

    private func answerPayload() -> PushPayload {
        let ref = PushRef.of(key: Self.pushKey, host: "here", kind: "request", id: "r1")
        return PushPayload(userInfo: ["tend": ["v": 1, "channel": "ch_new", "host": "here", "ref": ref]], category: PushCategory.question, body: "Ship?", subtitle: "%1")!
    }

    func testTurningOffWinsOverARegistrationInFlight() async {
        await withPreferences(wanted: true, content: false) {
            StubProtocol.on("/api/device", .init(status: 200, body: Self.pushDevice))
            StubProtocol.on("/api/device/push", .init(status: 200, body: Self.pushAnswer, delay: 0.5))
            StubProtocol.on("/api/device/push/remove", .init(status: 200, body: #"{"removed":true}"#))
            let model = pairedModel()
            await model.refreshDevice()
            let registering = Task { await model.receivedAPNsToken(Data(repeating: 1, count: 32)) }
            await eventually { StubProtocol.seen.contains("/api/device/push") }
            await model.turnNotificationsOff()
            await registering.value
            await eventually { StubProtocol.seen.contains("/api/device/push/remove") && model.pairing?.push == nil }
            XCTAssertFalse(model.notificationsWanted)
            XCTAssertNil(model.pairing?.push, "off lost to the late registration")
            XCTAssertTrue(StubProtocol.seen.contains("/api/device/push/remove"), "the host still has the registration")
        }
    }

    func testARegistrationThatCannotBeKeptIsTakenBack() async {
        await withPreferences(wanted: true, content: false) {
            StubProtocol.on("/api/device", .init(status: 200, body: Self.pushDevice))
            StubProtocol.on("/api/device/push", .init(status: 200, body: Self.pushAnswer))
            StubProtocol.on("/api/device/push/remove", .init(status: 200, body: #"{"removed":true}"#))
            let model = pairedModel()
            await model.refreshDevice()
            CredentialStore.failSaves = true
            defer { CredentialStore.failSaves = false }
            await model.receivedAPNsToken(Data(repeating: 1, count: 32))
            XCTAssertNil(model.pairing?.push)
            XCTAssertNotNil(model.pushProblem)
            XCTAssertTrue(StubProtocol.seen.contains("/api/device/push/remove"), "a registration this phone cannot use was left on the host")
        }
    }

    func testTheRegistrationFingerprintMatchesTheHosts() {
        // crates/tend/src/dashboard/push/tests.rs has the same vector.
        XCTAssertEqual(
            AppModel.registrationFingerprint(token: "AABBCCDDEEFF00112233445566778899aabbccddeeff00112233445566778899", environment: "production", channel: "ch_0123"),
            "344b519bd7247b4fe517b9edaf77801e"
        )
    }

    private func registrations() -> Int {
        StubProtocol.authorized.filter { $0.path == "/api/device/push" }.count
    }

    private static func pushDevice(holding registration: String?, content: Bool = false) -> String {
        let held = registration.map { #"{"registration":"\#($0)","content":\#(content)}"# } ?? "null"
        return String(Self.pushDevice.dropLast()) + #","push_registration":\#(held)}"#
    }

    func testAnOlderHostIsRegisteredOncePerLaunch() async {
        await withPreferences(wanted: true, content: false) {
            StubProtocol.on("/api/device", .init(status: 200, body: Self.pushDevice))
            // The host answers the content it will send: what was asked.
            StubProtocol.on("/api/device/push",
                            .init(status: 200, body: Self.pushAnswer.replacingOccurrences(of: #""content":true"#, with: #""content":false"#)),
                            .init(status: 200, body: Self.pushAnswer))
            let model = pairedModel()
            let token = Data(repeating: 1, count: 32)
            // A launch: the device read, the token, foregrounding twice, and
            // iOS handing the same token again (2026-11-24: four POSTs).
            await model.refreshDevice()
            await model.receivedAPNsToken(token)
            await model.refreshDevice()
            await model.refreshDevice()
            await model.receivedAPNsToken(token)
            XCTAssertEqual(registrations(), 1, "the same token was registered more than once")
            // A new wish or a new token is sent.
            model.notificationContentWanted = true
            await model.registerPush()
            XCTAssertEqual(registrations(), 2)
            await model.receivedAPNsToken(Data(repeating: 2, count: 32))
            XCTAssertEqual(registrations(), 3)
        }
    }

    func testAHostHoldingThisRegistrationIsNotSentItAgain() async {
        await withPreferences(wanted: true, content: false) {
            let token = Data(repeating: 1, count: 32)
            let hex = token.map { String(format: "%02x", $0) }.joined()
            let held = AppModel.registrationFingerprint(token: hex, environment: AppModel.pushEnvironment, channel: "ch_new")
            StubProtocol.on("/api/device",
                            .init(status: 200, body: Self.pushDevice(holding: nil)),
                            .init(status: 200, body: Self.pushDevice(holding: held)),
                            .init(status: 200, body: Self.pushDevice(holding: held)),
                            .init(status: 200, body: Self.pushDevice(holding: nil)),
                            .init(status: 200, body: Self.pushDevice(holding: held, content: true)))
            StubProtocol.on("/api/device/push", .init(status: 200, body: Self.pushAnswer.replacingOccurrences(of: #""content":true"#, with: #""content":false"#)))
            let model = pairedModel()
            await model.refreshDevice()
            await model.receivedAPNsToken(token)
            XCTAssertEqual(registrations(), 1)
            XCTAssertNotNil(model.pairing?.push)
            // A relaunch: the binding from the Keychain, the host holds it.
            let relaunched = AppModel()
            await relaunched.refreshDevice()
            await relaunched.receivedAPNsToken(token)
            await relaunched.refreshDevice()
            XCTAssertEqual(registrations(), 1, "a registration the host holds was sent again")
            // The host dropped it (Apple's 410, say): sent again.
            await relaunched.refreshDevice()
            XCTAssertEqual(registrations(), 2, "a registration the host lost was not sent again")
            // The host holds other content than wished: sent again.
            await relaunched.refreshDevice()
            XCTAssertEqual(registrations(), 3)
        }
    }

    func testAHostThatNowAllowsContentIsRegisteredAgain() async {
        await withPreferences(wanted: true, content: true) {
            let token = Data(repeating: 1, count: 32)
            let hex = token.map { String(format: "%02x", $0) }.joined()
            let held = AppModel.registrationFingerprint(token: hex, environment: AppModel.pushEnvironment, channel: "ch_new")
            let withheld = Self.pushDevice(holding: nil).replacingOccurrences(of: #""content_allowed":true"#, with: #""content_allowed":false"#)
            StubProtocol.on("/api/device",
                            .init(status: 200, body: withheld),
                            .init(status: 200, body: Self.pushDevice(holding: held, content: true)))
            StubProtocol.on("/api/device/push",
                            .init(status: 200, body: Self.pushAnswer.replacingOccurrences(of: #""content":true"#, with: #""content":false"#)),
                            .init(status: 200, body: Self.pushAnswer))
            let model = pairedModel()
            await model.refreshDevice()
            await model.receivedAPNsToken(token)
            XCTAssertEqual(model.pairing?.push?.content, false)
            // The host turns content on: the same row, but the binding is stale.
            await model.refreshDevice()
            XCTAssertEqual(registrations(), 2, "the binding still says no content")
            XCTAssertEqual(model.pairing?.push?.content, true)
        }
    }

    func testARemovalAtSignOutIsNotTakenAsRegistered() async {
        await withPreferences(wanted: true, content: false) {
            StubProtocol.on("/api/device", .init(status: 200, body: Self.pushDevice))
            StubProtocol.on("/api/device/push", .init(status: 200, body: Self.pushAnswer.replacingOccurrences(of: #""content":true"#, with: #""content":false"#)))
            StubProtocol.on("/api/device/push/remove", .init(status: 200, body: #"{"removed":true}"#))
            StubProtocol.on("/api/device/forget", .init(status: 500, body: #"{"error":"down"}"#))
            let model = pairedModel()
            let token = Data(repeating: 1, count: 32)
            await model.refreshDevice()
            await model.receivedAPNsToken(token)
            await model.signOut()
            XCTAssertNotNil(model.pairing, "sign out failed, so the pairing stays")
            await model.receivedAPNsToken(token)
            XCTAssertEqual(registrations(), 2, "the removed registration was taken as still held")
        }
    }

    func testALostAnswerIsNotTakenAsRegistered() async {
        await withPreferences(wanted: true, content: false) {
            StubProtocol.on("/api/device", .init(status: 200, body: Self.pushDevice))
            StubProtocol.on("/api/device/push",
                            .init(status: 200, body: Self.pushAnswer.replacingOccurrences(of: #""content":true"#, with: #""content":false"#)),
                            .init(status: 500, body: #"{"error":"lost"}"#),
                            .init(status: 200, body: Self.pushAnswer.replacingOccurrences(of: #""content":true"#, with: #""content":false"#)))
            let model = pairedModel()
            let token = Data(repeating: 1, count: 32)
            await model.refreshDevice()
            await model.receivedAPNsToken(token)
            // Content on; the host may have stored it, the answer is lost.
            model.notificationContentWanted = true
            await model.registerPush()
            XCTAssertEqual(registrations(), 2)
            // Back off: the host may hold content, so it is told.
            model.notificationContentWanted = false
            await model.registerPush()
            XCTAssertEqual(registrations(), 3, "content may still be on at the host")
        }
    }

    func testAnUpgradedPairingRegistersAgain() async {
        await withPreferences(wanted: true, content: false) {
            StubProtocol.on("/api/device", .init(status: 200, body: Self.pushDevice))
            StubProtocol.on("/api/device/push", .init(status: 200, body: Self.pushAnswer))
            StubProtocol.on("/auth/devices/request", .init(status: 201, body: #"{"device_code":"d2","user_code":"UPGR-ADE2","expires_in":600,"interval":1}"#))
            StubProtocol.on("/auth/devices/token", .init(status: 201, body: #"{"device":{"id":"dev_2","name":"phone"},"token":"tdv_new","scope":["operate"]}"#))
            StubProtocol.on("/api/device/forget", .init(status: 200, body: #"{"removed":"dev_1"}"#))
            let model = pairedModel()
            await model.refreshDevice()
            await model.receivedAPNsToken(Data(repeating: 1, count: 32))
            XCTAssertNotNil(model.pairing?.push)
            model.requestHostsAndWork()
            await eventually(4) { model.pairing?.token == "tdv_new" && model.pairing?.push != nil }
            XCTAssertEqual(model.pairing?.token, "tdv_new")
            XCTAssertNotNil(model.pairing?.push, "the new key was never registered")
            let registrations = StubProtocol.authorized.filter { $0.path == "/api/device/push" }.map(\.token)
            XCTAssertEqual(registrations.last, "tdv_new")
        }
    }

    func testAButtonReadsTheHostsConsentNow() async {
        await withPreferences(wanted: true, content: true) {
            StubProtocol.on("/api/device",
                            .init(status: 200, body: Self.pushDevice),
                            .init(status: 200, body: Self.pushDevice.replacingOccurrences(of: #""content_allowed":true"#, with: #""content_allowed":false"#)))
            StubProtocol.on("/api/decisions", .init(status: 200, body: Self.oneQuestion))
            StubProtocol.on("/api/decisions/requests/r1", .init(status: 200, body: "{}"))
            let model = pairedModel()
            await model.refreshDevice()
            model.adoptTestBinding(PushBinding(channel: "ch_new", refKey: Self.pushKey, content: true))
            let result = await model.act(on: answerPayload(), action: PushCategory.answer, text: "Go ahead")
            XCTAssertEqual(result, .refused(.contentOff))
            XCTAssertFalse(StubProtocol.seen.contains("/api/decisions/requests/r1"), "decided on the host's old consent")
        }
    }

    func testALostAnswerIsNotCalledUndecided() async {
        await withPreferences(wanted: true, content: true) {
            StubProtocol.on("/api/device", .init(status: 200, body: Self.pushDevice))
            StubProtocol.on("/api/decisions", .init(status: 200, body: Self.oneQuestion))
            StubProtocol.on("/api/decisions/requests/r1",
                            .init(status: 502, body: #"{"error":"relay lost"}"#),
                            .init(status: 409, body: #"{"error":"already answered"}"#))
            let model = pairedModel()
            await model.refreshDevice()
            model.adoptTestBinding(PushBinding(channel: "ch_new", refKey: Self.pushKey, content: true))
            let lost = await model.act(on: answerPayload(), action: PushCategory.answer, text: "Go ahead")
            XCTAssertEqual(lost, .unknown)
            let refused = await model.act(on: answerPayload(), action: PushCategory.answer, text: "Go ahead")
            XCTAssertEqual(refused, .refused(.changed))
            XCTAssertEqual(StubProtocol.authorized.filter { $0.path == "/api/decisions/requests/r1" }.count, 2)
        }
    }
}
