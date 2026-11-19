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
        do {
            let info = try await client.device()
            guard session == mine, capability == generation else { return }
            if info.scope == .decide, scope == .operate {
                narrow()
                await refresh()
                return
            }
            let next = scope == .operate ? info.features : []
            let leavesBoard = readsBoard && !next.contains("home")
            features = next
            let pushChanged = pushSupport != info.push
            pushSupport = info.push
            if pushChanged { await registerPush() }
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

    /// Keep and Close all act on the host this app is paired with, where
    /// it advertises them.
    func canKeep(_ decision: PendingDecision) -> Bool {
        decision.hostIsLocal && decision.decidable && localFlags.keep
    }

    /// The questions Close all would close, empty while the switch is off.
    var closable: [PendingDecision] {
        guard localFlags.close else { return [] }
        return cantAnswer.filter { $0.hostIsLocal && $0.decidable }
    }

    private var localFlags: (keep: Bool, close: Bool) {
        if let board = visibleBoard {
            let local = board.decisionHosts.first(where: \.local)
            return (local?.subjectCheck ?? false, local?.closeUnanswerable ?? false)
        }
        let local = hosts.first(where: \.local)
        return (local?.subjectCheck ?? false, local?.closeUnanswerable ?? false)
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
        if pairing?.push != nil { try? await client.removePush() }
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
        return await act(decision, done: String(localized: "Back in Your turn.")) {
            try await $0.keep(requestID: entry.requestID, episode: unanswerable.episode)
        }
    }

    /// Close all: exactly the listed questions, after the person's one
    /// confirmation; what was skipped is said.
    func closeAll(_ decisions: [PendingDecision]) async {
        guard let client else { return }
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
            let outcome = try await client.closeUnanswerable(pairs)
            guard session == mine else { return }
            closeAllResult = (outcome.skipped.isEmpty, Self.closeAllText(outcome))
            await refresh()
            await refreshHistory()
        } catch {
            guard session == mine else { return }
            let message = handle(error, session: mine)
            // A 401 unpaired this device: nothing of the old pairing stays.
            guard session == mine else { return }
            closeAllResult = (false, message)
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

    /// iOS gave this phone a token (at every launch, and when it changes).
    func receivedAPNsToken(_ token: Data) async {
        apnsToken = token.map { String(format: "%02x", $0) }.joined()
        await registerPush()
    }

    /// Tell the host this phone's token, if the person wants notifications
    /// and the host sends them; keep the binding it answers with.
    func registerPush() async {
        guard let client, let pairing, notificationsWanted, let token = apnsToken,
              pushSupport?.enabled == true else { return }
        let mine = session
        do {
            let answer = try await client.registerPush(token: token, environment: Self.pushEnvironment, content: notificationContentWanted)
            guard session == mine, self.pairing?.token == pairing.token else { return }
            switch answer {
            case .on(let binding):
                pushProblem = nil
                keep(binding)
            case .off(let reason):
                keep(nil)
                pushProblem = reason == "environment"
                    ? String(localized: "This dashboard's notification key does not serve this build of the app.")
                    : String(localized: "This dashboard does not send notifications.")
            }
        } catch {
            guard session == mine else { return }
            pushProblem = error.localizedDescription
        }
    }

    /// Stop notifications for this phone, here and on the host.
    func unregisterPush() async {
        notificationsWanted = false
        guard let client, pairing?.push != nil else { return }
        do {
            try await client.removePush()
            keep(nil)
        } catch {
            pushProblem = error.localizedDescription
        }
    }

    #if DEBUG
    /// UI tests: a binding as a registration would give, without a host
    /// that sends to Apple.
    func adoptTestBinding(_ binding: PushBinding) { keep(binding) }
    #endif

    private func keep(_ binding: PushBinding?) {
        guard var pairing, pairing.push != binding else { return }
        pairing.push = binding
        if CredentialStore.save(pairing) { phase = .paired(pairing) }
    }

    /// A tapped notification: open Your turn and its item, once a fresh
    /// read finds it. One from another pairing only opens the app.
    func open(_ payload: PushPayload) async {
        guard let binding = pairing?.push, binding.channel == payload.channel else { return }
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

    /// A notification's button. Decides only if every check holds; otherwise
    /// nothing is decided and the caller opens the item.
    func act(on payload: PushPayload, action: String, text: String?) async -> Result<String, PushRefusal> {
        guard let client else { return .failure(.otherChannel) }
        let mine = session
        let fresh = try? await client.queue().first { $0.alias == payload.host }
        guard session == mine else { return .failure(.otherChannel) }
        let checked = PushActionCheck.verify(
            payload,
            binding: pairing?.push,
            contentSwitch: notificationContentWanted,
            hostAllowsContent: pushSupport?.contentAllowed == true,
            fresh: fresh
        )
        guard case .success(let decision) = checked else {
            if case .failure(let refusal) = checked { return .failure(refusal) }
            return .failure(.changed)
        }
        let host = decision.hostIsLocal ? nil : decision.host
        do {
            switch (action, decision.kind) {
            case (PushCategory.accept, .result(let item)):
                try await client.review(id: item.id, disposition: .accepted, note: nil, option: nil, host: host, via: "notification")
                return .success(String(localized: "Accepted."))
            case (PushCategory.sendBack, .result(let item)):
                try await client.review(id: item.id, disposition: .returned, note: text, option: nil, host: host, via: "notification")
                return .success(String(localized: "Returned."))
            case (PushCategory.answer, .question(let entry)):
                guard let text, !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return .failure(.changed) }
                try await client.answer(requestID: entry.requestID, answer: text, declined: false, host: host, via: "notification")
                return .success(String(localized: "Answered."))
            default:
                return .failure(.otherKind)
            }
        } catch {
            return .failure(.changed)
        }
    }
}
