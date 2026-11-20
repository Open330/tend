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
        continueAfterFailure = true
        let env = ProcessInfo.processInfo.environment
        guard env["TEND_DEMO_TOUR"] != nil, let server = env["TEND_SERVER"], let token = env["TEND_TOKEN"] else {
            throw XCTSkip("set TEST_RUNNER_TEND_DEMO_TOUR, TEST_RUNNER_TEND_SERVER and TEST_RUNNER_TEND_TOKEN")
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
        URLSession.shared.dataTask(with: request) { _, _, _ in done.fulfill() }.resume()
        wait(for: [done], timeout: 15)
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
        let code = app.staticTexts["userCode"]
        XCTAssertTrue(code.waitForExistence(timeout: 15), "no pairing code; is the fixture's dashboard up?")
        try approve(code.label)

        // Home with the fixture's agents (the iPad may have nothing left to
        // accept: the iPhone run decided the last result).
        let card = app.staticTexts["fix the SSE reconnect backoff"].firstMatch
        XCTAssertTrue(card.waitForExistence(timeout: 20), "Home never showed the fixture's agents")
        pause(1.5)
        mark("home")
        pause(isPad ? 5 : 3.5)

        // Decide a finished result from the phone: the backfill one; the
        // Mac part of the video already merged the proration fix.
        let option = app.buttons["publish"].firstMatch
        if option.waitForExistence(timeout: 3) {
            mark("decide")
            if !option.isHittable { app.swipeUp(velocity: .slow); pause(0.8) }
            option.tap()
            pause(1.0)
            let confirm = app.buttons["confirmAccept"].firstMatch
            if confirm.waitForExistence(timeout: 5) {
                confirm.tap()
                _ = app.staticTexts["Accepted."].waitForExistence(timeout: 15)
            }
            pause(2.0)
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
