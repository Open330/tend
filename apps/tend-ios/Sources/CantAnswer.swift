// Your turn's folded group, **Can't be answered** (docs/proposals/
// STALE_QUESTIONS.md), as Tend for Mac and the web Home draw it: questions
// whose asking agent tendd established gone for a day. Not in Your turn and
// not on the badge; collapsed by default; each says why and since when.
// Keep puts one back in Your turn; Close all, only where the paired host
// allows it, declines exactly the listed ones after one confirmation.
import SwiftUI

extension TendUnanswerable {
    /// `Pane gone · since 3 days ago`.
    func reasonLine(relativeTo now: Date = Date()) -> String {
        guard let since = Timestamp.parse(since) else { return cause.text }
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .full
        let relative = formatter.localizedString(for: since, relativeTo: now)
        return cause.text + " · " + String(localized: "since \(relative)")
    }
}

struct CantAnswerGroup: View {
    @Environment(AppModel.self) private var model
    /// Collapsed by default, as on the Mac and the web.
    @State private var expanded = false
    @State private var confirming = false

    var body: some View {
        let count = model.cantAnswerCount
        if count > 0 {
            VStack(alignment: .leading, spacing: 10) {
                Divider()
                Button {
                    withAnimation(.easeOut(duration: 0.15)) { expanded.toggle() }
                } label: {
                    HStack(spacing: 6) {
                        Image(systemName: "chevron.right")
                            .font(.footnote.weight(.semibold))
                            .rotationEffect(.degrees(expanded ? 90 : 0))
                        Text("Can't be answered (\(count))")
                            .font(TendType.meta.weight(.semibold))
                            .monospacedDigit()
                        Spacer(minLength: 0)
                    }
                    .foregroundStyle(.secondary)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityValue(expanded ? String(localized: "Expanded") : String(localized: "Collapsed"))
                .accessibilityIdentifier("cantAnswer")
                if expanded { content(count: count) }
            }
            .confirmationDialog(confirmTitle, isPresented: $confirming, titleVisibility: .visible) {
                let closable = model.closable
                Button(String(localized: "Close \(closable.count)"), role: .destructive) {
                    Task { await model.closeAll(closable) }
                }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("Each asking agent gets a reply that you dismissed it. This can't be undone.")
            }
        }
    }

    private var confirmTitle: String {
        let n = model.closable.count
        return n == 1 ? String(localized: "Close 1 question?") : String(localized: "Close \(n) questions?")
    }

    @ViewBuilder
    private func content(count: Int) -> some View {
        Text("The agents that asked these are gone, so nobody will read an answer. They stay open until you close them.")
            .font(TendType.meta)
            .foregroundStyle(.tertiary)
            .fixedSize(horizontal: false, vertical: true)
        if !model.closable.isEmpty {
            Button("Close All…") { confirming = true }
                .buttonStyle(.card)
                .accessibilityIdentifier("closeAll")
        }
        if let result = model.closeAllResult {
            Label(result.text, systemImage: result.ok ? "checkmark.circle" : "exclamationmark.circle")
                .font(TendType.meta)
                .foregroundStyle(result.ok ? TendPalette.green : TendPalette.orange)
                .accessibilityIdentifier("closeAllResult")
        }
        let listed = model.cantAnswer
        ForEach(listed) { CantAnswerRow(decision: $0) }
        if count > listed.count {
            Text("\(count - listed.count) more not listed")
                .font(TendType.meta)
                .foregroundStyle(.tertiary)
        }
    }
}

/// One question nobody can answer: what it asked, why and since when, who
/// asked, and Keep.
struct CantAnswerRow: View {
    let decision: PendingDecision
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(verbatim: decision.title)
                .font(TendType.cardTitle)
                .lineLimit(3)
                .frame(maxWidth: .infinity, alignment: .leading)
                .textSelection(.enabled)
            if let unanswerable = decision.unanswerable {
                Label(unanswerable.reasonLine(), systemImage: "person.crop.circle.badge.xmark")
                    .font(TendType.meta)
                    .foregroundStyle(.secondary)
            }
            HStack(spacing: 6) {
                if let asker = decision.asker {
                    Text(verbatim: asker).lineLimit(1).truncationMode(.middle)
                }
                Text(verbatim: decision.host).lineLimit(1)
                if let since = decision.since {
                    Text(since, format: .relative(presentation: .numeric, unitsStyle: .abbreviated))
                        .monospacedDigit()
                        .fixedSize()
                }
                Spacer(minLength: 4)
                if model.busy.contains(decision.id) {
                    ProgressView().controlSize(.small)
                } else if model.canKeep(decision) {
                    Button("Keep") { Task { await model.keep(decision) } }
                        .buttonStyle(.card)
                        .accessibilityHint(Text("Put it back in Your turn"))
                        .accessibilityIdentifier("keep")
                }
            }
            .font(TendType.meta)
            .foregroundStyle(.tertiary)
            if let failure = model.failures[decision.id] {
                Label(failure, systemImage: "exclamationmark.circle")
                    .font(TendType.meta)
                    .foregroundStyle(TendPalette.red)
            }
        }
        .tendCard(stroke: TendSurface.border)
        .opacity(0.92)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("cantAnswerRow")
    }
}
