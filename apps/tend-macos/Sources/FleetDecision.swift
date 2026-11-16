// Home's aggregation of the decision queue across hosts: one
// `FleetDecision` per pending item. The wire models it is built from live
// in apps/shared/Sources/DecisionModels.swift, shared with the iOS app.
import Foundation

// MARK: - What Home shows

/// One decision waiting on the person, from the daemon's queue or, for
/// another host or a daemon without the queue, from the console mailbox
/// and the review list.
struct FleetDecision: Identifiable, Hashable, Sendable {
    enum Source: Hashable, Sendable {
        /// Answered with a human-request reply on `host`.
        case humanRequest(requestID: String)
        /// Decided with `review_dispose` on this Mac's daemon.
        case review(TendReviewItem)
    }

    let source: Source
    let host: TendFleetHostIdentity
    /// The question or the result to look at, as the asker wrote it.
    let summary: String
    let decision: TendDecision?
    /// Who asked, as they declared it.
    let asker: String?
    /// The asking agent's session, to find its checkout and its pane.
    let askerSessionID: String?
    let workID: String?
    let since: Date?
    let evidence: [TendEvidence]
    /// tendd opened it itself (peer recovery, a reply for a gone sender),
    /// not an agent: listed after agents' questions and results.
    var isSystemAsk = false
    /// The asking agent as Home sees it, when it is on the board.
    var askerAgentID: String?
    var askerCwd: String?
    /// The asking agent's session is not on its host's board as Home last
    /// read it. Not proof that it exited (a new asker can be missing from a
    /// board read before it started), so see `asksUnlisted(at:)`.
    var askerUnlisted = false

    /// How long a question must have waited before an unlisted asker is
    /// shown: longer than any board refresh, so a fresh asker is not
    /// flagged before its first snapshot.
    static let unlistedGrace: TimeInterval = 10 * 60

    /// Whether to tell the person the asker is not listed: unlisted, and
    /// asked longer ago than any board refresh takes.
    func asksUnlisted(at now: Date) -> Bool {
        guard askerUnlisted, let since else { return false }
        return now.timeIntervalSince(since) >= Self.unlistedGrace
    }

    /// The asking agent's project, for Home's project filter.
    var repo: String? {
        askerCwd.map { ($0 as NSString).lastPathComponent }
    }

    var id: String {
        switch source {
        case .humanRequest(let requestID): "human:\(host.alias):\(requestID)"
        case .review(let item): "review:\(item.id)"
        }
    }

    var isHumanRequest: Bool {
        if case .humanRequest = source { return true }
        return false
    }

    /// What to decide: the asker's question when they gave one, the summary
    /// otherwise.
    var title: String {
        if let needed = decision?.needed?.trimmingCharacters(in: .whitespacesAndNewlines), !needed.isEmpty {
            return needed
        }
        return summary
    }

    /// The summary under the title, when the title is the question.
    var detail: String? {
        guard title != summary else { return nil }
        let trimmed = summary.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    var options: [String] {
        (decision?.options ?? []).filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
    }

    var recommendation: String? {
        decision?.recommendation?.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty
    }

    /// The checks tendd ran itself, shown ahead of the asker's account.
    var verification: TendVerification? {
        guard case .review(let item) = source, let verification = item.verification, !verification.checks.isEmpty else { return nil }
        return verification
    }

    init(humanRequest entry: TendHumanRequestEntry, host: TendFleetHostIdentity) {
        source = .humanRequest(requestID: entry.requestID)
        self.host = host
        summary = entry.body
        decision = entry.decision
        asker = entry.fromAlias.map { "@\($0)" } ?? (entry.fromPane.isEmpty ? nil : entry.fromPane)
        askerSessionID = entry.fromSessionID.nilIfEmpty
        workID = entry.workID
        since = TendUsageTimestamp.parse(entry.createdAt)
        evidence = entry.decision?.evidence ?? []
        isSystemAsk = entry.system == true || Self.tendServices.contains(entry.fromSessionID)
    }

    /// The services an older tendd's own items come from.
    static let tendServices: Set<String> = ["tend:peer-recovery", "muxa:peer-recovery", "tend:reply-wake", "muxa:reply-wake"]

    init(humanRequest message: TendOperatorMessage) {
        let request = message.request
        source = .humanRequest(requestID: request.id)
        host = message.host
        summary = request.body
        decision = request.decision
        asker = request.from.label
        askerSessionID = request.from.agentSessionID.nilIfEmpty
        workID = request.workID
        since = TendUsageTimestamp.parse(request.createdAt)
        evidence = request.decision?.evidence ?? []
        isSystemAsk = Self.tendServices.contains(request.from.agentSessionID)
    }

    init(review item: TendReviewItem, host: TendFleetHostIdentity) {
        source = .review(item)
        self.host = host
        summary = item.summary
        decision = item.decision
        asker = item.requestedBy.label ?? (item.requestedBy.kind == "agent" ? String(localized: "an agent") : String(localized: "a person"))
        askerSessionID = item.target.sessionID
        workID = item.target.work
        since = item.requestedAt.flatMap(TendUsageTimestamp.parse)
        evidence = item.allEvidence
    }
}

enum FleetDecisions {
    /// The queue Home shows. With the daemon's queue: its human requests and
    /// review items in its order (waiting agents are already Work cards),
    /// then the console mailbox's human requests it does not hold: other
    /// hosts', and this Mac's only when the queue read was partial (a source
    /// unavailable, truncated, an entry this app could not decode). A
    /// complete queue is authoritative for this Mac: an older mailbox copy
    /// of a request answered since never comes back. Without
    /// it: this Mac's open review items, then every host's human requests.
    /// One request is listed once.
    static func merge(
        queue: TendPendingQueue?,
        reviews: [TendReviewItem],
        messages: [TendOperatorMessage],
        localHost: TendFleetHostIdentity
    ) -> [FleetDecision] {
        var result: [FleetDecision] = []
        var seen = Set<String>()
        func add(_ decision: FleetDecision) {
            if seen.insert(decision.id).inserted { result.append(decision) }
        }
        if let queue {
            for item in queue.items {
                switch item {
                case .humanRequest(let entry): add(FleetDecision(humanRequest: entry, host: localHost))
                case .review(let review): add(FleetDecision(review: review, host: localHost))
                case .agentWaiting: break
                }
            }
        } else {
            reviews.forEach { add(FleetDecision(review: $0, host: localHost)) }
        }
        let pending = messages
            .filter { $0.isHumanRequest && $0.request.needsHumanResponse }
            .filter { queue?.isComplete != true || $0.host.alias != localHost.alias }
            .sorted { $0.request.createdAt < $1.request.createdAt }
        pending.forEach { add(FleetDecision(humanRequest: $0)) }
        // tendd's own items after everything an agent asked or handed in,
        // whichever host or daemon version listed them.
        return result.filter { !$0.isSystemAsk } + result.filter(\.isSystemAsk)
    }
}

private extension String {
    var nilIfEmpty: String? { isEmpty ? nil : self }
}
