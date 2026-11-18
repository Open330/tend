// Recent output and Start Work on a device paired with hosts and new work,
// against a scratch tendd whose pipeline agent is a stand-in that tend does
// not launch (apps/tend-ios/Scripts/scratch-tendd.sh up && … seed-home).
// The app is launched with -TendSkipOwnerCheck (debug builds only): the
// simulator has no Face ID or passcode to ask for.
import XCTest

final class StartWorkUITests: XCTestCase {
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

    func testRecentOutputAndStartingWork() throws {
        XCUIDevice.shared.orientation = .portrait
        let deviceName = "Start \(Int.random(in: 1000...9999))"
        let app = XCUIApplication()
        app.launchArguments = ["-TendResetState", "-TendSkipOwnerCheck", "-AppleLanguages", "(en)"]
        app.launchEnvironment["TEND_SERVER"] = server
        app.launch()
        let name = app.textFields["deviceName"]
        XCTAssertTrue(name.waitForExistence(timeout: 10))
        name.tap()
        name.typeText(deviceName)
        app.buttons["scope-operate"].tap()
        app.buttons["pairButton"].tap()
        let code = app.staticTexts["userCode"]
        XCTAssertTrue(code.waitForExistence(timeout: 10))
        let (status, _) = try operatorCall("POST", "/api/devices/approve", body: ["user_code": code.label, "scope": "operate"])
        XCTAssertEqual(status, 200)

        // Recent output of an agent at work.
        if app.buttons["lane-inProgress"].waitForExistence(timeout: 10) {
            app.buttons["lane-inProgress"].tap()
        }
        let card = app.scrolledText("Add idempotency keys to the refund endpoint")
        XCTAssertTrue(card.waitForExistence(timeout: 15))
        card.tap()
        let show = app.buttons["showOutput"]
        XCTAssertTrue(show.waitForExistence(timeout: 10), "no output section: is capture offered?")
        show.tap()
        XCTAssertTrue(app.descendants(matching: .any)["agentOutput"].firstMatch.waitForExistence(timeout: 10))
        attach(app, "output")
        app.navigationBars.buttons.element(boundBy: 0).tap()

        // Start Work: request, the host's first project and pipeline, Start.
        let start = app.buttons["startWork"]
        XCTAssertTrue(start.waitForExistence(timeout: 10), "no Start Work: is work offered?")
        start.tap()
        let task = app.textViews["workTask"]
        XCTAssertTrue(task.waitForExistence(timeout: 10))
        task.tap()
        task.typeText("Write a haiku about caches")
        let go = app.buttons["workStart"]
        XCTAssertTrue(go.waitForExistence(timeout: 5))
        // A scratch tendd with SCRATCH_WORK_START=false: Start can never be
        // tapped, and the sheet says why and who can change it.
        if ProcessInfo.processInfo.environment["TEND_WORK_START_OFF"] != nil {
            XCTAssertTrue(app.descendants(matching: .any)["workStartOff"].firstMatch.waitForExistence(timeout: 15))
            XCTAssertFalse(go.isEnabled)
            attach(app, "work-start-off")
            return
        }
        // The options load from the host first; Start enables when they have.
        let end = Date().addingTimeInterval(15)
        while !go.isEnabled, Date() < end { Thread.sleep(forTimeInterval: 0.3) }
        XCTAssertTrue(go.isEnabled, "Start stayed disabled")
        attach(app, "start-work")
        go.tap()
        // The sheet closes on a start; a refusal stays in it, named.
        let closed = NSPredicate(format: "exists == false")
        let end2 = Date().addingTimeInterval(30)
        while task.exists, Date() < end2 {
            if app.staticTexts["workProblem"].exists || app.descendants(matching: .any)["workProblem"].exists { break }
            Thread.sleep(forTimeInterval: 0.3)
        }
        _ = closed
        XCTAssertFalse(task.exists, "the sheet stayed open: \(app.descendants(matching: .any)["workProblem"].firstMatch.label)")
        if app.buttons["lane-yourTurn"].exists { app.buttons["lane-yourTurn"].tap() }
        XCTAssertTrue(app.staticTexts["Started."].waitForExistence(timeout: 10), "the start did not report started")
        attach(app, "started")

        // The device stays the one that started it; clean up.
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
