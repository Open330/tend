import XCTest
@testable import Tend

/// Looks Good on a finished unit (docs/proposals/PARITY.md P6): offered
/// only on a device that reads the board, for a finished unit not yet set
/// aside that carries its review key; posted with the unit's id and key;
/// a success folds it under "Show N reviewed", a 409 says it changed.
@MainActor
final class LooksGoodTests: XCTestCase {
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

    private static let homeDevice = #"{"device":{"id":"dev_1","name":"phone"},"scope":["operate"],"features":["home"]}"#
    private static let fleetJSON = #"{"hosts":[{"alias":"here","local":true,"state":"online","remote":{"agents":[]}}]}"#

    private static func board(reviewed: Bool, key: String? = "home:k1") -> String {
        let keyField = key.map { #", "review_key": "\#($0)""# } ?? ""
        let unit = #"{"kind": "unit", "id": "u1", "lane": "finished", "title": "Done", "recent": true, "reviewed": \#(reviewed)\#(keyField)}"#
        return #"{"hosts":[{"alias":"here","local":true}],"decision_hosts":[{"alias":"here","local":true}],"reviewed_marks":true,"lanes":{"needs_you":{"groups":[]},"finished":{"groups":[{"project":"tend","items":[\#(unit)]}]}}}"#
    }

    private func pair(_ scope: DeviceScope) -> AppModel {
        let origin = try! ServerOrigin(parsing: "https://tend.test")
        CredentialStore.save(Pairing(origin: origin, device: PairedDevice(id: "dev_1", name: "phone"), token: "tdv_x", scope: scope))
        return AppModel()
    }

    private func operateModel(_ boards: String...) async -> AppModel {
        StubProtocol.on("/api/device", .init(status: 200, body: Self.homeDevice))
        StubProtocol.on("/api/device/fleet", .init(status: 200, body: Self.fleetJSON))
        for board in boards { StubProtocol.on("/api/device/home", .init(status: 200, body: board)) }
        let model = pair(.operate)
        await model.refreshDevice()
        await model.refresh()
        return model
    }

    private func unit(_ model: AppModel) throws -> BoardUnit {
        let board = try XCTUnwrap(model.visibleBoard)
        for case .unit(let unit) in board.items(.finished) { return unit }
        throw NoUnit()
    }

    func testTheReviewKeyIsReadFromTheRecordedBoard() throws {
        let url = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "device-home", withExtension: "json"))
        let board = try JSONDecoder().decode(DeviceHome.self, from: Data(contentsOf: url))
        let finished = board.items(.finished).compactMap { if case .unit(let u) = $0 { u } else { nil } }
        XCTAssertFalse(finished.isEmpty)
        XCTAssertTrue(finished.allSatisfy { $0.reviewKey?.hasPrefix("home:") == true })
        // A key that is not a string is no key, never fatal.
        let odd = try JSONDecoder().decode(DeviceHome.self, from: Data(Self.board(reviewed: false, key: nil).replacingOccurrences(of: #""reviewed": false"#, with: #""reviewed": false, "review_key": 7"#).utf8))
        if case .unit(let u)? = odd.items(.finished).first { XCTAssertNil(u.reviewKey) } else { XCTFail("no unit") }
    }

    func testPostsTheIdAndKeyThenFoldsItUnderReviewed() async throws {
        StubProtocol.on("/api/device/home/reviewed", .init(status: 200, body: #"{"review":{"id":"rv_1"},"review_key":"home:k1"}"#))
        let model = await operateModel(Self.board(reviewed: false), Self.board(reviewed: true))
        let shown = try unit(model)
        XCTAssertTrue(model.canMarkReviewed(shown))
        let marked = await model.markReviewed(shown)
        XCTAssertTrue(marked)
        let body = try XCTUnwrap(StubProtocol.bodies["/api/device/home/reviewed"])
        XCTAssertEqual(try JSONSerialization.jsonObject(with: body) as? [String: String], ["id": "u1", "review_key": "home:k1"])
        XCTAssertEqual(StubProtocol.authorized.first { $0.path == "/api/device/home/reviewed" }?.token, "tdv_x")
        // The board was read again: the unit is set aside, the button gone.
        XCTAssertEqual(StubProtocol.seen.filter { $0 == "/api/device/home" }.count, 2)
        let after = try unit(model)
        XCTAssertTrue(after.reviewed)
        XCTAssertFalse(model.canMarkReviewed(after))
        let board = try XCTUnwrap(model.visibleBoard)
        XCTAssertEqual(FoldedLane(board, lane: .finished).reviewed.map(\.id), ["u1"])
        XCTAssertEqual(board.reviewedCount(.finished), 1)
        XCTAssertNil(model.failures[AppModel.reviewSlot(shown)])
    }

    func testAResultThatChangedSaysSoAndReadsTheBoardAgain() async throws {
        StubProtocol.on("/api/device/home/reviewed", .init(status: 409, body: #"{"error":"this result changed since it was shown; look again"}"#))
        let model = await operateModel(Self.board(reviewed: false), Self.board(reviewed: false, key: "home:k2"))
        let shown = try unit(model)
        let marked = await model.markReviewed(shown)
        XCTAssertFalse(marked)
        XCTAssertEqual(model.failures[AppModel.reviewSlot(shown)], "This result changed since it was shown; look again.")
        XCTAssertEqual(StubProtocol.seen.filter { $0 == "/api/device/home" }.count, 2)
        let after = try unit(model)
        XCTAssertFalse(after.reviewed, "nothing was marked")
        XCTAssertEqual(after.reviewKey, "home:k2")
    }

    func testAnyOtherFailureMarksNothing() async throws {
        StubProtocol.on("/api/device/home/reviewed", .init(status: 404, body: #"{"error":"no such unit on the board"}"#))
        let model = await operateModel(Self.board(reviewed: false))
        let shown = try unit(model)
        let marked = await model.markReviewed(shown)
        XCTAssertFalse(marked)
        XCTAssertEqual(model.failures[AppModel.reviewSlot(shown)], "no such unit on the board")
        XCTAssertFalse(try unit(model).reviewed)
    }

    func testOfferedOnlyWhereItCanBeRecorded() async throws {
        let model = await operateModel(Self.board(reviewed: false))
        let decode = { (json: String) throws -> BoardUnit in
            let board = try JSONDecoder().decode(DeviceHome.self, from: Data(json.utf8))
            for lane in Lane.board { for case .unit(let u) in board.items(lane) { return u } }
            throw NoUnit()
        }
        XCTAssertTrue(model.canMarkReviewed(try decode(Self.board(reviewed: false))))
        XCTAssertFalse(model.canMarkReviewed(try decode(Self.board(reviewed: true))), "already set aside")
        XCTAssertFalse(model.canMarkReviewed(try decode(Self.board(reviewed: false, key: nil))), "no key, nothing to name")
        let running = Self.board(reviewed: false).replacingOccurrences(of: #""lane": "finished""#, with: #""lane": "running""#)
        XCTAssertFalse(model.canMarkReviewed(try decode(running)), "only finished units")

        // A device that only decides never offers it, nor posts.
        StubProtocol.reset()
        CredentialStore.clear()
        let decide = pair(.decide)
        let unit = try decode(Self.board(reviewed: false))
        XCTAssertFalse(decide.canMarkReviewed(unit))
        let marked = await decide.markReviewed(unit)
        XCTAssertFalse(marked)
        XCTAssertFalse(StubProtocol.seen.contains("/api/device/home/reviewed"))
    }

    func testA403SaysThisDeviceOnlyDecides() async throws {
        StubProtocol.on("/api/device/home/reviewed", .init(status: 403, body: #"{"error":"decide only"}"#))
        let model = await operateModel(Self.board(reviewed: false))
        let shown = try unit(model)
        _ = await model.markReviewed(shown)
        XCTAssertEqual(model.failures[AppModel.reviewSlot(shown)], DashboardError.notAllowed.localizedDescription)
    }
}

private struct NoUnit: Error {}
