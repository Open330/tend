// What the app knows and does: the pairing, the queue per host, the
// history, and the two ways to decide. Every decision goes to the
// dashboard's own routes (`/api/decisions/*`), the ones `/needs` uses, so
// it is checked and audited there exactly like a decision from the web.
import Foundation
import Observation
import SwiftUI
import UIKit

/// One item waiting on the person, as the app draws it.
struct PendingDecision: Identifiable, Hashable, Sendable {
    enum Kind: Hashable, Sendable {
        case question(TendHumanRequestEntry)
        case result(TendReviewItem)
        case waiting(TendAgentWaitingEntry)
    }

    let kind: Kind
    let host: String
    let hostIsLocal: Bool
    /// Whether it can be answered or decided from here; otherwise it is
    /// shown and decided on its own host (an observe-only Fleet host, an
    /// older tend, or an agent waiting in its pane).
    let decidable: Bool

    var id: String {
        switch kind {
        case .question(let entry): "\(host)/q/\(entry.requestID)"
        case .result(let item): "\(host)/r/\(item.id)"
        case .waiting(let entry): "\(host)/w/\(entry.sessionID)"
        }
    }

    var isQuestion: Bool { if case .question = kind { true } else { false } }

    var decision: TendDecision? {
        switch kind {
        case .question(let entry): entry.decision
        case .result(let item): item.decision
        case .waiting: nil
        }
    }

    var title: String {
        switch kind {
        case .question(let entry):
            entry.decision?.needed?.nonEmpty ?? entry.body
        case .result(let item):
            item.decision?.needed?.nonEmpty ?? item.summary
        case .waiting(let entry):
            entry.message?.nonEmpty ?? String(localized: "Waiting in its pane")
        }
    }

    /// The asker's own words under the title, when the title is the
    /// decision asked for.
    var detail: String? {
        switch kind {
        case .question(let entry):
            entry.decision?.needed?.nonEmpty == nil ? nil : entry.body
        case .result(let item):
            item.decision?.needed?.nonEmpty == nil ? nil : item.summary
        case .waiting: nil
        }
    }

    var options: [String] { decision?.options ?? [] }

    var recommendation: String? {
        decision?.recommendation?.trimmingCharacters(in: .whitespacesAndNewlines).nonEmpty
    }

    var evidence: [TendEvidence] {
        switch kind {
        case .question(let entry): entry.decision?.evidence ?? []
        case .result(let item): item.allEvidence
        case .waiting: []
        }
    }

    var verification: TendVerification? {
        if case .result(let item) = kind { return item.verification }
        return nil
    }

    var asker: String? {
        switch kind {
        case .question(let entry): entry.fromAlias ?? entry.workID ?? entry.fromPane
        case .result(let item): item.target.text
        case .waiting(let entry): entry.agentKind
        }
    }

    var since: Date? {
        switch kind {
        case .question(let entry): Timestamp.parse(entry.createdAt)
        case .result(let item): item.requestedAt.flatMap(Timestamp.parse)
        case .waiting(let entry): Timestamp.parse(entry.since)
        }
    }

    static func from(_ section: HostQueue) -> [PendingDecision] {
        section.queue.items.map { item in
            switch item {
            case .humanRequest(let entry):
                PendingDecision(kind: .question(entry), host: section.alias, hostIsLocal: section.local, decidable: section.local || section.control)
            case .review(let review):
                PendingDecision(kind: .result(review), host: section.alias, hostIsLocal: section.local, decidable: section.local || section.decidesReviews)
            case .agentWaiting(let entry):
                PendingDecision(kind: .waiting(entry), host: section.alias, hostIsLocal: section.local, decidable: false)
            }
        }
    }
}

/// One past decision, as the app draws it.
struct PastDecision: Identifiable, Hashable, Sendable {
    let item: TendDecidedItem
    let host: String

    var id: String {
        switch item {
        case .review(let review): "\(host)/r/\(review.id)"
        case .humanAnswer(let answer): "\(host)/q/\(answer.request.requestID)"
        }
    }

    var title: String {
        switch item {
        case .review(let review): review.decision?.needed?.nonEmpty ?? review.summary
        case .humanAnswer(let answer): answer.request.decision?.needed?.nonEmpty ?? answer.request.body
        }
    }

    /// The Mac app's words: Accepted, Returned, Withdrawn, Answered, Declined.
    var outcome: String {
        switch item {
        case .review(let review):
            switch review.disposition?.kind {
            case "accepted": String(localized: "Accepted")
            case "returned": String(localized: "Returned")
            case "withdrawn": String(localized: "Withdrawn")
            default: review.disposition?.kind ?? String(localized: "Decided")
            }
        case .humanAnswer(let answer):
            answer.status == "completed" ? String(localized: "Answered") : String(localized: "Declined")
        }
    }

    var isPositive: Bool {
        switch item {
        case .review(let review): review.disposition?.kind == "accepted"
        case .humanAnswer(let answer): answer.status == "completed"
        }
    }

    /// The answer, or the note recorded with the disposition.
    var text: String? {
        switch item {
        case .review(let review): review.disposition?.note?.nonEmpty
        case .humanAnswer(let answer): answer.answer.nonEmpty
        }
    }

    var by: String? {
        if case .review(let review) = item { return review.disposition?.by.label }
        return nil
    }

    var at: Date? {
        switch item {
        case .review(let review): review.disposition.flatMap { Timestamp.parse($0.at) }
        case .humanAnswer(let answer): Timestamp.parse(answer.answeredAt)
        }
    }
}

enum Timestamp {
    static func parse(_ text: String) -> Date? {
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return fractional.date(from: text) ?? ISO8601DateFormatter().date(from: text)
    }
}

extension String {
    var nonEmpty: String? {
        let trimmed = trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : self
    }
}

@MainActor
@Observable
final class AppModel {
    enum Phase: Equatable {
        case unpaired
        /// Waiting for an operator to approve `userCode` on the dashboard.
        case waitingForApproval(origin: ServerOrigin, userCode: String, expires: Date)
        case paired(Pairing)
    }

    private(set) var phase: Phase
    private(set) var hosts: [HostQueue] = []
    private(set) var history: [HostHistory] = []
    private(set) var loaded = false
    private(set) var historyLoaded = false
    private(set) var refreshing = false
    /// Why the last read failed; the last good queue stays on screen.
    private(set) var problem: String?
    private(set) var pairingProblem: String?
    /// What the last decision did, said once ("Answered.", "Accepted.").
    var outcome: String?
    /// Per card: why its decision failed.
    private(set) var failures: [String: String] = [:]
    private(set) var busy: Set<String> = []

    private var pairingTask: Task<Void, Never>?

    init() {
        if ProcessInfo.processInfo.arguments.contains("-TendResetState") {
            CredentialStore.clear()
        }
        phase = CredentialStore.load().map(Phase.paired) ?? .unpaired
    }

    var pairing: Pairing? {
        if case .paired(let pairing) = phase { return pairing }
        return nil
    }

    private var client: DashboardClient? {
        pairing.map { DashboardClient(origin: $0.origin, token: $0.token) }
    }

    var pending: [PendingDecision] { hosts.flatMap(PendingDecision.from) }

    /// What the person can act on from here, for the badge.
    var actionableCount: Int { pending.filter(\.decidable).count }

    var past: [PastDecision] {
        history
            .flatMap { section in section.history.items.map { PastDecision(item: $0, host: section.alias) } }
            .sorted { ($0.at ?? .distantPast) > ($1.at ?? .distantPast) }
    }

    // MARK: Pairing

    func startPairing(address: String, name: String) {
        pairingProblem = nil
        let origin: ServerOrigin
        do {
            origin = try ServerOrigin(parsing: address)
        } catch {
            pairingProblem = error.localizedDescription
            return
        }
        let deviceName = name.trimmingCharacters(in: .whitespacesAndNewlines).nonEmpty ?? UIDevice.current.name
        let platform = UIDevice.current.userInterfaceIdiom == .pad ? "ipados" : "ios"
        pairingTask?.cancel()
        pairingTask = Task { [weak self] in
            let client = DashboardClient(origin: origin)
            do {
                let request = try await client.requestPairing(name: deviceName, platform: platform)
                guard let self, !Task.isCancelled else { return }
                let expires = Date().addingTimeInterval(TimeInterval(request.expiresIn))
                self.phase = .waitingForApproval(origin: origin, userCode: request.userCode, expires: expires)
                let interval = UInt64(max(request.interval, 1))
                while !Task.isCancelled, Date() < expires {
                    try await Task.sleep(nanoseconds: interval * 1_000_000_000)
                    if case .issued(let device, let token) = try await client.pollPairing(deviceCode: request.deviceCode) {
                        let pairing = Pairing(origin: origin, device: device, token: token)
                        guard CredentialStore.save(pairing) else {
                            self.fail(String(localized: "Could not keep the key in this device's Keychain."))
                            return
                        }
                        self.phase = .paired(pairing)
                        await self.refresh()
                        return
                    }
                }
                if !Task.isCancelled {
                    self.fail(String(localized: "The code expired before it was approved. Start again."))
                }
            } catch is CancellationError {
                return
            } catch {
                self?.fail(error.localizedDescription)
            }
        }
    }

    private func fail(_ message: String) {
        pairingProblem = message
        phase = .unpaired
    }

    func cancelPairing() {
        pairingTask?.cancel()
        pairingTask = nil
        phase = .unpaired
    }

    /// Unpair on the dashboard, then forget the key here. The key is
    /// forgotten even when the dashboard cannot be reached.
    func signOut() async {
        if let client { try? await client.forget() }
        forgetLocally()
    }

    private func forgetLocally() {
        CredentialStore.clear()
        phase = .unpaired
        hosts = []
        history = []
        loaded = false
        historyLoaded = false
        problem = nil
    }

    private func handle(_ error: Error) -> String {
        if case DashboardError.signedOut = error {
            forgetLocally()
            pairingProblem = DashboardError.signedOut.localizedDescription
        }
        return error.localizedDescription
    }

    // MARK: Reading

    /// A refresh asked for while one was running: run once more after it,
    /// so a decision never leaves a stale card behind.
    private var refreshAgain = false

    func refresh() async {
        guard !refreshing else {
            refreshAgain = true
            return
        }
        refreshing = true
        defer { refreshing = false }
        repeat {
            refreshAgain = false
            guard let client else { return }
            do {
                hosts = try await client.queue()
                loaded = true
                problem = nil
            } catch {
                problem = handle(error)
            }
        } while refreshAgain
    }

    func refreshHistory() async {
        guard let client else { return }
        do {
            history = try await client.history()
            historyLoaded = true
        } catch {
            problem = handle(error)
        }
    }

    // MARK: Deciding

    private func act(_ decision: PendingDecision, done: String, _ call: (DashboardClient) async throws -> Void) async -> Bool {
        guard let client else { return false }
        busy.insert(decision.id)
        defer { busy.remove(decision.id) }
        do {
            try await call(client)
            failures[decision.id] = nil
            outcome = done
            await refresh()
            await refreshHistory()
            return true
        } catch {
            failures[decision.id] = handle(error)
            return false
        }
    }

    func answer(_ decision: PendingDecision, text: String, declined: Bool = false) async -> Bool {
        guard case .question(let entry) = decision.kind else { return false }
        let host = decision.hostIsLocal ? nil : decision.host
        return await act(decision, done: declined ? String(localized: "Declined.") : String(localized: "Answered.")) {
            try await $0.answer(requestID: entry.requestID, answer: text, declined: declined, host: host)
        }
    }

    func review(_ decision: PendingDecision, _ disposition: TendReviewDisposition, note: String? = nil, option: String? = nil) async -> Bool {
        guard case .result(let item) = decision.kind else { return false }
        let host = decision.hostIsLocal ? nil : decision.host
        let done = switch disposition {
        case .accepted: String(localized: "Accepted.")
        case .returned: String(localized: "Returned.")
        case .withdrawn: String(localized: "Withdrawn.")
        }
        return await act(decision, done: done) {
            try await $0.review(id: item.id, disposition: disposition, note: note, option: option, host: host)
        }
    }
}
