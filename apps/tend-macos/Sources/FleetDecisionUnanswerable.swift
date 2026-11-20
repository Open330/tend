// Your turn's folded group, **Can't be answered** (docs/proposals/
// STALE_QUESTIONS.md): questions whose asking agent tendd established gone
// for a day (`subject_check_v1`). They leave Your turn and every count, are
// collapsed by default, and say why and since when. Keep puts one back in
// Your turn; Close all, only where the daemon allows it, declines exactly
// the listed ones after one confirmation. Another Fleet host's are kept or
// closed on the dashboard, under its sign-in: this app opens them there and
// never acts on them itself (docs/proposals/FLEET_UNANSWERABLE.md).
import AppKit
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

    /// The same across hosts: each host's exact count (this Mac's queue, or
    /// another host's read over Fleet), never less than what that host
    /// lists, summed over the hosts `host` keeps (all when nil). A cut page
    /// limits the rows, never the count. Under a project filter only what
    /// is listed counts.
    static func count(
        listed: [FleetDecision],
        exact: [String: Int],
        host: String?,
        filtered: Bool
    ) -> Int {
        let rows = listed.filter { host == nil || $0.host.alias == host }
        if filtered { return rows.count }
        let listedByHost = Dictionary(grouping: rows, by: \.host.alias).mapValues(\.count)
        let hosts = Set(listedByHost.keys).union(exact.keys.filter { host == nil || $0 == host })
        return hosts.reduce(0) { $0 + max(listedByHost[$1] ?? 0, exact[$1] ?? 0) }
    }

    /// `Close 3 questions?`
    static func confirmTitle(_ count: Int) -> String {
        count == 1 ? String(localized: "Close 1 question?") : String(localized: "Close \(count) questions?")
    }

    static let confirmMessage = String(localized: "Each asking agent gets a reply that you dismissed it. This can't be undone.")

    /// Another host's question in the dashboard's Can't be answered group:
    /// `<origin>/needs#item=<id>&host=<alias>`, which opens the group.
    static func dashboardLink(_ decision: FleetDecision, on dashboard: URL?) -> URL? {
        guard !decision.host.local, let dashboard, case .humanRequest(let id) = decision.source else { return nil }
        return TendDashboardLink.item(id, host: decision.host.alias, on: dashboard)
    }
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
    /// The dashboard tendd names, where another host's are kept or closed.
    var dashboard: URL?
    /// Clears the Close all report.
    var dismissResult: () -> Void = {}
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
            if count == 0 {
                // The refresh after Close all emptied the group: what it
                // closed and skipped stays until dismissed.
                report
            } else {
                group
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
    private var group: some View {
        Button {
            withAnimation(.easeOut(duration: 0.15)) { expanded = !isExpanded }
        } label: {
            HStack(spacing: 5) {
                Image(systemName: "chevron.right")
                    .font(TendType.glyph)
                    .rotationEffect(.degrees(isExpanded ? 90 : 0))
                Text("Can't be answered (\(count))")
                    .font(TendType.label.weight(.semibold))
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
        } else {
            report
        }
    }

    /// What the last Close all did, in full (every skipped question and
    /// why), selectable, with Dismiss.
    @ViewBuilder
    private var report: some View {
        if let result {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Label(result.text, systemImage: result.ok ? "checkmark.circle" : "exclamationmark.circle")
                    .font(TendType.meta)
                    .foregroundStyle(result.ok ? Color.green : Color.orange)
                    .fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Button("Dismiss", action: dismissResult)
                    .buttonStyle(.tendGhost)
                    .controlSize(.small)
            }
            .padding(.horizontal, 4)
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
        report
        if decisions.contains(where: { !$0.host.local }) {
            Text(dashboard == nil
                ? String(localized: "tendd names no dashboard, so other hosts' questions are kept or closed on those hosts.")
                : String(localized: "Other hosts' questions are kept or closed on the dashboard."))
                .font(TendType.meta)
                .foregroundStyle(.tertiary)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.horizontal, 4)
        }
        ForEach(decisions) { decision in
            FleetCantAnswerRow(
                decision: decision,
                busy: busy.contains(decision.id),
                failure: failures[decision.id],
                keep: canKeep && decision.host.local ? { keep(decision) } : nil,
                dashboardLink: FleetCantAnswer.dashboardLink(decision, on: dashboard)
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
    /// Another host's question, opened on the dashboard to keep or close.
    var dashboardLink: URL?
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(verbatim: decision.title)
                .font(TendType.detail.weight(.semibold))
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
                } else if let dashboardLink {
                    Button("Open on the Dashboard") { NSWorkspace.shared.open(dashboardLink) }
                        .buttonStyle(.tendGhost)
                        .controlSize(.small)
                        .help(String(localized: "Keep or close it on the dashboard's Needs you page"))
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
