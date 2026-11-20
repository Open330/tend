// Your turn's folded group, **Can't be answered** (docs/proposals/
// STALE_QUESTIONS.md), as Tend for Mac and the web Home draw it: questions
// whose asking agent tendd established gone. Not in Your turn and
// not on the badge; collapsed by default; each says why and since when.
// Keep puts one back in Your turn; Close all, one per host that allows it
// (the paired one, or a Fleet host: docs/proposals/FLEET_UNANSWERABLE.md),
// declines exactly that host's listed ones after one confirmation.
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
    /// The host whose Close all is being confirmed.
    @State private var confirming: CloseAllGroup?

    var body: some View {
        let count = model.cantAnswerCount
        if count == 0, model.closeAllResult != nil {
            // The refresh after Close all emptied the group: what it closed
            // and skipped stays until dismissed.
            VStack(alignment: .leading, spacing: 10) {
                Divider()
                CloseAllReport()
            }
        } else if count > 0 {
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
                    .foregroundStyle(TendInk.secondary)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityValue(expanded ? String(localized: "Expanded") : String(localized: "Collapsed"))
                .accessibilityIdentifier("cantAnswer")
                if expanded { content(count: count) } else { CloseAllReport() }
            }
            .confirmationDialog(
                confirming.map(Self.confirmTitle) ?? "",
                isPresented: Binding(get: { confirming != nil }, set: { if !$0 { confirming = nil } }),
                titleVisibility: .visible,
                presenting: confirming
            ) { group in
                Button(String(localized: "Close \(group.decisions.count)"), role: .destructive) {
                    Task { await model.closeAll(group) }
                }
                Button("Cancel", role: .cancel) {}
            } message: { _ in
                Text("Each asking agent gets a reply that you dismissed it. This can't be undone.")
            }
        }
    }

    /// Names the host when it is not the paired one.
    static func confirmTitle(_ group: CloseAllGroup) -> String {
        let n = group.decisions.count
        if group.local {
            return n == 1 ? String(localized: "Close 1 question?") : String(localized: "Close \(n) questions?")
        }
        return n == 1
            ? String(localized: "Close 1 question on \(group.host)?")
            : String(localized: "Close \(n) questions on \(group.host)?")
    }

    @ViewBuilder
    private func content(count: Int) -> some View {
        Text("The agents that asked these are gone, so nobody will read an answer. They stay open until you close them.")
            .font(TendType.meta)
            .foregroundStyle(TendInk.tertiary)
            .fixedSize(horizontal: false, vertical: true)
        let groups = model.closable
        let named = groups.count > 1 || groups.contains { !$0.local }
        ForEach(groups) { group in
            Button(named ? String(localized: "Close All on \(group.host)…") : String(localized: "Close All…")) {
                confirming = group
            }
            .buttonStyle(.card)
            .accessibilityIdentifier("closeAll")
        }
        ForEach(model.cantAnswerNotes) { note in
            Text(note.text)
                .font(TendType.meta)
                .foregroundStyle(TendInk.tertiary)
                .fixedSize(horizontal: false, vertical: true)
        }
        CloseAllReport()
        let listed = model.cantAnswer
        ForEach(listed) { CantAnswerRow(decision: $0) }
        if count > listed.count {
            Text("\(count - listed.count) more not listed")
                .font(TendType.meta)
                .foregroundStyle(TendInk.tertiary)
        }
    }
}

/// What the last Close all closed and skipped, in full, with Dismiss.
struct CloseAllReport: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if let result = model.closeAllResult {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Label(result.text, systemImage: result.ok ? "checkmark.circle" : "exclamationmark.circle")
                    .font(TendType.meta)
                    .foregroundStyle(result.ok ? TendPalette.green : TendPalette.orange)
                    .fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityIdentifier("closeAllResult")
                Button("Dismiss") { model.dismissCloseAllResult() }
                    .font(TendType.meta.weight(.medium))
                    .buttonStyle(.borderless)
                    .accessibilityIdentifier("dismissCloseAllResult")
            }
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
                    .foregroundStyle(TendInk.secondary)
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
            .foregroundStyle(TendInk.tertiary)
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
