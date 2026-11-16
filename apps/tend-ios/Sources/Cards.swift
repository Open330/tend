// The cards outside Your turn: an agent at work or finished
// (`FleetWorkCard`), and a decision already taken.
import SwiftUI
import UIKit

/// One agent as a work card, as the Mac draws a single-stage unit: title,
/// stage bar (not in Finished), the agent, the headline, then the meta row.
struct AgentCard: View {
    let agent: LaneAgent
    let lane: Lane

    var body: some View {
        let a = agent.agent
        VStack(alignment: .leading, spacing: 8) {
            Text(verbatim: a.title)
                .font(TendType.cardTitle)
                .lineLimit(2)
                .frame(maxWidth: .infinity, alignment: .leading)
            if lane != .finished {
                Capsule().fill(a.status.color).frame(height: 4)
                    .accessibilityHidden(true)
                Text(verbatim: a.maker).font(TendType.meta.weight(.medium)).foregroundStyle(.secondary)
            }
            Text(TendMarkdown.inline(a.headline ?? a.status.label))
                .font(TendType.body)
                .foregroundStyle(.secondary)
                .lineLimit(lane == .finished ? 3 : 2)
            MetaRow(maker: a.maker, repo: a.repo, host: agent.host, since: a.since)
        }
        .tendCard()
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("agentCard")
    }
}

/// An agent's mark: Claude's orange asterisk, Codex's dark knot, else a
/// person (the Mac app's 12 pt agent logos).
struct AgentLogo: View {
    let maker: String

    var body: some View {
        switch maker {
        case "claude": Image(systemName: "asterisk").foregroundStyle(Color(uiColor: UIColor(hex: 0xD97757)))
        case "codex": Image(systemName: "hexagon").foregroundStyle(.primary)
        default: Image(systemName: "person.crop.circle").foregroundStyle(.secondary)
        }
    }
}

/// A project heading inside a lane (`folder` + name + count).
struct ProjectHeader: View {
    let name: String
    let count: Int

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "folder").imageScale(.small)
            Text(verbatim: name).font(TendType.meta.weight(.semibold)).foregroundStyle(.secondary)
            Text(verbatim: "\(count)").font(TendType.meta).monospacedDigit().foregroundStyle(.tertiary)
        }
        .foregroundStyle(.secondary)
        .padding(.top, 4)
        .accessibilityAddTraits(.isHeader)
    }
}

/// The Mac card's meta row: agent, project, host badge, then the time.
struct MetaRow: View {
    let maker: String
    let repo: String?
    let host: String
    let since: Date?

    var body: some View {
        HStack(spacing: 8) {
            AgentLogo(maker: maker).imageScale(.small)
            if let repo {
                Text(verbatim: repo).lineLimit(1)
            }
            HStack(spacing: 4) {
                HostGlyph(alias: host, size: 14)
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
        .foregroundStyle(.tertiary)
        .lineLimit(1)
    }
}

/// A decision already taken, in Finished.
struct DecidedCard: View {
    let past: PastDecision

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            HStack(spacing: 6) {
                Chip(text: past.outcome, color: past.isPositive ? .green : .orange, symbol: past.isPositive ? "checkmark" : "arrow.uturn.backward")
                    .accessibilityIdentifier("pastOutcome")
                Spacer(minLength: 4)
                if let at = past.at {
                    Text(at, format: .relative(presentation: .numeric, unitsStyle: .abbreviated))
                        .font(TendType.meta)
                        .monospacedDigit()
                        .foregroundStyle(.tertiary)
                }
            }
            Text(verbatim: past.title)
                .font(TendType.cardTitle)
                .lineLimit(3)
                .accessibilityIdentifier("pastTitle")
            if let text = past.text {
                Text(verbatim: text)
                    .font(TendType.body)
                    .foregroundStyle(.secondary)
                    .lineLimit(3)
            }
            HStack(spacing: 6) {
                HostGlyph(alias: past.host, size: 14)
                Text(verbatim: past.host)
                if let by = past.by {
                    Text(verbatim: "·")
                    Text("by \(by)").lineLimit(1)
                }
            }
            .font(TendType.meta)
            .foregroundStyle(.tertiary)
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
