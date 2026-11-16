// Home with every lane, the hosts and an agent, on a device paired with
// hosts and new work (`operate`), against a real scratch tendd seeded with
// a question, a result and agents at work and finished
// (apps/tend-ios/Scripts/scratch-tendd.sh up && … seed-home). Screenshots
// of each screen are kept as attachments, for the side-by-side with Tend
// for Mac.
import XCTest

final class HomeUITests: XCTestCase {
    private var server: String!
    private var token: String!

    override func setUpWithError() throws {
        continueAfterFailure = false
        let env = ProcessInfo.processInfo.environment
        guard let server = env["TEND_SERVER"], let token = env["TEND_TOKEN"] else {
            throw XCTSkip("set TEST_RUNNER_TEND_SERVER and TEST_RUNNER_TEND_TOKEN to a scratch tendd")
        }
        self.server = server
        self.token = token
    }

    private var isPad: Bool { UIDevice.current.userInterfaceIdiom == .pad }

    private func operatorCall(_ method: String, _ path: String, body: [String: Any]? = nil) throws -> (Int, Any?) {
        var request = URLRequest(url: URL(string: server + path)!)
        request.httpMethod = method
        request.setValue("Bearer \(token!)", forHTTPHeaderField: "Authorization")
        if let body {
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.httpBody = try JSONSerialization.data(withJSONObject: body)
        }
        let done = expectation(description: path)
        final class Box: @unchecked Sendable { var status = 0; var data: Data? }
        let box = Box()
        URLSession.shared.dataTask(with: request) { data, response, _ in
            box.status = (response as? HTTPURLResponse)?.statusCode ?? 0
            box.data = data
            done.fulfill()
        }.resume()
        wait(for: [done], timeout: 15)
        return (box.status, box.data.flatMap { try? JSONSerialization.jsonObject(with: $0) })
    }

    func testHomeLanesHostsAndAnAgent() throws {
        let deviceName = "Home \(isPad ? "iPad" : "iPhone") \(Int.random(in: 1000...9999))"
        // iPad in landscape: Home has room for the Mac's three columns.
        XCUIDevice.shared.orientation = isPad ? .landscapeLeft : .portrait
        let app = XCUIApplication()
        app.launchArguments = ["-TendResetState", "-AppleLanguages", "(en)"]
        app.launchEnvironment["TEND_SERVER"] = server
        app.launch()

        let name = app.textFields["deviceName"]
        XCTAssertTrue(name.waitForExistence(timeout: 10))
        attach(app, "pairing")
        name.tap()
        name.typeText(deviceName)
        app.buttons["scope-operate"].tap()
        app.buttons["pairButton"].tap()
        let code = app.staticTexts["userCode"]
        XCTAssertTrue(code.waitForExistence(timeout: 10), "no pairing code; is the scratch tendd up?")

        // The operator looks the code up first: it asked for hosts and work.
        let (lookupStatus, looked) = try operatorCall("POST", "/api/devices/lookup", body: ["user_code": code.label])
        XCTAssertEqual(lookupStatus, 200)
        XCTAssertEqual(((looked as? [String: Any])?["request"] as? [String: Any])?["scope"] as? String, "operate")
        let (status, _) = try operatorCall("POST", "/api/devices/approve", body: ["user_code": code.label, "scope": "operate"])
        XCTAssertEqual(status, 200)

        // Your turn, In progress and Finished, with the seeded items.
        XCTAssertTrue(app.buttons["acceptButton"].firstMatch.waitForExistence(timeout: 15), "the result never showed")
        if !app.buttons["lane-inProgress"].exists {
            XCTAssertTrue(app.staticTexts["Add idempotency keys to the refund endpoint"].firstMatch.waitForExistence(timeout: 15))
            XCTAssertTrue(app.staticTexts["Cut the cache warm-up time in half"].firstMatch.exists)
            attach(app, "home")
            // In portrait the lanes become the strip.
            XCUIDevice.shared.orientation = .portrait
            XCTAssertTrue(app.buttons["lane-inProgress"].waitForExistence(timeout: 5))
            attach(app, "home-portrait")
            app.buttons["lane-finished"].tap()
        } else {
            attach(app, "home-your-turn")
            app.buttons["lane-inProgress"].tap()
            XCTAssertTrue(app.scrolledText("Add idempotency keys to the refund endpoint").exists)
            attach(app, "home-in-progress")
            app.buttons["lane-finished"].tap()
            XCTAssertTrue(app.scrolledText("Cut the cache warm-up time in half").exists)
            attach(app, "home-finished")
        }

        // An agent: its last result, rendered from Markdown.
        app.scrolledText("Cut the cache warm-up time in half").tap()
        let summary = app.staticTexts["agentSummary"].firstMatch
        XCTAssertTrue(summary.waitForExistence(timeout: 10))
        XCTAssertTrue(summary.label.contains("41 s to 17 s"), summary.label)
        XCTAssertFalse(summary.label.contains("**"), "Markdown was shown raw")
        attach(app, "agent")

        // Hosts.
        if isPad {
            let sidebarHost = app.descendants(matching: .any)["sidebarHost"].firstMatch
            if !sidebarHost.waitForExistence(timeout: 2) {
                app.buttons["Show Sidebar"].firstMatch.tap()
            }
            XCTAssertTrue(sidebarHost.waitForExistence(timeout: 5))
            sidebarHost.tap()
            // Close the overlay sidebar so the host is seen whole.
            if sidebarHost.isHittable {
                app.coordinate(withNormalizedOffset: CGVector(dx: 0.98, dy: 0.5)).tap()
            }
        } else {
            app.navigationBars.buttons.firstMatch.tap()
            app.tabBars.buttons["Hosts"].tap()
            XCTAssertTrue(app.buttons["hostRow"].firstMatch.waitForExistence(timeout: 10) || app.otherElements["hostRow"].firstMatch.waitForExistence(timeout: 1))
            attach(app, "hosts")
            app.buttons["hostRow"].firstMatch.tap()
        }
        XCTAssertTrue(app.staticTexts["Make the settings page load without the blank flash"].firstMatch.waitForExistence(timeout: 10))
        attach(app, "host")

        // Clean up: unpair through the operator, by name.
        let (_, devices) = try operatorCall("GET", "/api/devices")
        for device in ((devices as? [String: Any])?["devices"] as? [[String: Any]]) ?? [] where device["name"] as? String == deviceName {
            if let id = device["id"] as? String { _ = try operatorCall("POST", "/api/devices/\(id)/remove") }
        }
    }

    private func attach(_ app: XCUIApplication, _ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
        hostShot(name)
    }

    /// With `TEST_RUNNER_TEND_SHOT_DIR` set, ask the host to take this
    /// screen with `simctl io screenshot` (XCUITest's own screenshots of a
    /// rotated iPad come out clipped) and wait until it has.
    private func hostShot(_ name: String) {
        guard let dir = ProcessInfo.processInfo.environment["TEND_SHOT_DIR"] else { return }
        let request = URL(fileURLWithPath: dir).appendingPathComponent("request")
        let done = URL(fileURLWithPath: dir).appendingPathComponent("done-\(name)")
        try? name.write(to: request, atomically: true, encoding: .utf8)
        let end = Date().addingTimeInterval(10)
        while !FileManager.default.fileExists(atPath: done.path), Date() < end {
            Thread.sleep(forTimeInterval: 0.2)
        }
    }
}
