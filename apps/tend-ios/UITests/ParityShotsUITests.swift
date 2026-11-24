// The iPhone and iPad screens for the cross-surface parity audit
// (scripts/parity-shots/run.sh, .github/workflows/parity-shots.yml): pair
// with the demo fixture's dashboard (operate scope), then photograph Home
// and each lane, a question card, a result card, Start Work, Hosts, a host,
// an agent, Settings, and the states after the dashboard removes the device
// and when the dashboard cannot be reached. Capture only: it answers,
// decides and starts nothing, so every surface shows the fixture's open
// state. Skipped unless TEST_RUNNER_TEND_PARITY=1, TEST_RUNNER_TEND_SERVER
// (loopback), TEST_RUNNER_TEND_TOKEN and TEST_RUNNER_TEND_PARITY_DIR are set.
//
// Each screen is an XCTAttachment (`ios-<screen>-<device>`), with its
// accessibility tree (`app.debugDescription`) as a text attachment. With
// the host's helper running (run.sh), the PNG is taken by `simctl io
// screenshot` instead (XCUITest's own shots of a rotated iPad are clipped)
// and the text written straight to TEND_PARITY_DIR.
import XCTest

final class ParityShotsUITests: XCTestCase {
    private var server: String!
    private var token: String!
    private var dir: URL!

    override func setUpWithError() throws {
        continueAfterFailure = true
        let env = ProcessInfo.processInfo.environment
        // Only from scripts/parity-shots/run.sh on a GitHub-hosted runner,
        // which passes these (TEST_RUNNER_-prefixed) after its own CI
        // check; nothing below (reset, pair, remove) runs without them.
        guard env["TEND_PARITY_CI"] == "1", env["GITHUB_ACTIONS"] == "true",
              env["RUNNER_ENVIRONMENT"] == "github-hosted" else {
            throw XCTSkip("runs only from scripts/parity-shots/run.sh on a GitHub-hosted runner")
        }
        guard env["TEND_PARITY"] == "1", let server = env["TEND_SERVER"], let token = env["TEND_TOKEN"],
              let dir = env["TEND_PARITY_DIR"],
              let url = URLComponents(string: server), url.scheme == "http", url.host == "127.0.0.1",
              url.port != nil, url.user == nil, url.password == nil, url.path.isEmpty else {
            throw XCTSkip("set TEST_RUNNER_TEND_PARITY=1, TEST_RUNNER_TEND_SERVER (loopback), TEST_RUNNER_TEND_TOKEN and TEST_RUNNER_TEND_PARITY_DIR")
        }
        self.server = server
        self.token = token
        self.dir = URL(fileURLWithPath: dir)
    }

    private var isPad: Bool { UIDevice.current.userInterfaceIdiom == .pad }
    private var device: String { isPad ? "ipad" : "iphone" }
    /// What could not be captured, written as it grows so a test that
    /// stops early still says why.
    private var missing: [String] = [] {
        didSet {
            try? Data((missing.map { "ios-\(device) \($0)" }.joined(separator: "\n") + "\n").utf8)
                .write(to: dir.appendingPathComponent("ios-missing-\(device).txt"))
        }
    }

    func testParityShots() throws {
        let deviceName = "Parity \(isPad ? "iPad" : "iPhone") \(Int.random(in: 1000...9999))"
        XCUIDevice.shared.orientation = isPad ? .landscapeLeft : .portrait
        let app = XCUIApplication()
        app.launchArguments = ["-TendResetState", "-TendSkipOwnerCheck", "-AppleLanguages", "(en)"]
        app.launchEnvironment["TEND_SERVER"] = server
        app.launch()

        // Pairing.
        let name = app.textFields["deviceName"]
        XCTAssertTrue(name.waitForExistence(timeout: 30), "no pairing screen")
        shot(app, "pairing")
        name.tap()
        name.typeText(deviceName)
        app.buttons["scope-operate"].tap()
        app.buttons["pairButton"].tap()
        let code = app.staticTexts["userCode"]
        XCTAssertTrue(code.waitForExistence(timeout: 60), "no pairing code; is the fixture's dashboard up?")
        shot(app, "pairing-code")
        let (status, _) = try operatorCall("POST", "/api/devices/approve", body: ["user_code": code.label, "scope": "operate"])
        XCTAssertEqual(status, 200, "the dashboard did not approve the pairing code")
        defer { try? unpair(deviceName) }

        // Home with the fixture's agents and decisions.
        let decision = app.buttons["acceptButton"].firstMatch
        let anyCard = app.descendants(matching: .any).matching(identifier: "workCard").firstMatch
        let up = Date().addingTimeInterval(180)
        while !decision.exists, !anyCard.exists, Date() < up { pause(1) }
        XCTAssertTrue(decision.exists || anyCard.exists, "Home never showed the fixture's agents")
        // Let In progress, Finished and the hosts arrive too.
        pause(8)
        shot(app, "home")

        // Each lane. The iPad's landscape board shows them side by side; in
        // portrait it is the iPhone's strip, a lane at a time.
        if isPad {
            XCUIDevice.shared.orientation = .portrait
            pause(3)
            shot(app, "home-portrait")
        }
        if app.buttons["lane-yourTurn"].waitForExistence(timeout: 5) {
            for lane in ["backlog", "yourTurn", "inProgress", "finished"] {
                let tab = app.buttons["lane-\(lane)"]
                guard tab.exists else { missing.append("home-\(lane): no such lane (empty)"); continue }
                tab.tap()
                pause(1.5)
                shot(app, "home-\(lane)")
                if lane != "yourTurn" { scrollShots(app, "home-\(lane)") }
            }
            app.buttons["lane-yourTurn"].tap()
            pause(1)
        } else {
            missing.append("home lanes: no lane strip on this layout")
        }

        // A question and a result, as Your turn draws them.
        card(app, button: "answerButton", name: "question")
        card(app, button: "acceptButton", name: "review-card")
        scrollShots(app, "home-yourTurn")

        // An agent asking a choice is a unit here: its card opens the
        // agent, where the question is.
        let choice = app.scrolledText("rotate the JWT signing keys across environments", timeout: 20)
        if choice.exists {
            if !choice.isHittable { app.swipeUp(velocity: .slow); pause(0.5) }
            choice.tap()
            pause(3)
            shot(app, "choice-detail")
            scrollShots(app, "choice-detail")
            back(app)
        } else {
            missing.append("choice-detail: the JWT agent is not in Your turn")
        }
        for _ in 0..<6 { app.swipeDown(velocity: .fast) }

        // Start Work, opened and cancelled.
        let start = app.buttons["startWork"].firstMatch
        if start.waitForExistence(timeout: 10) {
            start.tap()
            if app.textViews["workTask"].waitForExistence(timeout: 10) {
                pause(3)
                shot(app, "start-work")
            } else {
                missing.append("start-work: the sheet did not open")
            }
            let cancel = app.buttons["Cancel"].firstMatch
            if cancel.exists { cancel.tap() } else { app.swipeDown(velocity: .fast) }
            pause(1.5)
        } else {
            missing.append("start-work: no Start Work button")
        }

        // An agent at work.
        if app.buttons["lane-inProgress"].exists { app.buttons["lane-inProgress"].tap(); pause(1.5) }
        let work = app.descendants(matching: .any).matching(identifier: "workCard").firstMatch
        if work.waitForExistence(timeout: 10) {
            if !work.isHittable { app.swipeUp(velocity: .slow); pause(0.5) }
            work.tap()
            pause(3)
            shot(app, "agent-detail")
            let output = app.buttons["showOutput"].firstMatch
            if output.waitForExistence(timeout: 5) {
                output.tap()
                if app.descendants(matching: .any)["agentOutput"].firstMatch.waitForExistence(timeout: 15) {
                    pause(1.5)
                    shot(app, "agent-output")
                } else {
                    missing.append("agent-output: no output arrived")
                }
            } else {
                missing.append("agent-output: no Show output button")
            }
            back(app)
        } else {
            missing.append("agent-detail: no unit at work in In progress")
        }
        if isPad {
            XCUIDevice.shared.orientation = .landscapeLeft
            pause(2)
        }

        // Hosts, and a host.
        if isPad {
            let host = app.descendants(matching: .any)["sidebarHost"].firstMatch
            showSidebar(app, host)
            if onScreen(app, host) {
                shot(app, "sidebar")
                host.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
                pause(1)
                if host.isHittable { app.coordinate(withNormalizedOffset: CGVector(dx: 0.98, dy: 0.5)).tap() }
                pause(3)
                shot(app, "host-detail")
            } else {
                missing.append("hosts: no host in the sidebar")
            }
        } else {
            app.tabBars.buttons["Hosts"].tap()
            pause(3)
            shot(app, "hosts")
            let row = app.buttons["hostRow"].firstMatch.exists ? app.buttons["hostRow"].firstMatch : app.otherElements["hostRow"].firstMatch
            if row.waitForExistence(timeout: 10) {
                row.tap()
                pause(3)
                shot(app, "host-detail")
                scrollShots(app, "host-detail")
                back(app)
            } else {
                missing.append("host-detail: no host row")
            }
        }

        // Settings.
        if isPad {
            let settings = app.descendants(matching: .any)["sidebarSettings"].firstMatch
            showSidebar(app, settings)
            if onScreen(app, settings) {
                settings.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
                pause(1)
                if settings.isHittable { app.coordinate(withNormalizedOffset: CGVector(dx: 0.98, dy: 0.5)).tap() }
                pause(2)
                shot(app, "settings")
            } else {
                missing.append("settings: no Settings in the sidebar")
            }
        } else {
            app.tabBars.buttons["Settings"].tap()
            pause(2)
            shot(app, "settings")
            scrollShots(app, "settings")
        }

        // The dashboard removes the device: what the app says then.
        try unpair(deviceName)
        let repaired = app.buttons["pairButton"].firstMatch
        if repaired.waitForExistence(timeout: 40) {
            pause(1)
            shot(app, "removed")
        } else {
            shot(app, "removed-still-paired")
            missing.append("removed: the app did not notice the removal within 40 s")
        }

        // A dashboard that cannot be reached.
        app.terminate()
        let unreachable = XCUIApplication()
        unreachable.launchArguments = ["-TendResetState", "-AppleLanguages", "(en)"]
        unreachable.launchEnvironment["TEND_SERVER"] = "http://127.0.0.1:9"
        unreachable.launch()
        let field = unreachable.textFields["deviceName"]
        if field.waitForExistence(timeout: 20) {
            field.tap()
            field.typeText("Parity unreachable")
            unreachable.buttons["pairButton"].tap()
            if unreachable.descendants(matching: .any)["pairingProblem"].firstMatch.waitForExistence(timeout: 30) {
                pause(1)
                shot(unreachable, "error-unreachable")
            } else {
                shot(unreachable, "error-unreachable-none")
                missing.append("error-unreachable: no pairing problem shown")
            }
        }
        unreachable.terminate()
    }

    // MARK: - Screens

    /// A decision card by its action button, scrolled into view: the
    /// screen around it, and the card alone.
    private func card(_ app: XCUIApplication, button: String, name: String) {
        let action = app.buttons[button].firstMatch
        // A lane is a lazy stack: a card below the fold may not exist
        // until it is scrolled to.
        var tries = 0
        _ = action.waitForExistence(timeout: 3)
        while !(action.exists && action.isHittable), tries < 10 { app.swipeUp(velocity: .slow); tries += 1 }
        guard action.exists else {
            missing.append("\(name): no card with \(button) in Your turn")
            for _ in 0..<tries { app.swipeDown(velocity: .slow) }
            return
        }
        pause(1)
        shot(app, name)
        let card = app.descendants(matching: .any).matching(identifier: "card").containing(.button, identifier: button).firstMatch
        if card.exists {
            attach(card.screenshot(), "\(name)-card")
        }
        // Back to the top for what follows.
        for _ in 0..<tries { app.swipeDown(velocity: .slow) }
    }

    /// The iPad's sidebar, brought out until `item` can be tapped (in
    /// landscape it is an overlay the detail hides).
    private func showSidebar(_ app: XCUIApplication, _ item: XCUIElement) {
        for _ in 0..<3 {
            if item.waitForExistence(timeout: 2), onScreen(app, item) { return }
            let toggle = app.buttons["Show Sidebar"].firstMatch
            if toggle.exists, toggle.isHittable {
                toggle.tap()
            } else {
                app.swipeRight(velocity: .fast)
            }
            pause(1)
        }
    }

    /// Drawn inside the window (a list row is not always "hittable" to
    /// XCUITest even when it is plainly on screen).
    private func onScreen(_ app: XCUIApplication, _ item: XCUIElement) -> Bool {
        guard item.exists else { return false }
        let frame = item.frame
        guard !frame.isEmpty else { return false }
        return app.windows.firstMatch.frame.contains(CGPoint(x: frame.midX, y: frame.midY))
    }

    /// The rest of a long screen, a page at a time (at most three).
    private func scrollShots(_ app: XCUIApplication, _ name: String) {
        var last = app.debugDescription
        var page = 0
        for index in 2...4 {
            app.swipeUp(velocity: .slow)
            pause(1)
            let now = app.debugDescription
            if now == last { break }
            last = now
            shot(app, "\(name)-\(index)")
            page += 1
        }
        for _ in 0..<page { app.swipeDown(velocity: .slow) }
        pause(0.5)
    }

    private func back(_ app: XCUIApplication) {
        let button = app.navigationBars.buttons.element(boundBy: 0)
        if button.exists { button.tap(); pause(1.5) }
    }

    // MARK: - Capture

    private func shot(_ app: XCUIApplication, _ name: String) {
        let full = "ios-\(name)-\(device)"
        let text = app.debugDescription
        let words = XCTAttachment(string: text)
        words.name = "\(full).txt"
        words.lifetime = .keepAlways
        add(words)
        try? Data(text.utf8).write(to: dir.appendingPathComponent("\(full).txt"))
        let screenshot = XCUIScreen.main.screenshot()
        let image = XCTAttachment(screenshot: screenshot)
        image.name = full
        image.lifetime = .keepAlways
        add(image)
        if !hostShot(full) {
            try? screenshot.pngRepresentation.write(to: dir.appendingPathComponent("\(full).png"))
        }
    }

    private func attach(_ screenshot: XCUIScreenshot, _ name: String) {
        let full = "ios-\(name)-\(device)"
        let image = XCTAttachment(screenshot: screenshot)
        image.name = full
        image.lifetime = .keepAlways
        add(image)
        try? screenshot.pngRepresentation.write(to: dir.appendingPathComponent("\(full).png"))
    }

    /// Asks run.sh's helper to take the screen with `simctl io screenshot`
    /// and waits until it has; false when no helper answered.
    private func hostShot(_ name: String) -> Bool {
        let request = dir.appendingPathComponent("request-\(device)")
        let done = dir.appendingPathComponent("done-\(name)")
        guard (try? name.write(to: request, atomically: true, encoding: .utf8)) != nil else { return false }
        let end = Date().addingTimeInterval(10)
        while !FileManager.default.fileExists(atPath: done.path), Date() < end {
            Thread.sleep(forTimeInterval: 0.2)
        }
        let answered = FileManager.default.fileExists(atPath: done.path)
        try? FileManager.default.removeItem(at: done)
        return answered
    }

    private func pause(_ seconds: TimeInterval) {
        Thread.sleep(forTimeInterval: seconds)
    }

    // MARK: - The dashboard

    private func unpair(_ deviceName: String) throws {
        let (_, devices) = try operatorCall("GET", "/api/devices")
        for device in ((devices as? [String: Any])?["devices"] as? [[String: Any]]) ?? [] where device["name"] as? String == deviceName {
            if let id = device["id"] as? String { _ = try operatorCall("POST", "/api/devices/\(id)/remove") }
        }
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
}
