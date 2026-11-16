// One decision card, in the order Tend for Mac draws it: what tend checked
// itself, the question or result, the asker's recommendation and evidence,
// then the actions. Every string here is the asker's declaration, shown as
// recorded; evidence links open in Safari, which never gets the key.
import SwiftUI
import UIKit

struct DecisionCard: View {
    let decision: PendingDecision
    @Environment(AppModel.self) private var model
    @Environment(\.openURL) private var openURL
    /// The open composer's text; nil while the actions show.
    @State private var composing: String?
    /// Set when an option was picked: sent with Accept as the chosen option.
    @State private var chosenOption: String?

    private var busy: Bool { model.busy.contains(decision.id) }

    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            header
            if let verification = decision.verification, !verification.checks.isEmpty {
                VerificationChecks(verification: verification)
            }
            Text(verbatim: decision.title)
                .font(TendType.cardTitle)
                .lineLimit(composing == nil ? 6 : nil)
                .frame(maxWidth: .infinity, alignment: .leading)
                .textSelection(.enabled)
                .accessibilityIdentifier("cardTitle")
            if let detail = decision.detail {
                Text(verbatim: detail)
                    .font(TendType.body)
                    .foregroundStyle(.secondary)
                    .lineLimit(5)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            if let recommendation = decision.recommendation {
                Label {
                    Text(verbatim: recommendation).foregroundStyle(.primary)
                } icon: {
                    Image(systemName: "lightbulb.fill").foregroundStyle(.orange)
                }
                .font(TendType.detail)
            }
            if !decision.evidence.isEmpty {
                EvidenceList(evidence: decision.evidence, open: { openURL($0) })
            }
            if !decision.decidable {
                Label(notHere, systemImage: "desktopcomputer")
                    .font(TendType.meta)
                    .foregroundStyle(.secondary)
            } else if composing != nil {
                composer
            } else {
                actions
            }
            if let failure = model.failures[decision.id] {
                Label(failure, systemImage: "exclamationmark.circle")
                    .font(TendType.meta)
                    .foregroundStyle(.red)
                    .accessibilityIdentifier("cardFailure")
            }
        }
        .tendCard(stroke: Color.orange.opacity(decision.decidable ? 0.45 : 0.2))
        .disabled(busy)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("card")
    }

    private var notHere: String {
        switch decision.kind {
        case .waiting: String(localized: "Waiting in its pane; answer it on a computer.")
        default: String(localized: "Decide this on \(decision.host); its tend cannot take decisions from here.")
        }
    }

    private var header: some View {
        HStack(spacing: 6) {
            switch decision.kind {
            case .question:
                Label(String(localized: "Question"), systemImage: "questionmark.bubble.fill")
                    .foregroundStyle(.orange)
            case .result:
                Label(String(localized: "Review"), systemImage: "checklist")
                    .foregroundStyle(.orange)
            case .waiting:
                Label(String(localized: "Waiting"), systemImage: "hand.raised.fill")
                    .foregroundStyle(.orange)
            }
            if let asker = decision.asker {
                Text(verbatim: "·")
                Text(verbatim: asker).lineLimit(1).truncationMode(.middle)
            }
            if !decision.hostIsLocal {
                HostBadge(alias: decision.host, local: false, state: "online", size: 16, showsState: false)
                Text(verbatim: decision.host).lineLimit(1)
            }
            Spacer(minLength: 4)
            if busy {
                ProgressView().controlSize(.small)
            } else if let since = decision.since {
                Text(since, format: .relative(presentation: .numeric, unitsStyle: .abbreviated))
                    .monospacedDigit()
                    .lineLimit(1)
                    .fixedSize()
            }
        }
        .font(TendType.meta.weight(.medium))
        .foregroundStyle(.secondary)
        .labelStyle(.titleAndIcon)
        .lineLimit(1)
    }

    @ViewBuilder
    private var actions: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(Array(decision.options.enumerated()), id: \.offset) { index, option in
                Button {
                    chosenOption = option
                    composing = decision.isQuestion ? option : ""
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
                        chosenOption = nil
                        composing = ""
                    }
                    .buttonStyle(.card)
                    .accessibilityIdentifier("answerButton")
                case .result:
                    Button("Accept") { Task { await model.review(decision, .accepted) } }
                        .buttonStyle(.card)
                        .accessibilityIdentifier("acceptButton")
                    Button("Return…") {
                        chosenOption = nil
                        composing = ""
                    }
                    .buttonStyle(.card)
                    .accessibilityIdentifier("returnButton")
                    Spacer(minLength: 0)
                    Menu {
                        Button("Withdraw", role: .destructive) { Task { await model.review(decision, .withdrawn) } }
                    } label: {
                        Image(systemName: "ellipsis")
                            .frame(width: 20)
                    }
                    .buttonStyle(.card)
                    .accessibilityLabel(String(localized: "More"))
                case .waiting:
                    EmptyView()
                }
            }
        }
    }

    private var composer: some View {
        let text = Binding(get: { composing ?? "" }, set: { composing = $0 })
        let empty = (composing ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        return VStack(alignment: .leading, spacing: 8) {
            if let chosenOption, !decision.isQuestion {
                Label(String(localized: "Accept with: \(chosenOption)"), systemImage: "checkmark.circle")
                    .font(TendType.body)
            }
            TextField(
                decision.isQuestion ? String(localized: "Your answer") : String(localized: "Note for the asker"),
                text: text,
                axis: .vertical
            )
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
                Button("Cancel") {
                    composing = nil
                    chosenOption = nil
                }
                .buttonStyle(.card)
                Spacer(minLength: 0)
                switch decision.kind {
                case .question:
                    Button("Decline") { submit { await model.answer(decision, text: text.wrappedValue, declined: true) } }
                        .buttonStyle(.card)
                        .disabled(empty)
                        .accessibilityIdentifier("declineButton")
                    Button("Send") { submit { await model.answer(decision, text: text.wrappedValue) } }
                        .buttonStyle(.cardPrimary)
                        .disabled(empty)
                        .accessibilityIdentifier("sendButton")
                case .result:
                    if chosenOption == nil {
                        Button("Return") { submit { await model.review(decision, .returned, note: text.wrappedValue) } }
                            .buttonStyle(.card)
                            .disabled(empty)
                            .accessibilityIdentifier("confirmReturn")
                    }
                    Button("Accept") {
                        submit { await model.review(decision, .accepted, note: empty ? nil : text.wrappedValue, option: chosenOption) }
                    }
                    .buttonStyle(.cardPrimary)
                    .accessibilityIdentifier("confirmAccept")
                case .waiting:
                    EmptyView()
                }
            }
        }
    }

    private func submit(_ send: @escaping () async -> Bool) {
        Task {
            if await send() {
                composing = nil
                chosenOption = nil
            }
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
        .background(Color.secondary.opacity(0.08), in: RoundedRectangle(cornerRadius: TendSurface.controlRadius, style: .continuous))
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
                        .foregroundStyle(check.passed ? Color.green : Color.red)
                }
                .font(TendType.meta.weight(.medium))
                .foregroundStyle(.secondary)
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
                color: passed == true ? .green : passed == false ? .red : .secondary,
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
