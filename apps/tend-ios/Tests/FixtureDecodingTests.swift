import XCTest
@testable import Tend

/// The shared models decode what a real tendd sent (apps/shared/Fixtures).
final class FixtureDecodingTests: XCTestCase {
    private func fixture(_ name: String) throws -> Data {
        let url = try XCTUnwrap(Bundle(for: Self.self).url(forResource: name, withExtension: "json"), name)
        return try Data(contentsOf: url)
    }

    private struct Hosts<T: Decodable>: Decodable { let hosts: [T] }

    func testQueueFromARealDaemon() throws {
        let hosts = try JSONDecoder().decode(Hosts<HostQueue>.self, from: fixture("decisions")).hosts
        XCTAssertEqual(hosts.count, 1)
        let local = hosts[0]
        XCTAssertTrue(local.local)
        XCTAssertTrue(local.queue.isComplete)
        let decisions = PendingDecision.from(local)
        XCTAssertEqual(decisions.count, 3)
        XCTAssertTrue(decisions.allSatisfy(\.decidable))
        let question = try XCTUnwrap(decisions.first(where: \.isQuestion))
        XCTAssertTrue(question.title.hasPrefix("Ship the storage migration"))
        XCTAssertNotNil(question.since)
        let result = try XCTUnwrap(decisions.first(where: { !$0.isQuestion }))
        XCTAssertEqual(result.title, "Merge the retry path now?")
        XCTAssertEqual(result.options, ["merge now", "wait for the perf run"])
        XCTAssertEqual(result.recommendation, "merge now: the perf run only covers reads")
        XCTAssertEqual(result.evidence.count, 2)
        XCTAssertTrue(result.detail?.hasPrefix("Retry path") == true)
    }

    func testFleetSectionsUnknownKindsAndHostsThatCannotDecide() throws {
        let hosts = try JSONDecoder().decode(Hosts<HostQueue>.self, from: fixture("decisions-fleet")).hosts
        XCTAssertEqual(hosts.map(\.alias).count, 3)
        // An item kind this app does not know is counted, not fatal.
        XCTAssertEqual(hosts[0].queue.undecoded, 1)
        XCTAssertFalse(hosts[0].queue.isComplete)
        XCTAssertEqual(PendingDecision.from(hosts[0]).count, 3)
        // An offline host says why it is empty.
        XCTAssertEqual(hosts[1].unavailable, ["decision queue: host is offline"])
        // An observe-only host's waiting agent is shown, not decided here.
        let waiting = try XCTUnwrap(PendingDecision.from(hosts[2]).first)
        XCTAssertFalse(waiting.decidable)
        XCTAssertEqual(waiting.title, "Allow shell command?")
    }

    func testHistoryFromARealDaemon() throws {
        let hosts = try JSONDecoder().decode(Hosts<HostHistory>.self, from: fixture("decisions-history")).hosts
        let items = hosts.flatMap { section in section.history.items.map { PastDecision(item: $0, host: section.alias) } }
        XCTAssertEqual(items.count, 4)
        XCTAssertEqual(hosts[0].history.undecoded, 0)
        XCTAssertTrue(items.contains { $0.outcome == "Answered" && $0.text == "Yes, behind the flag" })
        let accepted = try XCTUnwrap(items.first { $0.outcome == "Accepted" })
        XCTAssertTrue(accepted.by?.hasPrefix("app:UI test") == true)
        XCTAssertNotNil(accepted.at)
    }
}
