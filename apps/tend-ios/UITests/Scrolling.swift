import XCTest

extension XCUIApplication {
    /// The text, scrolled into view: a lane is a lazy list, so a card below
    /// the fold does not exist until it is scrolled to.
    func scrolledText(_ label: String, timeout: TimeInterval = 15) -> XCUIElement {
        let text = staticTexts[label].firstMatch
        if text.waitForExistence(timeout: min(timeout, 5)) { return text }
        let end = Date().addingTimeInterval(timeout)
        while !text.exists, Date() < end {
            swipeUp(velocity: .slow)
        }
        return text
    }
}
