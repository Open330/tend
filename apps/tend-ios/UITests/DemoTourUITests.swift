// The iPhone and iPad part of the product demo video
// (.github/workflows/demo-video.yml): pair with the demo fixture's
// dashboard, then move through Home at a pace a viewer can follow while
// `simctl io recordVideo` records the simulator. Skipped unless
// TEST_RUNNER_TEND_DEMO_TOUR, TEST_RUNNER_TEND_SERVER and
// TEST_RUNNER_TEND_TOKEN are set. Prints `DEMO-MARK <name> <epoch>` lines
// the editor cuts the footage by.
import XCTest

final class DemoTourUITests: XCTestCase {
    private var server: String!
    private var token: String!

    override func setUpWithError() throws {
        continueAfterFailure = false
        let env = ProcessInfo.processInfo.environment
        // Only the demo fixture's dashboard, on this machine's loopback.
        guard env["TEND_DEMO_TOUR"] == "1", let server = env["TEND_SERVER"], let token = env["TEND_TOKEN"],
              server.hasPrefix("http://127.0.0.1:") else {
            throw XCTSkip("set TEST_RUNNER_TEND_DEMO_TOUR=1, TEST_RUNNER_TEND_SERVER (loopback) and TEST_RUNNER_TEND_TOKEN")
        }
        self.server = server
        self.token = token
    }

    private var isPad: Bool { UIDevice.current.userInterfaceIdiom == .pad }

    private func mark(_ name: String) {
        print(String(format: "DEMO-MARK %@ %.3f", name, Date().timeIntervalSince1970))
    }

    private func pause(_ seconds: TimeInterval) {
        Thread.sleep(forTimeInterval: seconds)
    }

    private func approve(_ code: String) throws {
        var request = URLRequest(url: URL(string: server + "/api/devices/approve")!)
        request.httpMethod = "POST"
        request.setValue("Bearer \(token!)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONSerialization.data(withJSONObject: ["user_code": code, "scope": "operate"])
        let done = expectation(description: "approve")
        final class Box: @unchecked Sendable { var status = 0 }
        let box = Box()
        URLSession.shared.dataTask(with: request) { _, response, _ in
            box.status = (response as? HTTPURLResponse)?.statusCode ?? 0
            done.fulfill()
        }.resume()
        wait(for: [done], timeout: 15)
        XCTAssertEqual(box.status, 200, "the dashboard did not approve the pairing code")
    }

    func testDemoTour() throws {
        XCUIDevice.shared.orientation = isPad ? .landscapeLeft : .portrait
        let app = XCUIApplication()
        app.launchArguments = ["-TendResetState", "-AppleLanguages", "(en)"]
        app.launchEnvironment["TEND_SERVER"] = server
        app.launch()

        let name = app.textFields["deviceName"]
        XCTAssertTrue(name.waitForExistence(timeout: 20))
        name.tap()
        name.typeText(isPad ? "Demo iPad" : "Demo iPhone")
        app.buttons["scope-operate"].tap()
        app.buttons["pairButton"].tap()
        // A cold simulator can take a while to reach the dashboard.
        let code = app.staticTexts["userCode"]
        XCTAssertTrue(code.waitForExistence(timeout: 60), "no pairing code; is the fixture's dashboard up?")
        XCTAssertFalse(code.label.isEmpty)
        try approve(code.label)

        // Home with the fixture's agents (the iPad may have nothing left to
        // accept: the iPhone run decided the last result).
        let card = app.staticTexts["fix the SSE reconnect backoff"].firstMatch
        XCTAssertTrue(card.waitForExistence(timeout: 180), "Home never showed the fixture's agents")
        pause(1.5)
        mark("home")
        pause(isPad ? 5 : 3.5)

        // Decide a finished result from the phone: the backfill one; the
        // Mac part of the video already merged the proration fix.
        // The iPhone decides it (the iPad, after it, has nothing left to).
        if !isPad {
            let option = app.buttons["publish"].firstMatch
            XCTAssertTrue(option.waitForExistence(timeout: 10), "the backfill result is not in Your turn")
            mark("decide")
            if !option.isHittable { app.swipeUp(velocity: .slow); pause(0.8) }
            option.tap()
            pause(1.0)
            let confirm = app.buttons["confirmAccept"].firstMatch
            XCTAssertTrue(confirm.waitForExistence(timeout: 5))
            confirm.tap()
            XCTAssertTrue(app.staticTexts["Accepted."].waitForExistence(timeout: 15), "the accept was not confirmed")
            pause(2.5)
        }

        if app.buttons["lane-inProgress"].exists {
            mark("lanes")
            app.buttons["lane-inProgress"].tap()
            pause(2.5)
            app.buttons["lane-finished"].tap()
            pause(2.5)
        } else {
            mark("lanes")
            pause(3)
        }
        mark("end")
        pause(1)
    }
}
