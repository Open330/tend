import SwiftUI

// Tend for Mac's design system (docs/proposals/MAC_DESIGN.md): type roles,
// spacing, radii, surfaces and inks from TendDesignTokens.Mac, which
// scripts/design-tokens.mjs generates from the `mac` section of
// apps/shared/Design/tokens.json. Mac only: the web and iOS keep the shared
// values. Views move onto these roles one PR at a time.

extension TendType {
    /// The content's title: Home, an agent's name. 20pt semibold.
    static let page = font(TendDesignTokens.Mac.page)
    /// What a card or row is about. 14pt semibold.
    static let cardTitle = font(TendDesignTokens.Mac.cardTitle)
    /// Lane headers and sheet sections. 13pt semibold.
    static let section = font(TendDesignTokens.Mac.section)
    /// Short labels only: a kind, a status word, a stage, a badge. 11pt medium.
    static let label = font(TendDesignTokens.Mac.label)
    /// Who, where and when, digits aligned. 11pt.
    static let tabularMeta = font(TendDesignTokens.Mac.meta)
    /// Commands, paths and pane text. 12pt monospaced.
    static let mono = font(TendDesignTokens.Mac.mono)

    static func font(_ role: TendDesignTokens.Mac.TypeRole) -> Font {
        let font = Font.system(size: role.size, weight: weight(role.weight), design: role.monospaced ? .monospaced : .default)
        return role.tabularDigits ? font.monospacedDigit() : font
    }

    private static func weight(_ name: String) -> Font.Weight {
        switch name {
        case "semibold": .semibold
        case "medium": .medium
        case "bold": .bold
        default: .regular
        }
    }
}

/// The 4-point spacing scale. Nothing between the steps.
enum TendSpace {
    static let xxs = CGFloat(TendDesignTokens.Mac.Space.xxs)
    static let xs = CGFloat(TendDesignTokens.Mac.Space.xs)
    static let s = CGFloat(TendDesignTokens.Mac.Space.s)
    static let m = CGFloat(TendDesignTokens.Mac.Space.m)
    static let l = CGFloat(TendDesignTokens.Mac.Space.l)
    static let xl = CGFloat(TendDesignTokens.Mac.Space.xl)
    static let xxl = CGFloat(TendDesignTokens.Mac.Space.xxl)
    static let xxxl = CGFloat(TendDesignTokens.Mac.Space.xxxl)
}

/// Corner radii, drawn with continuous corners.
enum TendRadius {
    static let control = CGFloat(TendDesignTokens.Mac.Radius.control)
    static let group = CGFloat(TendDesignTokens.Mac.Radius.group)
    static let card = CGFloat(TendDesignTokens.Mac.Radius.card)
    static let lane = CGFloat(TendDesignTokens.Mac.Radius.lane)
}

/// The four surfaces, each one step from the next. Content and cards stay
/// opaque; the sidebar's value is the system material's opaque fallback.
enum TendSurface {
    static func sidebar(_ scheme: ColorScheme) -> Color { TendTheme.token(TendDesignTokens.Mac.surfaceSidebar, scheme) }
    static func content(_ scheme: ColorScheme) -> Color { TendTheme.token(TendDesignTokens.Mac.surfaceContent, scheme) }
    static func lane(_ scheme: ColorScheme) -> Color { TendTheme.token(TendDesignTokens.Mac.surfaceLane, scheme) }
    static func card(_ scheme: ColorScheme) -> Color { TendTheme.token(TendDesignTokens.Mac.surfaceCard, scheme) }
}

/// Inks. Text that informs is `label` or `secondary` (4.5:1 or more on every
/// surface); `tertiary` is for icons, separators and placeholders only.
enum TendInk {
    static func label(_ scheme: ColorScheme) -> Color { TendTheme.token(TendDesignTokens.Mac.inkLabel, scheme) }
    static func secondary(_ scheme: ColorScheme) -> Color { TendTheme.token(TendDesignTokens.Mac.inkSecondary, scheme) }
    static func tertiary(_ scheme: ColorScheme) -> Color { TendTheme.token(TendDesignTokens.Mac.inkTertiary, scheme) }
    static func hairline(_ scheme: ColorScheme) -> Color { TendTheme.token(TendDesignTokens.Mac.inkHairline, scheme) }
    static func control(_ scheme: ColorScheme) -> Color { TendTheme.token(TendDesignTokens.Mac.inkControl, scheme) }
    static func track(_ scheme: ColorScheme) -> Color { TendTheme.token(TendDesignTokens.Mac.inkTrack, scheme) }
    static func selection(_ scheme: ColorScheme) -> Color { TendTheme.token(TendDesignTokens.Mac.inkSelection, scheme) }
    static func accentText(_ scheme: ColorScheme) -> Color { TendTheme.token(TendDesignTokens.Mac.inkAccentText, scheme) }
    /// The primary button's fill; white on it is 5:1 or more.
    static func accentFill(_ scheme: ColorScheme) -> Color { TendTheme.token(TendDesignTokens.Mac.accentFill, scheme) }
    /// Pressed: opaque and darker, never a translucent accent.
    static func accentFillPressed(_ scheme: ColorScheme) -> Color { TendTheme.token(TendDesignTokens.Mac.accentFillPressed, scheme) }

    /// A state's word. On a selected or tinted surface use `label` instead
    /// and keep the hue for the dot.
    static func state(_ hue: TendStateHue, _ scheme: ColorScheme) -> Color {
        TendTheme.token(hue.ink, scheme)
    }
}

/// The state hues whose words have their own ink.
enum TendStateHue: CaseIterable, Sendable {
    case orange, green, blue, red, purple

    var ink: TendTokenColor {
        switch self {
        case .orange: TendDesignTokens.Mac.stateInkOrange
        case .green: TendDesignTokens.Mac.stateInkGreen
        case .blue: TendDesignTokens.Mac.stateInkBlue
        case .red: TendDesignTokens.Mac.stateInkRed
        case .purple: TendDesignTokens.Mac.stateInkPurple
        }
    }
}
