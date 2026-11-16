import SwiftUI
import UIKit
import XCTest
@testable import Tend

/// Theme.swift against `apps/shared/Design/tokens.json`, the design tokens
/// Tend for Mac, the web dashboard and this app share: the app reads its
/// values from the same file the others are checked against, so the three
/// cannot drift apart unnoticed.
final class DesignTokensTests: XCTestCase {
    private struct Pair: Decodable { let light: String; let dark: String }
    private struct Tokens: Decodable {
        let color: [String: Pair]
        let state: [String: String]
        let brand: Brand
        let host: Host
        struct Brand: Decodable { let waiting: String; let error: String; let done: String; let ink: Pair }
        struct Host: Decodable { let gradients: [[String]] }
    }

    private func tokens() throws -> Tokens {
        let url = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "tokens", withExtension: "json"))
        return try JSONDecoder().decode(Tokens.self, from: Data(contentsOf: url))
    }

    /// `#RRGGBB` of a colour in one scheme.
    private func hex(_ color: Color, dark: Bool) -> String {
        let resolved = UIColor(color).resolvedColor(with: UITraitCollection(userInterfaceStyle: dark ? .dark : .light))
        var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
        resolved.getRed(&r, green: &g, blue: &b, alpha: &a)
        return String(format: "#%02X%02X%02X", Int((r * 255).rounded()), Int((g * 255).rounded()), Int((b * 255).rounded()))
    }

    private func assertPair(_ color: Color, _ token: Pair?, _ name: String, file: StaticString = #filePath, line: UInt = #line) {
        guard let token else { return XCTFail("tokens.json has no \(name)", file: file, line: line) }
        XCTAssertEqual(hex(color, dark: false), token.light.uppercased(), "\(name) light", file: file, line: line)
        XCTAssertEqual(hex(color, dark: true), token.dark.uppercased(), "\(name) dark", file: file, line: line)
    }

    func testSurfacesAreTheSharedTokens() throws {
        let t = try tokens()
        assertPair(TendSurface.workspace, t.color["editor"], "editor")
        assertPair(TendSurface.panel, t.color["lane"], "lane")
        assertPair(TendSurface.sideBar, t.color["side-bar"], "side-bar")
        assertPair(TendSurface.border, t.color["border"], "border")
        assertPair(TendSurface.inputBackground, t.color["input-background"], "input-background")
        assertPair(TendSurface.inputBorder, t.color["input-border"], "input-border")
    }

    /// States wear the system colour the tokens name for them. The hex
    /// values in tokens.json are Apple's classic ones; the system palette
    /// changes with the OS (iOS and macOS 26 drew orange #FF8D28, not
    /// #FF9500), and Tend for Mac draws the system colour, so the app does
    /// too: the check is by name, not by hex.
    func testStatesWearTheColoursTheTokensName() throws {
        let t = try tokens()
        let system: [String: UIColor] = [
            "orange": .systemOrange, "blue": .systemBlue, "green": .systemGreen,
            "red": .systemRed, "purple": .systemPurple, "mint": .systemMint,
        ]
        let ours: [String: Color] = [
            "error": AgentStatus.error.color, "needs-input": AgentStatus.needsInput.color,
            "asked": AgentStatus.asked.color, "limited": AgentStatus.limited.color,
            "working": AgentStatus.working.color, "idle": AgentStatus.idle.color,
            "done": AgentStatus.done.color, "your-turn": Lane.yourTurn.tint,
            "in-progress": Lane.inProgress.tint, "finished": Lane.finished.tint,
        ]
        for (state, color) in ours {
            let name = try XCTUnwrap(t.state[state], "tokens.json names no colour for \(state)")
            let expected = try XCTUnwrap(system[name], "\(state) wears \(name)")
            for dark in [false, true] {
                XCTAssertEqual(hex(color, dark: dark), hex(Color(uiColor: expected), dark: dark), "\(state) (\(dark ? "dark" : "light"))")
            }
        }
    }

    func testTheMarkAndHostColoursAreTheSharedTokens() throws {
        let t = try tokens()
        XCTAssertEqual(hex(TendColor.waiting, dark: false), t.brand.waiting)
        XCTAssertEqual(hex(TendColor.error, dark: false), t.brand.error)
        XCTAssertEqual(hex(TendColor.done, dark: false), t.brand.done)
        assertPair(TendColor.working, t.brand.ink, "ink")
        // Every alias maps into the shared palette, in its order.
        let shared = t.host.gradients.map { $0.map { $0.uppercased() } }
        var seen = Set<[String]>()
        for alias in ["local", "mini", "mbp", "rtzr", "jiun-mini", "a", "b", "c", "d", "e", "f", "g"] {
            let (top, bottom) = HostIdentity.colors(alias)
            let pair = [hex(top, dark: false), hex(bottom, dark: false)]
            XCTAssertTrue(shared.contains(pair), "\(alias): \(pair) is not in tokens.json")
            seen.insert(pair)
        }
        XCTAssertGreaterThan(seen.count, 3)
    }
}
