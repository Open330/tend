import AppKit
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
    /// A path or id inside a meta line or a dense evidence row. 11pt monospaced.
    static let monoSmall = font(TendDesignTokens.Mac.monoSmall)

    // Symbols. A symbol beside text takes that text's role; these are for
    // symbols that stand alone.

    /// Disclosure chevrons, close marks and separators between path parts,
    /// and the count inside a 15pt badge disc. 9pt semibold.
    static let glyph = font(TendDesignTokens.Mac.glyph)
    /// A row's or the activity bar's own icon. 16pt.
    static let icon = font(TendDesignTokens.Mac.icon)
    /// The symbol over an empty state or a short message. 28pt.
    static let emptyIcon = font(TendDesignTokens.Mac.emptyIcon)
    /// The symbol heading a sheet's finished or first-run state. 40pt.
    static let heroIcon = font(TendDesignTokens.Mac.heroIcon)
    /// The Welcome guide's own mark. 60pt.
    static let displayIcon = font(TendDesignTokens.Mac.displayIcon)

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
    /// A soft-filled control while pressed: a clear step from hover.
    static func controlPressed(_ scheme: ColorScheme) -> Color { TendTheme.token(TendDesignTokens.Mac.inkControlPressed, scheme) }
    static func accentText(_ scheme: ColorScheme) -> Color { TendTheme.token(TendDesignTokens.Mac.inkAccentText, scheme) }
    /// The primary button's fill; white on it is 5:1 or more.
    static func accentFill(_ scheme: ColorScheme) -> Color { TendTheme.token(TendDesignTokens.Mac.accentFill, scheme) }
    /// Pressed: opaque and darker, never a translucent accent.
    static func accentFillPressed(_ scheme: ColorScheme) -> Color { TendTheme.token(TendDesignTokens.Mac.accentFillPressed, scheme) }
    /// Hover: opaque, between the rest and pressed fills.
    static func accentFillHover(_ scheme: ColorScheme) -> Color { TendTheme.token(TendDesignTokens.Mac.accentFillHover, scheme) }
    /// A destructive primary button's fill, hovered, and pressed.
    static func dangerFill(_ scheme: ColorScheme) -> Color { TendTheme.token(TendDesignTokens.Mac.dangerFill, scheme) }
    static func dangerFillHover(_ scheme: ColorScheme) -> Color { TendTheme.token(TendDesignTokens.Mac.dangerFillHover, scheme) }
    static func dangerFillPressed(_ scheme: ColorScheme) -> Color { TendTheme.token(TendDesignTokens.Mac.dangerFillPressed, scheme) }
    /// A destructive secondary label: 4.5:1 on its rest, hover and pressed fills.
    static func dangerText(_ scheme: ColorScheme) -> Color { TendTheme.token(TendDesignTokens.Mac.dangerText, scheme) }

    /// A state's word. On a selected or tinted surface use `label` instead
    /// and keep the hue for the dot.
    static func state(_ hue: TendStateHue, _ scheme: ColorScheme) -> Color {
        TendTheme.token(hue.ink, scheme)
    }
}

extension TendInk {
    /// A token that resolves against the view's own appearance (NSAppearance)
    /// rather than an explicit scheme: for surfaces such as the menu bar
    /// popover, which can follow its own appearance setting.
    static func adaptive(_ token: TendTokenColor) -> Color {
        Color(nsColor: NSColor(name: nil) { appearance in
            let dark = appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
            let hex = dark ? token.dark : token.light
            return NSColor(
                srgbRed: CGFloat((hex >> 16) & 0xFF) / 255,
                green: CGFloat((hex >> 8) & 0xFF) / 255,
                blue: CGFloat(hex & 0xFF) / 255,
                alpha: CGFloat(dark ? token.darkAlpha : token.lightAlpha)
            )
        })
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

/// The leading column's background (the activity bar and the sidebar list):
/// the opaque sidebar surface, the value every ink in the design is checked
/// against. With `TendPreferences.sidebarMaterialKey` on, the system sidebar
/// material instead (blended with what is behind the window, following its
/// active state), except under Reduce Transparency or Increase Contrast.
struct TendSidebarBackground: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.colorSchemeContrast) private var contrast
    @AppStorage(TendPreferences.sidebarMaterialKey) private var usesMaterial = false

    var body: some View {
        if !usesMaterial || reduceTransparency || contrast == .increased {
            TendSurface.sidebar(colorScheme)
        } else {
            TendSidebarMaterial()
        }
    }
}

private struct TendSidebarMaterial: NSViewRepresentable {
    func makeNSView(context: Context) -> NSVisualEffectView {
        let view = NSVisualEffectView()
        view.material = .sidebar
        view.blendingMode = .behindWindow
        view.state = .followsWindowActiveState
        return view
    }

    func updateNSView(_ view: NSVisualEffectView, context: Context) {}
}
