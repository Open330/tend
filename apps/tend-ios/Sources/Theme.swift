// The visual language of Tend for Mac, on iOS: its surfaces, type roles,
// state colours and symbols, lane words and host identity colours. Values
// come from apps/tend-macos/Sources (TendTheme.swift, AgentStatus.swift,
// FleetHomeView.swift, ContentView.swift, MenuBarAgentAppearance.swift).
// Sizes are the Mac roles moved onto iOS's Dynamic Type styles: the Mac's
// 13 pt body is iOS's 17 pt body, so the hierarchy holds at every text size
// the person picks.
import SwiftUI
import UIKit

extension UIColor {
    convenience init(hex: UInt32, alpha: CGFloat = 1) {
        self.init(
            red: CGFloat((hex >> 16) & 0xFF) / 255,
            green: CGFloat((hex >> 8) & 0xFF) / 255,
            blue: CGFloat(hex & 0xFF) / 255,
            alpha: alpha
        )
    }
}

extension Color {
    /// A colour with a light and a dark value, like `TendSurfacePalette`.
    static func tend(light: UInt32, dark: UInt32) -> Color {
        Color(uiColor: UIColor { traits in
            UIColor(hex: traits.userInterfaceStyle == .dark ? dark : light)
        })
    }
}

extension Color {
    /// A colour from apps/shared/Design/tokens.json (TendDesignTokens.swift).
    init(token: TendTokenColor) {
        self.init(uiColor: UIColor { traits in
            traits.userInterfaceStyle == .dark
                ? UIColor(hex: token.dark, alpha: token.darkAlpha)
                : UIColor(hex: token.light, alpha: token.lightAlpha)
        })
    }
}

/// The state colours: the macOS/iOS 26 system palette, as tokens.json
/// records it. Tend for Mac draws the system colours, which are this
/// palette on macOS 26; reading the tokens keeps an iPhone on iOS 17 or 18
/// (whose system colours are the older ones) the same as the Mac.
enum TendPalette {
    static let orange = Color(token: TendDesignTokens.orange)
    static let blue = Color(token: TendDesignTokens.blue)
    static let green = Color(token: TendDesignTokens.green)
    static let red = Color(token: TendDesignTokens.red)
    static let purple = Color(token: TendDesignTokens.purple)
    static let mint = Color(token: TendDesignTokens.mint)
    /// controlAccentColor's default; the app's tint (AccentColor).
    static let accent = Color(token: TendDesignTokens.accent)
}

/// The Mac app's surfaces (`TendTheme.swift`, `TendSurfacePalette`).
enum TendSurface {
    /// The board behind the lanes, and a card's fill (`workspace`).
    static let workspace = Color(token: TendDesignTokens.editor)
    /// A lane column, a summary box (`panelFill`).
    static let panel = Color(token: TendDesignTokens.lane)
    /// The sidebar (`sideBar`).
    static let sideBar = Color(token: TendDesignTokens.sideBar)
    static let border = Color(token: TendDesignTokens.border)
    static let inputBackground = Color(token: TendDesignTokens.inputBackground)
    static let inputBorder = Color(token: TendDesignTokens.inputBorder)
    /// Meta rows (agent, project, host, time).
    static let metaText = TendInk.secondary

    static let cardRadius: CGFloat = 10
    static let laneRadius: CGFloat = 12
    static let controlRadius: CGFloat = 8
}

/// Quieter text, readable on a phone. The Mac's text-secondary and
/// text-tertiary tokens measure about 4:1 and 1.9:1 on a white card (iOS's
/// own secondary and tertiary label styles 3.4:1 and 1.7:1), under the
/// 4.5:1 that small text needs; on a phone outdoors they wash out. These
/// keep the same order at 4.5:1 or better on a card and on the lane, in
/// both schemes.
enum TendInk {
    /// Explanations, meta rows, subtitles: #5C5C5C, 6.7:1 on white and
    /// 6.3:1 on the lane; #B0B0B0, 7.6:1 on #1F1F1F and 7.1:1 on #252526.
    static let secondary = Color.tend(light: 0x5C5C5C, dark: 0xB0B0B0)
    /// Times, counts, hints: #6E6E6E, 5.1:1 on white and 4.8:1 on the
    /// lane; #8E8E8E, 5.0:1 on #1F1F1F and 4.7:1 on #252526.
    static let tertiary = Color.tend(light: 0x6E6E6E, dark: 0x8E8E8E)
}

/// Type roles (`TendType` plus the weights the Mac views use).
enum TendType {
    /// "Home", a screen's title: Mac 17 pt semibold.
    static let screenTitle = Font.title2.weight(.semibold)
    /// A lane's title: Mac 13 pt bold.
    static let laneTitle = Font.headline.weight(.bold)
    /// A card's title: Mac 13 pt semibold.
    static let cardTitle = Font.body.weight(.semibold)
    /// What a card says: Mac body.
    static let body = Font.subheadline
    /// Recommendation, detail: Mac callout.
    static let detail = Font.subheadline
    /// Meta rows, chips, lane subtitles: Mac 11 pt.
    static let meta = Font.footnote
    /// Section titles: Mac 11 pt semibold, upper case, tracked.
    static let section = Font.caption.weight(.semibold)
    /// Pane output, ids.
    static let mono = Font.system(.footnote, design: .monospaced)
}

/// The brand's state colours, for the mark. The app's state colours are
/// the system ones, as on the Mac (`AgentStatus`, `Lane.tint`).
enum TendColor {
    static let waiting = Color(uiColor: UIColor(hex: 0xFFAA2B))
    static let error = Color(uiColor: UIColor(hex: 0xFF5A3C))
    static let done = Color(uiColor: UIColor(hex: 0x3FBF84))
    static let working = Color.tend(light: 0x16181D, dark: 0xF2F3F5)
}

/// Home's three lanes (`FleetLane`), in the Mac app's words and tints.
enum Lane: String, CaseIterable, Identifiable, Hashable, Sendable {
    /// Planned Work, not started; shown left of Your turn only when there
    /// is some (the Mac's `FleetBacklogColumn`).
    case backlog
    case yourTurn, inProgress, finished

    /// The three lanes every Home shows.
    static let board: [Lane] = [.yourTurn, .inProgress, .finished]

    var id: String { rawValue }

    var title: String {
        switch self {
        case .backlog: String(localized: "Backlog")
        case .yourTurn: String(localized: "Your turn")
        case .inProgress: String(localized: "In progress")
        case .finished: String(localized: "Finished")
        }
    }

    var subtitle: String {
        switch self {
        case .backlog: String(localized: "Planned, not started")
        case .yourTurn: String(localized: "Decide or review")
        case .inProgress: String(localized: "Agents are on it")
        case .finished: String(localized: "Ready for the next instruction")
        }
    }

    var empty: String {
        switch self {
        case .backlog: String(localized: "Nothing planned.")
        case .yourTurn: String(localized: "Nothing needs you right now.")
        case .inProgress: String(localized: "No agent is working.")
        case .finished: String(localized: "Nothing finished yet.")
        }
    }

    var tint: Color {
        switch self {
        case .backlog: TendInk.secondary
        case .yourTurn: TendPalette.orange
        case .inProgress: TendPalette.blue
        case .finished: TendPalette.green
        }
    }
}

/// An agent's status as Tend for Mac shows it (`AgentStatus.swift`).
enum AgentStatus: Hashable, Sendable {
    case error, needsInput, asked, limited, working, idle, done, other

    init(state: String, awaitingReply: Bool = false) {
        switch state {
        case "error": self = .error
        case "waiting_input": self = .needsInput
        case "waiting_choice": self = .asked
        case "rate_limited", "limited": self = .limited
        case "working", "starting": self = .working
        case "idle": self = awaitingReply ? .asked : .idle
        case "stopped": self = .done
        default: self = .other
        }
    }

    var color: Color {
        switch self {
        case .error: TendPalette.red
        case .needsInput, .asked: TendPalette.orange
        case .limited: TendPalette.purple
        case .working: TendPalette.blue
        case .idle: TendPalette.mint
        case .done: TendPalette.green
        case .other: TendInk.secondary
        }
    }

    var symbol: String {
        switch self {
        case .error: "exclamationmark.octagon.fill"
        case .needsInput: "hand.raised.fill"
        case .asked: "questionmark.bubble.fill"
        case .limited: "hourglass"
        case .working: "circle.dotted"
        case .idle: "pause.circle"
        case .done: "checkmark.circle.fill"
        case .other: "circle"
        }
    }

    var label: String {
        switch self {
        case .error: String(localized: "Error")
        case .needsInput: String(localized: "Needs input")
        case .asked: String(localized: "Asking")
        case .limited: String(localized: "Limited")
        case .working: String(localized: "Working")
        case .idle: String(localized: "Idle")
        case .done: String(localized: "Done")
        case .other: String(localized: "Unknown")
        }
    }

    var needsAttention: Bool { self == .error || self == .needsInput || self == .asked }
}

/// A status dot with an optional label (`AgentStatusTag`).
struct AgentStatusTag: View {
    let status: AgentStatus
    var showsLabel = true

    var body: some View {
        HStack(spacing: 5) {
            Circle().fill(status.color).frame(width: 8, height: 8)
            if showsLabel {
                Text(status.label).font(TendType.meta.weight(.medium)).foregroundStyle(TendInk.secondary)
            }
        }
        .accessibilityElement(children: .combine)
    }
}

/// A Fleet host's connection state (the Mac host colours).
enum HostState {
    static func color(_ state: String) -> Color {
        switch state {
        case "online": TendPalette.green
        case "connecting": TendPalette.blue
        case "degraded", "version_skew": TendPalette.orange
        case "auth_failed": TendPalette.red
        default: TendInk.secondary
        }
    }

    static func label(_ state: String) -> String {
        switch state {
        case "online": String(localized: "Online")
        case "connecting": String(localized: "Connecting")
        case "degraded": String(localized: "Degraded")
        case "version_skew": String(localized: "Needs update")
        case "auth_failed": String(localized: "Sign-in failed")
        case "disabled": String(localized: "Disabled")
        default: String(localized: "Offline")
        }
    }
}

/// A host's own colour from its alias, as on the Mac (`TendGlyphFace`'s
/// palette, picked by FNV-1a of the alias).
enum HostIdentity {
    private static let palette: [(UInt32, UInt32)] = [
        (0x5F63DB, 0x3E419B), (0x2875AB, 0x18577F), (0x257E80, 0x17595B), (0x9156BB, 0x643782),
        (0xB54F78, 0x813351), (0xAA5C2B, 0x81451E), (0x477F48, 0x2C582E), (0x596FA0, 0x394A73),
    ]

    static func colors(_ alias: String) -> (Color, Color) {
        var hash: UInt32 = 2_166_136_261
        for byte in alias.utf8 {
            hash ^= UInt32(byte)
            hash = hash &* 16_777_619
        }
        let pair = palette[Int(hash % UInt32(palette.count))]
        return (Color(uiColor: UIColor(hex: pair.0)), Color(uiColor: UIColor(hex: pair.1)))
    }
}

/// A host's round badge with its initial (`TendGlyphFace`).
struct HostGlyph: View {
    let alias: String
    var size: CGFloat = 16

    var body: some View {
        let (top, bottom) = HostIdentity.colors(alias)
        Circle()
            .fill(LinearGradient(colors: [top, bottom], startPoint: .top, endPoint: .bottom))
            .frame(width: size, height: size)
            .overlay(
                Text(verbatim: String(alias.prefix(1)).uppercased())
                    .font(.system(size: size * 0.52, weight: .bold, design: .rounded))
                    .foregroundStyle(.white)
            )
            .accessibilityHidden(true)
    }
}

/// A host's square badge with its state dot (`HostIdentityBadge`).
struct HostBadge: View {
    let alias: String
    let local: Bool
    let state: String
    var size: CGFloat = 30
    var showsState = true

    var body: some View {
        let (tint, _) = HostIdentity.colors(alias)
        RoundedRectangle(cornerRadius: size * 0.25, style: .continuous)
            .fill(tint.opacity(0.14))
            .frame(width: size, height: size)
            .overlay(
                Image(systemName: local ? "laptopcomputer" : "server.rack")
                    .font(.system(size: size * 0.46, weight: .medium))
                    .foregroundStyle(tint)
            )
            .overlay(alignment: .bottomTrailing) {
                if showsState {
                    Circle()
                        .fill(HostState.color(state))
                        .frame(width: size * 0.3, height: size * 0.3)
                        .overlay(Circle().strokeBorder(TendSurface.workspace, lineWidth: 1.5))
                        .offset(x: 2, y: 2)
                }
            }
            .accessibilityHidden(true)
    }
}

/// Upper-case section title (`TendSectionTitle`).
struct SectionTitle: View {
    let text: String
    var count: Int?

    var body: some View {
        HStack(spacing: 6) {
            Text(verbatim: text.uppercased())
                .font(TendType.section)
                .tracking(0.4)
                .foregroundStyle(TendInk.secondary)
            if let count {
                Text(verbatim: "\(count)").font(TendType.section).monospacedDigit().foregroundStyle(TendInk.tertiary)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
    }
}

/// A small capsule label (Question, Review, Helper, an evidence chip).
struct Chip: View {
    let text: String
    var color: Color = TendInk.secondary
    var symbol: String?

    var body: some View {
        HStack(spacing: 4) {
            if let symbol { Image(systemName: symbol).imageScale(.small) }
            Text(verbatim: text).lineLimit(1)
        }
        .font(TendType.meta.weight(.medium))
        .padding(.horizontal, 8)
        .padding(.vertical, 3)
        .foregroundStyle(color)
        .background(color.opacity(0.12), in: Capsule())
    }
}

/// A card: the Mac app's work and decision cards (workspace fill, rounded,
/// 1 pt border; a decision card's border is orange).
struct CardBackground: ViewModifier {
    var stroke: Color = TendSurface.border
    var lineWidth: CGFloat = 1

    func body(content: Content) -> some View {
        content
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(TendSurface.workspace, in: RoundedRectangle(cornerRadius: TendSurface.cardRadius, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: TendSurface.cardRadius, style: .continuous)
                    .strokeBorder(stroke, lineWidth: lineWidth)
            )
    }
}

extension View {
    func tendCard(stroke: Color = TendSurface.border, lineWidth: CGFloat = 1) -> some View {
        modifier(CardBackground(stroke: stroke, lineWidth: lineWidth))
    }
}

/// Markdown as the Mac app renders it (`TendMarkdownText`): full syntax,
/// partial on error, with the block breaks `Text` would drop put back.
enum TendMarkdown {
    /// One line of inline Markdown (bold, code, links) for a card's
    /// headline: never block breaks, never the raw markers.
    static func inline(_ markdown: String) -> AttributedString {
        let options = AttributedString.MarkdownParsingOptions(
            interpretedSyntax: .inlineOnlyPreservingWhitespace,
            failurePolicy: .returnPartiallyParsedIfPossible
        )
        return (try? AttributedString(markdown: markdown, options: options)) ?? AttributedString(markdown)
    }

    static func attributed(_ markdown: String) -> AttributedString {
        let options = AttributedString.MarkdownParsingOptions(
            interpretedSyntax: .full,
            failurePolicy: .returnPartiallyParsedIfPossible
        )
        guard let parsed = try? AttributedString(markdown: markdown, options: options) else {
            return AttributedString(markdown)
        }
        var out = AttributedString()
        var lastBlock: Int?
        for run in parsed.runs {
            let components = run.presentationIntent?.components ?? []
            let block = components.first?.identity
            var piece = AttributedString(parsed[run.range])
            if let lastBlock, block != lastBlock {
                let isHeader = components.contains { if case .header = $0.kind { true } else { false } }
                out.append(AttributedString(isHeader ? "\n\n" : "\n"))
            }
            if block != lastBlock {
                for component in components {
                    if case .listItem(let ordinal) = component.kind {
                        let ordered = components.contains { if case .orderedList = $0.kind { true } else { false } }
                        piece = AttributedString(ordered ? "\(ordinal). " : "•  ") + piece
                        break
                    }
                }
            }
            if components.contains(where: { if case .header = $0.kind { true } else { false } }) {
                piece.font = TendType.cardTitle
            }
            if components.contains(where: { if case .codeBlock = $0.kind { true } else { false } }) {
                piece.font = TendType.mono
            }
            out.append(piece)
            lastBlock = block
        }
        return out
    }
}
