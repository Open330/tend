// Your turn's folded group, **Can't be answered** (docs/proposals/
// STALE_QUESTIONS.md): questions whose asking agent tendd established gone
// for a day (`subject_check_v1`). They leave Your turn and every count, are
// collapsed by default, and say why and since when. Keep puts one back in
// Your turn; Close all, only where the daemon allows it, declines exactly
// the listed ones after one confirmation.
import SwiftUI

extension TendUnanswerable {
    /// `since 3 days ago`.
    func sinceText(relativeTo now: Date = Date()) -> String? {
        guard let since = TendUsageTimestamp.parse(since) else { return nil }
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .full
        let relative = formatter.localizedString(for: since, relativeTo: now)
        return String(localized: "since \(relative)")
    }

    /// `Pane gone · since 3 days ago`.
    func reasonLine(relativeTo now: Date = Date()) -> String {
        [cause.text, sinceText(relativeTo: now)].compactMap { $0 }.joined(separator: " · ")
    }
}

enum FleetCantAnswer {
    /// The group's count: the daemon's exact count when the whole queue is
    /// shown (its list may be cut), never less than what is listed.
    static func count(listed: Int, exact: Int, filtered: Bool) -> Int {
        filtered ? listed : max(listed, exact)
    }

    /// `Close 3 questions?`
    static func confirmTitle(_ count: Int) -> String {
        count == 1 ? String(localized: "Close 1 question?") : String(localized: "Close \(count) questions?")
    }

    static let confirmMessage = String(localized: "Each asking agent gets a reply that you dismissed it. This can't be undone.")
}

struct FleetCantAnswerGroup: View {
    /// The listed questions, in the queue's order.
    let decisions: [FleetDecision]
    /// Their exact number (see `FleetCantAnswer.count`).
    let count: Int
    var canKeep = false
    var canCloseAll = false
    var busy: Set<String> = []
    var failures: [String: String] = [:]
    var result: (ok: Bool, text: String)?
    var keep: (FleetDecision) -> Void = { _ in }
    var closeAll: ([FleetDecision]) -> Void = { _ in }
    /// Opens unfolded, for previews and screenshots.
    var initiallyExpanded = false
    @State private var expanded: Bool?
    @State private var confirming = false

    private var isExpanded: Bool { expanded ?? initiallyExpanded }

    /// Close all closes only this Mac's questions (no Fleet relay).
    private var closable: [FleetDecision] { decisions.filter(\.host.local) }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Divider().padding(.bottom, 2)
            Button {
                withAnimation(.easeOut(duration: 0.15)) { expanded = !isExpanded }
            } label: {
                HStack(spacing: 5) {
                    Image(systemName: "chevron.right")
                        .font(.system(size: 9, weight: .semibold))
                        .rotationEffect(.degrees(isExpanded ? 90 : 0))
                    Text("Can't be answered (\(count))")
                        .font(.system(size: 11, weight: .semibold))
                        .monospacedDigit()
                    Spacer(minLength: 0)
                }
                .foregroundStyle(.secondary)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .padding(.horizontal, 4)
            .help(String(localized: "The agents that asked these are gone, so nobody will read an answer. They stay open until you close them."))
            .accessibilityValue(isExpanded ? String(localized: "Expanded") : String(localized: "Collapsed"))
            if isExpanded {
                expandedBody
            }
        }
        .alert(FleetCantAnswer.confirmTitle(closable.count), isPresented: $confirming) {
            Button(String(localized: "Close \(closable.count)"), role: .destructive) { closeAll(closable) }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(FleetCantAnswer.confirmMessage)
        }
    }

    @ViewBuilder
    private var expandedBody: some View {
        Text("The agents that asked these are gone, so nobody will read an answer. They stay open until you close them.")
            .font(TendType.meta)
            .foregroundStyle(.tertiary)
            .fixedSize(horizontal: false, vertical: true)
            .padding(.horizontal, 4)
        if canCloseAll, !closable.isEmpty {
            Button("Close All…") { confirming = true }
                .buttonStyle(.tendGhost)
                .controlSize(.small)
                .disabled(closable.contains { busy.contains($0.id) })
                .help(String(localized: "Close every question listed here; each asker gets a reply that you dismissed it"))
        }
        if let result {
            Label(result.text, systemImage: result.ok ? "checkmark.circle" : "exclamationmark.circle")
                .font(TendType.meta)
                .foregroundStyle(result.ok ? Color.green : Color.orange)
                .lineLimit(4)
                .padding(.horizontal, 4)
        }
        ForEach(decisions) { decision in
            FleetCantAnswerRow(
                decision: decision,
                busy: busy.contains(decision.id),
                failure: failures[decision.id],
                keep: canKeep && decision.host.local ? { keep(decision) } : nil
            )
        }
        if count > decisions.count {
            Text("\(count - decisions.count) more not listed")
                .font(TendType.meta)
                .foregroundStyle(.tertiary)
                .padding(.horizontal, 4)
        }
    }
}

/// One question nobody can answer: what it asked, why it cannot be
/// answered and since when, who asked, and Keep.
struct FleetCantAnswerRow: View {
    let decision: FleetDecision
    var busy = false
    var failure: String?
    var keep: (() -> Void)?
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(verbatim: decision.title)
                .font(.system(size: 12, weight: .semibold))
                .lineLimit(2)
                .frame(maxWidth: .infinity, alignment: .leading)
                .textSelection(.enabled)
            if let unanswerable = decision.unanswerable {
                Label {
                    Text(verbatim: unanswerable.reasonLine())
                } icon: {
                    Image(systemName: "person.crop.circle.badge.xmark")
                }
                .font(TendType.meta)
                .foregroundStyle(.secondary)
            }
            HStack(spacing: 6) {
                if let asker = decision.asker {
                    Text(verbatim: asker)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                Text(verbatim: decision.host.alias)
                    .lineLimit(1)
                if let since = decision.since {
                    Text(since, format: .relative(presentation: .numeric, unitsStyle: .abbreviated))
                        .monospacedDigit()
                        .fixedSize()
                }
                Spacer(minLength: 4)
                if busy {
                    ProgressView().controlSize(.mini)
                } else if let keep {
                    Button("Keep", action: keep)
                        .buttonStyle(.tendGhost)
                        .controlSize(.small)
                        .help(String(localized: "Put it back in Your turn"))
                }
            }
            .font(TendType.meta)
            .foregroundStyle(.tertiary)
            if let failure {
                Label(failure, systemImage: "exclamationmark.circle")
                    .font(TendType.meta)
                    .foregroundStyle(.red)
                    .lineLimit(3)
            }
        }
        .padding(10)
        .background(TendSurfacePalette.workspace(for: colorScheme).opacity(0.7), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .strokeBorder(Color.secondary.opacity(0.2), lineWidth: 1)
        )
        .accessibilityElement(children: .contain)
    }
}
