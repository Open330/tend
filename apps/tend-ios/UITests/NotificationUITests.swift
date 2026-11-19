// Opening an item from a notification, against a scratch tendd: pair, then
// launch again as a tap on a notification would, with a payload naming the
// seeded question by its `ref`. The scratch daemon never sends to Apple; the
// payload and the key a registration would give are handed to the app's own
// handler (DEBUG builds only).
import CryptoKit
import XCTest

final class NotificationUITests: XCTestCase {
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

    func testTappingANotificationOpensItsItem() throws {
        XCUIDevice.shared.orientation = .portrait
        let deviceName = "Notify \(Int.random(in: 1000...9999))"
        let app = XCUIApplication()
        app.launchArguments = ["-TendResetState", "-AppleLanguages", "(en)"]
        app.launchEnvironment["TEND_SERVER"] = server
        app.launch()
        let name = app.textFields["deviceName"]
        XCTAssertTrue(name.waitForExistence(timeout: 10))
        name.tap()
        name.typeText(deviceName)
        app.buttons["scope-decide"].tap()
        app.buttons["pairButton"].tap()
        let code = app.staticTexts["userCode"]
        XCTAssertTrue(code.waitForExistence(timeout: 10))
        XCTAssertEqual(try operatorCall("POST", "/api/devices/approve", body: ["user_code": code.label]).0, 200)
        XCTAssertTrue(app.buttons["answerButton"].firstMatch.waitForExistence(timeout: 15))

        // The newest question on this host, as the host would name it.
        let (_, queue) = try operatorCall("GET", "/api/decisions")
        let section = try XCTUnwrap(((queue as? [String: Any])?["hosts"] as? [[String: Any]])?.first)
        let alias = try XCTUnwrap(section["alias"] as? String)
        let items = ((section["queue"] as? [String: Any])?["items"] as? [[String: Any]]) ?? []
        let question = try XCTUnwrap(items.last { $0["kind"] as? String == "human_request" })
        let id = try XCTUnwrap(question["request_id"] as? String)
        let key = Data((0..<32).map { _ in UInt8.random(in: 0...255) })
        let mac = HMAC<SHA256>.authenticationCode(for: Data("\(alias)\u{0}request\u{0}\(id)".utf8), using: SymmetricKey(data: key))
        let ref = Data(mac).prefix(16).map { String(format: "%02x", $0) }.joined()
        let payload = try JSONSerialization.data(withJSONObject: ["tend": ["v": 1, "channel": "ch_test", "host": alias, "ref": ref]])

        app.terminate()
        app.launchArguments = ["-AppleLanguages", "(en)"]
        app.launchEnvironment["TEND_TEST_NOTIFICATION"] = String(data: payload, encoding: .utf8)
        app.launchEnvironment["TEND_TEST_PUSH_KEY"] = key.base64EncodedString()
        app.launch()
        let focused = app.descendants(matching: .any)["focusedCard"].firstMatch
        XCTAssertTrue(focused.waitForExistence(timeout: 20), "the notified question was not opened")
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = "opened-from-notification"
        shot.lifetime = .keepAlways
        add(shot)

        // A warm tap while Settings is in front: Home comes back with it.
        app.terminate()
        app.launchEnvironment["TEND_TEST_NOTIFICATION_DELAY"] = "15"
        app.launch()
        XCTAssertTrue(app.buttons["answerButton"].firstMatch.waitForExistence(timeout: 15))
        if UIDevice.current.userInterfaceIdiom == .pad {
            let settings = app.descendants(matching: .any)["sidebarSettings"].firstMatch
            if !settings.waitForExistence(timeout: 2) { app.buttons.element(boundBy: 0).tap() }
            XCTAssertTrue(settings.waitForExistence(timeout: 5))
            settings.tap()
        } else {
            app.tabBars.buttons.element(boundBy: 2).tap()
        }
        XCTAssertTrue(app.buttons["signOut"].waitForExistence(timeout: 5), "Settings did not open")
        XCTAssertTrue(app.descendants(matching: .any)["focusedCard"].firstMatch.waitForExistence(timeout: 30), "a warm tap did not bring Home back")

        let (_, devices) = try operatorCall("GET", "/api/devices")
        for device in ((devices as? [String: Any])?["devices"] as? [[String: Any]]) ?? [] where device["name"] as? String == deviceName {
            if let id = device["id"] as? String { _ = try operatorCall("POST", "/api/devices/\(id)/remove") }
        }
    }
}
