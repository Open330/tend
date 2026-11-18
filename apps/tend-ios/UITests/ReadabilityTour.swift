// Every screen, for the readability pass: run against a scratch tendd
// seeded with `seed` and `seed-home`, with the language and text size in
// the runner's environment, and take the screenshots from the result
// bundle. It checks that each screen opens, not how it looks.
//
//   TEST_RUNNER_TEND_TOUR=1 TEST_RUNNER_TEND_TOUR_LANG=ko \
//   TEST_RUNNER_TEND_TOUR_SIZE=UICTContentSizeCategoryAccessibilityM \
//   TEST_RUNNER_TEND_SERVER=… TEST_RUNNER_TEND_TOKEN=… \
//   xcodebuild test -only-testing:TendUITests/ReadabilityTour …
import XCTest

final class ReadabilityTour: XCTestCase {
    private var server: String!
    private var token: String!

    override func setUpWithError() throws {
        continueAfterFailure = false
        let env = ProcessInfo.processInfo.environment
        guard env["TEND_TOUR"] != nil, let server = env["TEND_SERVER"], let token = env["TEND_TOKEN"] else {
            throw XCTSkip("set TEST_RUNNER_TEND_TOUR, TEST_RUNNER_TEND_SERVER and TEST_RUNNER_TEND_TOKEN")
        }
        self.server = server
        self.token = token
    }

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

    func testEveryScreen() throws {
        let env = ProcessInfo.processInfo.environment
        let lang = env["TEND_TOUR_LANG"] ?? "en"
        let size = env["TEND_TOUR_SIZE"] ?? "UICTContentSizeCategoryL"
        let isPad = UIDevice.current.userInterfaceIdiom == .pad
        XCUIDevice.shared.orientation = .portrait
        let deviceName = "Tour \(Int.random(in: 1000...9999))"
        let app = XCUIApplication()
        app.launchArguments = [
            "-TendResetState", "-TendSkipOwnerCheck",
            "-AppleLanguages", "(\(lang))", "-AppleLocale", lang == "ko" ? "ko_KR" : "en_US",
            "-UIPreferredContentSizeCategoryName", size,
        ]
        app.launchEnvironment["TEND_SERVER"] = server
        app.launch()

        let name = app.textFields["deviceName"]
        XCTAssertTrue(name.waitForExistence(timeout: 10))
        shot(app, "pairing")
        name.tap()
        name.typeText(deviceName)
        app.buttons["scope-operate"].firstMatch.tap()
        app.buttons["pairButton"].firstMatch.tap()
        let code = app.staticTexts["userCode"]
        XCTAssertTrue(code.waitForExistence(timeout: 10))
        shot(app, "pairing-code")
        let (status, _) = try operatorCall("POST", "/api/devices/approve", body: ["user_code": code.label, "scope": "operate"])
        XCTAssertEqual(status, 200)

        XCTAssertTrue(app.buttons["acceptButton"].firstMatch.waitForExistence(timeout: 15))
        shot(app, "your-turn")
        for lane in ["inProgress", "finished"] {
            let button = app.buttons["lane-\(lane)"]
            XCTAssertTrue(button.waitForExistence(timeout: 5))
            button.tap()
            Thread.sleep(forTimeInterval: 0.6)
            shot(app, lane)
        }
        // A finished agent or Work, then back.
        let card = app.descendants(matching: .any).matching(NSPredicate(format: "identifier IN {'workCard', 'agentCard'}")).firstMatch
        if card.waitForExistence(timeout: 5) {
            card.tap()
            Thread.sleep(forTimeInterval: 1)
            shot(app, "detail")
            app.navigationBars.buttons.firstMatch.tap()
        }
        // Start Work.
        let start = app.buttons["startWork"]
        if start.waitForExistence(timeout: 5) {
            start.tap()
            XCTAssertTrue(app.textViews["workTask"].waitForExistence(timeout: 10))
            Thread.sleep(forTimeInterval: 1.5)
            app.textViews["workTask"].typeText("\n")
            shot(app, "start-work")
            app.navigationBars.buttons.firstMatch.tap()
        }
        // Hosts and one host, then Settings.
        if isPad {
            let sidebarHost = app.descendants(matching: .any)["sidebarHost"].firstMatch
            if !sidebarHost.waitForExistence(timeout: 2) { app.buttons.element(boundBy: 0).tap() }
            if sidebarHost.waitForExistence(timeout: 5) {
                shot(app, "sidebar")
                sidebarHost.tap()
                if sidebarHost.isHittable { app.coordinate(withNormalizedOffset: CGVector(dx: 0.98, dy: 0.5)).tap() }
                Thread.sleep(forTimeInterval: 1)
                shot(app, "host")
            }
        } else {
            app.tabBars.buttons.element(boundBy: 1).tap()
            Thread.sleep(forTimeInterval: 1)
            shot(app, "hosts")
            let row = app.descendants(matching: .any)["hostRow"].firstMatch
            if row.waitForExistence(timeout: 5) {
                row.tap()
                Thread.sleep(forTimeInterval: 1)
                shot(app, "host")
            }
            app.tabBars.buttons.element(boundBy: 2).tap()
            Thread.sleep(forTimeInterval: 0.6)
            shot(app, "settings")
        }

        let (_, devices) = try operatorCall("GET", "/api/devices")
        for device in ((devices as? [String: Any])?["devices"] as? [[String: Any]]) ?? [] where device["name"] as? String == deviceName {
            if let id = device["id"] as? String { _ = try operatorCall("POST", "/api/devices/\(id)/remove") }
        }
    }

    private func shot(_ app: XCUIApplication, _ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }
}
