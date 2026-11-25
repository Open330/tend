// One decision card, in the order apps/shared/Design/CARDS.md gives for
// every surface: the kind row, the title, the inset box (what tend checked
// itself, the detail, the asker's recommendation and evidence, the
// options, then the actions or a composer), a failure, and the meta row.
// Every string here is the asker's declaration, shown as recorded;
// evidence links open in Safari, which never gets the key.
import SwiftUI
import UIKit

struct DecisionCard: View {
    let decision: PendingDecision
    /// The project the board grouped it under, for the meta row.
    var project: String?
    @Environment(AppModel.self) private var model
    @Environment(\.openURL) private var openURL
    /// The open composer; nil while the action row shows.
    @State private var composer: Composer?
    /// The open composer's text.
    @State private var text = ""
    /// Set when a review option was picked: sent with Accept.
    @State private var chosenOption: String?

    /// What a composer sends.
    enum Composer: Hashable {
        /// A question's answer: Send needs text.
        case answer
        /// A question declined: the reason is optional.
        case decline
        /// A result returned: the note is optional.
        case returnNote
        /// A result accepted with the option picked.
        case accept
    }

    private var focused: Bool { model.focusedDecision == decision.id }

    private var busy: Bool { model.busy.contains(decision.id) }

    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            kindRow
            Text(verbatim: decision.title)
                .font(TendType.cardTitle)
                .lineLimit(composer == nil ? 6 : nil)
                .frame(maxWidth: .infinity, alignment: .leading)
                .textSelection(.enabled)
                .accessibilityIdentifier("cardTitle")
            InsetBox {
                if let verification = decision.verification, !verification.checks.isEmpty {
                    VerificationChecks(verification: verification)
                }
                if let detail = decision.detail {
                    Text(verbatim: detail)
                        .font(TendType.body)
                        .foregroundStyle(TendInk.secondary)
                        .lineLimit(5)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                if let recommendation = decision.recommendation {
                    Label {
                        Text(verbatim: recommendation).foregroundStyle(.primary)
                    } icon: {
                        Image(systemName: "lightbulb.fill").foregroundStyle(TendPalette.orange)
                    }
                    .font(TendType.detail)
                }
                if !decision.evidence.isEmpty {
                    EvidenceList(evidence: decision.evidence, open: { openURL($0) })
                }
                if !decision.decidable {
                    Label(notHere, systemImage: "desktopcomputer")
                        .font(TendType.meta)
                        .foregroundStyle(TendInk.secondary)
                        .accessibilityIdentifier("notDecidable")
                } else if let composer {
                    composerView(composer)
                } else {
                    actions
                }
            }
            if let failure = model.failures[decision.id] {
                Label(failure, systemImage: "exclamationmark.circle")
                    .font(TendType.meta)
                    .foregroundStyle(TendPalette.red)
                    .accessibilityIdentifier("cardFailure")
            }
            meta
        }
        .tendCard(attention: true)
        .overlay {
            // The item a notification named.
            if focused {
                RoundedRectangle(cornerRadius: TendSurface.cardRadius, style: .continuous)
                    .strokeBorder(TendPalette.orange, lineWidth: 2.5)
            }
        }
        .id(decision.id)
        .disabled(busy)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(focused ? "focusedCard" : "card")
    }

    private var notHere: String {
        switch decision.kind {
        case .waiting: String(localized: "Waiting in its pane; answer it on a computer.")
        default: String(localized: "Decide this on \(decision.host).")
        }
    }

    /// The kind's icon and word, "Sending…" while a decision is in flight,
    /// and ⋯ at the end.
    private var kindRow: some View {
        HStack(spacing: 8) {
            Group {
                switch decision.kind {
                case .question:
                    Label(String(localized: "Question"), systemImage: "questionmark.bubble.fill")
                case .result:
                    Label(String(localized: "Review"), systemImage: "checklist")
                case .waiting:
                    Label(String(localized: "Waiting"), systemImage: "hand.raised.fill")
                }
            }
            .font(TendType.meta.weight(.semibold))
            .foregroundStyle(TendPalette.orange)
            .labelStyle(.titleAndIcon)
            .lineLimit(1)
            if busy {
                HStack(spacing: 4) {
                    ProgressView().controlSize(.mini)
                    Text("Sending…")
                }
                .font(TendType.meta)
                .foregroundStyle(TendInk.secondary)
                .lineLimit(1)
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("sending")
            }
            Spacer(minLength: 4)
            if case .result = decision.kind, decision.decidable {
                Menu {
                    Button("Withdraw", role: .destructive) { Task { await model.review(decision, .withdrawn) } }
                        .accessibilityIdentifier("withdrawButton")
                } label: {
                    Image(systemName: "ellipsis")
                        .font(TendType.body.weight(.semibold))
                        .foregroundStyle(TendInk.secondary)
                        .frame(width: 44, height: 28)
                        .contentShape(Rectangle())
                }
                .accessibilityLabel(String(localized: "More"))
                .accessibilityIdentifier("moreButton")
                // The 44 pt target without making the row taller.
                .padding(.vertical, -6)
                .padding(.trailing, -10)
            }
        }
    }

    /// Asker · host · project, then the time; at a narrow width the time
    /// moves under.
    private var meta: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 6) { who; Spacer(minLength: 4); time }
            VStack(alignment: .leading, spacing: 3) { who; time }
        }
        .font(TendType.meta)
        .foregroundStyle(TendSurface.metaText)
        .lineLimit(1)
        .accessibilityIdentifier("cardMeta")
    }

    private var who: some View {
        HStack(spacing: 6) {
            if let asker = decision.asker {
                Text(verbatim: asker).truncationMode(.middle)
                Text(verbatim: "·")
            }
            HStack(spacing: 4) {
                HostBadge(alias: decision.host, local: decision.hostIsLocal, state: "online", size: 16, showsState: false)
                Text(verbatim: decision.host)
            }
            if let project {
                Text(verbatim: "·")
                Text(verbatim: project)
            }
        }
        .accessibilityElement(children: .combine)
    }

    @ViewBuilder
    private var time: some View {
        if let since = decision.since {
            Text(since, format: .relative(presentation: .numeric, unitsStyle: .abbreviated))
                .monospacedDigit()
                .fixedSize()
        }
    }

    private func open(_ next: Composer, text start: String = "", option: String? = nil) {
        composer = next
        text = start
        chosenOption = option
    }

    private func close() {
        composer = nil
        text = ""
        chosenOption = nil
    }

    /// The option rows, then the kind's actions.
    @ViewBuilder
    private var actions: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(Array(decision.options.enumerated()), id: \.offset) { index, option in
                Button {
                    if decision.isQuestion {
                        open(.answer, text: option)
                    } else {
                        open(.accept, option: option)
                    }
                } label: {
                    OptionRow(number: index + 1, text: option)
                }
                .buttonStyle(.plain)
                .accessibilityLabel(Text(verbatim: option))
                .accessibilityIdentifier("option")
            }
            HStack(spacing: 8) {
                switch decision.kind {
                case .question:
                    Button(decision.options.isEmpty ? String(localized: "Answer…") : String(localized: "Other…")) {
                        open(.answer)
                    }
                    .buttonStyle(.card)
                    .accessibilityIdentifier("answerButton")
                    Button(String(localized: "Decline…")) { open(.decline) }
                        .buttonStyle(.card)
                        .accessibilityIdentifier("declineButton")
                case .result:
                    Button("Accept") { Task { await model.review(decision, .accepted) } }
                        .buttonStyle(.card)
                        .accessibilityIdentifier("acceptButton")
                    Button("Return…") { open(.returnNote) }
                        .buttonStyle(.card)
                        .accessibilityIdentifier("returnButton")
                case .waiting:
                    EmptyView()
                }
            }
        }
    }

    private func composerView(_ kind: Composer) -> some View {
        let empty = text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        let placeholder = switch kind {
        case .answer: String(localized: "Your answer")
        case .decline: String(localized: "Reason (optional)")
        case .returnNote, .accept: String(localized: "Note for the asker (optional)")
        }
        return VStack(alignment: .leading, spacing: 8) {
            if kind == .accept, let chosenOption {
                Label(String(localized: "Accept with: \(chosenOption)"), systemImage: "checkmark.circle")
                    .font(TendType.body)
                    .accessibilityIdentifier("chosenOption")
            }
            TextField(placeholder, text: $text, axis: .vertical)
                .font(TendType.body)
                .lineLimit(1...6)
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
                .background(TendSurface.inputBackground, in: RoundedRectangle(cornerRadius: TendSurface.controlRadius, style: .continuous))
                .overlay(
                    RoundedRectangle(cornerRadius: TendSurface.controlRadius, style: .continuous)
                        .strokeBorder(TendSurface.inputBorder, lineWidth: 1)
                )
                .accessibilityIdentifier("composer")
            HStack(spacing: 8) {
                Button("Cancel") { close() }
                    .buttonStyle(.card)
                    .accessibilityIdentifier("cancelComposer")
                Spacer(minLength: 0)
                switch kind {
                case .answer:
                    Button("Send") { submit { await model.answer(decision, text: text) } }
                        .buttonStyle(.cardPrimary)
                        .disabled(empty)
                        .accessibilityIdentifier("sendButton")
                case .decline:
                    // An empty reason is sent empty: tendd gives the asker
                    // its standard "declined without an answer" text.
                    Button("Decline") { submit { await model.answer(decision, text: empty ? "" : text, declined: true) } }
                        .buttonStyle(.cardPrimary)
                        .accessibilityIdentifier("confirmDecline")
                case .returnNote:
                    Button("Return") { submit { await model.review(decision, .returned, note: empty ? nil : text) } }
                        .buttonStyle(.cardPrimary)
                        .accessibilityIdentifier("confirmReturn")
                case .accept:
                    Button("Accept") {
                        submit { await model.review(decision, .accepted, note: empty ? nil : text, option: chosenOption) }
                    }
                    .buttonStyle(.cardPrimary)
                    .accessibilityIdentifier("confirmAccept")
                }
            }
        }
    }

    private func submit(_ send: @escaping () async -> Bool) {
        Task {
            if await send() { close() }
        }
    }
}

/// A numbered option (`FleetDecisionViews` option rows).
struct OptionRow: View {
    let number: Int
    let text: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text(verbatim: "\(number)")
                .font(TendType.meta.weight(.bold))
                .monospacedDigit()
                .foregroundStyle(Color.accentColor)
                .frame(width: 22, height: 22)
                .background(Color.accentColor.opacity(0.15), in: Circle())
                .alignmentGuide(.firstTextBaseline) { $0[VerticalAlignment.center] + 5 }
            Text(verbatim: text)
                .font(TendType.body)
                .foregroundStyle(.primary)
                .multilineTextAlignment(.leading)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 9)
        .background(TendInk.secondary.opacity(0.08), in: RoundedRectangle(cornerRadius: TendSurface.controlRadius, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: TendSurface.controlRadius, style: .continuous)
                .strokeBorder(TendSurface.inputBorder, lineWidth: 1)
        )
        .contentShape(Rectangle())
    }
}

/// What tend checked itself: a green or red seal per check.
struct VerificationChecks: View {
    let verification: TendVerification

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            ForEach(Array(verification.checks.enumerated()), id: \.offset) { _, check in
                Label {
                    Text(verbatim: check.name ?? check.command).lineLimit(2)
                } icon: {
                    Image(systemName: check.passed ? "checkmark.seal.fill" : "xmark.seal.fill")
                        .foregroundStyle(check.passed ? TendPalette.green : TendPalette.red)
                }
                .font(TendType.meta.weight(.medium))
                .foregroundStyle(TendInk.secondary)
            }
        }
    }
}

/// Evidence as chips (`FleetDecisionViews` evidence): links open in
/// Safari, which never gets the key; tests are tinted by their result.
struct EvidenceList: View {
    let evidence: [TendEvidence]
    let open: (URL) -> Void

    var body: some View {
        FlowLayout(spacing: 6) {
            ForEach(Array(evidence.enumerated()), id: \.offset) { _, item in
                chip(item)
            }
        }
    }

    @ViewBuilder
    private func chip(_ item: TendEvidence) -> some View {
        switch item {
        case .pullRequest(let url):
            let label = Self.shortPR(url)
            if let link = URL(string: url), link.scheme == "https" {
                Button { open(link) } label: {
                    Chip(text: label, color: .accentColor, symbol: "arrow.triangle.pull")
                }
                .buttonStyle(.plain)
                .accessibilityLabel(Text(verbatim: url))
            } else {
                Chip(text: label, symbol: "arrow.triangle.pull")
            }
        case .commit(let sha, _):
            Chip(text: String(sha.prefix(8)), symbol: "smallcircle.filled.circle")
        case .diff(let range, _):
            Chip(text: range, symbol: "plusminus")
        case .test(let command, let passed, _):
            Chip(
                text: command,
                color: passed == true ? TendPalette.green : passed == false ? TendPalette.red : TendInk.secondary,
                symbol: passed == true ? "checkmark" : passed == false ? "xmark" : "circle"
            )
        case .path(let path):
            Chip(text: (path as NSString).lastPathComponent, symbol: "doc")
        }
    }

    /// "Open330/tend#1" for a GitHub pull request, else the URL's path.
    static func shortPR(_ url: String) -> String {
        guard let parsed = URL(string: url) else { return url }
        let parts = parsed.pathComponents.filter { $0 != "/" }
        if parts.count >= 4, parts[2] == "pull" {
            return "\(parts[0])/\(parts[1])#\(parts[3])"
        }
        return parsed.host.map { $0 + parsed.path } ?? url
    }
}
