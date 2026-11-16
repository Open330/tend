// Home's cards in its two layouts: status-emphasis cards (a coloured
// stripe, a bold title, the question in an inset box) and a compact list
// of one row per item whose details open beside the board.
import SwiftUI

/// How Home draws the items in its lanes.
enum FleetCardLayout: String, CaseIterable, Identifiable, Sendable {
    /// Cards led by a status stripe, Your turn tinted, the question and
    /// its choices in an inset box.
    case emphasis
    /// One row per item; the selected item's details open in the panel.
    case compact

    var id: Self { self }

    var title: String {
        switch self {
        case .emphasis: String(localized: "Status emphasis")
        case .compact: String(localized: "Compact list")
        }
    }

    var systemImage: String {
        switch self {
        case .emphasis: "rectangle.grid.1x2"
        case .compact: "list.bullet"
        }
    }
}

extension FleetLane {
    /// The lane's colour on stripes, dots and labels.
    var tint: Color {
        switch self {
        case .needsYou: .orange
        case .running: .blue
        case .finished: .green
        }
    }

    /// The state a card in this lane is in, as its meta line names it.
    var statusLabel: String {
        switch self {
        case .needsYou: String(localized: "Your turn")
        case .running: String(localized: "Working")
        case .finished: String(localized: "Finished")
        }
    }

    var statusImage: String {
        switch self {
        case .needsYou: "hand.raised.fill"
        case .running: "circle.dotted.circle"
        case .finished: "checkmark.circle.fill"
        }
    }
}

extension FleetBoard {
    /// The time a card shows: the same moment its lane orders it by.
    static func shownTime(_ work: FleetWork) -> Date? {
        work.lane == .needsYou ? work.since : (work.lastChange ?? work.since)
    }
}

// MARK: - Surfaces

enum FleetCardSurface {
    /// A card's ground: Your turn tinted so it stands out at a glance.
    static func fill(_ lane: FleetLane, emphasis: Bool, _ scheme: ColorScheme) -> Color {
        let base = TendSurfacePalette.workspace(for: scheme)
        guard emphasis, lane == .needsYou else { return base }
        return scheme == .dark
            ? Color(red: 0.20, green: 0.16, blue: 0.11)
            : Color(red: 1.0, green: 0.965, blue: 0.92)
    }

    /// The inset box that holds the question and its choices.
    static func inset(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? Color.white.opacity(0.05) : Color.black.opacity(0.035)
    }

    static func insetBorder(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? Color.white.opacity(0.07) : Color.black.opacity(0.06)
    }
}

/// Draws a card in the status-emphasis style: the lane's stripe on the
/// leading edge, its ground, a border (the accent when selected).
struct FleetEmphasisCardFrame: ViewModifier {
    let lane: FleetLane
    var isSelected = false
    @Environment(\.colorScheme) private var colorScheme

    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: 8, style: .continuous)
        content
            .padding(.leading, 5)
            .background(FleetCardSurface.fill(lane, emphasis: true, colorScheme), in: shape)
            .overlay(alignment: .leading) {
                Rectangle()
                    .fill(lane.tint)
                    .frame(width: 5)
                    .allowsHitTesting(false)
            }
            .clipShape(shape)
            .overlay(
                shape.strokeBorder(
                    isSelected ? Color.accentColor : (lane == .needsYou ? lane.tint.opacity(0.35) : TendTheme.border(colorScheme)),
                    lineWidth: isSelected ? 1.5 : 1
                )
            )
    }
}

/// The inset box a card's question and choices sit in.
struct FleetInsetBox<Content: View>: View {
    @ViewBuilder var content: () -> Content
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        VStack(alignment: .leading, spacing: 7, content: content)
            .padding(9)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(FleetCardSurface.inset(colorScheme), in: RoundedRectangle(cornerRadius: 6, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 6, style: .continuous).strokeBorder(FleetCardSurface.insetBorder(colorScheme)))
    }
}

/// Whether decision cards draw in the status-emphasis style; Home sets it
/// for its lanes in that layout.
private struct FleetCardEmphasisKey: EnvironmentKey {
    static let defaultValue = false
}

extension EnvironmentValues {
    var fleetCardEmphasis: Bool {
        get { self[FleetCardEmphasisKey.self] }
        set { self[FleetCardEmphasisKey.self] = newValue }
    }
}

// MARK: - Work card

/// A unit of work on the board. Tapping it opens the inspector, where its
/// question is answered, its pane opened, or its result marked reviewed.
struct FleetWorkCard: View {
    let work: FleetWork
    let isSelected: Bool
    let select: () -> Void
    var movedFrom: FleetLane?
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                if work.isOrphanHelper {
                    Text("Helper")
                        .font(.system(size: 10, weight: .semibold))
                        .padding(.horizontal, 5)
                        .padding(.vertical, 1)
                        .background(Color.secondary.opacity(0.15), in: Capsule())
                        .foregroundStyle(.secondary)
                        .help("Working on another agent's request; that agent is no longer running")
                }
                Text(work.title)
                    .font(.system(size: 14.5, weight: .bold))
                    .foregroundStyle(work.lane == .finished ? Color.primary.opacity(0.82) : Color.primary)
                    .lineLimit(2)
                    .frame(maxWidth: .infinity, alignment: .leading)
                if let movedFrom {
                    Label(String(localized: "From \(movedFrom.title)"), systemImage: "arrow.turn.down.right")
                        .font(.system(size: 10, weight: .semibold))
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(work.lane.tint.opacity(0.18), in: Capsule())
                        .foregroundStyle(work.lane.tint)
                        .fixedSize()
                        .transition(.opacity)
                }
            }
            .animation(FleetBoard.moveMarkAnimation(movedFrom), value: movedFrom)
            if work.lane != .finished || work.stages.count > 1 {
                FleetStageBar(work: work)
            }
            FleetInsetBox {
                Text(work.headline)
                    .font(work.lane == .needsYou ? TendType.body.weight(.medium) : TendType.body)
                    .foregroundStyle(work.lane == .needsYou ? Color.primary : Color.secondary)
                    .lineLimit(work.lane == .needsYou ? 4 : 2)
                    .frame(maxWidth: .infinity, alignment: .leading)
                if work.handledLimit {
                    Label("Rate limited · the limit policy will resume it", systemImage: "arrow.triangle.2.circlepath")
                        .font(TendType.meta)
                        .foregroundStyle(.secondary)
                }
                if work.lane == .needsYou, FleetBoard.waiting(work).count > 1 {
                    Label(String(localized: "\(FleetBoard.waiting(work).count) agents wait on you"), systemImage: "person.2.badge.gearshape")
                        .font(TendType.meta)
                        .foregroundStyle(.orange)
                }
            }
            meta
        }
        .padding(12)
        .modifier(FleetEmphasisCardFrame(lane: work.lane, isSelected: isSelected))
        .overlay(
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .strokeBorder(work.lane.tint, lineWidth: 2)
                .opacity(movedFrom == nil ? 0 : 0.9)
                .animation(FleetBoard.moveMarkAnimation(movedFrom), value: movedFrom)
                .allowsHitTesting(false)
        )
        .contentShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
        .onTapGesture(perform: select)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
        .accessibilityAction(named: Text("Inspect"), select)
    }

    /// The state, then who and where, then when, on one subdued line.
    private var meta: some View {
        HStack(spacing: 6) {
            Label(work.lane.statusLabel, systemImage: work.lane.statusImage)
                .labelStyle(TendButtonLabelStyle())
                .fontWeight(.semibold)
                .foregroundStyle(work.lane.tint)
                .lineLimit(1)
                .fixedSize()
            if !work.agentKinds.isEmpty {
                TendAgentLogoStack(kinds: work.agentKinds, size: 12)
            }
            FleetHostBadges(hosts: work.hosts)
            if let repo = work.repo {
                Text(verbatim: "· \(repo)")
                    .lineLimit(1)
            }
            if !work.helpers.isEmpty {
                Label("\(work.helpers.count)", systemImage: "person.2")
                    .labelStyle(.titleAndIcon)
                    .help("\(work.helpers.count) helper sessions")
            }
            Spacer(minLength: 4)
            if let time = FleetBoard.shownTime(work) {
                Text(time, format: .relative(presentation: .numeric, unitsStyle: .abbreviated))
                    .monospacedDigit()
                    .lineLimit(1)
                    .fixedSize()
            }
        }
        .font(TendType.meta)
        .foregroundStyle(.secondary)
    }
}

// MARK: - Compact rows

/// One item in the compact list: a status dot, the title, the host and
/// how long ago. Selecting it opens its details beside the board.
struct FleetCompactRow: View {
    let tint: Color
    let title: String
    let host: String?
    let time: Date?
    let isSelected: Bool
    /// A decision from tendd's queue rather than a unit of work.
    var isDecision = false
    var movedFrom: FleetLane?
    let select: () -> Void
    @State private var hovering = false
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        HStack(spacing: 8) {
            Circle()
                .fill(tint)
                .frame(width: 8, height: 8)
            if isDecision {
                Image(systemName: "questionmark.bubble")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(tint)
            }
            Text(verbatim: title)
                .font(.system(size: 13, weight: .medium))
                .lineLimit(1)
                .truncationMode(.tail)
                .frame(maxWidth: .infinity, alignment: .leading)
            if let movedFrom {
                Image(systemName: "arrow.turn.down.right")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(tint)
                    .help(String(localized: "From \(movedFrom.title)"))
                    .transition(.opacity)
            }
            if let host {
                Text(verbatim: host)
                    .font(TendType.meta)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .frame(maxWidth: 110, alignment: .trailing)
                    .fixedSize(horizontal: true, vertical: false)
            }
            if let time {
                Text(time, format: .relative(presentation: .numeric, unitsStyle: .abbreviated))
                    .font(TendType.meta)
                    .monospacedDigit()
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
                    .fixedSize()
            }
        }
        .animation(FleetBoard.moveMarkAnimation(movedFrom), value: movedFrom)
        .padding(.horizontal, 8)
        .frame(minHeight: 30)
        .background(
            RoundedRectangle(cornerRadius: 6, style: .continuous)
                .fill(isSelected ? Color.accentColor.opacity(colorScheme == .dark ? 0.32 : 0.16)
                    : hovering ? TendTheme.hover(colorScheme) : Color.clear)
        )
        .contentShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
        .onHover { hovering = $0 }
        .onTapGesture(perform: select)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(isSelected ? [.isButton, .isSelected] : .isButton)
        .accessibilityAction(named: Text("Inspect"), select)
    }
}

extension FleetCompactRow {
    init(work: FleetWork, isSelected: Bool, movedFrom: FleetLane?, select: @escaping () -> Void) {
        self.init(
            tint: work.lane.tint,
            title: work.title,
            host: work.hostAliases.isEmpty ? nil : work.hostAliases.joined(separator: ", "),
            time: FleetBoard.shownTime(work),
            isSelected: isSelected,
            movedFrom: movedFrom,
            select: select
        )
    }

    init(decision: FleetDecision, isSelected: Bool, select: @escaping () -> Void) {
        self.init(
            tint: FleetLane.needsYou.tint,
            title: decision.title,
            host: decision.host.alias,
            time: decision.since,
            isSelected: isSelected,
            isDecision: true,
            select: select
        )
    }
}

// MARK: - Decision details

/// A decision's details beside the board in the compact layout: the whole
/// card with its options, answer and evidence, and the asker to open.
struct FleetDecisionDetail: View {
    let decision: FleetDecision
    let card: FleetDecisionCard
    var openAsker: (() -> Void)?
    let close: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .top, spacing: 8) {
                VStack(alignment: .leading, spacing: 4) {
                    Label(decision.isHumanRequest ? String(localized: "Question") : String(localized: "Review"),
                          systemImage: decision.isHumanRequest ? "questionmark.bubble" : "checklist")
                        .font(.system(size: 11, weight: .bold))
                        .foregroundStyle(.orange)
                    Text([decision.asker, decision.host.alias, decision.repo].compactMap { $0 }.joined(separator: " · "))
                        .font(TendType.meta)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                Spacer(minLength: 4)
                if let openAsker {
                    Button("Open Agent", action: openAsker)
                        .buttonStyle(.tendSecondary)
                }
                Button(action: close) {
                    Image(systemName: "xmark")
                }
                .buttonStyle(.tendGhost)
                .help("Close")
                .keyboardShortcut(.cancelAction)
            }
            .padding(16)
            Divider()
            ScrollView {
                card
                    .padding(16)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }
}

extension FleetBoard {
    /// How cards glide when one changes lane.
    static let moveAnimation = Animation.spring(response: 0.5, dampingFraction: 0.86)

    /// How a card's "From …" mark goes away a few seconds after a move: a
    /// short fade inside the card. Animating that fade for the whole board
    /// redrew every card each frame. It appears as before, set right after
    /// the move outside its animation.
    static func moveMarkAnimation(_ movedFrom: FleetLane?) -> Animation? {
        movedFrom == nil ? .easeOut(duration: 0.4) : nil
    }
}
