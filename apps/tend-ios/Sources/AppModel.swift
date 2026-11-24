// What the app knows and does: the pairing, the queue per host, the
// history, and the two ways to decide. Every decision goes to the
// dashboard's own routes (`/api/decisions/*`), the ones `/needs` uses, so
// it is checked and audited there exactly like a decision from the web.
import CryptoKit
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

    /// Nobody can answer it: its asker is gone (`subject_check_v1`).
    var unanswerable: TendUnanswerable? {
        if case .question(let entry) = kind { return entry.unanswerable }
        return nil
    }

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
        // Who asked for the review (`impl`), as the Mac shows it; the
        // target only when the requester left no label.
        case .result(let item): item.requestedBy.label?.nonEmpty ?? item.target.text
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

    /// A queue entry; `decidable` as its host's queue says (the board's
    /// `actionable`).
    init(_ item: TendPendingItem, host: String, hostIsLocal: Bool, decidable: Bool) {
        let kind: Kind = switch item {
        case .humanRequest(let entry): .question(entry)
        case .review(let review): .result(review)
        case .agentWaiting(let entry): .waiting(entry)
        }
        // An agent waiting in its pane is answered there.
        if case .waiting = kind {
            self.init(kind: kind, host: host, hostIsLocal: hostIsLocal, decidable: false)
        } else {
            self.init(kind: kind, host: host, hostIsLocal: hostIsLocal, decidable: decidable)
        }
    }

    init(kind: Kind, host: String, hostIsLocal: Bool, decidable: Bool) {
        self.kind = kind
        self.host = host
        self.hostIsLocal = hostIsLocal
        self.decidable = decidable
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
    var hostIsLocal = false

    var id: String {
        switch item {
        case .review(let review): "\(host)/r/\(review.id)"
        case .humanAnswer(let answer): "\(host)/q/\(answer.request.requestID)"
        case .closedByTend(let closed): "\(host)/c/\(closed.request.requestID)"
        }
    }

    var title: String {
        switch item {
        case .review(let review): review.decision?.needed?.nonEmpty ?? review.summary
        case .humanAnswer(let answer): answer.request.decision?.needed?.nonEmpty ?? answer.request.body
        case .closedByTend(let closed): closed.request.decision?.needed?.nonEmpty ?? closed.request.body
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
            answer.closure != nil ? String(localized: "Closed")
                : answer.status == "completed" ? String(localized: "Answered") : String(localized: "Declined")
        case .closedByTend: String(localized: "Closed by tend")
        }
    }

    var isPositive: Bool {
        switch item {
        case .review(let review): review.disposition?.kind == "accepted"
        case .humanAnswer(let answer): answer.status == "completed"
        case .closedByTend: false
        }
    }

    /// The answer, or the note recorded with the disposition.
    var text: String? {
        switch item {
        case .review(let review): review.disposition?.note?.nonEmpty
        case .humanAnswer(let answer): answer.closure?.text ?? answer.answer.nonEmpty
        case .closedByTend(let closed): closed.reasonDescription
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
        case .closedByTend(let closed): Timestamp.parse(closed.closedAt)
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
    /// Every host and its agents; nil until read, or when this device may
    /// only decide.
    private(set) var fleet: FleetSnapshot?
    /// Home's board (`/api/device/home`) when this device operates and the
    /// dashboard serves it; then the lanes, Your turn included, come from
    /// it, as on the Mac. nil otherwise: lanes from the queue and Fleet.
    private(set) var board: DeviceHome?
    /// What this device may do, as the dashboard last said.
    private(set) var scope: DeviceScope = .decide
    /// Why the hosts could not be read (the queue may still be fine).
    private(set) var fleetProblem: String?
    /// What this dashboard serves the device beyond the hosts (`capture`,
    /// `work`), from `GET /api/device`.
    private(set) var features: Set<String> = []
    /// Bumped whenever this device's capabilities shrink: a device or
    /// Fleet read that started before then is stale and is dropped.
    private var capability = UUID()
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
    /// Which pairing attempt may still change state: a newer attempt, a
    /// cancel or a pairing makes every older one's late answer a no-op.
    private var attempt = UUID()
    /// Which pairing the reads and decisions in flight belong to: after a
    /// sign-out or a new pairing, their late answers (a 401 included) are
    /// dropped instead of landing on the new one.
    private var session = UUID() {
        didSet { closeAllResult = nil }
    }
    /// Why Sign out could not unpair this device on the dashboard; the key
    /// is kept so it can be retried.
    private(set) var signOutProblem: String?
    /// Asking the dashboard again, for hosts and new work, while this
    /// device keeps its decide key: the old key works until the new one is
    /// issued, so a code nobody approves leaves the device as it was.
    struct Upgrade: Equatable {
        let origin: ServerOrigin
        let userCode: String
        let expires: Date
    }
    private(set) var upgrade: Upgrade?
    private(set) var upgradeProblem: String?
    /// Why this device decides only, when the app knows: the operator
    /// approved less than it asked for, or limited it later.
    enum ScopeNote: Equatable { case grantedLess, limited }
    private(set) var scopeNote: ScopeNote?
    /// Sign out is waiting for the dashboard; no upgrade may start.
    private(set) var signingOut = false
    /// Whether this dashboard sends notifications, from `GET /api/device`.
    private(set) var pushSupport: DeviceInfo.PushSupport?
    /// This phone's APNs token, as hex, once iOS gave one.
    private(set) var apnsToken: String?
    /// Why the last registration did not take, said in Settings.
    private(set) var pushProblem: String?
    /// The item a tapped notification names, opened once a fresh read of
    /// Your turn finds it.
    var pushFocus: PushPayload?
    /// The decision Your turn should scroll to and mark.
    var focusedDecision: String?
    /// Bumped when a notification asks for Home: the root view selects it
    /// and closes whatever detail covers it.
    private(set) var showHome = 0
    /// One push reconciliation at a time; another request while it runs
    /// makes it go round again with the latest wishes.
    private var pushSyncing = false
    private var pushSyncAgain = false
    /// What the host holds for this device, as far as the app knows: from
    /// `GET /api/device`, or from the last write's answer. Unknown (nil)
    /// while a write is out or after one failed, so the next sync sends.
    private var hostPushRegistration: DeviceInfo.HostPushRegistration??
    /// Bumped at every push write: a device read that started before one
    /// does not say what the host holds now.
    private var pushWrites = 0
    /// Push writes sent and not yet answered.
    private var pushWritesInFlight = 0
    /// When the last write was answered.
    private var pushWrittenAt = Date.distantPast
    /// How long a tend that does not report registrations is taken to
    /// still hold the last one written.
    static let legacyPushConfirmation: TimeInterval = 300

    init() {
        if ProcessInfo.processInfo.arguments.contains("-TendResetState") {
            CredentialStore.clear()
        }
        let stored = CredentialStore.load()
        phase = stored.map(Phase.paired) ?? .unpaired
        scope = stored?.scope ?? .decide
    }

    /// Whether this device may see hosts and agents.
    var operates: Bool { scope == .operate }
    /// Whether it can read an agent's recent output here.
    var canCapture: Bool { operates && features.contains("capture") }
    /// Whether it can start work here.
    var canStartWork: Bool { operates && features.contains("work") }
    /// Whether Home reads the daemon's board.
    var readsBoard: Bool { operates && features.contains("home") }
    /// The board, only while this device may read it.
    var visibleBoard: DeviceHome? { readsBoard ? board : nil }

    /// Re-read what the dashboard says this device is and may do. Scope
    /// only ever narrows for one key: an answer can never widen it, and an
    /// answer that started before a narrowing is dropped.
    func refreshDevice() async {
        guard let client else { return }
        let mine = session
        let generation = capability
        let writes = pushWrites
        let quiet = pushWritesInFlight == 0
        do {
            let info = try await client.device()
            guard session == mine, capability == generation else { return }
            pushSupport = info.push
            // A read that started before a write says nothing about now. An
            // older tend does not say at all: then the last write's answer
            // stands for a few minutes (one launch's burst), and a later
            // foregrounding sends again, so a row it dropped comes back.
            if quiet, pushWritesInFlight == 0, pushWrites == writes, !pushSyncing {
                if let held = info.pushRegistration {
                    hostPushRegistration = held
                } else if Date().timeIntervalSince(pushWrittenAt) > Self.legacyPushConfirmation {
                    hostPushRegistration = nil
                }
            }
            if info.scope == .decide, scope == .operate {
                narrow()
                await syncPush()
                await refresh()
                return
            }
            let next = scope == .operate ? info.features : []
            let leavesBoard = readsBoard && !next.contains("home")
            features = next
            await syncPush()
            if leavesBoard {
                // The queue was not being read: never show its empty
                // fallback as all clear. Read it now.
                leaveBoard()
                await refresh()
            }
        } catch is CancellationError {
            return
        } catch {
            guard session == mine else { return }
            _ = handle(error, session: mine)
        }
    }

    var pairing: Pairing? {
        if case .paired(let pairing) = phase { return pairing }
        return nil
    }

    private var client: DashboardClient? {
        pairing.map { DashboardClient(origin: $0.origin, token: $0.token) }
    }

    var pending: [PendingDecision] {
        visibleBoard?.decisions ?? hosts.flatMap(PendingDecision.from).filter { $0.unanswerable == nil }
    }

    /// Your turn's Can't be answered group: questions whose asker is gone,
    /// not in Your turn and not counted on the badge.
    var cantAnswer: [PendingDecision] {
        visibleBoard?.cantAnswer ?? hosts.flatMap(PendingDecision.from).filter { $0.unanswerable != nil }
    }

    /// The group's count: every host's exact count, never less than listed.
    var cantAnswerCount: Int {
        if let board = visibleBoard { return board.cantAnswerCount }
        return max(cantAnswer.count, hosts.reduce(0) { $0 + $1.queue.unanswerableCount })
    }

    /// Keep and Close all act on the host that holds the question, the
    /// paired one or a Fleet host whose section says it takes them
    /// (docs/proposals/FLEET_UNANSWERABLE.md).
    func canKeep(_ decision: PendingDecision) -> Bool {
        decision.decidable && flags(decision.host).keep
    }

    /// One Close all per host, each with exactly its listed questions:
    /// the paired host first, then the others by name. Empty where a
    /// host's switch is off.
    var closable: [CloseAllGroup] {
        let listed = cantAnswer.filter { $0.decidable && flags($0.host).close }
        let hosts = Dictionary(grouping: listed, by: \.host)
        return hosts.map { CloseAllGroup(host: $0.key, local: $0.value.first?.hostIsLocal ?? false, decisions: $0.value) }
            .sorted { ($0.local ? 0 : 1, $0.host) < ($1.local ? 0 : 1, $1.host) }
    }

    /// Another host's folded questions that cannot be kept or closed from
    /// here, and why.
    var cantAnswerNotes: [CantAnswerNote] {
        let remote = Set(cantAnswer.filter { !$0.hostIsLocal }.map(\.host))
        return remote.sorted().compactMap { host in
            let f = flags(host)
            let reason: CantAnswerNote.Reason? = if !f.control {
                .observeOnly
            } else if !f.keep {
                .upgrade
            } else if !f.close {
                .closeOff
            } else {
                nil
            }
            return reason.map { CantAnswerNote(host: host, reason: $0) }
        }
    }

    private func flags(_ host: String) -> (keep: Bool, close: Bool, control: Bool) {
        if let board = visibleBoard {
            let section = board.decisionHosts.first { $0.alias == host }
            return (section?.subjectCheck ?? false, section?.closeUnanswerable ?? false, section?.control ?? false)
        }
        let section = hosts.first { $0.alias == host }
        return (section?.subjectCheck ?? false, section?.closeUnanswerable ?? false, section?.control ?? false)
    }

    /// What the last Close all did, in words, until dismissed. It belongs
    /// to the pairing it was made under: a new session clears it.
    private(set) var closeAllResult: (ok: Bool, text: String)?

    func dismissCloseAllResult() { closeAllResult = nil }

    /// "Nothing needs you" only when every host was read in full, every
    /// entry decoded, and none is waiting: an item this app cannot read, a
    /// cut-off list or a host that could not be read is never all clear.
    static func allClear(_ hosts: [HostQueue]) -> Bool {
        // Questions nobody can answer wait in their own group, not here.
        hosts.allSatisfy { section in
            section.queue.isComplete && section.unavailable.isEmpty
                && section.queue.items.allSatisfy { item in
                    if case .humanRequest(let entry) = item { return entry.unanswerable != nil }
                    return false
                }
        }
    }

    /// What the person can act on from here, for the badge.
    var actionableCount: Int { pending.filter(\.decidable).count }

    var past: [PastDecision] {
        history
            .flatMap { section in section.history.items.map { PastDecision(item: $0, host: section.alias, hostIsLocal: section.local) } }
            .sorted { ($0.at ?? .distantPast) > ($1.at ?? .distantPast) }
    }

    // MARK: Pairing

    func startPairing(address: String, name: String, scope wanted: DeviceScope = .decide) {
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
        let mine = UUID()
        attempt = mine
        pairingTask = Task { [weak self] in
            let client = DashboardClient(origin: origin)
            // Only the latest, uncancelled attempt may touch state.
            @MainActor func current() -> AppModel? {
                guard let self, self.attempt == mine, !Task.isCancelled else { return nil }
                return self
            }
            do {
                let request = try await client.requestPairing(name: deviceName, platform: platform, scope: wanted)
                guard let model = current() else { return }
                let expires = Date().addingTimeInterval(TimeInterval(request.expiresIn))
                model.phase = .waitingForApproval(origin: origin, userCode: request.userCode, expires: expires)
                let interval = UInt64(max(request.interval, 1))
                while Date() < expires {
                    try await Task.sleep(nanoseconds: interval * 1_000_000_000)
                    let answer = try await client.pollPairing(deviceCode: request.deviceCode)
                    guard let model = current() else { return }
                    if case .issued(let device, let token, let scope) = answer {
                        model.paired(Pairing(origin: origin, device: device, token: token, scope: scope))
                        model.scopeNote = wanted == .operate && scope == .decide ? .grantedLess : nil
                        await model.refreshDevice()
                        await model.refresh()
                        return
                    }
                }
                current()?.fail(String(localized: "The code expired before it was approved. Start again."))
            } catch is CancellationError {
                return
            } catch {
                current()?.fail(error.localizedDescription)
            }
        }
    }

    private func paired(_ pairing: Pairing) {
        guard install(pairing) else {
            fail(String(localized: "Could not keep the key in this device's Keychain."))
            return
        }
    }

    /// Keep a new key and use it from now on; false, changing nothing, when
    /// the Keychain would not take it.
    private func install(_ pairing: Pairing) -> Bool {
        guard CredentialStore.save(pairing) else { return false }
        attempt = UUID()
        session = UUID()
        hostPushRegistration = nil
        signOutProblem = nil
        upgrade = nil
        upgradeProblem = nil
        scope = pairing.scope
        phase = .paired(pairing)
        return true
    }

    private func fail(_ message: String) {
        attempt = UUID()
        pairingProblem = message
        phase = .unpaired
    }

    func cancelPairing() {
        attempt = UUID()
        pairingTask?.cancel()
        pairingTask = nil
        phase = .unpaired
    }

    /// Ask the dashboard for hosts and new work as a new device, keeping
    /// this decide key until the new one is issued. Then the new key
    /// replaces it here, and the old one is removed on the dashboard.
    func requestHostsAndWork() {
        guard let old = pairing, !operates, upgrade == nil, !signingOut else { return }
        upgradeProblem = nil
        let platform = UIDevice.current.userInterfaceIdiom == .pad ? "ipados" : "ios"
        pairingTask?.cancel()
        let mine = UUID()
        attempt = mine
        pairingTask = Task { [weak self] in
            let client = DashboardClient(origin: old.origin)
            // Only the latest attempt, on the key it started from, counts.
            @MainActor func current() -> AppModel? {
                guard let self, self.attempt == mine, !Task.isCancelled,
                      self.pairing?.token == old.token else { return nil }
                return self
            }
            do {
                let request = try await client.requestPairing(name: old.device.name, platform: platform, scope: .operate)
                guard let model = current() else { return }
                let expires = Date().addingTimeInterval(TimeInterval(request.expiresIn))
                model.upgrade = Upgrade(origin: old.origin, userCode: request.userCode, expires: expires)
                let interval = UInt64(max(request.interval, 1))
                while Date() < expires {
                    try await Task.sleep(nanoseconds: interval * 1_000_000_000)
                    let answer = try await client.pollPairing(deviceCode: request.deviceCode)
                    guard let model = current() else { return }
                    if case .issued(let device, let token, let scope) = answer {
                        guard model.install(Pairing(origin: old.origin, device: device, token: token, scope: scope)) else {
                            // The old key stays, here and on the dashboard;
                            // the new one, never kept, is given back.
                            model.failUpgrade(String(localized: "Could not keep the new key in this device's Keychain. This device still decides with its current key; ask again."))
                            try? await DashboardClient(origin: old.origin, token: token).forget()
                            return
                        }
                        model.scopeNote = scope == .decide ? .grantedLess : nil
                        // The old key is not needed any more; if the
                        // dashboard cannot be told, it expires unused.
                        do {
                            try await DashboardClient(origin: old.origin, token: old.token).forget()
                        } catch {
                            model.outcome = String(localized: "The previous key could not be removed on the dashboard. Remove \"\(old.device.name)\" under access ▸ Devices, or let it expire.")
                        }
                        await model.refreshDevice()
                        await model.refresh()
                        return
                    }
                }
                current()?.failUpgrade(String(localized: "The code expired before it was approved. This device still decides; ask again."))
            } catch is CancellationError {
                return
            } catch {
                current()?.failUpgrade(error.localizedDescription)
            }
        }
    }

    func cancelUpgrade() {
        attempt = UUID()
        pairingTask?.cancel()
        pairingTask = nil
        upgrade = nil
    }

    private func failUpgrade(_ message: String) {
        attempt = UUID()
        upgrade = nil
        upgradeProblem = message
    }

    /// Unpair on the dashboard, then forget the key here. When the
    /// dashboard cannot be reached the key is kept and the failure shown,
    /// so revoking can be retried; `forgetLocally` is the explicit way out.
    func signOut() async {
        guard let client, !signingOut else { return }
        // An upgrade answered while the dashboard forgets this key must not
        // install a new one: stop it before anything is awaited.
        cancelUpgrade()
        signingOut = true
        defer { signingOut = false }
        let mine = session
        // Best effort: the host also drops it with the device.
        if pairing?.push != nil {
            _ = beginPushWrite()
            try? await client.removePush()
            // Whatever overlapped it, the host may hold nothing now.
            hostPushRegistration = nil
            pushWritesInFlight -= 1
        }
        do {
            try await client.forget()
        } catch DashboardError.signedOut {
            // Already unpaired there: nothing left to revoke.
        } catch {
            guard session == mine else { return }
            signOutProblem = String(localized: "The dashboard could not be reached, so this device is still paired there: \(error.localizedDescription)")
            return
        }
        guard session == mine else { return }
        forgetLocally()
    }

    /// Forget the key on this device only. The dashboard still lists the
    /// device until it is removed under access ▸ Devices or expires.
    func forgetLocally() {
        CredentialStore.clear()
        cancelUpgrade()
        upgradeProblem = nil
        scopeNote = nil
        pushSupport = nil
        pushProblem = nil
        hostPushRegistration = nil
        pushFocus = nil
        focusedDecision = nil
        session = UUID()
        signOutProblem = nil
        phase = .unpaired
        hosts = []
        history = []
        fleet = nil
        board = nil
        fleetProblem = nil
        features = []
        scope = .decide
        loaded = false
        historyLoaded = false
        problem = nil
        failures = [:]
        outcome = nil
        closeAllResult = nil
    }

    /// The user-facing message for a failed call made under `mine`; a 401
    /// means this device was removed, so it is unpaired here too.
    private func handle(_ error: Error, session mine: UUID) -> String {
        if case DashboardError.signedOut = error, session == mine {
            forgetLocally()
            pairingProblem = DashboardError.signedOut.localizedDescription
        }
        return error.localizedDescription
    }

    // MARK: Reading

    /// A refresh asked for while one was running: run once more after it,
    /// so a decision never leaves a stale card behind.
    private var refreshAgain = false
    /// Callers whose refresh was folded into the running one: they return
    /// when it ends, so what they read next comes from a read that started
    /// after they asked.
    private var refreshWaiters: [CheckedContinuation<Void, Never>] = []

    /// Read the board. When a refresh is already running, this one runs
    /// as its next round, and returns only once that round is done.
    func refresh() async {
        guard !refreshing else {
            refreshAgain = true
            await withCheckedContinuation { refreshWaiters.append($0) }
            return
        }
        refreshing = true
        defer {
            refreshing = false
            let waiters = refreshWaiters
            refreshWaiters = []
            waiters.forEach { $0.resume() }
        }
        repeat {
            refreshAgain = false
            guard let client else { return }
            let mine = session
            async let fleetRead: Void = refreshFleet(client, session: mine)
            if readsBoard {
                await refreshBoard(client, session: mine)
                // Still on the board (not narrowed while reading): done.
                if readsBoard {
                    await fleetRead
                    continue
                }
            }
            do {
                let read = try await client.queue()
                guard session == mine else { await fleetRead; return }
                hosts = read
                loaded = true
                problem = nil
            } catch is CancellationError {
                await fleetRead
                return
            } catch {
                guard session == mine else { await fleetRead; return }
                problem = handle(error, session: mine)
            }
            await fleetRead
        } while refreshAgain
    }

    /// Read the hosts when this device may. A 403 means the dashboard
    /// narrowed it to deciding: say so and stop asking.
    private func refreshFleet(_ client: DashboardClient, session mine: UUID) async {
        guard scope == .operate else { return }
        let generation = capability
        do {
            let read = try await client.fleet()
            // Narrowed while this was in flight: the answer is no longer ours.
            guard session == mine, capability == generation, scope == .operate else { return }
            if fleet != read { fleet = read }
            fleetProblem = nil
        } catch DashboardError.notAllowed {
            guard session == mine else { return }
            narrow()
        } catch is CancellationError {
            return
        } catch {
            guard session == mine else { return }
            fleetProblem = handle(error, session: mine)
        }
    }

    /// Read Home's board. Your turn comes from it too, so a failure is
    /// shown like a failed queue read; the last good board stays.
    private func refreshBoard(_ client: DashboardClient, session mine: UUID) async {
        let generation = capability
        do {
            let read = try await client.home()
            guard session == mine, capability == generation, readsBoard else { return }
            if board != read { board = read }
            loaded = true
            problem = nil
        } catch DashboardError.notAllowed {
            guard session == mine else { return }
            narrow()
        } catch is CancellationError {
            return
        } catch {
            guard session == mine else { return }
            problem = handle(error, session: mine)
        }
    }

    /// Home stops reading the board: the queue it skipped is unknown, so
    /// Your turn waits for it instead of showing an empty queue as clear.
    private func leaveBoard() {
        board = nil
        hosts = []
        loaded = false
        refreshAgain = refreshing
    }

    /// The dashboard limited this device to deciding.
    private func narrow() {
        if readsBoard { leaveBoard() }
        capability = UUID()
        scope = .decide
        scopeNote = .limited
        features = []
        fleet = nil
        board = nil
        fleetProblem = DashboardError.notAllowed.localizedDescription
        if var pairing {
            pairing.scope = .decide
            if CredentialStore.save(pairing) { phase = .paired(pairing) }
        }
    }

    // MARK: Lanes

    /// The hosts, only while this device may see them.
    var visibleFleet: FleetSnapshot? { operates ? fleet : nil }
    var inProgress: [LaneAgent] { visibleFleet.map(FleetLanes.inProgress) ?? [] }
    var finishedAgents: [LaneAgent] { visibleFleet.map { FleetLanes.finished($0) } ?? [] }

    /// Home's lanes: the backlog first when there is planned Work.
    var lanes: [Lane] {
        (visibleBoard?.backlog.isEmpty == false ? [.backlog] : []) + Lane.board
    }

    /// Decisions taken within the Finished window.
    var recentPast: [PastDecision] {
        let now = Date()
        return past.filter { $0.at.map { now.timeIntervalSince($0) <= FleetLanes.finishedWindow } ?? false }
    }

    func count(_ lane: Lane) -> Int {
        if let board = visibleBoard {
            switch lane {
            case .backlog: return board.backlog.count
            // Every decided item, the older ones behind their fold too
            // (fromServerBoard counts them).
            case .finished: return FoldedLane(board, lane: lane).count + past.count
            default: return FoldedLane(board, lane: lane).count
            }
        }
        switch lane {
        case .backlog: return 0
        case .yourTurn: return pending.count
        case .inProgress: return inProgress.count
        case .finished: return recentPast.count + finishedAgents.count
        }
    }

    // MARK: Agents and work

    /// The Fleet record of an agent a unit names, to open its detail.
    func laneAgent(_ ref: BoardAgent) -> LaneAgent? {
        guard let session = ref.sessionID,
              let host = visibleFleet?.hosts.first(where: { $0.alias == ref.host }),
              let agent = host.agents.first(where: { $0.sessionID == session })
        else { return nil }
        return LaneAgent(agent: agent, host: host.alias, hostIsLocal: host.local)
    }

    func capture(_ agent: LaneAgent) async throws -> String {
        guard let client, !agent.agent.sessionID.isEmpty else { return "" }
        guard canCapture else { throw DashboardError.notAllowed }
        let mine = session
        do {
            return try await client.capture(host: agent.host, sessionID: agent.agent.sessionID)
        } catch DashboardError.notAllowed {
            if session == mine { narrow() }
            throw DashboardError.notAllowed
        } catch {
            _ = handle(error, session: mine)
            throw error
        }
    }

    func workOptions(host: String) async throws -> WorkOptions {
        guard let client else { throw DashboardError.signedOut }
        guard canStartWork else { throw DashboardError.notAllowed }
        return try await client.workOptions(host: host)
    }

    /// Start new work. The caller has already asked for Face ID. The answer
    /// says whether it started, failed, or may have started (unknown).
    func startWork(_ request: StartWorkRequest) async throws -> StartedWork {
        guard let client else { throw DashboardError.signedOut }
        guard canStartWork else { throw DashboardError.notAllowed }
        let mine = session
        do {
            let started = try await client.startWork(request)
            guard session == mine else { return started }
            outcome = switch started.state {
            case "started": String(localized: "Started.")
            case "unknown": String(localized: "It may have started; check the host.")
            default: String(localized: "Not started.")
            }
            await refresh()
            return started
        } catch DashboardError.notAllowed {
            if session == mine { narrow() }
            throw DashboardError.notAllowed
        } catch {
            _ = handle(error, session: mine)
            throw error
        }
    }

    func refreshHistory() async {
        guard let client else { return }
        let mine = session
        do {
            let read = try await client.history()
            guard session == mine else { return }
            history = read
            historyLoaded = true
        } catch is CancellationError {
            return
        } catch {
            guard session == mine else { return }
            problem = handle(error, session: mine)
        }
    }

    // MARK: Deciding

    private func act(_ decision: PendingDecision, done: String, _ call: (DashboardClient) async throws -> Void) async -> Bool {
        guard let client else { return false }
        let mine = session
        busy.insert(decision.id)
        defer { busy.remove(decision.id) }
        do {
            try await call(client)
            guard session == mine else { return false }
            failures[decision.id] = nil
            outcome = done
            await refresh()
            await refreshHistory()
            return true
        } catch {
            guard session == mine else { return false }
            failures[decision.id] = handle(error, session: mine)
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

    /// Keep: back in Your turn until its asker is seen again and gone again.
    func keep(_ decision: PendingDecision) async -> Bool {
        guard case .question(let entry) = decision.kind, let unanswerable = entry.unanswerable else { return false }
        let host = decision.hostIsLocal ? nil : decision.host
        return await act(decision, done: String(localized: "Back in Your turn.")) {
            try await $0.keep(requestID: entry.requestID, episode: unanswerable.episode, host: host)
        }
    }

    /// Close all: exactly the listed questions, after the person's one
    /// confirmation; what was skipped is said.
    func closeAll(_ group: CloseAllGroup) async {
        guard let client else { return }
        let decisions = group.decisions
        let pairs: [(requestID: String, episode: String)] = decisions.compactMap { decision in
            guard case .question(let entry) = decision.kind, let unanswerable = entry.unanswerable else { return nil }
            return (entry.requestID, unanswerable.episode)
        }
        guard !pairs.isEmpty else { return }
        let mine = session
        let ids = Set(decisions.map(\.id))
        busy.formUnion(ids)
        closeAllResult = nil
        defer { busy.subtract(ids) }
        do {
            let outcome = try await client.closeUnanswerable(pairs, host: group.local ? nil : group.host)
            guard session == mine else { return }
            let text = Self.closeAllText(outcome)
            closeAllResult = (outcome.skipped.isEmpty, group.local ? text : "\(group.host): \(text)")
            await refresh()
            await refreshHistory()
        } catch {
            guard session == mine else { return }
            let message = handle(error, session: mine)
            // A 401 unpaired this device: nothing of the old pairing stays.
            guard session == mine else { return }
            closeAllResult = (false, group.local ? message : "\(group.host): \(message)")
        }
    }

    static func closeAllText(_ outcome: TendCloseOutcome) -> String {
        let closed = outcome.closed.count == 1
            ? String(localized: "Closed 1 question.")
            : String(localized: "Closed \(outcome.closed.count) questions.")
        guard !outcome.skipped.isEmpty else { return closed }
        let reasons = outcome.skipped.map { "\($0.requestID) (\($0.reason))" }.joined(separator: "; ")
        return closed + " " + String(localized: "Skipped \(outcome.skipped.count): \(reasons)")
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

// MARK: - Notifications

/// What a notification's button did.
enum PushActOutcome: Equatable, Sendable {
    /// Decided; what to say.
    case done(String)
    /// A check failed before anything was sent: nothing was decided.
    case refused(PushRefusal)
    /// The decision was sent and its answer lost: it may have been taken.
    case unknown
}

extension AppModel {
    /// The person's choice, on this phone only (not a secret).
    static let notificationsKey = "notificationsWanted"
    static let notificationContentKey = "notificationContent"

    var notificationsWanted: Bool {
        get { UserDefaults.standard.bool(forKey: Self.notificationsKey) }
        set { UserDefaults.standard.set(newValue, forKey: Self.notificationsKey) }
    }

    var notificationContentWanted: Bool {
        get { UserDefaults.standard.bool(forKey: Self.notificationContentKey) }
        set { UserDefaults.standard.set(newValue, forKey: Self.notificationContentKey) }
    }

    /// The APNs environment this build's entitlement names.
    static var pushEnvironment: String {
        #if DEBUG
        "development"
        #else
        "production"
        #endif
    }

    /// The host's fingerprint of a registration (`PushRow::registration`):
    /// hex of the first 16 bytes of SHA-256(token NUL environment NUL
    /// channel NUL), the token as lower-case hex. Never the token itself.
    nonisolated static func registrationFingerprint(token: String, environment: String, channel: String) -> String {
        var message = Data()
        for part in [token.lowercased(), environment, channel] {
            message.append(Data(part.utf8))
            message.append(0)
        }
        return SHA256.hash(data: message).prefix(16).map { String(format: "%02x", $0) }.joined()
    }

    /// iOS gave this phone a token (at every launch, and when it changes).
    func receivedAPNsToken(_ token: Data) async {
        apnsToken = token.map { String(format: "%02x", $0) }.joined()
        await syncPush()
    }

    /// The person changed what they want: register, refresh or remove.
    func registerPush() async { await syncPush() }

    /// Stop notifications for this phone, here and on the host, whatever a
    /// registration in flight answers.
    func unregisterPush() async {
        notificationsWanted = false
        await syncPush()
    }

    /// Bring the host's registration and the binding kept here to what the
    /// person wants now. One run at a time: a request while one runs makes
    /// it run again, so the last wish always wins, and every answer is
    /// checked against the pairing it was asked for.
    func syncPush() async {
        if pushSyncing {
            pushSyncAgain = true
            return
        }
        pushSyncing = true
        defer { pushSyncing = false }
        repeat {
            pushSyncAgain = false
            await syncPushOnce()
        } while pushSyncAgain
    }

    private func syncPushOnce() async {
        guard let client, let pairing else { return }
        let mine = session
        let wanted = notificationsWanted && pushSupport?.enabled == true
        if wanted, let token = apnsToken, let support = pushSupport {
            let content = notificationContentWanted
            // The host already holds exactly this: iOS hands the same token
            // at every launch, and every foregrounding re-reads the device.
            // The binding must still be what the host's offer gives now (its
            // content switch, this build's environment), or registering again
            // refreshes it, or says why not.
            if let binding = pairing.push, case .some(.some(let held)) = hostPushRegistration,
               held.content == content,
               binding.content == (content && support.contentAllowed),
               support.environments.contains(Self.pushEnvironment),
               held.registration == Self.registrationFingerprint(token: token, environment: Self.pushEnvironment, channel: binding.channel) {
                return
            }
            var write = beginPushWrite()
            defer { pushWritesInFlight -= 1 }
            do {
                let answer = try await client.registerPush(token: token, environment: Self.pushEnvironment, content: content)
                guard session == mine, self.pairing?.token == pairing.token else { return }
                switch answer {
                case .on(let binding):
                    // Only a write no other overlapped says what the host
                    // holds: the order they reach the host is not known.
                    if soleWrite(write) {
                        pushWrittenAt = Date()
                        hostPushRegistration = .some(DeviceInfo.HostPushRegistration(
                            registration: Self.registrationFingerprint(token: token, environment: Self.pushEnvironment, channel: binding.channel),
                            content: content
                        ))
                    } else {
                        hostPushRegistration = nil
                    }
                    // Turned off while the host answered: undo it now.
                    guard notificationsWanted else {
                        pushSyncAgain = true
                        return
                    }
                    if keep(binding) {
                        pushProblem = nil
                    } else {
                        // A registration this phone cannot act on: take it back.
                        pushProblem = String(localized: "Could not keep the notification key in this device's Keychain. Notifications stay off; try again.")
                        write = beginPushWrite()
                        let removed = (try? await client.removePush()) != nil
                        settle(write, as: removed && session == mine ? .some(nil) : nil)
                        pushWritesInFlight -= 1
                    }
                case .off(let reason):
                    settle(write, as: .some(nil))
                    _ = keep(nil)
                    pushProblem = reason == "environment"
                        ? String(localized: "This dashboard's notification key does not serve this build of the app.")
                        : String(localized: "This dashboard does not send notifications.")
                }
            } catch {
                guard session == mine else { return }
                pushProblem = error.localizedDescription
            }
        } else if !notificationsWanted, pairing.push != nil || pushRemovalOwed {
            // Off: remove on the host whether or not a binding was kept here.
            let write = beginPushWrite()
            defer { pushWritesInFlight -= 1 }
            do {
                try await client.removePush()
                guard session == mine, self.pairing?.token == pairing.token else { return }
                settle(write, as: .some(nil))
                pushRemovalOwed = false
                _ = keep(nil)
            } catch {
                guard session == mine else { return }
                pushProblem = error.localizedDescription
            }
        }
    }

    /// A push write starts: what the host holds is unknown until it is
    /// answered, and any answer or read from before it no longer counts.
    /// The caller ends it (`pushWritesInFlight -= 1`).
    private func beginPushWrite() -> Int {
        hostPushRegistration = nil
        pushWrites += 1
        pushWritesInFlight += 1
        return pushWrites
    }

    /// Whether `write`, still counted in flight, overlapped no other write.
    private func soleWrite(_ write: Int) -> Bool {
        pushWrites == write && pushWritesInFlight == 1
    }

    /// A write was answered: what the host holds, if no other write
    /// overlapped it, else unknown.
    private func settle(_ write: Int, as held: DeviceInfo.HostPushRegistration??) {
        hostPushRegistration = soleWrite(write) ? held : nil
    }

    /// Set when the person turns notifications off, until the host says it
    /// dropped this phone: a registration may exist with no binding here.
    private var pushRemovalOwed: Bool {
        get { UserDefaults.standard.bool(forKey: "pushRemovalOwed") }
        set { UserDefaults.standard.set(newValue, forKey: "pushRemovalOwed") }
    }

    /// The switch in Settings or the offer card turned notifications off.
    func turnNotificationsOff() async {
        pushRemovalOwed = true
        await unregisterPush()
    }

    #if DEBUG
    /// UI tests: a binding as a registration would give, without a host
    /// that sends to Apple.
    func adoptTestBinding(_ binding: PushBinding) { _ = keep(binding) }
    #endif

    /// Keep (or drop) the binding with the pairing; false when the Keychain
    /// would not take it.
    @discardableResult
    private func keep(_ binding: PushBinding?) -> Bool {
        guard var pairing else { return false }
        guard pairing.push != binding else { return true }
        pairing.push = binding
        guard CredentialStore.save(pairing) else { return false }
        phase = .paired(pairing)
        return true
    }

    /// A tapped notification: Home, Your turn, and its item once a fresh
    /// read finds it. One from another pairing only opens the app.
    func open(_ payload: PushPayload) async {
        guard let binding = pairing?.push, binding.channel == payload.channel else { return }
        showHome += 1
        pushFocus = payload
        await refresh()
        focusPushedItem()
    }

    /// Mark the item a notification named, if the queue holds it now.
    func focusPushedItem() {
        guard let payload = pushFocus, let key = pairing?.push?.refKey, loaded else { return }
        pushFocus = nil
        let match = pending.first { $0.host == payload.host && PushRef.of(key: key, $0) == payload.ref }
        focusedDecision = match?.id
        if match == nil {
            outcome = String(localized: "That item was already decided, or is no longer waiting.")
        }
    }

    /// A notification's button. Every check is made on what the host says
    /// now (the item, and whether it still allows content); anything
    /// missing refuses, and nothing is decided.
    func act(on payload: PushPayload, action: String, text: String?) async -> PushActOutcome {
        guard let client, let started = pairing else { return .refused(.otherChannel) }
        let mine = session
        guard let info = try? await client.device(), let fresh = try? await client.queue().first(where: { $0.alias == payload.host }) else {
            return .refused(.incompleteRead)
        }
        // Signed out, paired again, or turned off while reading.
        guard session == mine, let now = pairing, now.token == started.token else { return .refused(.otherChannel) }
        pushSupport = info.push
        let checked = PushActionCheck.verify(
            payload,
            binding: now.push,
            contentSwitch: notificationsWanted && notificationContentWanted,
            hostAllowsContent: info.push?.enabled == true && info.push?.contentAllowed == true,
            fresh: fresh
        )
        let decision: PendingDecision
        switch checked {
        case .success(let found): decision = found
        case .failure(let refusal): return .refused(refusal)
        }
        let host = decision.hostIsLocal ? nil : decision.host
        do {
            switch (action, decision.kind) {
            case (PushCategory.accept, .result(let item)):
                try await client.review(id: item.id, disposition: .accepted, note: nil, option: nil, host: host, via: "notification")
                return .done(String(localized: "Accepted."))
            case (PushCategory.sendBack, .result(let item)):
                try await client.review(id: item.id, disposition: .returned, note: text, option: nil, host: host, via: "notification")
                return .done(String(localized: "Returned."))
            case (PushCategory.answer, .question(let entry)):
                guard let text, !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return .refused(.changed) }
                try await client.answer(requestID: entry.requestID, answer: text, declined: false, host: host, via: "notification")
                return .done(String(localized: "Answered."))
            default:
                return .refused(.otherKind)
            }
        } catch let error as DashboardError {
            // A 4xx is the host refusing: nothing was decided. A 5xx (a
            // relay to another host, say) or a lost answer may have come
            // after the decision was taken.
            switch error {
            case .server(let status, _) where (400..<500).contains(status): return .refused(.changed)
            case .signedOut: return .refused(.otherChannel)
            default: return .unknown
            }
        } catch {
            return .unknown
        }
    }
}

/// One host's Close all: exactly its listed questions.
struct CloseAllGroup: Hashable, Identifiable {
    let host: String
    /// The host this app is paired with.
    let local: Bool
    let decisions: [PendingDecision]

    var id: String { host }
}

/// Why another host's folded questions cannot be kept, or closed, here.
struct CantAnswerNote: Hashable, Identifiable {
    enum Reason: Hashable {
        case observeOnly, upgrade, closeOff
    }

    let host: String
    let reason: Reason

    var id: String { host }

    var text: String {
        switch reason {
        case .observeOnly: String(localized: "\(host) is observe-only: keep or close its questions there.")
        case .upgrade: String(localized: "Keep or close \(host)'s questions on that host, or upgrade tend there.")
        case .closeOff: String(localized: "Close all is turned off on \(host); Keep still works.")
        }
    }
}
