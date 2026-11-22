// Pairing for hosts and new work, then Your turn's Can't be answered group,
// against a real scratch tendd. Keep puts a question back in Your turn,
// and Close all closes what is still listed. Settings says this dashboard
// sends no notifications.
//
// The group needs questions whose askers have been gone for tendd's grace
// (15 minutes), so this test runs only when asked:
//
//   apps/tend-ios/Scripts/scratch-tendd.sh up
//   apps/tend-ios/Scripts/scratch-tendd.sh seed-gone   # prints TEND_GONE_IDS and _NONCE; wait 16 minutes
//   TEST_RUNNER_TEND_SERVER=http://127.0.0.1:7979 \
//   TEST_RUNNER_TEND_TOKEN=$(apps/tend-ios/Scripts/scratch-tendd.sh token) \
//   TEST_RUNNER_TEND_GONE=1 \
//   TEST_RUNNER_TEND_GONE_IDS=<the ids seed-gone printed> \
//   TEST_RUNNER_TEND_GONE_NONCE=<the nonce seed-gone printed> \
//   xcodebuild test -scheme Tend -only-testing:TendUITests/CantAnswerUITests …
//
// Close all closes every question the group lists. The test therefore
// talks only to a dashboard on this machine (loopback), and before every
// step that changes anything it requires that the dashboard's only Can't be
// answered questions are exactly the seeded ids, on its local host, each
// carrying the seeding's nonce, and that the group lists exactly those. It
// stops on anything else, so a dashboard holding someone's real questions
// fails here instead of losing them.
import XCTest

final class CantAnswerUITests: XCTestCase {
    private var server: String!
    private var token: String!
    private var fixtures: Set<String> = []
    private var nonce = ""

    override func setUpWithError() throws {
        continueAfterFailure = false
        let env = ProcessInfo.processInfo.environment
        guard env["TEND_GONE"] == "1", let server = env["TEND_SERVER"], let token = env["TEND_TOKEN"],
              let ids = env["TEND_GONE_IDS"], let nonce = env["TEND_GONE_NONCE"] else {
            throw XCTSkip("set TEST_RUNNER_TEND_SERVER, TEST_RUNNER_TEND_TOKEN, TEST_RUNNER_TEND_GONE=1 and TEST_RUNNER_TEND_GONE_IDS/_NONCE (from scratch-tendd.sh seed-gone, 16 minutes earlier)")
        }
        guard let host = URL(string: server)?.host, ["127.0.0.1", "localhost", "::1"].contains(host) else {
            throw XCTSkip("TEST_RUNNER_TEND_SERVER must be a scratch tendd on this machine, not \(server)")
        }
        guard nonce.count >= 12, nonce.allSatisfy(\.isHexDigit) else { throw XCTSkip("TEST_RUNNER_TEND_GONE_NONCE is not seed-gone's nonce") }
        self.nonce = nonce
        self.server = server
        self.token = token
        fixtures = Set(ids.split(separator: ",").map { String($0).trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty })
        guard fixtures.count >= 2 else { throw XCTSkip("TEST_RUNNER_TEND_GONE_IDS needs the two ids seed-gone printed") }
    }

    /// One open question on the dashboard, as the operator reads it.
    private struct Open: Equatable {
        let id: String
        let body: String
        let local: Bool
        let unanswerable: Bool
    }

    /// Every open question, from a complete, decoded read.
    private func openQuestions() throws -> [String: Open] {
        let (status, text) = try operatorCall("GET", "/api/decisions")
        XCTAssertEqual(status, 200, text)
        let root = try XCTUnwrap(try JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any])
        let hosts = try XCTUnwrap(root["hosts"] as? [[String: Any]])
        var open: [String: Open] = [:]
        for host in hosts {
            let queue = try XCTUnwrap(host["queue"] as? [String: Any])
            XCTAssertEqual(queue["truncated"] as? Bool, false, "a truncated queue cannot show what is safe to close")
            for item in queue["items"] as? [[String: Any]] ?? [] where item["kind"] as? String == "human_request" {
                let id = try XCTUnwrap(item["request_id"] as? String)
                open[id] = Open(id: id, body: item["body"] as? String ?? "", local: host["local"] as? Bool == true,
                                unanswerable: item["unanswerable"] is [String: Any])
            }
        }
        return open
    }

    /// The only questions Close all could take are the seeded ones, on this
    /// dashboard's own host. Anything else stops the test.
    private func requireOnlyFixturesUnanswerable(_ expected: Set<String>) throws -> [String: Open] {
        let open = try openQuestions()
        let unanswerable = Set(open.values.filter(\.unanswerable).map(\.id))
        guard unanswerable == expected,
              expected.allSatisfy({ open[$0]?.local == true && open[$0]?.body.contains("[\(nonce)]") == true }) else {
            XCTFail("not a scratch dashboard with exactly the seeded questions: Can't be answered holds \(unanswerable.sorted()), expected \(expected.sorted())")
            throw XCTSkip("stopped before Keep or Close all")
        }
        return open
    }

    private func operatorCall(_ method: String, _ path: String, body: [String: Any]? = nil) throws -> (Int, String) {
        var request = URLRequest(url: URL(string: server + path)!)
        request.httpMethod = method
        request.setValue("Bearer \(token!)", forHTTPHeaderField: "Authorization")
        if let body {
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.httpBody = try JSONSerialization.data(withJSONObject: body)
        }
        let done = expectation(description: path)
        final class Box: @unchecked Sendable { var status = 0; var data = Data() }
        let box = Box()
        URLSession.shared.dataTask(with: request) { data, response, _ in
            box.status = (response as? HTTPURLResponse)?.statusCode ?? 0
            box.data = data ?? Data()
            done.fulfill()
        }.resume()
        wait(for: [done], timeout: 15)
        return (box.status, String(data: box.data, encoding: .utf8) ?? "")
    }

    private var isPad: Bool { UIDevice.current.userInterfaceIdiom == .pad }

    func testPairForHostsThenKeepAndCloseAll() throws {
        // Before the app is paired: this must be the scratch fixture.
        let seeded = try requireOnlyFixturesUnanswerable(fixtures)
        let deviceName = "UI gone \(isPad ? "iPad" : "iPhone") \(Int.random(in: 1000...9999))"
        let app = XCUIApplication()
        app.launchArguments = ["-TendResetState", "-AppleLanguages", "(en)"]
        app.launchEnvironment["TEND_SERVER"] = server
        app.launch()

        // Pair for deciding, hosts and new work; the "operator" approves it so.
        let name = app.textFields["deviceName"]
        XCTAssertTrue(name.waitForExistence(timeout: 10))
        name.tap()
        name.typeText(deviceName)
        app.buttons["scope-operate"].tap()
        app.buttons["pairButton"].tap()
        let code = app.staticTexts["userCode"]
        XCTAssertTrue(code.waitForExistence(timeout: 10), "no pairing code; is the scratch tendd up?")
        let (lookupStatus, looked) = try operatorCall("POST", "/api/devices/lookup", body: ["user_code": code.label])
        XCTAssertEqual(lookupStatus, 200)
        XCTAssertTrue(looked.contains(#""scope":"operate""#), "the request did not ask for operate: \(looked)")
        let (status, _) = try operatorCall("POST", "/api/devices/approve", body: ["user_code": code.label, "scope": "operate"])
        XCTAssertEqual(status, 200)

        // The group, folded, below Your turn.
        let group = app.buttons["cantAnswer"].firstMatch
        let end = Date().addingTimeInterval(30)
        while !group.exists, Date() < end {
            app.swipeUp(velocity: .slow)
            _ = group.waitForExistence(timeout: 1)
        }
        XCTAssertTrue(group.exists, "no Can't be answered group; did seed-gone run 16 minutes ago?")
        XCTAssertTrue(group.label.hasPrefix("Can't be answered ("), group.label)
        attach(app, "group-folded")
        group.tap()
        let rows = app.descendants(matching: .any).matching(identifier: "cantAnswerRow")
        XCTAssertTrue(rows.firstMatch.waitForExistence(timeout: 5))
        attach(app, "group-open")
        let before = rows.count

        XCTAssertEqual(before, fixtures.count, "the group does not list exactly the seeded questions")

        // Keep the first one: it leaves the group and is back in Your turn.
        let first = rows.firstMatch
        let kept = try XCTUnwrap(seeded.values.first { $0.unanswerable && first.staticTexts[$0.body].exists }, "the first row is not a seeded question")
        let title = kept.body
        _ = try requireOnlyFixturesUnanswerable(fixtures)
        first.buttons["keep"].tap()
        let keepEnd = Date().addingTimeInterval(20)
        while rows.count != before - 1, Date() < keepEnd { _ = group.waitForExistence(timeout: 1) }
        XCTAssertEqual(rows.count, before - 1, "the kept question is still in the group")
        XCTAssertTrue(group.label.hasPrefix("Can't be answered (\(before - 1))"), group.label)
        XCTAssertFalse((0..<rows.count).contains { rows.element(boundBy: $0).staticTexts[title].exists }, "the kept question is still listed")
        // Back in Your turn: open, no longer unanswerable, and on a card.
        let rest = fixtures.subtracting([kept.id])
        let afterKeep = try requireOnlyFixturesUnanswerable(rest)
        XCTAssertEqual(afterKeep[kept.id]?.unanswerable, false, "the kept question is not back in Your turn")
        attach(app, "kept")

        // Close all: the group lists exactly the remaining seeded questions,
        // one confirmation, then the report says what it closed.
        let restBodies = Set(rest.compactMap { afterKeep[$0]?.body })
        XCTAssertEqual(rows.count, rest.count, "the group lists more than the seeded questions")
        let rowBodies = Set((0..<rows.count).compactMap { index in
            restBodies.first { rows.element(boundBy: index).staticTexts[$0].exists }
        })
        XCTAssertEqual(rowBodies, restBodies, "the group does not list exactly the seeded questions")
        app.buttons["closeAll"].firstMatch.tap()
        let confirm = app.buttons["Close \(rest.count)"].firstMatch
        XCTAssertTrue(confirm.waitForExistence(timeout: 5), "no Close confirmation")
        attach(app, "close-all-confirm")
        _ = try requireOnlyFixturesUnanswerable(rest)
        confirm.tap()
        let report = app.descendants(matching: .any)["closeAllResult"].firstMatch
        XCTAssertTrue(report.waitForExistence(timeout: 15), "no Close all report")
        XCTAssertTrue(report.label.contains(rest.count == 1 ? "Closed 1 question." : "Closed \(rest.count) questions."), report.label)
        attach(app, "closed")
        let after = try openQuestions()
        for id in rest { XCTAssertNil(after[id], "\(id) is still open") }
        XCTAssertEqual(after[kept.id]?.unanswerable, false, "Close all took the kept question too")
        // The kept question's card, above the group in Your turn.
        let card = app.descendants(matching: .any).matching(identifier: "card")
            .containing(NSPredicate(format: "identifier == 'cardTitle' AND label == %@", title)).firstMatch
        let cardEnd = Date().addingTimeInterval(15)
        while !card.exists, Date() < cardEnd { app.swipeDown(velocity: .slow) }
        XCTAssertTrue(card.exists, "the kept question is not shown in Your turn")

        // Settings: this dashboard sends no notifications, and says who can
        // turn them on.
        if isPad {
            let settings = app.descendants(matching: .any)["sidebarSettings"].firstMatch
            if !settings.waitForExistence(timeout: 2) { app.buttons["Show Sidebar"].firstMatch.tap() }
            XCTAssertTrue(settings.waitForExistence(timeout: 5))
            settings.tap()
        } else {
            app.tabBars.buttons["Settings"].tap()
        }
        let scope = app.staticTexts["scopeLabel"].firstMatch
        XCTAssertTrue(scope.waitForExistence(timeout: 10))
        XCTAssertTrue(scope.label.hasSuffix("Decide, hosts and new work"), scope.label)
        let off = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'This dashboard does not send notifications'")).firstMatch
        let scrollEnd = Date().addingTimeInterval(10)
        while !off.exists, Date() < scrollEnd { app.swipeUp(velocity: .slow) }
        XCTAssertTrue(off.exists, "Settings does not say notifications are unavailable")
        XCTAssertFalse(app.switches["notificationsToggle"].exists)
        attach(app, "push-unavailable")
    }

    private func attach(_ app: XCUIApplication, _ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }
}
