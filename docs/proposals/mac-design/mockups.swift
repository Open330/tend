// Tend for Mac design proposal (docs/proposals/MAC_DESIGN.md): the proposed
// system as SwiftUI, drawn to PNG with ImageRenderer. Proposal code only: it
// never ships. The content is the demo fixture fleet's (docs/demo-setup.sh).
//
//   swift docs/proposals/mac-design/mockups.swift docs/proposals/mac-design
//
// No window, no app: ImageRenderer draws off-screen in this process. It does
// not draw materials or scroll views, so the sidebar material is shown as its
// opaque fallback and overflow is drawn as a clipped column with a fade.
import AppKit
import SwiftUI

// MARK: - Language

nonisolated(unsafe) var korean = false
/// UI chrome in the run's language; agent content stays as the agent wrote it.
func L(_ en: String, _ ko: String) -> String { korean ? ko : en }

// MARK: - The system

/// Spacing: a 4-point scale. Nothing between the steps.
enum Space {
    static let xxs: CGFloat = 2, xs: CGFloat = 4, s: CGFloat = 8, m: CGFloat = 12
    static let l: CGFloat = 16, xl: CGFloat = 20, xxl: CGFloat = 24, xxxl: CGFloat = 32
}

enum Radius {
    static let control: CGFloat = 6, group: CGFloat = 8, card: CGFloat = 10, lane: CGFloat = 12
}

/// The type ramp: eight roles, three weights. 13pt is the floor for anything
/// a decision rests on; 11pt only for short labels and annotations.
enum Ramp {
    static let page = Font.system(size: 20, weight: .semibold)       // the content's title
    static let section = Font.system(size: 13, weight: .semibold)    // lane and group headers
    static let cardTitle = Font.system(size: 14, weight: .semibold)  // what a card is about
    static let body = Font.system(size: 13)                          // decision text, options, answers
    static let secondary = Font.system(size: 12)                     // detail under a title
    static let label = Font.system(size: 11, weight: .medium)        // kind labels, status words, badges
    static let meta = Font.system(size: 11).monospacedDigit()        // who, where, when
    static let mono = Font.system(size: 12, design: .monospaced)     // commands, paths, pane text
}

struct Palette {
    let scheme: ColorScheme
    var dark: Bool { scheme == .dark }
    func c(_ light: UInt32, _ dark: UInt32, _ a: Double = 1) -> Color {
        let hex = self.dark ? dark : light
        return Color(.sRGB, red: Double((hex >> 16) & 0xFF) / 255, green: Double((hex >> 8) & 0xFF) / 255,
                     blue: Double(hex & 0xFF) / 255, opacity: a)
    }
    private func ink(_ light: Double, _ dark: Double) -> Color {
        self.dark ? Color.white.opacity(dark) : Color.black.opacity(light)
    }
    // Four surfaces, each one step from the next.
    var sidebar: Color { c(0xEFEFF2, 0x232326) }       // the sidebar material's opaque fallback
    var content: Color { c(0xFFFFFF, 0x1C1C1E) }       // the window's content area
    var lane: Color { c(0xF4F4F6, 0x232325) }          // a lane: a quiet well, no border
    var card: Color { c(0xFFFFFF, 0x2C2C2F) }          // a card: fill and a hairline
    var hairline: Color { ink(0.08, 0.08) }
    var control: Color { ink(0.05, 0.08) }
    var controlBorder: Color { ink(0.12, 0.12) }
    var track: Color { ink(0.16, 0.20) }               // an empty stage segment: visible, 1.6:1
    var selection: Color { ink(0.08, 0.11) }
    // Text levels. Tertiary is for icons, separators and placeholders only.
    var label: Color { ink(0.86, 0.88) }
    var secondary: Color { ink(0.55, 0.58) }
    var tertiary: Color { ink(0.45, 0.42) }
    // The accent fill carries white text at 5.4:1 in both appearances.
    var accent: Color { c(0x0066D6, 0x0A6CDA) }
    var accentText: Color { c(0x0066D6, 0x4DA3FF) }
    // State hues: dots, bars, segments.
    var orange: Color { c(0xF08A24, 0xFF9F2E) }
    var blue: Color { c(0x0A84FF, 0x409CFF) }
    var green: Color { c(0x2DB55D, 0x32D35F) }
    var red: Color { c(0xE5484D, 0xFF5A5F) }
    var purple: Color { c(0xA94FD8, 0xC77DFF) }
    var waiting: Color { track }
    /// A state's word: the hue in dark; a deeper ink in light so the word
    /// reaches 4.5:1 on a card, a lane and the sidebar.
    func text(_ state: Color) -> Color {
        guard !dark else { return state }
        switch state {
        case orange: return c(0xA85400, 0)
        case green: return c(0x16743A, 0)
        case blue: return c(0x0066D6, 0)
        case red: return c(0xC8302E, 0)
        case purple: return c(0x8E3FC0, 0)
        default: return state
        }
    }
    var shadow: Color { dark ? .clear : Color.black.opacity(0.06) }
}

private struct PaletteKey: EnvironmentKey { static let defaultValue = Palette(scheme: .light) }
extension EnvironmentValues {
    var palette: Palette { get { self[PaletteKey.self] } set { self[PaletteKey.self] = newValue } }
}

// MARK: - Components

/// A card: one container. Fill and a hairline lift it off the lane; a card
/// waiting for the person carries a 3-point bar at its leading edge (and its
/// kind label says so in words), never a tinted border or fill.
struct Card<Content: View>: View {
    @Environment(\.palette) var p
    var state: Color? = nil
    @ViewBuilder var content: Content
    var body: some View {
        VStack(alignment: .leading, spacing: Space.s) { content }
            .padding(Space.m)
            .padding(.leading, state == nil ? 0 : 2)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(p.card, in: RoundedRectangle(cornerRadius: Radius.card, style: .continuous))
            .overlay(alignment: .leading) {
                if let state {
                    Capsule().fill(state).frame(width: 3).padding(.vertical, Space.m).padding(.leading, 5)
                }
            }
            .overlay(RoundedRectangle(cornerRadius: Radius.card, style: .continuous).strokeBorder(p.hairline))
            .shadow(color: p.shadow, radius: 1.5, y: 1)
    }
}

/// Kind, then time: the card's first line.
struct CardHead: View {
    @Environment(\.palette) var p
    let symbol: String
    let kind: String
    var tint: Color? = nil
    let time: String
    var body: some View {
        HStack(spacing: Space.xs + 1) {
            Image(systemName: symbol).font(.system(size: 11, weight: .semibold)).foregroundStyle(tint.map(p.text) ?? p.secondary)
            Text(kind).font(Ramp.label).foregroundStyle(tint.map(p.text) ?? p.secondary)
            Spacer(minLength: Space.s)
            Text(time).font(Ramp.meta).foregroundStyle(p.secondary)
        }
    }
}

/// The product mark, the host disc and the project: who and where.
struct MetaLine: View {
    @Environment(\.palette) var p
    let agent: String
    let host: String
    let project: String
    var helpers: Int = 0
    var body: some View {
        HStack(spacing: Space.xs + 2) {
            Image(systemName: agent == "codex" ? "chevron.left.forwardslash.chevron.right" : "asterisk")
                .font(.system(size: 9, weight: .bold))
                .foregroundStyle(agent == "codex" ? p.secondary : Color(red: 0.85, green: 0.47, blue: 0.34))
            HostDisc(host: host)
            Text(host).foregroundStyle(p.secondary)
            Text("·").foregroundStyle(p.tertiary)
            Text(project).foregroundStyle(p.secondary)
            if helpers > 0 { Text("+\(helpers)").foregroundStyle(p.secondary) }
        }
        .font(Ramp.meta)
        .lineLimit(1)
    }
}

/// Today's identity disc: a gradient by the alias, kept as is.
struct HostDisc: View {
    let host: String
    var body: some View {
        let gradients: [(UInt32, UInt32)] = [(0x5F63DB, 0x3E419B), (0x2875AB, 0x18577F), (0x9156BB, 0x643782)]
        let g = gradients[host.count % gradients.count]
        func col(_ h: UInt32) -> Color { Color(.sRGB, red: Double((h >> 16) & 0xFF) / 255, green: Double((h >> 8) & 0xFF) / 255, blue: Double(h & 0xFF) / 255) }
        return Circle().fill(LinearGradient(colors: [col(g.0), col(g.1)], startPoint: .top, endPoint: .bottom))
            .frame(width: 12, height: 12)
            .overlay(Image(systemName: "laptopcomputer").font(.system(size: 6, weight: .bold)).foregroundStyle(.white))
    }
}

/// Status: a dot and a word. On a selected row the word is neutral and the
/// dot keeps the colour, so the word never sits on a tinted fill.
struct StatusTag: View {
    @Environment(\.palette) var p
    let color: Color
    let text: String
    var selected = false
    var body: some View {
        HStack(spacing: Space.xs + 1) {
            Circle().fill(color).frame(width: 6, height: 6)
            Text(text).font(Ramp.label).foregroundStyle(selected ? p.label : p.text(color))
        }
    }
}

enum StageState { case done, running, needsYou, waiting }

/// The stage bar as today: one 4-point segment per stage in its state, the
/// current stage's name and n/N under it. The active segment also differs in
/// shape (a notch at its end), so colour is not the only cue.
struct StageBar: View {
    @Environment(\.palette) var p
    let stages: [StageState]
    let current: String
    var body: some View {
        VStack(alignment: .leading, spacing: Space.xs + 1) {
            HStack(spacing: 3) {
                ForEach(Array(stages.enumerated()), id: \.offset) { _, state in
                    Capsule().fill(color(state)).frame(height: 4)
                        .overlay(alignment: .trailing) {
                            if state == .running || state == .needsYou {
                                Circle().fill(p.card).frame(width: 2, height: 2).padding(.trailing, 2)
                            }
                        }
                }
            }
            HStack {
                Text(current).font(Ramp.label).foregroundStyle(p.secondary)
                Spacer()
                if stages.count > 1 {
                    Text("\(stages.filter { $0 == .done }.count)/\(stages.count)").font(Ramp.meta).foregroundStyle(p.secondary)
                }
            }
        }
    }
    func color(_ s: StageState) -> Color {
        switch s { case .done: p.green; case .running: p.blue; case .needsYou: p.orange; case .waiting: p.waiting }
    }
}

/// Numbered options in one grouped list.
struct OptionList: View {
    @Environment(\.palette) var p
    let options: [String]
    var body: some View {
        VStack(spacing: 0) {
            ForEach(Array(options.enumerated()), id: \.offset) { i, option in
                if i > 0 { Rectangle().fill(p.hairline).frame(height: 1).padding(.leading, 34) }
                HStack(alignment: .firstTextBaseline, spacing: Space.s) {
                    Text("\(i + 1)").font(.system(size: 11, weight: .semibold).monospacedDigit())
                        .foregroundStyle(p.secondary)
                        .frame(width: 18, height: 18)
                        .background(p.control, in: RoundedRectangle(cornerRadius: 5))
                    Text(option).font(Ramp.body).foregroundStyle(p.label).fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, Space.s).padding(.vertical, Space.s - 1)
            }
        }
        .background(p.control.opacity(0.6), in: RoundedRectangle(cornerRadius: Radius.group))
    }
}

/// What tend checked first, then the asker's own recommendation, as today.
struct Evidence: View {
    @Environment(\.palette) var p
    let check: String
    let recommendation: String
    var body: some View {
        VStack(alignment: .leading, spacing: Space.xs + 1) {
            HStack(spacing: Space.xs + 1) {
                Image(systemName: "checkmark.seal.fill").font(.system(size: 10)).foregroundStyle(p.green)
                Text(L("Checked by tend", "tend가 확인함")).font(Ramp.label).foregroundStyle(p.text(p.green))
                Text(check).font(Ramp.mono).foregroundStyle(p.secondary).lineLimit(1)
            }
            HStack(alignment: .firstTextBaseline, spacing: Space.xs + 1) {
                Image(systemName: "lightbulb").font(.system(size: 10)).foregroundStyle(p.secondary)
                Text(recommendation).font(Ramp.secondary).foregroundStyle(p.secondary).fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

struct PrimaryButton: View {
    @Environment(\.palette) var p
    let title: String
    var symbol: String? = nil
    var body: some View {
        HStack(spacing: Space.xs + 1) {
            if let symbol { Image(systemName: symbol).font(.system(size: 10, weight: .bold)) }
            Text(title).font(.system(size: 13, weight: .medium))
        }
        .foregroundStyle(.white)
        .padding(.horizontal, Space.m).frame(height: 28)
        .background(p.accent, in: RoundedRectangle(cornerRadius: Radius.control + 1, style: .continuous))
    }
}

struct SecondaryButton: View {
    @Environment(\.palette) var p
    let title: String
    var symbol: String? = nil
    var badge: String? = nil
    var compact = false
    var body: some View {
        HStack(spacing: Space.xs + 1) {
            if let symbol { Image(systemName: symbol).font(.system(size: 11, weight: .medium)).foregroundStyle(p.secondary) }
            if !title.isEmpty { Text(title).font(.system(size: compact ? 12 : 13, weight: .medium)).foregroundStyle(p.label) }
            if let badge {
                Text(badge).font(.system(size: 10, weight: .semibold).monospacedDigit()).foregroundStyle(.white)
                    .padding(.horizontal, 5).frame(height: 15).background(p.accent, in: Capsule())
            }
        }
        .padding(.horizontal, Space.s + 2).frame(height: compact ? 24 : 28)
        .background(p.control, in: RoundedRectangle(cornerRadius: Radius.control + 1, style: .continuous))
    }
}

struct PlainButton: View {
    @Environment(\.palette) var p
    let title: String
    var body: some View {
        Text(title).font(.system(size: 12, weight: .medium)).foregroundStyle(p.accentText)
            .padding(.horizontal, Space.xs + 2).frame(height: 24)
    }
}

/// Today's card-layout picker, restyled: a segmented pair of icons.
struct LayoutPicker: View {
    @Environment(\.palette) var p
    var body: some View {
        HStack(spacing: 2) {
            ForEach(["rectangle.grid.1x2", "list.bullet"], id: \.self) { symbol in
                Image(systemName: symbol).font(.system(size: 11)).foregroundStyle(symbol == "rectangle.grid.1x2" ? p.label : p.secondary)
                    .frame(width: 28, height: 22)
                    .background(symbol == "rectangle.grid.1x2" ? p.card : .clear, in: RoundedRectangle(cornerRadius: 5))
                    .shadow(color: symbol == "rectangle.grid.1x2" ? p.shadow : .clear, radius: 1, y: 0.5)
            }
        }
        .padding(3)
        .background(p.control, in: RoundedRectangle(cornerRadius: Radius.control + 1, style: .continuous))
    }
}

// MARK: - Window chrome

struct TrafficLights: View {
    var body: some View {
        HStack(spacing: 8) {
            ForEach([0xFF5F57, 0xFEBC2E, 0x28C840], id: \.self) { hex in
                Circle().fill(Color(.sRGB, red: Double((hex >> 16) & 0xFF) / 255, green: Double((hex >> 8) & 0xFF) / 255, blue: Double(hex & 0xFF) / 255))
                    .frame(width: 12, height: 12)
            }
        }
    }
}

/// The activity bar keeps its places and its numeric badges; it sits on the
/// sidebar surface, and selection is a soft pill instead of a blue rule.
struct ActivityBar: View {
    @Environment(\.palette) var p
    var selected = 0
    var body: some View {
        VStack(spacing: Space.xs) {
            TrafficLights().frame(width: 64, height: 36).padding(.bottom, Space.s)
            ForEach(Array(["house", "sidebar.left", "tray", "sparkles", "terminal", "folder"].enumerated()), id: \.offset) { i, symbol in
                ZStack(alignment: .topTrailing) {
                    Image(systemName: symbol)
                        .font(.system(size: 15))
                        .foregroundStyle(i == selected ? p.label : p.secondary)
                        .frame(width: 36, height: 32)
                        .background(i == selected ? p.selection : .clear, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                    if i == 1 || i == 2 {
                        Text(i == 1 ? "3" : "2").font(.system(size: 9, weight: .bold).monospacedDigit()).foregroundStyle(.white)
                            .frame(minWidth: 14, minHeight: 14).background(p.accent, in: Capsule()).offset(x: 2, y: -1)
                    }
                }
            }
            Spacer()
            Image(systemName: "gearshape").font(.system(size: 15)).foregroundStyle(p.secondary).frame(width: 36, height: 32)
                .padding(.bottom, Space.m)
        }
        .frame(width: 64)
        .frame(maxHeight: .infinity)
        .background(p.sidebar)
    }
}

/// The editor tab strip stays; the selected tab is the content's colour
/// joined to it, and the strip doubles as the title bar.
struct TabStrip: View {
    @Environment(\.palette) var p
    let tabs: [(String, String)]
    var body: some View {
        HStack(spacing: 0) {
            ForEach(Array(tabs.enumerated()), id: \.offset) { i, tab in
                HStack(spacing: Space.s - 2) {
                    Image(systemName: tab.0).font(.system(size: 11)).foregroundStyle(i == 0 ? p.label : p.secondary)
                    Text(tab.1).font(.system(size: 12, weight: i == 0 ? .medium : .regular)).foregroundStyle(i == 0 ? p.label : p.secondary)
                    if i == 0 { Image(systemName: "xmark").font(.system(size: 8, weight: .bold)).foregroundStyle(p.tertiary).padding(.leading, Space.xs) }
                }
                .padding(.horizontal, Space.m).frame(height: 30)
                .background(i == 0 ? p.content : .clear, in: UnevenRoundedRectangle(topLeadingRadius: 7, topTrailingRadius: 7, style: .continuous))
                .padding(.top, 6)
            }
            Spacer()
            HStack(spacing: Space.m + 2) {
                ForEach(["sidebar.right", "magnifyingglass", "plus"], id: \.self) { symbol in
                    Image(systemName: symbol).font(.system(size: 12)).foregroundStyle(p.secondary)
                }
            }.padding(.trailing, Space.l)
        }
        .padding(.leading, Space.s)
        .frame(height: 36)
        .background(p.sidebar)
    }
}

/// The status bar: tendd's state as a dot and a word (a problem keeps its
/// text and colour), the counts in Meta.
struct StatusBar: View {
    @Environment(\.palette) var p
    var body: some View {
        HStack(spacing: Space.l) {
            HStack(spacing: Space.xs + 1) {
                Circle().fill(p.green).frame(width: 6, height: 6)
                Text("tendd").font(Ramp.meta).foregroundStyle(p.secondary)
            }
            Spacer()
            ForEach([("server.rack", L("1 host", "호스트 1")), ("person.2", L("19 agents", "에이전트 19")), ("terminal", L("0 shells", "셸 0"))], id: \.1) { symbol, text in
                HStack(spacing: Space.xs) {
                    Image(systemName: symbol).font(.system(size: 10))
                    Text(text).font(Ramp.meta)
                }.foregroundStyle(p.secondary)
            }
        }
        .padding(.horizontal, Space.m)
        .frame(height: 24)
        .overlay(alignment: .top) { Rectangle().fill(p.hairline).frame(height: 1) }
    }
}

// MARK: - Home

struct LaneHeader: View {
    @Environment(\.palette) var p
    let title: String
    let count: Int
    let color: Color
    var body: some View {
        HStack(spacing: Space.s) {
            Circle().fill(color).frame(width: 7, height: 7)
            Text(title).font(Ramp.section).foregroundStyle(p.label)
            Text("\(count)").font(.system(size: 12, weight: .medium).monospacedDigit()).foregroundStyle(p.secondary)
            Spacer()
        }
        .padding(.horizontal, Space.xs).padding(.bottom, Space.xs)
    }
}

struct GroupHeader: View {
    @Environment(\.palette) var p
    let title: String
    let count: Int
    var body: some View {
        HStack(spacing: Space.xs + 1) {
            Image(systemName: "folder").font(.system(size: 10)).foregroundStyle(p.tertiary)
            Text(title).font(Ramp.label).foregroundStyle(p.secondary)
            Text("\(count)").font(Ramp.meta).foregroundStyle(p.secondary)
        }
        .padding(.horizontal, Space.xs).padding(.top, Space.xs)
    }
}

/// A lane that holds more than fits: clipped with a fade, as a scroll view
/// shows it before scrolling.
struct Overflow: ViewModifier {
    @Environment(\.palette) var p
    func body(content: Content) -> some View {
        content.overlay(alignment: .bottom) {
            LinearGradient(colors: [p.lane.opacity(0), p.lane], startPoint: .top, endPoint: .bottom).frame(height: 36)
        }
        .clipped()
    }
}

struct HomeMock: View {
    @Environment(\.palette) var p
    var empty = false
    var narrow = false
    var body: some View {
        HStack(spacing: 0) {
            ActivityBar(selected: 0)
            Rectangle().fill(p.hairline).frame(width: 1)
            VStack(spacing: 0) {
                TabStrip(tabs: [("house", L("Home", "홈")), ("asterisk", "api · reconciler")])
                toolbar
                HStack(alignment: .top, spacing: narrow ? Space.s : Space.l) {
                    lane(title: L("Your turn", "내 차례"), count: empty ? 0 : 9, color: p.orange,
                         emptyText: L("Nothing waits for you. Questions and results to review appear here.", "기다리는 일이 없습니다. 질문과 검토할 결과가 여기에 나타납니다.")) { yourTurn }
                    lane(title: L("In progress", "진행 중"), count: empty ? 0 : 8, color: p.blue,
                         emptyText: L("No agent is working. Start Work hands a task to one.", "작업 중인 에이전트가 없습니다. 작업 시작으로 에이전트에게 맡기세요.")) { inProgress }
                    lane(title: L("Finished", "완료"), count: empty ? 0 : 5, color: p.green,
                         emptyText: L("Finished work waits here for its next instruction.", "끝난 작업이 다음 지시를 기다리며 여기에 남습니다.")) { finished }
                }
                .padding(.horizontal, narrow ? Space.m : Space.xxl).padding(.bottom, Space.m)
                StatusBar()
            }
            .background(p.content)
        }
    }

    var toolbar: some View {
        HStack(spacing: Space.s) {
            Text(L("Home", "홈")).font(Ramp.page).foregroundStyle(p.label)
            Spacer(minLength: Space.s)
            LayoutPicker()
            SecondaryButton(title: narrow ? "" : L("Review", "검토"), symbol: "checklist", badge: "5")
            SecondaryButton(title: narrow ? "" : L("Filter", "필터"), symbol: "line.3.horizontal.decrease")
            SecondaryButton(title: "1/1", symbol: "circle.fill")
            SecondaryButton(title: "", symbol: "wand.and.stars")
            Rectangle().fill(p.hairline).frame(width: 1, height: 18).padding(.horizontal, Space.xxs)
            PrimaryButton(title: L("Start Work", "작업 시작"), symbol: "play.fill")
        }
        .padding(.horizontal, narrow ? Space.m : Space.xxl)
        .frame(height: 52)
    }

    func lane<C: View>(title: String, count: Int, color: Color, emptyText: String, @ViewBuilder _ content: () -> C) -> some View {
        VStack(alignment: .leading, spacing: Space.s) {
            LaneHeader(title: title, count: count, color: color)
            // An overlay never grows the lane: overflow is clipped, as a
            // scroll view shows it before scrolling.
            Color.clear.frame(maxWidth: .infinity, maxHeight: .infinity).overlay(alignment: .top) {
                if empty {
                    Text(emptyText).font(Ramp.secondary).foregroundStyle(p.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.horizontal, Space.xs)
                        .frame(maxWidth: .infinity, alignment: .leading)
                } else {
                    VStack(alignment: .leading, spacing: Space.s) { content() }
                }
            }
            .clipped()
        }
        .padding(Space.s)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(p.lane, in: RoundedRectangle(cornerRadius: Radius.lane, style: .continuous))
        .modifier(Overflow())
        .clipShape(RoundedRectangle(cornerRadius: Radius.lane, style: .continuous))
    }

    /// A Work card, as today's FleetWorkCard carries it: title, stage bar,
    /// the headline (only when it says more than the state), and one row
    /// with the status, who, where and when.
    func work(_ title: String, _ stages: [StageState], _ stage: String, _ headline: String?, _ status: (Color, String), _ agent: String, _ project: String, waiting: Bool = false) -> some View {
        Card(state: waiting ? p.orange : nil) {
            Text(title).font(Ramp.cardTitle).foregroundStyle(p.label).lineLimit(3).fixedSize(horizontal: false, vertical: true)
            StageBar(stages: stages, current: stage)
            if let headline {
                Text(headline).font(Ramp.body).foregroundStyle(p.label).fixedSize(horizontal: false, vertical: true)
            }
            HStack(spacing: Space.s) {
                StatusTag(color: status.0, text: status.1)
                MetaLine(agent: agent, host: "local", project: project)
                Spacer(minLength: Space.xs)
                Text(L("18 min", "18분")).font(Ramp.meta).foregroundStyle(p.secondary)
            }
        }
    }

    var yourTurnStatus: (Color, String) { (p.orange, L("Your turn", "내 차례")) }
    var workingStatus: (Color, String) { (p.blue, L("Working", "작업 중")) }
    var finishedStatus: (Color, String) { (p.green, L("Finished", "완료")) }

    @ViewBuilder var yourTurn: some View {
        GroupHeader(title: "tend", count: 1)
        work("audit the legacy auth middleware for token handling — flag anything that stores raw bearer tokens at rest", [.needsYou], "codex", "codex permission: shell", yourTurnStatus, "codex", "tend", waiting: true)
        GroupHeader(title: "web-console", count: 3)
        work("fix the SSE reconnect backoff", [.needsYou], "claude", L("Asks you to choose. Open to see the question.", "선택을 기다립니다. 열어서 질문을 보세요."), yourTurnStatus, "claude", "web-console", waiting: true)
        work("regenerate the typescript client from the openapi spec", [.needsYou], "claude", "500 from the codegen service after 3 retries — spec fetch succeeded, generation did not", yourTurnStatus, "claude", "web-console", waiting: true)
        work("bump the RN version and unbreak the notification module", [.needsYou], "codex", "codex permission: shell", yourTurnStatus, "codex", "web-console", waiting: true)
    }

    @ViewBuilder var inProgress: some View {
        GroupHeader(title: "web-console", count: 1)
        work("cache the geo lookup at the edge and add a stale-while-revalidate window", [.running], "codex", nil, workingStatus, "codex", "web-console")
        GroupHeader(title: "infra", count: 2)
        work("make the nightly load idempotent — it double-counts when the retry lands after midnight", [.running], "claude", nil, workingStatus, "claude", "infra")
        work("terraform: split the network module and pin providers", [.running], "codex", nil, workingStatus, "codex", "infra")
        GroupHeader(title: "acme-api", count: 3)
        work("swap the tantivy index build onto a background thread and add a progress channel", [.running], "claude", nil, workingStatus, "claude", "acme-api")
    }

    @ViewBuilder var finished: some View {
        GroupHeader(title: "infra", count: 2)
        work("review PR #482 — focus on the new sorting knob in tend watch and whether the [Workspace, Activity] default should change", [.done], "codex", nil, finishedStatus, "codex", "infra")
        work("backfill the analytics rollups for last week", [.done], "claude", nil, finishedStatus, "claude", "infra")
        GroupHeader(title: "tend", count: 1)
        work("keep an eye on the fleet while the migration lands", [.done], "claude", nil, finishedStatus, "claude", "tend")
        GroupHeader(title: "acme-api", count: 1)
        work("reconcile the proration edge case when a plan changes twice in one period", [.done], "claude", nil, finishedStatus, "claude", "acme-api")
        GroupHeader(title: "web-console", count: 1)
        work("re-record the playwright traces for the timeline view", [.done], "claude", nil, finishedStatus, "claude", "web-console")
    }
}

// MARK: - Explore

struct TreeRow: View {
    @Environment(\.palette) var p
    let depth: Int
    let symbol: String
    let name: String
    var detail: String? = nil
    var state: (Color, String)? = nil
    var expanded: Bool? = nil
    var selected = false
    var body: some View {
        HStack(spacing: Space.s - 2) {
            Group {
                if let expanded {
                    Image(systemName: expanded ? "chevron.down" : "chevron.right").font(.system(size: 9, weight: .semibold)).foregroundStyle(p.tertiary)
                } else { Color.clear }
            }.frame(width: 10)
            Image(systemName: symbol).font(.system(size: 12)).foregroundStyle(p.secondary).frame(width: 16)
            Text(name).font(.system(size: 13, weight: depth == 0 ? .medium : .regular)).foregroundStyle(p.label).lineLimit(1)
                .layoutPriority(1)
            Spacer(minLength: Space.s)
            if let state {
                StatusTag(color: state.0, text: state.1, selected: selected)
            } else if let detail {
                Text(detail).font(Ramp.meta).foregroundStyle(p.secondary)
            }
        }
        .padding(.leading, CGFloat(depth) * 14 + Space.s).padding(.trailing, Space.s)
        .frame(height: 26)
        .background(selected ? p.selection : .clear, in: RoundedRectangle(cornerRadius: Radius.control, style: .continuous))
    }
}

struct SidebarSection: View {
    @Environment(\.palette) var p
    let title: String
    var count: Int? = nil
    var body: some View {
        HStack(spacing: Space.xs + 1) {
            Text(title).font(Ramp.label).foregroundStyle(p.secondary)
            if let count { Text("\(count)").font(Ramp.meta).foregroundStyle(p.secondary) }
        }
        .padding(.horizontal, Space.s + 2).padding(.top, Space.s).padding(.bottom, Space.xxs)
    }
}

/// A Needs attention row: who, what it waits on, where. Two lines, the
/// name first; the state as a dot and word.
struct AttentionRow: View {
    @Environment(\.palette) var p
    let name: String
    let line: String
    let path: String
    let state: (Color, String)
    var selected = false
    var body: some View {
        HStack(alignment: .top, spacing: Space.s) {
            HostDisc(host: "local").padding(.top, 2)
            VStack(alignment: .leading, spacing: 1) {
                HStack {
                    Text(name).font(.system(size: 13, weight: .medium)).foregroundStyle(p.label).lineLimit(1).layoutPriority(1)
                    Spacer(minLength: Space.xs)
                    StatusTag(color: state.0, text: state.1, selected: selected)
                }
                Text(line).font(Ramp.secondary).foregroundStyle(selected ? p.label : p.secondary).lineLimit(1)
                Text(path).font(Ramp.meta).foregroundStyle(selected ? p.label : p.secondary).lineLimit(1)
            }
        }
        .padding(.horizontal, Space.s).padding(.vertical, Space.s - 2)
        .background(selected ? p.selection : .clear, in: RoundedRectangle(cornerRadius: Radius.control + 1, style: .continuous))
    }
}

/// Text tabs (Overview, Collaborate, Changes): sentence case, the selected
/// one in label ink with an accent underline.
struct TextTabs: View {
    @Environment(\.palette) var p
    let tabs: [String]
    var body: some View {
        HStack(spacing: Space.l) {
            ForEach(Array(tabs.enumerated()), id: \.offset) { i, tab in
                VStack(spacing: Space.xs) {
                    Text(tab).font(.system(size: 12, weight: i == 0 ? .semibold : .medium)).foregroundStyle(i == 0 ? p.label : p.secondary)
                    Capsule().fill(i == 0 ? p.accent : .clear).frame(height: 2)
                }.fixedSize()
            }
        }
    }
}

struct ExploreMock: View {
    @Environment(\.palette) var p
    var body: some View {
        HStack(spacing: 0) {
            ActivityBar(selected: 1)
            VStack(alignment: .leading, spacing: Space.xs) {
                HStack(spacing: Space.s) {
                    Text(L("Explore", "탐색")).font(Ramp.section).foregroundStyle(p.label)
                    Text("22").font(Ramp.meta).foregroundStyle(p.secondary)
                    Spacer()
                    ForEach(["square.on.square", "plus", "ellipsis"], id: \.self) {
                        Image(systemName: $0).font(.system(size: 12)).foregroundStyle(p.secondary)
                    }
                }.padding(.horizontal, Space.m).frame(height: 36)
                HStack(spacing: Space.s - 2) {
                    Image(systemName: "magnifyingglass").font(.system(size: 11)).foregroundStyle(p.tertiary)
                    Text(L("Filter Explore", "탐색 필터")).font(.system(size: 12)).foregroundStyle(p.tertiary)
                    Spacer()
                    Image(systemName: "line.3.horizontal.decrease").font(.system(size: 11)).foregroundStyle(p.secondary)
                }
                .padding(.horizontal, Space.s).frame(height: 26)
                .background(p.control, in: RoundedRectangle(cornerRadius: Radius.control + 1, style: .continuous))
                .padding(.horizontal, Space.m).padding(.bottom, Space.xs)
                VStack(alignment: .leading, spacing: 1) {
                    SidebarSection(title: L("Needs attention", "확인 필요"), count: 6)
                    AttentionRow(name: "sdk", line: "500 from the codegen service after 3 retries", path: "local · sdk › sdk", state: (p.red, L("Error", "오류")))
                    AttentionRow(name: "auth", line: "waiting on AskUserQuestion", path: "local · auth › auth", state: (p.orange, L("Choice", "선택")), selected: true)
                    AttentionRow(name: "review", line: "codex permission: shell", path: "local · main › review", state: (p.orange, L("Approval", "승인")))
                    Text(L("Show all 6", "6개 모두 보기")).font(.system(size: 12, weight: .medium)).foregroundStyle(p.accentText)
                        .padding(.horizontal, Space.s + 2).padding(.vertical, Space.xs)
                    SidebarSection(title: L("Execution topology", "실행 토폴로지"))
                    TreeRow(depth: 0, symbol: "laptopcomputer", name: "local", detail: "22", expanded: true)
                    TreeRow(depth: 1, symbol: "square.stack.3d.up", name: "api", detail: "2", expanded: true)
                    TreeRow(depth: 2, symbol: "asterisk", name: "api", state: (p.blue, L("Working", "작업 중")))
                    TreeRow(depth: 2, symbol: "chevron.left.forwardslash.chevron.right", name: "api-rev", state: (p.blue, L("Working", "작업 중")))
                    TreeRow(depth: 1, symbol: "square.stack.3d.up", name: "auth", detail: "1", expanded: true)
                    TreeRow(depth: 2, symbol: "asterisk", name: "auth", state: (p.orange, L("Choice", "선택")), selected: true)
                    TreeRow(depth: 1, symbol: "square.stack.3d.up", name: "billing", detail: "1", expanded: true)
                    TreeRow(depth: 2, symbol: "asterisk", name: "billing", state: (p.green, L("Done", "완료")))
                    TreeRow(depth: 1, symbol: "square.stack.3d.up", name: "crm", detail: "1", expanded: true)
                    TreeRow(depth: 2, symbol: "asterisk", name: "crm", state: (p.purple, L("Limited", "제한됨")))
                    TreeRow(depth: 1, symbol: "square.stack.3d.up", name: "data", detail: "1", expanded: false)
                    TreeRow(depth: 1, symbol: "square.stack.3d.up", name: "docs", detail: "1", expanded: false)
                }.padding(.horizontal, Space.s)
                Spacer()
            }
            .frame(width: 300)
            .background(p.sidebar)
            Rectangle().fill(p.hairline).frame(width: 1)
            VStack(spacing: 0) {
                TabStrip(tabs: [("waveform.path.ecg", "Live Watch"), ("house", L("Home", "홈"))])
                VStack(alignment: .leading, spacing: Space.m) {
                    HStack(alignment: .center, spacing: Space.m) {
                        HostDisc(host: "local").scaleEffect(2.2).frame(width: 30, height: 30)
                        VStack(alignment: .leading, spacing: 2) {
                            Text("auth").font(Ramp.page).foregroundStyle(p.label)
                            HStack(spacing: Space.s) {
                                Text("local").font(Ramp.meta).foregroundStyle(p.secondary)
                                Text("·").foregroundStyle(p.tertiary)
                                Image(systemName: "asterisk").font(.system(size: 9, weight: .bold)).foregroundStyle(Color(red: 0.85, green: 0.47, blue: 0.34))
                                Text("Claude Code").font(Ramp.meta).foregroundStyle(p.secondary)
                                StatusTag(color: p.orange, text: L("Waiting for choice", "선택 대기"))
                            }
                        }
                        Spacer()
                        SecondaryButton(title: L("Details", "세부사항"), symbol: "info.circle", compact: true)
                        SecondaryButton(title: L("Open in Shell", "셸에서 열기"), symbol: "rectangle.on.rectangle", compact: true)
                    }
                    TextTabs(tabs: [L("Overview", "개요"), L("Collaborate", "협업"), L("Changes", "변경 사항")])
                    VStack(alignment: .leading, spacing: Space.xs + 1) {
                        Text(L("Latest notice", "최근 알림")).font(Ramp.label).foregroundStyle(p.secondary)
                        Text("waiting on AskUserQuestion").font(Ramp.body).foregroundStyle(p.label)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(Space.m)
                    .background(p.lane, in: RoundedRectangle(cornerRadius: Radius.card, style: .continuous))
                    // The Live Pane, as today: read-only until Click to Type.
                    VStack(alignment: .leading, spacing: 0) {
                        HStack(spacing: Space.s) {
                            Image(systemName: "terminal").font(.system(size: 11)).foregroundStyle(p.secondary)
                            Text(L("Live Pane", "라이브 페인")).font(Ramp.section).foregroundStyle(p.label)
                            Text("local · auth › auth · %10").font(Ramp.meta).foregroundStyle(p.secondary)
                            Text(L("Read-only", "읽기 전용")).font(Ramp.label).foregroundStyle(p.secondary)
                                .padding(.horizontal, 6).frame(height: 18).background(p.control, in: Capsule())
                            Spacer()
                            SecondaryButton(title: L("Click to Type", "클릭해서 입력"), symbol: "keyboard", compact: true)
                        }
                        .padding(.horizontal, Space.m).frame(height: 40)
                        Rectangle().fill(p.hairline).frame(height: 1)
                        VStack(alignment: .leading, spacing: 4) {
                            Text("● claude").foregroundStyle(p.label)
                            Text("")
                            Text("› rotate the JWT signing keys across environments").foregroundStyle(p.label)
                            Text("")
                            Text("  ? Which environments should rotate first?").foregroundStyle(p.text(p.orange))
                            Text("")
                            Text("    ❯ staging, then prod on approval").foregroundStyle(p.label)
                            Text("      all environments at once").foregroundStyle(p.secondary)
                            Text("      prod only — staging already rotated").foregroundStyle(p.secondary)
                        }
                        .font(Ramp.mono)
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                        .padding(Space.m)
                    }
                    .background(p.c(0xF7F7F7, 0x212121), in: RoundedRectangle(cornerRadius: Radius.card, style: .continuous))
                    .overlay(RoundedRectangle(cornerRadius: Radius.card, style: .continuous).strokeBorder(p.hairline))
                    HStack(spacing: Space.s) {
                        Text(L("Send a prompt to this agent", "이 에이전트에게 프롬프트 보내기")).font(Ramp.body).foregroundStyle(p.tertiary)
                        Spacer()
                        PrimaryButton(title: L("Send", "보내기"))
                    }
                    .padding(.leading, Space.m).padding(.trailing, 5).frame(height: 38)
                    .background(p.card, in: RoundedRectangle(cornerRadius: Radius.card, style: .continuous))
                    .overlay(RoundedRectangle(cornerRadius: Radius.card, style: .continuous).strokeBorder(p.controlBorder))
                }
                .padding(Space.xxl)
                StatusBar()
            }
            .background(p.content)
        }
    }
}

// MARK: - Decision cards

/// Today's two decision cards (a question, a result to review) in the new
/// system, with what tend checked before the asker's recommendation.
struct DecisionMock: View {
    @Environment(\.palette) var p
    var body: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            Card(state: p.orange) {
                CardHead(symbol: "questionmark.bubble", kind: L("Question", "질문"), tint: p.orange, time: L("4 min", "4분"))
                Text("How should the storage migration roll out?").font(Ramp.cardTitle).foregroundStyle(p.label)
                Text("The migration touches every host's data directory.").font(Ramp.body).foregroundStyle(p.label)
                HStack(alignment: .firstTextBaseline, spacing: Space.xs + 1) {
                    Image(systemName: "lightbulb").font(.system(size: 10)).foregroundStyle(p.secondary)
                    Text("Flag first").font(Ramp.secondary).foregroundStyle(p.secondary)
                }
                OptionList(options: [
                    "Ship it behind a flag first, enable it on jiun-mini for a day, then turn it on for every host once the nightly backup finished cleanly",
                    "Migrate everything now",
                    "Hold it until the next release: the schema may still change and a second migration would be needed",
                ])
                HStack(spacing: Space.xs) { PlainButton(title: L("Other…", "기타…")); Spacer(minLength: 0) }
                MetaLine(agent: "claude", host: "local", project: "tend")
            }
            Card(state: p.orange) {
                CardHead(symbol: "checklist", kind: L("Review", "리뷰"), tint: p.orange, time: L("4 min", "4분"))
                Text("Merge the retry path now?").font(Ramp.cardTitle).foregroundStyle(p.label)
                Text("Retry path for the storage migration").font(Ramp.body).foregroundStyle(p.label)
                Evidence(check: "cargo test -p tend review", recommendation: "merge now: the perf run only covers reads")
                HStack(spacing: Space.xs + 1) {
                    Image(systemName: "arrow.triangle.pull").font(.system(size: 10)).foregroundStyle(p.accentText)
                    Text("Open330/tend#1").font(Ramp.label).foregroundStyle(p.accentText)
                }
                OptionList(options: ["merge now", "wait for the perf run"])
                HStack(spacing: Space.xs) {
                    SecondaryButton(title: L("Accept", "수락"), compact: true)
                    PlainButton(title: L("Return…", "반환…"))
                    Spacer(minLength: 0)
                }
                MetaLine(agent: "claude", host: "local", project: "tend")
            }
        }
        .padding(Space.l)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(p.lane)
    }
}

// MARK: - Menu bar

/// Today's popover: agent rows (product, name, path, state), the summary
/// lines, a usage warning, and Open Tend with settings and more. Same
/// content, the system's type and spacing.
struct MenuBarMock: View {
    @Environment(\.palette) var p
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Tend").font(Ramp.section).foregroundStyle(p.label)
                .padding(.horizontal, Space.l).padding(.top, Space.m).padding(.bottom, Space.xs)
            VStack(spacing: 2) {
                row("chevron.left.forwardslash.chevron.right", "codex", "local › ops › ops", (p.green, L("Idle", "대기")))
                row("asterisk", "claude code", "local › main › ipc", (p.blue, L("Working", "작업 중")), selected: true)
                row("asterisk", "claude code", "local › docs › docs", (p.blue, L("Working", "작업 중")))
                row("asterisk", "claude code", "local › api › api", (p.blue, L("Working", "작업 중")))
            }.padding(.horizontal, Space.s)
            VStack(alignment: .leading, spacing: Space.s) {
                Text(L("19 agents · 0 Work items", "에이전트 19개 · Work 항목 0개")).font(Ramp.meta).foregroundStyle(p.secondary)
                HStack(spacing: Space.xs + 1) {
                    Image(systemName: "exclamationmark.circle.fill").font(.system(size: 11)).foregroundStyle(p.orange)
                    Text(L("6 agents need attention · 1 decision", "확인이 필요한 에이전트 6개 · 결정 1건")).font(Ramp.secondary).foregroundStyle(p.text(p.orange))
                }
                HStack(spacing: Space.s) {
                    Image(systemName: "arrow.triangle.2.circlepath").font(.system(size: 11)).foregroundStyle(p.secondary)
                    Text(L("1 agent reached its usage limit: switch account…", "에이전트 1개가 사용 한도에 도달함: 계정 전환…")).font(Ramp.secondary).foregroundStyle(p.label).lineLimit(1)
                }
                .padding(.horizontal, Space.s + 2).frame(height: 28)
                .background(p.control, in: RoundedRectangle(cornerRadius: Radius.control + 1))
            }
            .padding(.horizontal, Space.l).padding(.top, Space.s)
            Rectangle().fill(p.hairline).frame(height: 1).padding(.top, Space.m)
            HStack(spacing: Space.m) {
                SecondaryButton(title: L("Open Tend", "Tend 열기"), compact: true)
                Spacer()
                Image(systemName: "gearshape").font(.system(size: 13)).foregroundStyle(p.secondary)
                Image(systemName: "ellipsis.circle").font(.system(size: 13)).foregroundStyle(p.secondary)
            }.padding(.horizontal, Space.l).frame(height: 44)
        }
        .frame(width: 340)
        .background(p.card)
    }

    func row(_ symbol: String, _ name: String, _ path: String, _ state: (Color, String), selected: Bool = false) -> some View {
        HStack(alignment: .center, spacing: Space.s + 2) {
            Image(systemName: symbol).font(.system(size: 12, weight: .bold))
                .foregroundStyle(symbol == "asterisk" ? Color(red: 0.85, green: 0.47, blue: 0.34) : p.label)
                .frame(width: 26, height: 26).background(p.control, in: RoundedRectangle(cornerRadius: 7))
            VStack(alignment: .leading, spacing: 1) {
                Text(name).font(.system(size: 13, weight: .medium)).foregroundStyle(p.label).lineLimit(1)
                Text(path).font(Ramp.meta).foregroundStyle(selected ? p.label : p.secondary).lineLimit(1)
            }
            Spacer()
            StatusTag(color: state.0, text: state.1, selected: selected)
            Image(systemName: "chevron.right").font(.system(size: 10, weight: .semibold)).foregroundStyle(p.tertiary)
        }
        .padding(.horizontal, Space.s).padding(.vertical, Space.s - 2)
        .background(selected ? p.selection : .clear, in: RoundedRectangle(cornerRadius: Radius.control + 1))
    }
}

// MARK: - Rendering

@MainActor
func render<V: View>(_ view: V, size: CGSize, scheme: ColorScheme, to url: URL) {
    let palette = Palette(scheme: scheme)
    let root = view
        .environment(\.palette, palette)
        .environment(\.colorScheme, scheme)
        .frame(width: size.width, height: size.height)
        .background(palette.content)
        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
    let renderer = ImageRenderer(content: root)
    renderer.scale = 2
    guard let image = renderer.nsImage, let tiff = image.tiffRepresentation,
          let png = NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:]) else {
        print("failed: \(url.lastPathComponent)")
        return
    }
    try? png.write(to: url)
    print("wrote \(url.path)")
}

MainActor.assumeIsolated {
    let out = URL(fileURLWithPath: CommandLine.arguments.dropFirst().first ?? ".")
    for ko in [false, true] {
        korean = ko
        let lang = ko ? "ko" : "en"
        for scheme in [ColorScheme.light, .dark] {
            let name = "\(scheme == .dark ? "dark" : "light")-\(lang)"
            render(HomeMock(), size: CGSize(width: 1440, height: 900), scheme: scheme,
                   to: out.appendingPathComponent("after-home-\(name).png"))
            render(HomeMock(narrow: true), size: CGSize(width: 920, height: 580), scheme: scheme,
                   to: out.appendingPathComponent("after-home-minimum-\(name).png"))
            render(HomeMock(empty: true), size: CGSize(width: 1440, height: 900), scheme: scheme,
                   to: out.appendingPathComponent("after-home-empty-\(name).png"))
            render(ExploreMock(), size: CGSize(width: 1440, height: 900), scheme: scheme,
                   to: out.appendingPathComponent("after-explore-\(name).png"))
            render(DecisionMock(), size: CGSize(width: 380, height: 860), scheme: scheme,
                   to: out.appendingPathComponent("after-cards-\(name).png"))
            render(MenuBarMock(), size: CGSize(width: 340, height: 420), scheme: scheme,
                   to: out.appendingPathComponent("after-menubar-\(name).png"))
        }
    }
}
