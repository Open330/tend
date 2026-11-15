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
        VStack(alignment: .leading, spacing: 10) {
            header
            if let verification = decision.verification, !verification.checks.isEmpty {
                VerificationChecks(verification: verification)
            }
            Text(verbatim: decision.title)
                .font(.headline)
                .frame(maxWidth: .infinity, alignment: .leading)
                .textSelection(.enabled)
                .accessibilityIdentifier("cardTitle")
            if let detail = decision.detail {
                Text(verbatim: detail)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            if let recommendation = decision.recommendation {
                Label {
                    Text(verbatim: recommendation)
                } icon: {
                    Image(systemName: "lightbulb").foregroundStyle(TendColor.waiting)
                }
                .font(.subheadline)
                .foregroundStyle(.secondary)
            }
            if !decision.evidence.isEmpty {
                EvidenceList(evidence: decision.evidence, open: { openURL($0) })
            }
            if !decision.decidable {
                Label(notHere, systemImage: "desktopcomputer")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            } else if composing != nil {
                composer
            } else {
                actions
            }
            if let failure = model.failures[decision.id] {
                Label(failure, systemImage: "exclamationmark.circle")
                    .font(.footnote)
                    .foregroundStyle(TendColor.error)
                    .accessibilityIdentifier("cardFailure")
            }
        }
        .padding(14)
        .background(Color(uiColor: .secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .strokeBorder(TendColor.waiting.opacity(decision.decidable ? 0.5 : 0.2), lineWidth: 1)
        )
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
            case .question: Chip(text: String(localized: "Question"), color: TendColor.waiting)
            case .result: Chip(text: String(localized: "Review"), color: TendColor.waiting)
            case .waiting: Chip(text: String(localized: "Waiting"), color: .secondary)
            }
            if let asker = decision.asker {
                Text(verbatim: asker).lineLimit(1).truncationMode(.middle)
            }
            Spacer(minLength: 4)
            if busy {
                ProgressView().controlSize(.small)
            } else if let since = decision.since {
                Text(since, format: .relative(presentation: .numeric, unitsStyle: .abbreviated))
                    .monospacedDigit()
            }
        }
        .font(.caption)
        .foregroundStyle(.secondary)
    }

    @ViewBuilder
    private var actions: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(decision.options, id: \.self) { option in
                Button {
                    chosenOption = option
                    composing = decision.isQuestion ? option : ""
                } label: {
                    Text(verbatim: option)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .buttonStyle(.bordered)
                .accessibilityIdentifier("option")
            }
            HStack(spacing: 8) {
                switch decision.kind {
                case .question:
                    Button(decision.options.isEmpty ? String(localized: "Answer…") : String(localized: "Other…")) {
                        chosenOption = nil
                        composing = ""
                    }
                    .buttonStyle(.borderedProminent)
                    .accessibilityIdentifier("answerButton")
                case .result:
                    Button("Accept") { Task { await model.review(decision, .accepted) } }
                        .buttonStyle(.borderedProminent)
                        .accessibilityIdentifier("acceptButton")
                    Button("Return…") {
                        chosenOption = nil
                        composing = ""
                    }
                    .buttonStyle(.bordered)
                    .accessibilityIdentifier("returnButton")
                    Spacer()
                    Menu {
                        Button("Withdraw", role: .destructive) { Task { await model.review(decision, .withdrawn) } }
                    } label: {
                        Image(systemName: "ellipsis.circle")
                    }
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
                    .font(.subheadline)
            }
            TextField(
                decision.isQuestion ? String(localized: "Your answer") : String(localized: "Note for the asker"),
                text: text,
                axis: .vertical
            )
            .textFieldStyle(.roundedBorder)
            .lineLimit(1...6)
            .accessibilityIdentifier("composer")
            HStack(spacing: 8) {
                Button("Cancel") {
                    composing = nil
                    chosenOption = nil
                }
                .buttonStyle(.borderless)
                Spacer()
                switch decision.kind {
                case .question:
                    Button("Decline") { submit { await model.answer(decision, text: text.wrappedValue, declined: true) } }
                        .buttonStyle(.bordered)
                        .disabled(empty)
                        .accessibilityIdentifier("declineButton")
                    Button("Send") { submit { await model.answer(decision, text: text.wrappedValue) } }
                        .buttonStyle(.borderedProminent)
                        .disabled(empty)
                        .accessibilityIdentifier("sendButton")
                case .result:
                    if chosenOption == nil {
                        Button("Return") { submit { await model.review(decision, .returned, note: text.wrappedValue) } }
                            .buttonStyle(.bordered)
                            .disabled(empty)
                            .accessibilityIdentifier("confirmReturn")
                    }
                    Button("Accept") {
                        submit { await model.review(decision, .accepted, note: empty ? nil : text.wrappedValue, option: chosenOption) }
                    }
                    .buttonStyle(.borderedProminent)
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

struct VerificationChecks: View {
    let verification: TendVerification

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Label(verification.headline, systemImage: "checkmark.shield")
                .font(.caption.weight(.semibold))
            ForEach(Array(verification.checks.enumerated()), id: \.offset) { _, check in
                Label {
                    Text(verbatim: check.name ?? check.command).lineLimit(2)
                } icon: {
                    Image(systemName: check.passed ? "checkmark.circle.fill" : "xmark.circle.fill")
                        .foregroundStyle(check.passed ? TendColor.done : TendColor.error)
                }
                .font(.caption)
            }
        }
        .foregroundStyle(.secondary)
    }
}

struct EvidenceList: View {
    let evidence: [TendEvidence]
    let open: (URL) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            ForEach(Array(evidence.enumerated()), id: \.offset) { _, item in
                row(item)
            }
        }
        .font(.caption)
    }

    @ViewBuilder
    private func row(_ item: TendEvidence) -> some View {
        switch item {
        case .pullRequest(let url):
            if let link = URL(string: url), link.scheme == "https" {
                Button { open(link) } label: {
                    Label(url, systemImage: "arrow.triangle.pull").lineLimit(1).truncationMode(.middle)
                }
                .buttonStyle(.borderless)
            } else {
                Label(url, systemImage: "arrow.triangle.pull").foregroundStyle(.secondary)
            }
        case .commit(let sha, _):
            Label(String(sha.prefix(12)), systemImage: "smallcircle.filled.circle")
                .foregroundStyle(.secondary)
                .textSelection(.enabled)
        case .diff(let range, _):
            Label(range, systemImage: "plusminus").foregroundStyle(.secondary)
        case .test(let command, let passed, _):
            Label {
                Text(verbatim: command).lineLimit(2)
            } icon: {
                Image(systemName: passed == true ? "checkmark.circle" : passed == false ? "xmark.circle" : "circle")
                    .foregroundStyle(passed == true ? TendColor.done : passed == false ? TendColor.error : .secondary)
            }
            .foregroundStyle(.secondary)
        case .path(let path):
            Label(path, systemImage: "doc").lineLimit(1).truncationMode(.middle).foregroundStyle(.secondary)
        }
    }
}
