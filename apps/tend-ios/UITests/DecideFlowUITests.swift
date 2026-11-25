// The phase 1 flow against a real scratch tendd: pair, see what needs you,
// answer a question, accept a result, see both under Decided, and check the
// dashboard recorded them as this device.
//
// Needs a scratch daemon seeded with a question and a result
// (apps/tend-ios/Scripts/scratch-tendd.sh up), and its address and
// operator token in the test runner's environment:
//
//   TEST_RUNNER_TEND_SERVER=http://127.0.0.1:7979 \
//   TEST_RUNNER_TEND_TOKEN=$(apps/tend-ios/Scripts/scratch-tendd.sh token) \
//   xcodebuild test -scheme Tend -only-testing:TendUITests …
//
// The operator token is used only by the test, to approve the pairing the
// way a person would in the dashboard's access dialog.
import XCTest

final class DecideFlowUITests: XCTestCase {
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

    private var isPad: Bool { UIDevice.current.userInterfaceIdiom == .pad }

    func testPairAnswerAndDecide() throws {
        let deviceName = "UI test \(isPad ? "iPad" : "iPhone") \(Int.random(in: 1000...9999))"
        let app = XCUIApplication()
        app.launchArguments = ["-TendResetState", "-AppleLanguages", "(en)"]
        app.launchEnvironment["TEND_SERVER"] = server
        app.launch()

        // Pair: the phone asks, the "operator" approves the code it shows.
        let name = app.textFields["deviceName"]
        XCTAssertTrue(name.waitForExistence(timeout: 10))
        name.tap()
        name.typeText(deviceName)
        app.buttons["scope-decide"].tap()
        app.buttons["pairButton"].tap()
        let code = app.staticTexts["userCode"]
        XCTAssertTrue(code.waitForExistence(timeout: 10), "no pairing code; is the scratch tendd up?")
        let (status, approved) = try operatorCall("POST", "/api/devices/approve", body: ["user_code": code.label])
        XCTAssertEqual(status, 200)
        let approvedName = ((approved as? [String: Any])?["approved"] as? [String: Any])?["name"] as? String
        XCTAssertEqual(approvedName, deviceName)

        // What needs you: the seeded question and result.
        let answer = app.buttons["answerButton"].firstMatch
        XCTAssertTrue(answer.waitForExistence(timeout: 15), "the question never showed")
        XCTAssertTrue(app.buttons["acceptButton"].firstMatch.exists, "the result never showed")
        // A question closes without an answer only through Decline…, beside Answer….
        XCTAssertTrue(app.buttons["declineButton"].firstMatch.exists, "Decline… is not in the question's action row")
        XCTAssertTrue(app.staticTexts["Merge the retry path now?"].firstMatch.exists)
        XCTAssertTrue(app.staticTexts["merge now: the perf run only covers reads"].firstMatch.exists)
        attach(app, "needs-you")

        // Answer the question.
        answer.tap()
        let composer = app.textFields["composer"].firstMatch.exists ? app.textFields["composer"].firstMatch : app.textViews["composer"].firstMatch
        XCTAssertTrue(composer.waitForExistence(timeout: 5))
        composer.tap()
        composer.typeText("Yes, behind the flag")
        app.buttons["sendButton"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["Answered."].waitForExistence(timeout: 15))

        // Accept the result, with one of its options.
        let option = app.buttons["merge now"].firstMatch
        XCTAssertTrue(option.waitForExistence(timeout: 5))
        option.tap()
        let confirm = app.buttons["confirmAccept"].firstMatch
        XCTAssertTrue(confirm.waitForExistence(timeout: 5))
        confirm.tap()
        XCTAssertTrue(app.staticTexts["Accepted."].waitForExistence(timeout: 15))
        attach(app, "decided-inline")

        // The outcome under Finished (iPad shows all three lanes at once).
        if app.buttons["lane-finished"].exists {
            app.buttons["lane-finished"].tap()
        }
        XCTAssertTrue(app.staticTexts["Answered"].firstMatch.waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Accepted"].firstMatch.exists)
        XCTAssertTrue(app.staticTexts["Yes, behind the flag"].firstMatch.exists)
        attach(app, "decided")

        // The dashboard recorded the review as this device, not as the token.
        let (historyStatus, history) = try operatorCall("GET", "/api/decisions/history")
        XCTAssertEqual(historyStatus, 200)
        let text = String(data: try JSONSerialization.data(withJSONObject: history ?? [:]), encoding: .utf8) ?? ""
        XCTAssertTrue(text.contains("app:\(deviceName)#"), "the decision was not recorded as the device")

        // A decide-only device does not see agents, says why, and asks for
        // more from where it says so, keeping its key until approved.
        if app.buttons["lane-inProgress"].exists { app.buttons["lane-inProgress"].tap() }
        XCTAssertTrue(app.staticTexts["This device decides only"].firstMatch.waitForExistence(timeout: 5))
        attach(app, "decide-only")
        let repair = app.buttons["repair"].firstMatch
        XCTAssertTrue(repair.exists)
        repair.tap()
        let ask = app.buttons["Ask the dashboard"].firstMatch
        XCTAssertTrue(ask.waitForExistence(timeout: 5))
        ask.tap()
        let upgradeCode = app.staticTexts["upgradeCode"].firstMatch
        XCTAssertTrue(upgradeCode.waitForExistence(timeout: 10), "no code for hosts and new work")
        attach(app, "asking-for-hosts")
        let (lookupStatus, looked) = try operatorCall("POST", "/api/devices/lookup", body: ["user_code": upgradeCode.label])
        XCTAssertEqual(lookupStatus, 200)
        XCTAssertEqual(((looked as? [String: Any])?["request"] as? [String: Any])?["scope"] as? String, "operate")
        let (upgradeStatus, _) = try operatorCall("POST", "/api/devices/approve", body: ["user_code": upgradeCode.label, "scope": "operate"])
        XCTAssertEqual(upgradeStatus, 200)
        // The new key replaced the old one, here and on the dashboard.
        XCTAssertTrue(app.staticTexts["This device decides only"].firstMatch.waitForNonExistence(timeout: 20), "still decide-only")
        let (_, listed) = try operatorCall("GET", "/api/devices")
        let mine = ((listed as? [String: Any])?["devices"] as? [[String: Any]])?.filter { $0["name"] as? String == deviceName } ?? []
        XCTAssertEqual(mine.count, 1, "the old decide key was not removed")
        XCTAssertEqual(mine.first?["scope"] as? String, "operate")
        attach(app, "after-hosts-approved")

        // Sign out: the dashboard forgets this device.
        if isPad {
            let settings = app.descendants(matching: .any)["sidebarSettings"].firstMatch
            if !settings.waitForExistence(timeout: 2) {
                app.buttons["Show Sidebar"].firstMatch.tap()
            }
            XCTAssertTrue(settings.waitForExistence(timeout: 5))
            settings.tap()
        } else {
            app.tabBars.buttons["Settings"].tap()
        }
        // The confirmation is a sheet on iPhone and a popover on iPad. On
        // iPad the first tap may only close the sidebar overlay.
        let confirmSignOut = app.buttons.matching(NSPredicate(format: "label == 'Sign out' AND identifier != 'signOut'")).firstMatch
        app.buttons["signOut"].tap()
        if !confirmSignOut.waitForExistence(timeout: 2) { app.buttons["signOut"].tap() }
        XCTAssertTrue(confirmSignOut.waitForExistence(timeout: 5))
        confirmSignOut.tap()
        XCTAssertTrue(app.buttons["pairButton"].waitForExistence(timeout: 10))
        let (_, devices) = try operatorCall("GET", "/api/devices")
        let names = ((devices as? [String: Any])?["devices"] as? [[String: Any]])?.compactMap { $0["name"] as? String } ?? []
        XCTAssertFalse(names.contains(deviceName))
    }

    private func attach(_ app: XCUIApplication, _ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }
}
