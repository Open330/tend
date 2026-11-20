import AppKit
import SwiftUI

/// The workbench's design tokens. The workbench is laid out like a
/// professional editor (activity bar, side bar, editor groups, status bar),
/// and these values follow the density and neutral surfaces of VS Code's
/// "Modern" themes rather than stock AppKit controls, so every region reads as
/// one tool instead of a stack of system panels.
/// The workbench's type scale. macOS sets `.caption`, `.caption2`, and
/// `.footnote` all at 10pt, a footnote size; when most of a window is set in
/// them, nothing stands out and the GUI reads worse than the CLI it wraps.
/// Content takes its size from these steps instead, and 11pt is the floor.
enum TendType {
    /// What the reader came for: messages, prompts, row titles. 13pt.
    static let body = Font.body
    /// Text that supports body text: subtitles, explanations, row details,
    /// field help. 12pt.
    static let detail = Font.callout
    /// Labels that only annotate: badges, counts, timestamps, IDs, paths in
    /// a header. 11pt.
    static let meta = Font.subheadline
}

enum TendTheme {
    // MARK: Metrics

    /// One Explore/tree row. VS Code lists are 22px; a touch taller keeps the
    /// Korean glyphs from crowding.
    static let rowHeight = CGFloat(TendDesignTokens.Metric.rowHeight)
    static let treeIndent: CGFloat = 12
    static let sectionHeaderHeight = CGFloat(TendDesignTokens.Metric.sectionHeaderHeight)
    static let breadcrumbHeight: CGFloat = 28
    static let statusBarHeight = CGFloat(TendDesignTokens.Metric.statusBarHeight)
    static let activityBarWidth = CGFloat(TendDesignTokens.Metric.activityBarWidth)
    static let controlRadius = CGFloat(TendDesignTokens.Metric.controlRadius)

    /// Cards and grouped content inside an editor: small corners and a
    /// hairline edge rather than a floating material slab.
    static let panelRadius = CGFloat(TendDesignTokens.Metric.panelRadius)

    /// The card fill, following the appearance on its own so views that
    /// don't read `colorScheme` can use it as a plain `ShapeStyle`.
    static let panelFill = Color(nsColor: NSColor(name: nil) { appearance in
        let tokens = TendDesignTokens.lane
        return appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            ? nsColor(tokens.dark, alpha: tokens.darkAlpha)
            : nsColor(tokens.light, alpha: tokens.lightAlpha)
    })

    // MARK: Surfaces

    static func activityBar(_ scheme: ColorScheme) -> Color {
        token(TendDesignTokens.activityBar, scheme)
    }

    static func sideBar(_ scheme: ColorScheme) -> Color {
        token(TendDesignTokens.sideBar, scheme)
    }

    static func editor(_ scheme: ColorScheme) -> Color {
        token(TendDesignTokens.editor, scheme)
    }

    /// Tab strip and inactive tabs: the side bar's tone, so the active tab
    /// visibly joins the editor below it.
    static func tabStrip(_ scheme: ColorScheme) -> Color {
        sideBar(scheme)
    }

    /// Panels inside an editor (inspector cards, the live-pane header).
    static func panel(_ scheme: ColorScheme) -> Color {
        token(TendDesignTokens.panel, scheme)
    }

    static func border(_ scheme: ColorScheme) -> Color {
        token(TendDesignTokens.border, scheme)
    }

    static func inputBackground(_ scheme: ColorScheme) -> Color {
        token(TendDesignTokens.inputBackground, scheme)
    }

    static func inputBorder(_ scheme: ColorScheme) -> Color {
        token(TendDesignTokens.inputBorder, scheme)
    }

    static func hover(_ scheme: ColorScheme) -> Color {
        token(TendDesignTokens.hover, scheme)
    }

    static func pressed(_ scheme: ColorScheme) -> Color {
        token(TendDesignTokens.pressed, scheme)
    }

    /// The active row in a list that has focus: the accent, muted.
    static func selection(_ scheme: ColorScheme) -> Color {
        Color.accentColor.opacity(scheme == .dark ? 0.30 : 0.16)
    }

    static func segmentTrack(_ scheme: ColorScheme) -> Color {
        token(TendDesignTokens.segmentTrack, scheme)
    }

    static func segmentChip(_ scheme: ColorScheme) -> Color {
        token(TendDesignTokens.segmentChip, scheme)
    }

    /// A token from apps/shared/Design/tokens.json (TendDesignTokens.swift,
    /// generated; the dashboard reads the same file).
    static func token(_ token: TendTokenColor, _ scheme: ColorScheme) -> Color {
        scheme == .dark ? rgb(token.dark).opacity(token.darkAlpha) : rgb(token.light).opacity(token.lightAlpha)
    }

    private static func nsColor(_ hex: UInt32, alpha: Double) -> NSColor {
        NSColor(
            srgbRed: CGFloat((hex >> 16) & 0xFF) / 255,
            green: CGFloat((hex >> 8) & 0xFF) / 255,
            blue: CGFloat(hex & 0xFF) / 255,
            alpha: CGFloat(alpha)
        )
    }

    private static func rgb(_ hex: UInt32) -> Color {
        Color(
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255
        )
    }
}

extension View {
    /// An editor group takes focus to route ⌘-shortcuts, not to be edited;
    /// the system's focus ring around the whole group only adds noise (the
    /// active tab already shows which group has focus).
    @ViewBuilder
    func tendFocusEffectDisabled() -> some View {
        if #available(macOS 14.0, *) {
            focusEffectDisabled()
        } else {
            self
        }
    }
}

// MARK: - Buttons

/// A square icon button for action bars (side bar titles, tab strip, editor
/// headers): no chrome at rest, a soft fill on hover like VS Code's actions.
struct TendIconButtonStyle: ButtonStyle {
    var size: CGFloat = 22

    func makeBody(configuration: Configuration) -> some View {
        TendIconButtonBody(configuration: configuration, size: size)
    }
}

private struct TendIconButtonBody: View {
    let configuration: ButtonStyle.Configuration
    let size: CGFloat
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.colorScheme) private var colorScheme
    @State private var hovering = false

    var body: some View {
        configuration.label
            .labelStyle(.iconOnly)
            .font(TendType.body)
            .frame(width: size, height: size)
            .contentShape(Rectangle())
            .background(
                RoundedRectangle(cornerRadius: TendTheme.controlRadius + 1, style: .continuous)
                    .fill(fill)
            )
            .foregroundStyle(isEnabled ? Color.primary.opacity(0.78) : Color.secondary.opacity(0.45))
            .onHover { hovering = $0 && isEnabled }
    }

    private var fill: Color {
        guard isEnabled else { return .clear }
        if configuration.isPressed { return TendTheme.pressed(colorScheme) }
        return hovering ? TendTheme.hover(colorScheme) : .clear
    }
}

extension ButtonStyle where Self == TendIconButtonStyle {
    static var tendIcon: TendIconButtonStyle { TendIconButtonStyle() }
    static func tendIcon(size: CGFloat) -> TendIconButtonStyle { TendIconButtonStyle(size: size) }
}

/// The workbench's text buttons, sized by `controlSize` (22pt small, 26pt
/// regular), on the Mac design tokens (docs/proposals/MAC_DESIGN.md):
/// - `primary`: the accent fill (white on it is 5:1 or more; a destructive
///   one the danger fill), one per surface, for the action that moves work
///   forward (Send, Start, Click to Type). Pressed is a darker opaque fill.
/// - `secondary`: a soft fill without an outline (an outline again under
///   Increase Contrast), the everyday action (Details, Open in Shell).
/// - `ghost`: no chrome until hovered, for actions inside dense rows.
struct TendButtonStyle: ButtonStyle {
    enum Kind { case primary, secondary, ghost }
    var kind: Kind = .secondary

    func makeBody(configuration: Configuration) -> some View {
        TendButtonBody(configuration: configuration, kind: kind)
    }
}

private struct TendButtonBody: View {
    let configuration: ButtonStyle.Configuration
    let kind: TendButtonStyle.Kind
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.controlSize) private var controlSize
    @Environment(\.colorSchemeContrast) private var contrast
    @State private var hovering = false

    private var compact: Bool { controlSize == .small || controlSize == .mini }

    var body: some View {
        configuration.label
            .labelStyle(TendButtonLabelStyle())
            .font((compact ? TendType.detail : TendType.body).weight(kind == .primary ? .semibold : .medium))
            .lineLimit(1)
            .padding(.horizontal, compact ? 8 : 10)
            .frame(minHeight: compact ? 22 : 26)
            .background(
                RoundedRectangle(cornerRadius: TendRadius.control, style: .continuous)
                    .fill(fill)
            )
            .overlay {
                RoundedRectangle(cornerRadius: TendRadius.control, style: .continuous)
                    .strokeBorder(stroke, lineWidth: 1)
            }
            .foregroundStyle(foreground)
            .opacity(isEnabled ? 1 : 0.45)
            .contentShape(Rectangle())
            .onHover { hovering = $0 && isEnabled }
            .animation(.easeOut(duration: 0.12), value: hovering)
    }

    private var destructive: Bool { configuration.role == .destructive }

    private var fill: Color {
        switch kind {
        case .primary:
            // Opaque at rest and pressed: white text keeps its contrast.
            if destructive {
                if configuration.isPressed { return TendInk.dangerFillPressed(colorScheme) }
                return hovering ? TendInk.dangerFillHover(colorScheme) : TendInk.dangerFill(colorScheme)
            }
            if configuration.isPressed { return TendInk.accentFillPressed(colorScheme) }
            return hovering ? TendInk.accentFillHover(colorScheme) : TendInk.accentFill(colorScheme)
        case .secondary:
            if configuration.isPressed { return TendInk.controlPressed(colorScheme) }
            return hovering ? TendInk.selection(colorScheme) : TendInk.control(colorScheme)
        case .ghost:
            if configuration.isPressed { return TendTheme.pressed(colorScheme) }
            return hovering ? TendTheme.hover(colorScheme) : .clear
        }
    }

    private var stroke: Color {
        switch kind {
        case .primary: .clear
        // No outline, except where the person asked for more contrast.
        case .secondary: contrast == .increased ? TendTheme.inputBorder(colorScheme) : .clear
        case .ghost: .clear
        }
    }

    private var foreground: Color {
        switch kind {
        case .primary: .white
        case .secondary:
            destructive ? TendInk.dangerText(colorScheme) : TendInk.label(colorScheme)
        case .ghost:
            // Unchanged in this pass.
            destructive ? .red : Color.primary.opacity(0.85)
        }
    }
}

/// Icon and title a little closer than `Label`'s default, the icon a step
/// smaller than the text so it doesn't outweigh it.
struct TendButtonLabelStyle: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 5) {
            configuration.icon
                .font(TendType.label)
                .imageScale(.small)
            configuration.title
        }
    }
}

extension ButtonStyle where Self == TendButtonStyle {
    static var tendPrimary: TendButtonStyle { TendButtonStyle(kind: .primary) }
    static var tendSecondary: TendButtonStyle { TendButtonStyle(kind: .secondary) }
    static var tendGhost: TendButtonStyle { TendButtonStyle(kind: .ghost) }
}

// MARK: - Segmented control

/// A flat segmented control: the selected segment is a raised chip inside a
/// sunken track, in place of AppKit's bezeled `.segmented` picker.
struct TendSegmented<Value: Hashable>: View {
    @Binding var selection: Value
    let options: [Value]
    let label: (Value) -> Text
    /// A product logo before an option's label, for an agent picker.
    var logo: ((Value) -> TendAgentLogoAsset?)? = nil
    @Environment(\.colorScheme) private var colorScheme
    @Namespace private var namespace

    var body: some View {
        HStack(spacing: 2) {
            ForEach(options, id: \.self) { option in
                let active = option == selection
                Button {
                    withAnimation(.easeOut(duration: 0.15)) { selection = option }
                } label: {
                    HStack(spacing: 4) {
                        if let asset = logo?(option) {
                            TendAgentLogo(asset: asset, fallbackSymbol: "terminal", size: 13)
                        }
                        label(option)
                    }
                        .font(TendType.detail.weight(active ? .semibold : .medium))
                        .lineLimit(1)
                        .foregroundStyle(active ? Color.primary : Color.secondary)
                        .padding(.horizontal, 10)
                        .frame(minHeight: 20)
                        .background {
                            if active {
                                RoundedRectangle(cornerRadius: 4, style: .continuous)
                                    .fill(TendTheme.segmentChip(colorScheme))
                                    .shadow(color: .black.opacity(colorScheme == .dark ? 0.35 : 0.1), radius: 1, y: 0.5)
                                    .matchedGeometryEffect(id: "chip", in: namespace)
                            }
                        }
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
        }
        .padding(2)
        .background(
            RoundedRectangle(cornerRadius: 6, style: .continuous)
                .fill(TendTheme.segmentTrack(colorScheme))
        )
        .fixedSize()
    }
}

// MARK: - Text field

extension View {
    /// Flat field chrome for a `.plain` TextField: the input background, a
    /// hairline border, and 5pt corners, replacing `.roundedBorder`.
    func tendFieldChrome(focused: Bool = false) -> some View {
        modifier(TendFieldChrome(focused: focused))
    }
}

private struct TendFieldChrome: ViewModifier {
    let focused: Bool
    @Environment(\.colorScheme) private var colorScheme

    func body(content: Content) -> some View {
        content
            .textFieldStyle(.plain)
            .font(TendType.body)
            .padding(.horizontal, 8)
            .padding(.vertical, 5)
            .background(
                RoundedRectangle(cornerRadius: 5, style: .continuous)
                    .fill(TendTheme.inputBackground(colorScheme))
            )
            .overlay(
                RoundedRectangle(cornerRadius: 5, style: .continuous)
                    .strokeBorder(focused ? Color.accentColor : TendTheme.inputBorder(colorScheme), lineWidth: 1)
            )
    }
}

// MARK: - Section header

/// VS Code's side bar title: small, uppercase, tracked, with the view's
/// actions on the trailing edge.
struct TendSectionTitle<Actions: View>: View {
    let title: String
    @ViewBuilder var actions: Actions

    var body: some View {
        HStack(spacing: 2) {
            Text(verbatim: title.uppercased())
                .font(TendType.label.weight(.semibold))
                .tracking(0.4)
                .foregroundStyle(.secondary)
                .lineLimit(1)
            Spacer(minLength: 6)
            actions
        }
        .padding(.leading, 14)
        .padding(.trailing, 8)
        .frame(height: TendTheme.sectionHeaderHeight)
    }
}

// MARK: - Text tabs

/// Underlined text tabs, the way VS Code switches panel views (Problems,
/// Output, Terminal): used instead of a segmented control inside editors.
struct TendTextTabs<Value: Hashable & Identifiable>: View {
    let items: [Value]
    @Binding var selection: Value
    let title: (Value) -> LocalizedStringKey

    var body: some View {
        HStack(spacing: 14) {
            ForEach(items) { item in
                let active = item == selection
                Button {
                    selection = item
                } label: {
                    Text(title(item))
                        .font(TendType.meta.weight(active ? .semibold : .regular))
                        .textCase(.uppercase)
                        .tracking(0.3)
                        .foregroundStyle(active ? Color.primary : Color.secondary)
                        .frame(maxHeight: .infinity)
                        .overlay(alignment: .bottom) {
                            Rectangle()
                                .fill(active ? Color.accentColor : Color.clear)
                                .frame(height: 1.5)
                        }
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
        }
    }
}

// MARK: - Search field

/// The side bar's filter box: a flat, bordered 26pt field whose border takes
/// the accent while it has focus.
struct TendFilterField<Trailing: View>: View {
    let prompt: String
    @Binding var text: String
    var focused: FocusState<Bool>.Binding
    @ViewBuilder var trailing: Trailing
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        HStack(spacing: 5) {
            Image(systemName: "magnifyingglass")
                .font(TendType.meta)
                .foregroundStyle(.secondary)
            TextField(prompt, text: $text)
                .textFieldStyle(.plain)
                .font(TendType.body)
                .focused(focused)
            if !text.isEmpty {
                Button {
                    text = ""
                } label: {
                    Image(systemName: "xmark.circle.fill")
                }
                .buttonStyle(.tendIcon(size: 18))
                .foregroundStyle(.secondary)
                .help("Clear")
            }
            trailing
        }
        .padding(.leading, 7)
        .padding(.trailing, 3)
        .frame(height: 26)
        .background(
            RoundedRectangle(cornerRadius: TendTheme.controlRadius, style: .continuous)
                .fill(TendTheme.inputBackground(colorScheme))
        )
        .overlay(
            RoundedRectangle(cornerRadius: TendTheme.controlRadius, style: .continuous)
                .strokeBorder(
                    focused.wrappedValue ? Color.accentColor : TendTheme.inputBorder(colorScheme),
                    lineWidth: 1
                )
        )
    }
}
