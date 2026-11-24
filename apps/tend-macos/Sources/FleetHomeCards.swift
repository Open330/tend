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

    /// The hue whose ink a state word in this lane wears (4.5:1 or more).
    var stateHue: TendStateHue {
        switch self {
        case .needsYou: .orange
        case .running: .blue
        case .finished: .green
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

/// Draws a card (docs/proposals/MAC_DESIGN.md): one container, lifted off
/// its lane by the card fill and a hairline. A card that waits for the
/// person carries a 3pt orange bar at its leading edge (its status word
/// says so too); no tinted border or fill. Selected: an accent border.
struct FleetEmphasisCardFrame: ViewModifier {
    let lane: FleetLane
    var isSelected = false
    @Environment(\.colorScheme) private var colorScheme

    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: TendRadius.card, style: .continuous)
        content
            .padding(.leading, lane == .needsYou ? 4 : 0)
            .background(TendSurface.card(colorScheme), in: shape)
            .overlay(alignment: .leading) {
                if lane == .needsYou {
                    Capsule()
                        .fill(lane.tint)
                        .frame(width: 3)
                        .padding(.vertical, TendSpace.m)
                        .padding(.leading, 5)
                        .allowsHitTesting(false)
                }
            }
            .overlay(
                shape.strokeBorder(
                    isSelected ? Color.accentColor : TendInk.hairline(colorScheme),
                    lineWidth: isSelected ? 1.5 : 1
                )
            )
    }
}

/// A card's question and choices, grouped. No box of its own: the card is
/// the one container (docs/proposals/MAC_DESIGN.md).
struct FleetInsetBox<Content: View>: View {
    @ViewBuilder var content: () -> Content

    var body: some View {
        VStack(alignment: .leading, spacing: TendSpace.s, content: content)
            .frame(maxWidth: .infinity, alignment: .leading)
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
                        .font(TendType.label)
                        .padding(.horizontal, 5)
                        .padding(.vertical, 1)
                        .background(Color.secondary.opacity(0.15), in: Capsule())
                        .foregroundStyle(.secondary)
                        .help("Working on another agent's request; that agent is no longer running")
                }
                Text(work.title)
                    .font(TendType.cardTitle)
                    .foregroundStyle(TendInk.label(colorScheme))
                    .lineLimit(3)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            if work.lane != .finished || work.stages.count > 1 {
                FleetStageBar(work: work)
            }
            FleetInsetBox {
                Text(work.headline)
                    .font(work.lane == .needsYou ? TendType.body.weight(.medium) : TendType.body)
                    .foregroundStyle(work.lane == .needsYou ? TendInk.label(colorScheme) : TendInk.secondary(colorScheme))
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
                        .foregroundStyle(TendInk.state(.orange, colorScheme))
                }
            }
            meta
        }
        .padding(TendSpace.m)
        .modifier(FleetEmphasisCardFrame(lane: work.lane, isSelected: isSelected))
        .overlay(
            RoundedRectangle(cornerRadius: TendRadius.card, style: .continuous)
                .strokeBorder(work.lane.tint, lineWidth: 2)
                .opacity(movedFrom == nil ? 0 : 0.9)
                .animation(FleetBoard.moveMarkAnimation(movedFrom), value: movedFrom)
                .allowsHitTesting(false)
        )
        .contentShape(RoundedRectangle(cornerRadius: TendRadius.card, style: .continuous))
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
                .foregroundStyle(TendInk.state(work.lane.stateHue, colorScheme))
                .lineLimit(1)
                .fixedSize()
            // On the meta line, not beside the title: there it narrowed the
            // title, so the title wrapped to another line and the card
            // shrank with a jump when the mark went. The meta line is one
            // line either way, and the capsule's padding is taken back so
            // it adds no height.
            if let movedFrom {
                // Label ink on the tinted capsule; the arrow keeps the
                // lane's colour.
                Label {
                    Text(String(localized: "From \(movedFrom.title)"))
                        .foregroundStyle(TendInk.label(colorScheme))
                } icon: {
                    Image(systemName: "arrow.turn.down.right")
                        .foregroundStyle(work.lane.tint)
                }
                    .font(TendType.label)
                    .lineLimit(1)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(TendInk.control(colorScheme), in: Capsule())
                    .padding(.vertical, -2)
                    .fixedSize()
                    .transition(.opacity)
            }
            if !work.agentKinds.isEmpty {
                TendAgentLogoStack(kinds: work.agentKinds, size: 12)
            }
            // The repo gives way to the "From …" mark for the few seconds
            // it shows, and the host keeps its name before the repo does.
            FleetHostBadges(hosts: work.hosts)
                .layoutPriority(1)
            if let repo = work.repo, movedFrom == nil {
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
        .foregroundStyle(TendInk.secondary(colorScheme))
        .animation(FleetBoard.moveMarkAnimation(movedFrom), value: movedFrom)
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
                    .font(TendType.label.weight(.semibold))
                    .foregroundStyle(tint)
            }
            Text(verbatim: title)
                .font(TendType.body.weight(.medium))
                .lineLimit(1)
                .truncationMode(.tail)
                .frame(maxWidth: .infinity, alignment: .leading)
            if let movedFrom {
                Image(systemName: "arrow.turn.down.right")
                    .font(TendType.label)
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
                        .font(TendType.label.weight(.semibold))
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
