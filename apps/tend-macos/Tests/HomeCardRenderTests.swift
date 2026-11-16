import AppKit
import Foundation
import SwiftUI
import Testing
@testable import Tend

/// Renders Home's cards off-screen, light and dark, so a card that fails to
/// lay out fails a test. Set TEND_RENDER_DIR (TEST_RUNNER_TEND_RENDER_DIR
/// through xcodebuild) to write each image there for a look.
@MainActor
private func render<V: View>(_ view: V, name: String, scheme: ColorScheme, width: CGFloat = 340) throws -> NSImage {
    let renderer = ImageRenderer(content: view
        .frame(width: width)
        .padding(16)
        .background(TendSurfacePalette.workspace(for: scheme))
        .environment(\.colorScheme, scheme))
    renderer.scale = 2
    let image = try #require(renderer.nsImage, "\(name) did not render")
    if let dir = ProcessInfo.processInfo.environment["TEND_RENDER_DIR"],
       let tiff = image.tiffRepresentation,
       let png = NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:]) {
        try png.write(to: URL(fileURLWithPath: dir).appendingPathComponent("\(name)-\(scheme == .dark ? "dark" : "light").png"))
    }
    return image
}

private let local = TendFleetHostIdentity(alias: "local", local: true, state: "online", mode: "local")

private func longQuestion() throws -> FleetDecision {
    let object: [String: Any] = [
        "request_id": "req_1", "body": "The migration touches every host's data directory.",
        "from_session_id": "s1", "from_pane": "%3", "from_alias": "impl",
        "created_at": "2026-11-16T01:00:00Z",
        "decision": [
            "needed": "How should the storage migration roll out?",
            "options": [
                "Ship it behind a flag first, enable it on jiun-mini for a day, then turn it on for every host once the nightly backup finished cleanly",
                "Migrate everything now",
                "Hold it until the next release:\nthe schema may still change and a second migration would be needed",
            ],
            "recommendation": "Flag first",
        ],
    ]
    let entry = try JSONDecoder().decode(TendHumanRequestEntry.self, from: JSONSerialization.data(withJSONObject: object))
    return FleetDecision(humanRequest: entry, host: local)
}

@MainActor
@Test func aDecisionWithLongOptionsShowsEveryLine() throws {
    let decision = try longQuestion()
    for scheme in [ColorScheme.light, .dark] {
        let card = FleetDecisionCard(decision: decision, openRevision: { _ in }, answer: { _ in true }, decide: { _, _ in true })
        let wrapped = try render(card, name: "decision-long-options", scheme: scheme)
        // A third option of one short line would leave the card far
        // shorter: every line of every option takes its own height.
        let short = FleetDecisionCard(
            decision: try FleetDecision.withOptions(["A", "B", "C"], from: decision),
            openRevision: { _ in }, answer: { _ in true }, decide: { _, _ in true })
        let compact = try render(short, name: "decision-short-options", scheme: scheme)
        #expect(wrapped.size.height > compact.size.height + 50, "\(wrapped.size) vs \(compact.size)")
    }
}

private extension FleetDecision {
    /// The same request with other options.
    static func withOptions(_ options: [String], from decision: FleetDecision) throws -> FleetDecision {
        let object: [String: Any] = [
            "request_id": "req_2", "body": decision.summary, "from_session_id": "s1", "from_pane": "%3",
            "created_at": "2026-11-16T01:00:00Z",
            "decision": ["needed": decision.title, "options": options, "recommendation": "Flag first"],
        ]
        let entry = try JSONDecoder().decode(TendHumanRequestEntry.self, from: JSONSerialization.data(withJSONObject: object))
        return FleetDecision(humanRequest: entry, host: local)
    }
}
