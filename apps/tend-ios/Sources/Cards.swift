// The cards outside Your turn: an agent at work or finished
// (`FleetWorkCard`), and a decision already taken.
import SwiftUI
import UIKit

/// One agent as a work card, as the Mac draws a single-stage unit: title,
/// stage bar (not in Finished), the agent, the headline, then the meta row.
struct AgentCard: View {
    let agent: LaneAgent
    let lane: Lane
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        let a = agent.agent
        VStack(alignment: .leading, spacing: 8) {
            Text(verbatim: a.title)
                .font(TendType.cardTitle)
                .lineLimit(TendLines.more(2, typeSize))
                .frame(maxWidth: .infinity, alignment: .leading)
            if lane != .finished {
                Capsule().fill(a.status.color).frame(height: 4)
                    .accessibilityHidden(true)
                Text(verbatim: a.maker).font(TendType.meta.weight(.medium)).foregroundStyle(TendInk.secondary)
            }
            Text(TendMarkdown.inline(a.headline ?? a.status.label))
                .font(TendType.body)
                .foregroundStyle(TendInk.secondary)
                .lineLimit(TendLines.more(lane == .finished ? 3 : 2, typeSize))
            MetaRow(kind: a.kind, repo: a.repo, host: agent.host, hostIsLocal: agent.hostIsLocal, since: a.since)
        }
        .tendCard()
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("agentCard")
    }
}

/// A product's own mark (Tend for Mac's `AgentLogos`, copied with its
/// NOTICE): drawn only scaled, never recoloured; the OpenAI blossom in
/// exactly black or white. A kind without a mark gets an SF Symbol.
struct AgentLogo: View {
    let kind: String
    var size: CGFloat = 15
    @Environment(\.colorScheme) private var colorScheme

    private var asset: (name: String, scale: CGFloat, monochrome: Bool)? {
        switch kind {
        case "claude_code", "claude": ("AgentLogos/claude", 0.98, false)
        case "codex", "openai": ("AgentLogos/openai", 1.37, true)
        case "agy", "antigravity": ("AgentLogos/antigravity", 1.35, false)
        default: nil
        }
    }

    var body: some View {
        Group {
            if let asset {
                Image(asset.name)
                    .resizable()
                    .renderingMode(asset.monochrome ? .template : .original)
                    .scaledToFit()
                    .foregroundStyle(colorScheme == .dark ? Color.white : Color.black)
                    .scaleEffect(asset.scale)
            } else {
                Image(systemName: "terminal").resizable().scaledToFit().foregroundStyle(TendInk.secondary)
            }
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }
}

/// A project heading inside a lane (`folder` + name + count).
struct ProjectHeader: View {
    let name: String
    let count: Int

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "folder").imageScale(.small)
            Text(verbatim: name).font(TendType.meta.weight(.semibold)).foregroundStyle(TendInk.secondary)
            Text(verbatim: "\(count)").font(TendType.meta).monospacedDigit().foregroundStyle(TendInk.tertiary)
        }
        .foregroundStyle(TendInk.secondary)
        .padding(.top, 4)
        .accessibilityAddTraits(.isHeader)
    }
}

/// The Mac card's meta row: agent, project, host badge, then the time.
struct MetaRow: View {
    let kind: String
    let repo: String?
    let host: String
    let hostIsLocal: Bool
    let since: Date?

    var body: some View {
        HStack(spacing: 8) {
            AgentLogo(kind: kind)
            if let repo {
                Text(verbatim: repo).lineLimit(1)
            }
            HStack(spacing: 4) {
                HostBadge(alias: host, local: hostIsLocal, state: "online", size: 16, showsState: false)
                Text(verbatim: host).lineLimit(1)
            }
            Spacer(minLength: 4)
            if let since {
                Text(since, format: .relative(presentation: .numeric, unitsStyle: .abbreviated))
                    .monospacedDigit()
                    .lineLimit(1)
                    .fixedSize()
            }
        }
        .font(TendType.meta)
        .foregroundStyle(TendSurface.metaText)
        .lineLimit(1)
    }
}

/// A decision already taken, in Finished.
struct DecidedCard: View {
    let past: PastDecision

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            HStack(spacing: 6) {
                Chip(text: past.outcome, color: past.isPositive ? TendPalette.green : TendPalette.orange, symbol: past.isPositive ? "checkmark" : "arrow.uturn.backward")
                    .accessibilityIdentifier("pastOutcome")
                Spacer(minLength: 4)
                if let at = past.at {
                    Text(at, format: .relative(presentation: .numeric, unitsStyle: .abbreviated))
                        .font(TendType.meta)
                        .monospacedDigit()
                        .foregroundStyle(TendSurface.metaText)
                }
            }
            Text(verbatim: past.title)
                .font(TendType.cardTitle)
                .lineLimit(3)
                .accessibilityIdentifier("pastTitle")
            if let text = past.text {
                Text(verbatim: text)
                    .font(TendType.body)
                    .foregroundStyle(TendInk.secondary)
                    .lineLimit(3)
            }
            HStack(spacing: 6) {
                HostBadge(alias: past.host, local: past.hostIsLocal, state: "online", size: 16, showsState: false)
                Text(verbatim: past.host)
                if let by = past.by {
                    Text(verbatim: "·")
                    Text("by \(ActorName.display(by))").lineLimit(1)
                }
            }
            .font(TendType.meta)
            .foregroundStyle(TendSurface.metaText)
            .accessibilityElement(children: .combine)
        }
        .tendCard()
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("pastDecision")
    }
}

/// Lays children out in rows, wrapping (evidence chips).
struct FlowLayout: Layout {
    var spacing: CGFloat = 6

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? .infinity
        var x: CGFloat = 0, y: CGFloat = 0, row: CGFloat = 0, widest: CGFloat = 0
        for subview in subviews {
            let size = subview.sizeThatFits(.init(width: width, height: nil))
            if x > 0, x + size.width > width {
                y += row + spacing
                x = 0
                row = 0
            }
            x += size.width + spacing
            row = max(row, size.height)
            widest = max(widest, x - spacing)
        }
        return CGSize(width: min(widest, width), height: y + row)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var x = bounds.minX, y = bounds.minY, row: CGFloat = 0
        for subview in subviews {
            let size = subview.sizeThatFits(.init(width: bounds.width, height: nil))
            if x > bounds.minX, x + size.width > bounds.maxX {
                y += row + spacing
                x = bounds.minX
                row = 0
            }
            subview.place(at: CGPoint(x: x, y: y), proposal: .init(size))
            x += size.width + spacing
            row = max(row, size.height)
        }
    }
}

/// Who decided, for people: a paired device's name (`app:<name>#<id>`), a
/// signed-in operator's address (`web:<email>`), else the label as recorded.
enum ActorName {
    static func display(_ label: String) -> String {
        if label.hasPrefix("app:") {
            let rest = label.dropFirst(4)
            return String(rest.split(separator: "#", maxSplits: 1).first ?? rest)
        }
        if label.hasPrefix("web:") { return String(label.dropFirst(4)) }
        return label
    }
}

/// The card actions' one style: outlined like the Mac's secondary buttons
/// (input border, control radius), filled for the one primary action, and
/// at least 44 pt tall for a finger.
struct CardButtonStyle: ButtonStyle {
    var primary = false
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(TendType.body.weight(primary ? .semibold : .medium))
            .lineLimit(1)
            .padding(.horizontal, 14)
            .frame(minHeight: 44)
            .foregroundStyle(primary ? Color.white : Color.accentColor)
            .background(
                RoundedRectangle(cornerRadius: TendSurface.controlRadius, style: .continuous)
                    .fill(primary ? Color.accentColor.opacity(configuration.isPressed ? 0.8 : 1) : (configuration.isPressed ? TendInk.secondary.opacity(0.12) : Color.clear))
            )
            .overlay(
                RoundedRectangle(cornerRadius: TendSurface.controlRadius, style: .continuous)
                    .strokeBorder(primary ? Color.clear : TendSurface.inputBorder, lineWidth: 1)
            )
            .opacity(isEnabled ? 1 : 0.45)
            .contentShape(Rectangle())
    }
}

extension ButtonStyle where Self == CardButtonStyle {
    static var card: CardButtonStyle { CardButtonStyle() }
    static var cardPrimary: CardButtonStyle { CardButtonStyle(primary: true) }
}

// MARK: - Work units (Home's board)

extension Lane {
    /// The state word on a card's meta row (`FleetLane.statusLabel`).
    var statusLabel: String {
        switch self {
        case .backlog: String(localized: "Planned")
        case .yourTurn: String(localized: "Your turn")
        case .inProgress: String(localized: "Working")
        case .finished: String(localized: "Finished")
        }
    }

    var statusImage: String {
        switch self {
        case .backlog: "tray"
        case .yourTurn: "hand.raised.fill"
        case .inProgress: "circle.dotted.circle"
        case .finished: "checkmark.circle.fill"
        }
    }
}

extension BoardStage.State {
    /// The Mac's stage colours (`FleetWorkStage.State.color`).
    var color: Color {
        switch self {
        case .done: TendPalette.green
        case .running: TendPalette.blue
        case .needsYou: TendPalette.orange
        case .waiting: TendInk.secondary.opacity(0.25)
        }
    }

    var label: String {
        switch self {
        case .done: String(localized: "Done")
        case .running: String(localized: "Working")
        case .needsYou: String(localized: "Your turn")
        case .waiting: String(localized: "Waiting")
        }
    }
}

/// The stage bar: one segment per stage, then the stage on now and how
/// many are done (`FleetStageBar`).
struct StageBar: View {
    let unit: BoardUnit

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 3) {
                ForEach(Array(unit.stages.enumerated()), id: \.offset) { _, stage in
                    Capsule().fill(stage.state.color).frame(height: 4)
                }
            }
            .accessibilityHidden(true)
            if let index = unit.currentStage {
                HStack {
                    Text(verbatim: unit.stages[index].name).fontWeight(.medium).foregroundStyle(TendInk.secondary)
                    Spacer()
                    if unit.stages.count > 1 {
                        Text(verbatim: "\(unit.doneStages)/\(unit.stages.count)").monospacedDigit()
                    }
                }
                .font(TendType.meta)
                .foregroundStyle(TendInk.tertiary)
            }
        }
    }
}

/// The inset box a card's headline sits in (`FleetInsetBox`).
struct InsetBox<Content: View>: View {
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 7) { content }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color(token: TendDesignTokens.inset), in: RoundedRectangle(cornerRadius: CGFloat(TendDesignTokens.Metric.insetRadius), style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: CGFloat(TendDesignTokens.Metric.insetRadius), style: .continuous).strokeBorder(Color(token: TendDesignTokens.insetBorder)))
    }
}

/// Card line limits: one more line at the accessibility text sizes, where
/// two lines hold only a few words.
enum TendLines {
    static func more(_ lines: Int, _ size: DynamicTypeSize) -> Int {
        size.isAccessibilitySize ? lines + 1 : lines
    }
}

/// The products' marks side by side (`TendAgentLogoStack`).
struct AgentLogoStack: View {
    let kinds: [String]
    var size: CGFloat = 14

    var body: some View {
        HStack(spacing: 2) {
            ForEach(Array(Self.products(kinds).prefix(3)), id: \.self) { AgentLogo(kind: $0, size: size) }
        }
    }

    /// One mark per product, in order.
    static func products(_ kinds: [String]) -> [String] {
        var seen: [String] = []
        for kind in kinds where !seen.contains(kind) { seen.append(kind) }
        return seen
    }
}

/// A unit of work as the Mac draws it (`FleetWorkCard`): a Helper pill, the
/// title, the stage bar (not for a finished single-stage unit), the
/// headline in its inset box, then state, agents, hosts, project, helpers
/// and time on one line.
struct WorkCard: View {
    let unit: BoardUnit
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                if unit.orphanHelper {
                    Chip(text: String(localized: "Helper"))
                }
                Text(verbatim: unit.titleText)
                    .font(TendType.cardTitle)
                    .foregroundStyle(unit.lane == .finished ? Color.primary.opacity(0.82) : Color.primary)
                    .lineLimit(TendLines.more(2, typeSize))
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            if unit.lane != .finished || unit.stages.count > 1 {
                StageBar(unit: unit)
            }
            if cardHeadline != nil || unit.handledLimit {
                InsetBox {
                    if let headline = cardHeadline {
                        // One paragraph on the card; the detail shows it whole.
                        Text(TendMarkdown.inline(headline.split(whereSeparator: \.isNewline).joined(separator: " ")))
                            .font(unit.lane == .yourTurn ? TendType.body.weight(.medium) : TendType.body)
                            .foregroundStyle(unit.lane == .yourTurn ? Color.primary : TendInk.secondary)
                            .lineLimit(TendLines.more(unit.lane == .yourTurn ? 4 : 2, typeSize))
                            // Measured and drawn at the same height in a lazy lane.
                            .fixedSize(horizontal: false, vertical: true)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    if unit.handledLimit {
                        Label(String(localized: "Rate limited · the limit policy will resume it"), systemImage: "arrow.triangle.2.circlepath")
                            .font(TendType.meta)
                            .foregroundStyle(TendInk.secondary)
                    }
                }
            }
            meta
        }
        .tendCard(attention: unit.lane == .yourTurn)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("workCard")
    }

    /// The headline, unless it only repeats the state the meta row shows.
    private var cardHeadline: String? { Self.shownHeadline(unit) }

    /// Hidden only when it is exactly the lane's own state, as the meta row
    /// shows it ("Working" over "Working" in progress). A status the row
    /// does not show (an error, a question waiting) always stays.
    static func shownHeadline(_ unit: BoardUnit) -> String? {
        guard let headline = unit.headlineText else { return nil }
        let said = headline.trimmingCharacters(in: .whitespacesAndNewlines.union(.punctuationCharacters))
        if unit.lane == .inProgress, said.caseInsensitiveCompare(unit.lane.statusLabel) == .orderedSame { return nil }
        return headline
    }

    /// In a narrow column the host names, then the project, give way
    /// first; the state and the time always stay. At large text sizes the
    /// row becomes two: the state and the time, then who and where.
    private var meta: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 6) { status; who(hostNames: true, project: true); Spacer(minLength: 4); time }
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 6) { status; Spacer(minLength: 4); time }
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 6) { who(hostNames: true, project: true) }
                    HStack(spacing: 6) { who(hostNames: true, project: false) }
                    HStack(spacing: 6) { who(hostNames: false, project: false) }
                }
            }
        }
        .lineLimit(1)
        .font(TendType.meta)
        .foregroundStyle(TendSurface.metaText)
    }

    private var status: some View {
        Label(unit.lane.statusLabel, systemImage: unit.lane.statusImage)
            .labelStyle(.titleAndIcon)
            .fontWeight(.semibold)
            .foregroundStyle(unit.lane.tint)
            .fixedSize()
    }

    @ViewBuilder
    private var time: some View {
        if let time = unit.time {
            Text(time, format: .relative(presentation: .numeric, unitsStyle: .abbreviated))
                .monospacedDigit()
                .fixedSize()
        }
    }

    @ViewBuilder
    private func who(hostNames: Bool, project: Bool) -> some View {
            if !unit.agentKinds.isEmpty {
                AgentLogoStack(kinds: unit.agentKinds, size: 13)
            }
            ForEach(unit.hosts.prefix(2), id: \.self) { alias in
                HStack(spacing: 3) {
                    HostGlyph(alias: alias, size: 14)
                    if hostNames { Text(verbatim: alias).fixedSize() }
                }
            }
            if project, let repo = unit.repo {
                Text(verbatim: "· \(repo)").fixedSize()
            }
            if !unit.helpers.isEmpty {
                Label("\(unit.helpers.count)", systemImage: "person.2")
                    .labelStyle(.titleAndIcon)
                    .fixedSize()
                    .accessibilityLabel(Text("\(unit.helpers.count) helper sessions"))
            }
    }
}

/// Planned Work in the backlog (`FleetBacklogCard`), without Start: a
/// phone starts new work only, never a Work that already exists.
struct BacklogCard: View {
    let record: BacklogRecord
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text(verbatim: record.titleText)
                    .font(TendType.cardTitle)
                    .lineLimit(TendLines.more(2, typeSize))
                    .frame(maxWidth: .infinity, alignment: .leading)
                if let priority = record.priority {
                    Chip(text: "P\(priority)")
                }
            }
            if let line = record.line?.nonEmpty {
                Text(verbatim: line).font(TendType.body).foregroundStyle(TendInk.secondary).lineLimit(TendLines.more(2, typeSize))
            }
            let meta = [record.workspace, record.pipeline].compactMap { $0?.nonEmpty }
            if !meta.isEmpty {
                Text(verbatim: meta.joined(separator: " · ")).font(TendType.meta).foregroundStyle(TendSurface.metaText).lineLimit(1)
            }
        }
        .tendCard()
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("backlogCard")
    }
}
