// Home's decisions: the daemon's queue drawn as cards in Your turn and the
// review sheet. A card says what to decide, offers the asker's options and
// recommendation, links the evidence, and records the answer with tendd
// (a human-request reply, or a review disposition).
import AppKit
import Combine
import SwiftUI

// MARK: - Feed

/// Reads this Mac's decision queue, or the review list from a daemon
/// without it, and records what the person decides. Other hosts' human
/// requests come from the console mailbox the app already reads; their open
/// review items from their Fleet queues, decided on the dashboard.
@MainActor
final class FleetDecisionFeed: ObservableObject {
    @Published private(set) var queue: TendPendingQueue? { didSet { sourceRevision &+= 1 } }
    @Published private(set) var reviews: [TendReviewItem] = [] { didSet { sourceRevision &+= 1 } }
    /// Other Fleet hosts' queues, for their review items; read at most every
    /// `remoteEvery` seconds, kept between reads.
    @Published private(set) var remoteQueues: [(host: TendFleetHostIdentity, queue: TendPendingQueue)] = [] {
        didSet { sourceRevision &+= 1 }
    }
    /// The dashboard tendd named, where another host's review is decided.
    @Published private(set) var dashboard: URL?
    private var lastRemoteLoad = Date.distantPast
    static let remoteEvery: TimeInterval = 15
    /// Bumped with every change of `queue`, `reviews` or `remoteQueues`.
    private var sourceRevision: UInt64 = 0
    /// The last merged decisions and what they were built from: Home asks
    /// on every redraw, and they change only when one of these does.
    private var cachedDecisions: (key: DecisionsKey, value: [FleetDecision])?
    /// How many times the decisions were merged, for tests.
    private(set) var decisionBuilds = 0

    private struct DecisionsKey: Equatable {
        let source: UInt64
        let messages: UInt64
        let agents: UInt64
        let localHost: TendFleetHostIdentity
    }
    @Published private(set) var problem: String?
    /// Decisions being recorded, by id.
    @Published private(set) var busy: Set<String> = []
    /// The last error recording a decision, by id.
    @Published var failures: [String: String] = [:]
    /// The open answer and the text typed so far, per card: a keystroke
    /// redraws its card, not every view that watches the feed.
    let drafts = FleetDecisionDrafts()
    /// Past decisions on this Mac's daemon, for the history view.
    @Published private(set) var history: TendDecisionHistory?
    /// Why the last history read failed; the view offers a retry.
    @Published private(set) var historyError: String?
    /// The daemon has no `decision_history` (an older tendd).
    @Published private(set) var historyUnsupported = false
    @Published private(set) var historyLoading = false
    /// Results marked Looks Good in Home that tendd holds a record of; nil
    /// while the daemon cannot record or report them, when Home keeps its
    /// own marks instead.
    @Published private(set) var reviewedKeys: Set<String>?
    /// Looks Good sent and not yet seen in the history.
    var pendingKeys: Set<String> = []
    var lastHistoryLoad = Date.distantPast
    var lastReviewedLoad = Date.distantPast
    /// Bumped by every read of the review records and every record, so a
    /// read that started earlier never overwrites a later answer.
    var reviewedGeneration = 0
    /// Where an unfinished read of the review records stopped, and what it
    /// found so far.
    private var scan: (offset: Int, keys: Set<String>)?
    static let historyEvery: TimeInterval = 30
    private var loading = false

    static let refreshEvery: Duration = .seconds(5)
    private var perfSubscription: AnyCancellable?

    init() {
        perfSubscription = TendPerf.observe(self, "decision.publish")
    }

    /// Reads the open review items of every Fleet host Home lists them for
    /// (`FleetDecisions.decidesReviews`), and the dashboard to decide them on.
    func loadRemote(_ client: TendIPCClient, hosts: [TendFleetHost], force: Bool = false) async {
        guard force || Date().timeIntervalSince(lastRemoteLoad) >= Self.remoteEvery else { return }
        lastRemoteLoad = Date()
        let link = await client.dashboardURL
        if dashboard != link { dashboard = link }
        var queues: [(host: TendFleetHostIdentity, queue: TendPendingQueue)] = []
        for host in hosts where FleetDecisions.decidesReviews(host) {
            // A host that cannot be read now shows nothing to act on.
            if let queue = try? await client.remoteDecisionQueue(host: host.identity) {
                queues.append((host.identity, queue))
            }
        }
        // Nothing before or after: no publish, no rebuilt queue.
        if !(queues.isEmpty && remoteQueues.isEmpty) { remoteQueues = queues }
    }

    func load(_ client: TendIPCClient, works: [FleetWork] = []) async {
        guard !loading else { return }
        TendPerf.count("decision.poll")
        loading = true
        defer { loading = false }
        do {
            // Polled every few seconds while Home is open: assign only what
            // changed, so an unchanged queue does not redraw the board.
            if let queue = try await client.decisionQueue() {
                if self.queue != queue { self.queue = queue }
                if !reviews.isEmpty { reviews = [] }
                setProblem(queue.unavailable.isEmpty
                    ? nil
                    : String(localized: "Some decisions could not be read: \(queue.unavailable.joined(separator: "; "))"))
            } else {
                if queue != nil { queue = nil }
                let page = try await client.reviewItems(offset: 0, limit: 200)
                if reviews != page.items { reviews = page.items }
                setProblem(nil)
            }
        } catch {
            setProblem(String(localized: "Could not read decisions: \(error.localizedDescription)"))
        }
        // Without the board there is nothing to decide what to hide for.
        if !works.isEmpty { await loadReviewed(client, works: works, force: false) }
    }

    private func setProblem(_ problem: String?) {
        if self.problem != problem { self.problem = problem }
    }

    /// Reads the history (for the history view) at most every
    /// `historyEvery` seconds, or now.
    func loadHistory(_ client: TendIPCClient, force: Bool) async {
        guard await client.supports(tendDecisionQueueCapability) else {
            if history != nil { history = nil }
            if !historyUnsupported { historyUnsupported = true }
            if historyError != nil { historyError = nil }
            return
        }
        if historyUnsupported { historyUnsupported = false }
        guard force || Date().timeIntervalSince(lastHistoryLoad) >= Self.historyEvery else { return }
        lastHistoryLoad = Date()
        historyLoading = true
        defer { historyLoading = false }
        do {
            guard let history = try await client.decisionHistory() else { return }
            if self.history != history { self.history = history }
            if historyError != nil { historyError = nil }
        } catch {
            // The last good history stays on screen under the error.
            historyError = String(localized: "Could not read the decision history: \(error.localizedDescription)")
        }
    }

    /// Which of the board's finished results tendd recorded as Looks Good.
    /// Reads every review item newest first, page by page, back to the
    /// oldest finished result on the board (a review of it cannot be older
    /// than its finish), so nothing that still matters falls out of a
    /// window. An overlapping older read never replaces a newer one.
    func loadReviewed(_ client: TendIPCClient, works: [FleetWork], force: Bool) async {
        guard await client.supports("review_record_v1"), await client.supports("review_items_v1") else {
            if reviewedKeys != nil { reviewedKeys = nil }
            return
        }
        guard force || Date().timeIntervalSince(lastReviewedLoad) >= Self.historyEvery else { return }
        lastReviewedLoad = Date()
        reviewedGeneration += 1
        let generation = reviewedGeneration
        let cutoff = FleetDecisionRecord.cutoff(for: works)
        // A scan that ran out of pages last time goes on where it stopped:
        // newer records only push older ones further down, so nothing is
        // skipped, at worst read twice.
        var recorded = scan?.keys ?? []
        var offset = scan?.offset ?? 0
        var complete = false
        do {
            for _ in 0..<FleetDecisionRecord.maxReviewPages {
                let page = try await client.reviewPage(offset: offset, limit: FleetDecisionRecord.reviewPageSize)
                recorded.formUnion(FleetDecisionRecord.reviewedKeys(in: page.items))
                offset += page.items.count
                let oldest = page.items.last?.requestedAt.flatMap(TendUsageTimestamp.parse)
                if !page.hasMore || page.items.isEmpty { complete = true; break }
                if let cutoff, let oldest, oldest < cutoff { complete = true; break }
            }
        } catch {
            TendLog.app.warning("reading review records failed: \(error.localizedDescription, privacy: .public)")
            return
        }
        guard generation == reviewedGeneration else { return }
        pendingKeys.subtract(recorded)
        let keys: Set<String>
        if complete {
            scan = nil
            keys = recorded.union(pendingKeys)
        } else {
            // Unread records may still hold a review: what is known stays
            // hidden, and nothing is unhidden until the scan completes.
            scan = (offset, recorded)
            // Go on at the next refresh (seconds away), not in 30 s.
            lastReviewedLoad = .distantPast
            keys = recorded.union(pendingKeys).union(reviewedKeys ?? [])
        }
        if reviewedKeys != keys { reviewedKeys = keys }
    }

    /// Whether the last read of the review records reached every one that
    /// could matter.
    var reviewedIsComplete: Bool { scan == nil }

    /// Looks Good, recorded by tendd. Hidden at once; shown again with the
    /// reason if the record fails.
    func recordLooksGood(_ work: FleetWork, client: TendIPCClient, works: [FleetWork]) async {
        guard let target = FleetBoard.reviewTarget(work) else { return }
        let key = FleetBoard.reviewKey(work)
        pendingKeys.insert(key)
        reviewedKeys?.insert(key)
        // A read already under way predates this record.
        reviewedGeneration += 1
        do {
            let recorded = try await client.recordDecision(
                target: target, key: key, summary: FleetDecisionRecord.summary(work), disposition: .accepted, note: nil
            )
            if !recorded { throw TendIPCError.server(String(localized: "tendd cannot record reviews; update tend and restart tendd")) }
        } catch {
            pendingKeys.remove(key)
            reviewedKeys?.remove(key)
            problem = String(localized: "Could not record Looks Good: \(error.localizedDescription)")
        }
        await loadReviewed(client, works: works, force: true)
    }

    /// Records the results the person marked reviewed in this Mac's
    /// settings before tendd kept the record, once. Only marks that still
    /// hide a unit on the board mean anything; the rest are dropped. True
    /// only when every mark that mattered was recorded.
    func migrate(_ marks: FleetReviewMarks, works: [FleetWork], client: TendIPCClient) async -> Bool {
        var succeeded = true
        for work in works where marks.marks[work.id] == work.revision && work.lane == .finished {
            guard let target = FleetBoard.reviewTarget(work) else { continue }
            do {
                let recorded = try await client.recordDecision(
                    target: target, key: FleetBoard.reviewKey(work), summary: FleetDecisionRecord.summary(work),
                    disposition: .accepted, note: nil
                )
                // The daemon stopped recording (a reconnect to an older one):
                // keep the marks for the next try.
                if !recorded { succeeded = false }
            } catch {
                succeeded = false
                TendLog.app.warning("moving a review mark to tendd failed: \(error.localizedDescription, privacy: .public)")
            }
        }
        await loadReviewed(client, works: works, force: true)
        return succeeded
    }

    /// What Home shows, oldest-blocked first, each with the asking agent's
    /// checkout when Home knows it.
    func decisions(_ model: AppModel) -> [FleetDecision] {
        guard let local = model.fleetHosts.first(where: \.local)?.identity else { return [] }
        let key = DecisionsKey(
            source: sourceRevision, messages: model.operatorMessagesRevision,
            agents: model.askersRevision, localHost: local
        )
        if let cachedDecisions, cachedDecisions.key == key { return cachedDecisions.value }
        decisionBuilds += 1
        TendPerf.count("decision.merge")
        let merged = FleetDecisions.merge(queue: queue, reviews: reviews, messages: model.operatorMessages, localHost: local)
            + FleetDecisions.remoteReviews(remoteQueues)
        let value = FleetDecisions.attachAskers(merged, index: model.hostedAgentIndex)
        cachedDecisions = (key, value)
        drafts.prune(keeping: Set(value.map(\.id)))
        return value
    }

    /// The open answer for `id`: one composer at a time across Home and the
    /// Review sheet, so a keyboard Send can only reach the one on screen.
    func draft(for id: String) -> Binding<String?> {
        drafts.binding(for: id)
    }

    /// Replies to a human request as the person; tendd records the answer.
    func answer(_ decision: FleetDecision, text: String, model: AppModel) async -> Bool {
        guard case .humanRequest(let requestID) = decision.source else { return false }
        guard let route = model.consoleRoute(hostAlias: decision.host.alias) else {
            failures[decision.id] = String(localized: "\(decision.host.alias) is not connected.")
            return false
        }
        return await record(decision, model: model) {
            try await model.client.replyCollaboration(
                host: route.host, pane: route.pane, requestID: requestID, status: "completed", body: text
            )
            await model.refreshOperatorInbox(force: false, hostAliases: [decision.host.alias])
        }
    }

    /// Declines a human request without an answer: the asker, if it is
    /// still there, reads that the person dismissed it; tendd's own items
    /// record it as their decision. Either way it leaves every client's
    /// queue, on the host that holds it.
    func dismiss(_ decision: FleetDecision, model: AppModel) async -> Bool {
        guard case .humanRequest(let requestID) = decision.source else { return false }
        guard let route = model.consoleRoute(hostAlias: decision.host.alias) else {
            failures[decision.id] = String(localized: "\(decision.host.alias) is not connected.")
            return false
        }
        return await record(decision, model: model) {
            try await model.client.replyCollaboration(
                host: route.host, pane: route.pane, requestID: requestID, status: "declined",
                body: FleetDecisions.dismissedBody
            )
            await model.refreshOperatorInbox(force: false, hostAliases: [decision.host.alias])
        }
    }

    /// Accepts, returns or withdraws a review item on this Mac's daemon.
    /// Another host's is decided on the dashboard, never from here.
    func decide(_ decision: FleetDecision, _ disposition: TendReviewDisposition, note: String?, model: AppModel) async -> Bool {
        guard case .review(let item) = decision.source else { return false }
        guard !decision.decidedOnDashboard else {
            failures[decision.id] = String(localized: "Decide this result on the dashboard.")
            return false
        }
        return await record(decision, model: model) {
            try await model.client.decideReview(id: item.id, disposition: disposition, note: note)
        }
    }

    private func record(_ decision: FleetDecision, model: AppModel, _ body: () async throws -> Void) async -> Bool {
        busy.insert(decision.id)
        failures[decision.id] = nil
        defer { busy.remove(decision.id) }
        var succeeded = true
        do {
            try await body()
        } catch {
            failures[decision.id] = error.localizedDescription
            succeeded = false
        }
        // Either way, show where it stands now: decided here, or already
        // decided elsewhere.
        await load(model.client)
        return succeeded
    }
}

/// One decision's answer: open or not, and the text typed so far, which a
/// closed answer keeps for when it is reopened. Each card watches only its
/// own, so typing redraws one card.
@MainActor
final class FleetDecisionDraft: ObservableObject {
    /// For cards with no shared draft (previews, tests): never changes.
    static let detached = FleetDecisionDraft(id: "", owner: nil)

    let id: String
    fileprivate weak var owner: FleetDecisionDrafts?
    @Published fileprivate(set) var isOpen = false
    @Published fileprivate(set) var text: String?

    fileprivate init(id: String, owner: FleetDecisionDrafts?) {
        self.id = id
        self.owner = owner
    }

    /// The answer field's text, nil while it is closed.
    var value: String? { isOpen ? (text ?? "") : nil }

    func update(_ value: String?) {
        owner?.set(value, for: id)
    }
}

/// Every card's answer, with one open at a time across Home and the Review
/// sheet.
@MainActor
final class FleetDecisionDrafts {
    private var rows: [String: FleetDecisionDraft] = [:]
    private var openID: String?

    func row(for id: String) -> FleetDecisionDraft {
        if let row = rows[id] { return row }
        let row = FleetDecisionDraft(id: id, owner: self)
        rows[id] = row
        return row
    }

    func binding(for id: String) -> Binding<String?> {
        let row = row(for: id)
        return Binding(get: { row.value }, set: { row.update($0) })
    }

    /// The decisions with a row, for tests.
    var ids: Set<String> { Set(rows.keys) }

    /// Forgets the rows of decisions that are gone, so answered and
    /// withdrawn decisions do not pile up for the app's lifetime. The open
    /// answer stays even if its decision is missing from one merge: a
    /// queue that briefly fails to list it must not lose what is typed.
    func prune(keeping ids: Set<String>) {
        guard rows.keys.contains(where: { !ids.contains($0) && $0 != openID }) else { return }
        rows = rows.filter { ids.contains($0.key) || $0.key == openID }
    }

    fileprivate func set(_ value: String?, for id: String) {
        let row = row(for: id)
        if let value {
            // Reopening keeps what was typed before; an option (a non-empty
            // value) replaces it on purpose.
            let reopening = openID != id && value.isEmpty
            if openID != id {
                if let openID, let open = rows[openID], open.isOpen { open.isOpen = false }
                openID = id
            }
            if !(reopening && row.text != nil), row.text != value { row.text = value }
            if !row.isOpen { row.isOpen = true }
        } else if openID == id {
            openID = nil
            if row.isOpen { row.isOpen = false }
            if row.text != nil { row.text = nil }
        }
    }
}

extension FleetDecisions {
    /// The reply a dismissed question gets. In English: it is read by the
    /// asking agent, not shown to the person.
    static let dismissedBody = "The person dismissed this question from Tend Home without answering it. Do not wait for an answer to it; ask again only if it still matters."

    /// Fills in each decision's asking agent (its checkout and repository)
    /// from the agents Home sees on the same host.
    static func attachAskers(_ decisions: [FleetDecision], agents: [TendHostedAgent]) -> [FleetDecision] {
        attachAskers(decisions, index: TendHostedAgentIndex(agents))
    }

    static func attachAskers(_ decisions: [FleetDecision], index: TendHostedAgentIndex) -> [FleetDecision] {
        decisions.map { decision in
            guard let session = decision.askerSessionID else { return decision }
            var decision = decision
            guard let agent = index.agent(host: decision.host.alias, session: session) else {
                decision.askerUnlisted = isUnlisted(decision, session: session, index: index)
                return decision
            }
            decision.askerAgentID = agent.id
            decision.askerCwd = agent.agent.cwd
            return decision
        }
    }

    /// An agent's own question whose session its host does not list. Only
    /// on a host whose agents Home has (a disconnected host lists none), and
    /// never for tendd's own items or an address still waiting for its
    /// session (`pending-pane:`), which no agent row carries.
    static func isUnlisted(_ decision: FleetDecision, session: String, index: TendHostedAgentIndex) -> Bool {
        decision.isHumanRequest && !decision.isSystemAsk
            && !session.hasPrefix("pending-pane:")
            && index.hosts.contains(decision.host.alias)
    }
}

extension AppModel {
    /// The host and a pane on it to reach its console mailbox through, the
    /// way the Inbox reads it.
    func consoleRoute(hostAlias: String) -> (host: TendFleetHostIdentity, pane: TendPaneInfo)? {
        if let watch = executionSnapshot.watchHosts.first(where: { $0.host.alias == hostAlias }) {
            let pane = watch.sessions.flatMap(\.windows).flatMap(\.panes).first?.pane ?? .operatorRoute
            return (watch.host.identity, pane)
        }
        guard let host = fleetHosts.first(where: { $0.alias == hostAlias }) else { return nil }
        return (host.identity, .operatorRoute)
    }
}

// MARK: - Evidence

/// A recorded revision to open in Changes.
struct FleetRevisionTarget: Identifiable, Hashable, Sendable {
    let repo: String
    let revision: ChangesRevision
    var id: String { "\(repo)\n\(revision.label)" }
}

/// What one click on a piece of evidence does.
enum FleetEvidenceAction: Equatable, Sendable {
    case changes(FleetRevisionTarget)
    case url(URL)
    case reveal(String)
    /// Shown, not openable from here, with the reason.
    case unavailable(String)

    /// Commits and ranges open in Changes on this Mac's checkout: the one
    /// the asker recorded, else the asking agent's working directory. A
    /// pull request opens in the browser; a path is revealed in Finder.
    static func of(_ evidence: TendEvidence, decision: FleetDecision) -> FleetEvidenceAction {
        func revision(_ revision: ChangesRevision?, repo: String?) -> FleetEvidenceAction {
            guard let revision else { return .unavailable(String(localized: "Not a revision Tend can read")) }
            guard revision.comparesCommits else { return .unavailable(ChangesRevision.workingTreeReason) }
            guard decision.host.local else {
                return .unavailable(String(localized: "On \(decision.host.alias); Changes reads this Mac's checkouts"))
            }
            guard let repo = (repo ?? decision.askerCwd)?.trimmingCharacters(in: .whitespaces), repo.hasPrefix("/") else {
                return .unavailable(String(localized: "No checkout recorded"))
            }
            return .changes(FleetRevisionTarget(repo: repo, revision: revision))
        }
        switch evidence {
        case .commit(let sha, let repo):
            return revision(ChangesRevision(commit: sha), repo: repo)
        case .diff(let range, let repo):
            return revision(ChangesRevision(range: range), repo: repo)
        case .pullRequest(let url):
            guard let url = URL(string: url), ["https", "http"].contains(url.scheme?.lowercased() ?? "") else {
                return .unavailable(String(localized: "Not a web address"))
            }
            return .url(url)
        case .path(let path):
            guard decision.host.local else {
                return .unavailable(String(localized: "On \(decision.host.alias); Changes reads this Mac's checkouts"))
            }
            guard path.hasPrefix("/") else { return .unavailable(String(localized: "Not an absolute path")) }
            return .reveal(path)
        case .test(let command, let passed, let tail):
            let verdict = switch passed {
            case true?: String(localized: "Passed")
            case false?: String(localized: "Failed")
            case nil: String(localized: "Result not recorded")
            }
            return .unavailable([verdict + ": " + command, tail].compactMap { $0 }.joined(separator: "\n\n"))
        }
    }
}

/// The evidence as small chips; a chip that opens something is a button.
struct FleetEvidenceChips: View {
    let decision: FleetDecision
    let openRevision: (FleetRevisionTarget) -> Void

    var body: some View {
        PipelineFlowLayout(spacing: 5) {
            ForEach(Array(decision.evidence.enumerated()), id: \.offset) { _, evidence in
                chip(evidence, action: FleetEvidenceAction.of(evidence, decision: decision))
            }
        }
    }

    private func tint(_ evidence: TendEvidence) -> Color {
        if case .test(_, let passed, _) = evidence {
            switch passed {
            case true?: return .green
            case false?: return .red
            case nil: return .secondary
            }
        }
        return .accentColor
    }

    @ViewBuilder
    private func chip(_ evidence: TendEvidence, action: FleetEvidenceAction) -> some View {
        let label = Label {
            Text(verbatim: evidence.label)
                .lineLimit(1)
                .truncationMode(.middle)
        } icon: {
            Image(systemName: evidence.systemImage)
        }
        .font(.system(size: 11, weight: .medium))
        .frame(maxWidth: 220)
        .fixedSize(horizontal: false, vertical: true)
        .padding(.horizontal, 7)
        .padding(.vertical, 2)
        switch action {
        case .unavailable(let reason):
            label
                .foregroundStyle(tint(evidence) == .accentColor ? Color.secondary : tint(evidence))
                .background(Color.secondary.opacity(0.1), in: Capsule())
                .help(reason)
        case .changes(let target):
            Button { openRevision(target) } label: {
                label.background(Color.accentColor.opacity(0.12), in: Capsule())
            }
            .buttonStyle(.plain)
            .foregroundStyle(Color.accentColor)
            .help(String(localized: "Open \(target.revision.label) in Changes"))
        case .url(let url):
            Button { NSWorkspace.shared.open(url) } label: {
                label.background(Color.accentColor.opacity(0.12), in: Capsule())
            }
            .buttonStyle(.plain)
            .foregroundStyle(Color.accentColor)
            .help(url.absoluteString)
        case .reveal(let path):
            Button { NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)]) } label: {
                label.background(Color.accentColor.opacity(0.12), in: Capsule())
            }
            .buttonStyle(.plain)
            .foregroundStyle(Color.accentColor)
            .help(path)
        }
    }
}

// MARK: - Checks

/// The Work's declared checks as tendd ran them: a headline, then one line
/// per check; a failed check shows how it ended and the end of its output.
struct FleetVerificationChecks: View {
    let verification: TendVerification

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Label {
                Text(verbatim: verification.headline)
                    .fontWeight(.semibold)
            } icon: {
                Image(systemName: verification.passed ? "checkmark.seal.fill" : "xmark.seal.fill")
                    .foregroundStyle(verification.passed ? .green : .red)
            }
            ForEach(Array(verification.checks.enumerated()), id: \.offset) { _, check in
                VStack(alignment: .leading, spacing: 1) {
                    Label {
                        Text(verbatim: check.ending.map { "\(check.label) (\($0))" } ?? check.label)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    } icon: {
                        Image(systemName: check.passed ? "checkmark" : "xmark")
                            .foregroundStyle(check.passed ? .green : .red)
                    }
                    .help(check.command)
                    if !check.passed, !check.tail.isEmpty {
                        Text(verbatim: check.tail)
                            .font(.system(size: 10, design: .monospaced))
                            .foregroundStyle(.secondary)
                            .lineLimit(4)
                            .textSelection(.enabled)
                    }
                }
            }
        }
        .font(TendType.detail)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
        .help(String(localized: "Checks tend ran itself, before the agent's own account"))
    }
}

// MARK: - Card

/// One decision: what to decide, the asker's options and recommendation,
/// the evidence, and the answer. An option fills the answer for a look
/// before it goes; nothing is sent on a single stray click.
struct FleetDecisionCard: View {
    let decision: FleetDecision
    var busy = false
    var failure: String?
    let openRevision: (FleetRevisionTarget) -> Void
    /// Replies to a human request; true when it was recorded.
    let answer: (String) async -> Bool
    /// Disposes a review item; true when it was recorded.
    let decide: (TendReviewDisposition, String?) async -> Bool
    var openAsker: (() -> Void)?
    /// Declines a human request without an answer; true when it was recorded.
    var dismiss: (() async -> Bool)?
    /// The dashboard tendd named: another host's review item is decided
    /// there (`FleetDecision.decidedOnDashboard`).
    var dashboard: URL?
    /// Opens with the answer field showing, for previews and tests.
    var initialAnswer: String?
    /// The open answer, shared by every card so only one composer (and one
    /// ⌘↩ Send) exists at a time; a card without one keeps its own.
    @ObservedObject private var draft: FleetDecisionDraft
    private let sharesDraft: Bool
    @State private var localDraft: String?

    init(
        decision: FleetDecision,
        busy: Bool = false,
        failure: String? = nil,
        openRevision: @escaping (FleetRevisionTarget) -> Void,
        answer: @escaping (String) async -> Bool,
        decide: @escaping (TendReviewDisposition, String?) async -> Bool,
        openAsker: (() -> Void)? = nil,
        dismiss: (() async -> Bool)? = nil,
        dashboard: URL? = nil,
        initialAnswer: String? = nil,
        draft: FleetDecisionDraft? = nil
    ) {
        self.decision = decision
        self.busy = busy
        self.failure = failure
        self.openRevision = openRevision
        self.answer = answer
        self.decide = decide
        self.openAsker = openAsker
        self.dismiss = dismiss
        self.dashboard = dashboard
        self.initialAnswer = initialAnswer
        _draft = ObservedObject(wrappedValue: draft ?? .detached)
        sharesDraft = draft != nil
    }

    private var composing: String? {
        get { sharesDraft ? draft.value : localDraft }
        nonmutating set {
            if sharesDraft { draft.update(newValue) } else { localDraft = newValue }
        }
    }
    @Environment(\.colorScheme) private var colorScheme

    private var answerText: String { composing ?? initialAnswer ?? "" }
    private var isComposing: Bool { composing != nil || initialAnswer != nil }

    @Environment(\.fleetCardEmphasis) private var emphasis

    var body: some View {
        Group {
            if emphasis {
                emphasisBody
            } else {
                plainBody
            }
        }
        .disabled(busy)
        .contextMenu {
            if let openAsker {
                Button("Open Agent", action: openAsker)
            }
            if let link = dashboardLink {
                Button("Decide on the Dashboard") { NSWorkspace.shared.open(link) }
            } else if case .review = decision.source, !decision.decidedOnDashboard {
                Button("Withdraw") { Task { _ = await decide(.withdrawn, nil) } }
            }
            if let dismiss {
                Button("Dismiss Question") { Task { _ = await dismiss() } }
            }
        }
    }

    private var plainBody: some View {
        VStack(alignment: .leading, spacing: 7) {
            header
            // What tend checked itself comes first; the asker's account
            // (title, recommendation, evidence) follows.
            if let verification = decision.verification {
                FleetVerificationChecks(verification: verification)
            }
            Text(verbatim: decision.title)
                .font(.system(size: 13, weight: .semibold))
                .lineLimit(4)
                .frame(maxWidth: .infinity, alignment: .leading)
                .textSelection(.enabled)
            account
            answering
            failureLabel
        }
        .padding(12)
        .background(TendSurfacePalette.workspace(for: colorScheme), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .strokeBorder(Color.orange.opacity(0.45), lineWidth: 1)
        )
    }

    /// Status emphasis: the kind, the question as a bold title, everything
    /// to decide with in an inset box, who and when on one line under it.
    private var emphasisBody: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 5) {
                Image(systemName: decision.isHumanRequest ? "questionmark.bubble.fill" : "checklist")
                Text(decision.isHumanRequest ? String(localized: "Question") : String(localized: "Review"))
                Spacer(minLength: 4)
                if busy { ProgressView().controlSize(.mini) }
            }
            .font(.system(size: 11, weight: .bold))
            .foregroundStyle(.orange)
            Text(verbatim: decision.title)
                .font(.system(size: 14.5, weight: .bold))
                .lineLimit(4)
                .frame(maxWidth: .infinity, alignment: .leading)
                .textSelection(.enabled)
            FleetInsetBox {
                if let verification = decision.verification {
                    FleetVerificationChecks(verification: verification)
                }
                account
                answering
            }
            failureLabel
            HStack(spacing: 6) {
                if let asker = decision.asker {
                    Text(verbatim: asker)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                Text(verbatim: decision.host.alias)
                    .lineLimit(1)
                if let repo = decision.repo {
                    Text(verbatim: "· \(repo)")
                        .lineLimit(1)
                }
                if decision.asksUnlisted(at: Date()) {
                    Text("Asker not listed")
                        .foregroundStyle(.orange)
                        .lineLimit(1)
                        .help(String(localized: "The agent that asked is not listed on its host; it may have exited. Dismiss the question if it no longer matters."))
                }
                Spacer(minLength: 4)
                if let since = decision.since {
                    Text(since, format: .relative(presentation: .numeric, unitsStyle: .abbreviated))
                        .monospacedDigit()
                        .fixedSize()
                }
            }
            .font(TendType.meta)
            .foregroundStyle(.secondary)
        }
        .padding(12)
        .modifier(FleetEmphasisCardFrame(lane: .needsYou))
    }

    /// The asker's account: the summary under the question, the
    /// recommendation, the evidence.
    @ViewBuilder
    private var account: some View {
        if let detail = decision.detail {
            Text(verbatim: detail)
                .font(TendType.detail)
                .foregroundStyle(.secondary)
                .lineLimit(3)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        if let recommendation = decision.recommendation {
            Label {
                Text(verbatim: recommendation)
                    .lineLimit(3)
            } icon: {
                Image(systemName: "lightbulb")
                    .foregroundStyle(.orange)
            }
            .font(TendType.detail)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .help(String(localized: "What the asker recommends"))
        }
        if !decision.evidence.isEmpty {
            FleetEvidenceChips(decision: decision, openRevision: openRevision)
        }
    }

    @ViewBuilder
    private var answering: some View {
        if isComposing {
            composer
        } else {
            actions
        }
    }

    @ViewBuilder
    private var failureLabel: some View {
        if let failure {
            Label(failure, systemImage: "exclamationmark.circle")
                .font(TendType.meta)
                .foregroundStyle(.red)
                .lineLimit(3)
        }
    }

    private var header: some View {
        HStack(spacing: 6) {
            Image(systemName: decision.isHumanRequest ? "questionmark.bubble" : "checklist")
                .foregroundStyle(.orange)
            Text(decision.isHumanRequest ? String(localized: "Question") : String(localized: "Review"))
                .fontWeight(.semibold)
            if let asker = decision.asker {
                Text(verbatim: asker)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            if !decision.host.local {
                Text(verbatim: decision.host.alias)
                    .lineLimit(1)
            }
            if decision.asksUnlisted(at: Date()) {
                Text("Asker not listed")
                    .foregroundStyle(.orange)
                    .lineLimit(1)
                    .help(String(localized: "The agent that asked is not listed on its host; it may have exited. Dismiss the question if it no longer matters."))
            }
            Spacer(minLength: 4)
            if busy {
                ProgressView().controlSize(.mini)
            } else if let since = decision.since {
                Text(since, format: .relative(presentation: .numeric, unitsStyle: .abbreviated))
                    .monospacedDigit()
            }
        }
        .font(TendType.meta)
        .foregroundStyle(.secondary)
    }

    /// This card on the dashboard's Needs you page, for another host's
    /// review item.
    private var dashboardLink: URL? {
        guard decision.decidedOnDashboard, case .review(let item) = decision.source, let dashboard else { return nil }
        return TendDashboardLink.item(item.id, host: decision.host.alias, on: dashboard)
    }

    /// Another host's review item: the person decides it on the dashboard,
    /// under its sign-in. This app never decides it itself.
    @ViewBuilder
    private var dashboardAction: some View {
        if let link = dashboardLink {
            Button("Decide on the Dashboard") { NSWorkspace.shared.open(link) }
                .buttonStyle(.tendSecondary)
                .help(String(localized: "Open this result on the dashboard's Needs you page, and decide it there"))
        } else {
            Text("tendd names no dashboard, so this result is decided on \(decision.host.alias). Turn on tend's dashboard to decide it from here.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    @ViewBuilder
    private var actions: some View {
        if decision.decidedOnDashboard {
            dashboardAction
        } else {
            localActions
        }
    }

    private var localActions: some View {
        VStack(alignment: .leading, spacing: 6) {
            if !decision.options.isEmpty {
                // One full-width row per option: a long or multi-line option
                // wraps under its own number instead of being cut off.
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(Array(decision.options.enumerated()), id: \.offset) { index, option in
                        FleetDecisionOptionRow(number: index + 1, text: option) { composing = option }
                    }
                }
            }
            otherActions
        }
    }

    private var otherActions: some View {
        PipelineFlowLayout(spacing: 6) {
            switch decision.source {
            case .humanRequest:
                Button(decision.options.isEmpty ? String(localized: "Answer…") : String(localized: "Other…")) {
                    composing = ""
                }
                .buttonStyle(decision.options.isEmpty ? TendButtonStyle.tendSecondary : TendButtonStyle.tendGhost)
                if let dismiss {
                    Button("Dismiss") { Task { _ = await dismiss() } }
                        .buttonStyle(decision.asksUnlisted(at: Date()) ? TendButtonStyle.tendSecondary : TendButtonStyle.tendGhost)
                        .help(String(localized: "Close this question without answering it"))
                }
            case .review:
                Button("Accept") { Task { _ = await decide(.accepted, nil) } }
                    .buttonStyle(.tendSecondary)
                Button("Return…") { composing = "" }
                    .buttonStyle(.tendGhost)
            }
        }
    }

    private var composer: some View {
        let text = Binding(get: { answerText }, set: { composing = $0 })
        let empty = answerText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        return VStack(alignment: .leading, spacing: 6) {
            TextField(decision.isHumanRequest ? String(localized: "Your answer") : String(localized: "Note for the asker"), text: text, axis: .vertical)
                .textFieldStyle(.roundedBorder)
                .lineLimit(1...5)
            HStack(spacing: 6) {
                Spacer(minLength: 0)
                Button("Cancel") { composing = nil }
                    .buttonStyle(.tendGhost)
                switch decision.source {
                case .humanRequest:
                    Button("Send") { submit { await answer(answerText) } }
                        .buttonStyle(.tendPrimary)
                        .disabled(empty)
                        .keyboardShortcut(.return, modifiers: .command)
                case .review:
                    Button("Accept") { submit { await decide(.accepted, empty ? nil : answerText) } }
                        .buttonStyle(.tendSecondary)
                    Button("Return") { submit { await decide(.returned, answerText) } }
                        .buttonStyle(.tendPrimary)
                        .disabled(empty)
                }
            }
        }
    }

    private func submit(_ send: @escaping () async -> Bool) {
        Task {
            if await send() { composing = nil }
        }
    }
}

/// One of the asker's options on a decision card: its number, then its
/// whole text wrapped beside it. A click fills the answer for a look
/// before it goes.
struct FleetDecisionOptionRow: View {
    let number: Int
    let text: String
    let choose: () -> Void
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.isEnabled) private var isEnabled
    @State private var hovering = false

    var body: some View {
        Button(action: choose) {
            HStack(alignment: .top, spacing: 8) {
                Text(verbatim: "\(number)")
                    .font(.system(size: 11, weight: .bold).monospacedDigit())
                    .foregroundStyle(Color.accentColor)
                    .frame(width: 18, height: 18)
                    .background(Color.accentColor.opacity(0.15), in: Circle())
                Text(verbatim: text)
                    .font(TendType.body)
                    .foregroundStyle(.primary)
                    .multilineTextAlignment(.leading)
                    .lineLimit(nil)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    // The first line centred on the number badge.
                    .padding(.top, 1)
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
            .contentShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
        }
        .buttonStyle(.plain)
        .background(
            RoundedRectangle(cornerRadius: 6, style: .continuous)
                .fill(hovering ? TendTheme.hover(colorScheme) : Color.secondary.opacity(0.08))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 6, style: .continuous)
                .strokeBorder(TendTheme.inputBorder(colorScheme), lineWidth: 1)
        )
        .opacity(isEnabled ? 1 : 0.45)
        .onHover { hovering = $0 && isEnabled }
        .help(String(localized: "Answer with this option; you see it before it is sent"))
        .accessibilityLabel(Text(verbatim: "\(number). \(text)"))
    }
}
