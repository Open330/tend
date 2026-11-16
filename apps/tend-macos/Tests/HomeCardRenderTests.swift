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

/// Draws a view the way a window does, in an off-screen window, so scroll
/// views and lazy stacks lay out their content (ImageRenderer leaves them
/// empty).
@MainActor
private func snapshot<V: View>(_ view: V, name: String, scheme: ColorScheme, size: CGSize) throws -> NSBitmapImageRep {
    let host = NSHostingView(rootView: view
        .frame(width: size.width, height: size.height)
        .background(TendSurfacePalette.workspace(for: scheme))
        .environment(\.colorScheme, scheme))
    host.frame = CGRect(origin: .zero, size: size)
    let window = NSWindow(contentRect: host.frame, styleMask: [.borderless], backing: .buffered, defer: false)
    window.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
    window.contentView = host
    window.setFrameOrigin(NSPoint(x: -10_000, y: -10_000))
    window.orderBack(nil)
    defer { window.orderOut(nil) }
    // Let SwiftUI lay out and the lazy stacks fill in.
    for _ in 0..<5 {
        host.layoutSubtreeIfNeeded()
        RunLoop.main.run(until: Date().addingTimeInterval(0.05))
    }
    let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds), "\(name) did not render")
    host.cacheDisplay(in: host.bounds, to: rep)
    if let dir = ProcessInfo.processInfo.environment["TEND_RENDER_DIR"],
       let png = rep.representation(using: .png, properties: [:]) {
        try png.write(to: URL(fileURLWithPath: dir).appendingPathComponent("\(name)-\(scheme == .dark ? "dark" : "light").png"))
    }
    return rep
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

// MARK: - Both layouts

private let mini = TendFleetHostIdentity(alias: "jiun-mini", local: false, state: "online", mode: "control")

private func hosted(_ id: String, state: String, at: String, host: TendFleetHostIdentity = mini) throws -> TendHostedAgent {
    let object: [String: Any] = [
        "kind": "claude_code", "agent_session_id": id, "state": state, "state_entered_at": at,
    ]
    let agent = try JSONDecoder().decode(TendAgent.self, from: JSONSerialization.data(withJSONObject: object))
    return TendHostedAgent(host: host, agent: agent, pane: nil)
}

private func sampleWork(
    _ id: String, _ title: String, lane: FleetLane, headline: String, minutesAgo: Double,
    repo: String = "tend", stages: [FleetWorkStage.State]? = nil, host: TendFleetHostIdentity = mini
) throws -> FleetWork {
    let at = Date.now.addingTimeInterval(-minutesAgo * 60)
    let state = switch lane {
    case .needsYou: "waiting_input"
    case .running: "working"
    case .finished: "idle"
    }
    let agent = try hosted(id, state: state, at: ISO8601DateFormatter().string(from: at), host: host)
    return FleetWork(
        source: .agent(id),
        title: title,
        repo: repo,
        lane: lane,
        stages: (stages ?? [lane == .needsYou ? .needsYou : .running]).enumerated().map { index, state in
            FleetWorkStage(name: ["Implement", "Review", "Ship"][index % 3], owner: "claude · \(host.alias)", state: state, kind: "claude_code")
        },
        headline: headline,
        since: at,
        agents: [agent],
        focus: agent,
        handledLimit: false,
        lastChange: at
    )
}

@MainActor
private func sampleBoard() throws -> (works: [FleetWork], decisions: [FleetDecision]) {
    let works = [
        try sampleWork("w1", "Sandbox location for the bench", lane: .needsYou,
                       headline: "Where should the sandbox live? /tmp is cleared on reboot, the data dir is shared with the real tendd.",
                       minutesAgo: 95),
        try sampleWork("w2", "Fix the tour exit race", lane: .running, headline: "Running the onboarding tests again after the fix",
                       minutesAgo: 3, stages: [.done, .running, .waiting]),
        try sampleWork("w3", "Release notes for 0.11", lane: .running, headline: "Collecting merged PRs since v0.10.9",
                       minutesAgo: 40, repo: "site", host: local),
        try sampleWork("w4", "Lockfile check in CI", lane: .finished, headline: "Added a --locked build; CI fails on a stale Cargo.lock",
                       minutesAgo: 12, stages: [.done]),
        try sampleWork("w5", "Menu bar idle CPU", lane: .finished, headline: "Idle CPU down to 0.3% in a 15 s sample",
                       minutesAgo: 300, stages: [.done, .done]),
    ].sorted(by: FleetBoard.order)
    return (works, [try longQuestion()])
}

@MainActor
private func board(_ layout: FleetCardLayout, works: [FleetWork], decisions: [FleetDecision], selectedID: String? = nil) -> some View {
    HStack(alignment: .top, spacing: 12) {
        ForEach(FleetLane.allCases) { lane in
            FleetLaneColumn(
                lane: lane,
                works: works.filter { $0.lane == lane },
                decisions: lane == .needsYou ? decisions : [],
                decisionCard: { decision in
                    FleetDecisionCard(decision: decision, openRevision: { _ in }, answer: { _ in true }, decide: { _, _ in true })
                },
                selectedID: selectedID,
                select: { _ in },
                open: { _ in },
                showsReviewed: .constant(false),
                layout: layout
            )
        }
    }
    .environment(\.fleetCardEmphasis, layout == .emphasis)
    .frame(height: 720)
}

@MainActor
@Test func bothCardLayoutsRenderInBothAppearances() throws {
    let (works, decisions) = try sampleBoard()
    for scheme in [ColorScheme.light, .dark] {
        _ = try snapshot(board(.emphasis, works: works, decisions: decisions).padding(16),
                         name: "home-emphasis", scheme: scheme, size: CGSize(width: 1100, height: 760))
        // The compact list beside its panel, a decision selected.
        let selected = FleetTurnItem.decision(decisions[0]).id
        let detail = FleetDecisionDetail(
            decision: decisions[0],
            card: FleetDecisionCard(decision: decisions[0], openRevision: { _ in }, answer: { _ in true }, decide: { _, _ in true }),
            openAsker: {},
            close: {}
        )
        let compact = HStack(spacing: 0) {
            board(.compact, works: works, decisions: decisions, selectedID: selected).padding(16)
            Divider()
            detail.frame(width: 400)
        }
        _ = try snapshot(compact, name: "home-compact", scheme: scheme, size: CGSize(width: 1400, height: 760))
        // Each emphasis card on its own, outside a scroll view.
        let cards = VStack(spacing: 8) {
            FleetDecisionCard(decision: decisions[0], openRevision: { _ in }, answer: { _ in true }, decide: { _, _ in true })
            ForEach(works) { work in
                FleetWorkCard(work: work, isSelected: work.id == "agent:w2", select: {})
            }
        }
        .environment(\.fleetCardEmphasis, true)
        _ = try render(cards, name: "home-emphasis-cards", scheme: scheme)
        let rows = VStack(spacing: 2) {
            FleetCompactRow(decision: decisions[0], isSelected: true) {}
            ForEach(works) { work in
                FleetCompactRow(work: work, isSelected: false, movedFrom: nil) {}
            }
        }
        _ = try render(rows, name: "home-compact-rows", scheme: scheme)
    }
}

@Test func theCardLayoutIsStoredByName() {
    // @AppStorage keeps the raw value; a renamed case would reset everyone.
    #expect(FleetCardLayout.allCases.map(\.rawValue) == ["emphasis", "compact"])
    #expect(FleetCardLayout(rawValue: "compact") == .compact)
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
