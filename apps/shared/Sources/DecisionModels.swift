// The daemon's decision queue (`decision_queue_v1`): everything pending on
// the person, in the order tendd ranks it, each with the asker's decision
// context: what to decide, the options, their recommendation and evidence.
//
// Every string here is declared by the asker and recorded by tendd; the app
// renders it and never derives or verifies it. Evidence is a pointer the
// person can open; opening it is their own click.
import Foundation

/// Daemons that serve `decision_queue` and `decision_history`.
let tendDecisionQueueCapability = "decision_queue_v1"
/// Daemons that fold questions nobody can answer and keep one on request
/// (`decision_keep`).
let tendSubjectCheckCapability = "subject_check_v1"
/// Daemons that close folded questions on the person's word
/// (`decision_close_unanswerable`); advertised only while `[review]
/// close_unanswerable` is on.
let tendCloseUnanswerableCapability = "close_unanswerable_v1"
/// Daemons whose `decision_history` lists tend's own closures when asked
/// (`include_closures`).
let tendHistoryClosuresCapability = "decision_history_closures_v1"

/// What the asker wants decided and why (`review_decision_v1`,
/// `collaboration_human_decision_v1`). Every field is optional on the wire.
struct TendDecision: Decodable, Hashable, Sendable {
    var needed: String?
    var options: [String] = []
    var recommendation: String?
    var evidence: [TendEvidence] = []

    enum CodingKeys: String, CodingKey {
        case needed, options, recommendation, evidence
    }

    init(needed: String? = nil, options: [String] = [], recommendation: String? = nil, evidence: [TendEvidence] = []) {
        self.needed = needed
        self.options = options
        self.recommendation = recommendation
        self.evidence = evidence
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        needed = try values.decodeIfPresent(String.self, forKey: .needed)
        options = (try? values.decodeIfPresent([String].self, forKey: .options)) ?? []
        recommendation = try values.decodeIfPresent(String.self, forKey: .recommendation)
        // One entry of a kind this app does not know never hides the rest.
        let raw = (try? values.decodeIfPresent([TendLenient<Provenanced>].self, forKey: .evidence)) ?? []
        var evidence: [TendEvidence] = []
        var ranByTend = Set<Int>()
        for entry in raw.compactMap(\.value) {
            if entry.verified { ranByTend.insert(evidence.count) }
            evidence.append(entry.evidence)
        }
        self.evidence = evidence
        self.ranByTend = ranByTend
    }

    /// Indexes into `evidence` of the entries tendd marked `verified`: its
    /// own copies of the checks it ran, never an agent's claim (tendd clears
    /// that flag on everything a client sends).
    var ranByTend: Set<Int> = []

    /// One evidence entry and whether tendd ran it.
    private struct Provenanced: Decodable, Sendable {
        let evidence: TendEvidence
        let verified: Bool

        private enum CodingKeys: String, CodingKey { case verified }

        init(from decoder: Decoder) throws {
            evidence = try TendEvidence(from: decoder)
            verified = (try? decoder.container(keyedBy: CodingKeys.self).decodeIfPresent(Bool.self, forKey: .verified)) ?? false
        }
    }

    var isEmpty: Bool {
        needed == nil && options.isEmpty && recommendation == nil && evidence.isEmpty
    }
}

/// A typed pointer to something the person can open.
enum TendEvidence: Decodable, Hashable, Sendable {
    case commit(sha: String, repo: String?)
    case pullRequest(url: String)
    case diff(range: String, repo: String?)
    case test(command: String, passed: Bool?, tail: String?)
    case path(String)

    private enum CodingKeys: String, CodingKey {
        case kind, sha, repo, url, range, command, passed, tail, path
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        let kind = try values.decode(String.self, forKey: .kind)
        switch kind {
        case "commit":
            self = .commit(sha: try values.decode(String.self, forKey: .sha), repo: try values.decodeIfPresent(String.self, forKey: .repo))
        case "pull_request":
            self = .pullRequest(url: try values.decode(String.self, forKey: .url))
        case "diff":
            self = .diff(range: try values.decode(String.self, forKey: .range), repo: try values.decodeIfPresent(String.self, forKey: .repo))
        case "test":
            self = .test(
                command: try values.decode(String.self, forKey: .command),
                passed: try values.decodeIfPresent(Bool.self, forKey: .passed),
                tail: try values.decodeIfPresent(String.self, forKey: .tail)
            )
        case "path":
            self = .path(try values.decode(String.self, forKey: .path))
        default:
            throw DecodingError.dataCorruptedError(forKey: .kind, in: values, debugDescription: "unknown evidence kind \(kind)")
        }
    }

    /// A review anchor (`commit:SHA`, `diff:RANGE`, a URL) read as evidence,
    /// the way `tend review show` reads it. Anything else stays free-form.
    static func fromAnchor(_ anchor: String) -> TendEvidence? {
        let anchor = anchor.trimmingCharacters(in: .whitespacesAndNewlines)
        if anchor.hasPrefix("https://") || anchor.hasPrefix("http://") { return .pullRequest(url: anchor) }
        guard let colon = anchor.firstIndex(of: ":") else { return nil }
        let kind = anchor[..<colon].trimmingCharacters(in: .whitespaces)
        let value = anchor[anchor.index(after: colon)...].trimmingCharacters(in: .whitespaces)
        switch kind {
        case "commit" where TendRevisionSpec.isCommitSHA(value): return .commit(sha: value, repo: nil)
        case "diff" where TendRevisionSpec.isRange(value): return .diff(range: value, repo: nil)
        case "pr" where value.hasPrefix("https://") || value.hasPrefix("http://"): return .pullRequest(url: value)
        default: return nil
        }
    }

    /// The short label on the evidence chip.
    var label: String {
        switch self {
        case .commit(let sha, _): String(sha.prefix(8))
        case .pullRequest(let url): TendEvidence.pullRequestLabel(url)
        case .diff(let range, _): range
        case .test(let command, _, _): command
        case .path(let path): (path as NSString).lastPathComponent
        }
    }

    var systemImage: String {
        switch self {
        case .commit: "smallcircle.filled.circle"
        case .pullRequest: "arrow.triangle.pull"
        case .diff: "plusminus"
        case .test(_, let passed, _):
            switch passed {
            case true?: "checkmark.circle"
            case false?: "xmark.octagon"
            case nil: "testtube.2"
            }
        case .path: "doc"
        }
    }

    /// `owner/repo#12` for a GitHub pull request, the URL otherwise.
    static func pullRequestLabel(_ url: String) -> String {
        guard let components = URLComponents(string: url), components.host == "github.com" else { return url }
        let parts = components.path.split(separator: "/")
        if parts.count >= 4, parts[2] == "pull" {
            return "\(parts[0])/\(parts[1])#\(parts[3])"
        }
        return url
    }
}

/// Revision syntax the app accepts from evidence before handing it to git:
/// the same rule tendd validates with, and never anything that reads as an
/// option.
enum TendRevisionSpec {
    static func isCommitSHA(_ value: String) -> Bool {
        (4...64).contains(value.count) && value.allSatisfy(\.isHexDigit)
    }

    static func isRange(_ value: String) -> Bool {
        guard !value.isEmpty, !value.hasPrefix("-"), value.utf8.count <= 2048 else { return false }
        let allowed = Set("._/~^@{}-")
        return value.allSatisfy { ($0.isASCII && ($0.isLetter || $0.isNumber)) || allowed.contains($0) }
    }
}

// MARK: - Queue entries

/// A human request (`tend_request_attention`) as the queue lists it.
struct TendHumanRequestEntry: Decodable, Hashable, Sendable {
    let requestID: String
    let action: String?
    let body: String
    let decision: TendDecision?
    let fromSessionID: String
    let fromPane: String
    let fromAlias: String?
    let workspaceID: String?
    let workID: String?
    let createdAt: String
    /// tendd opened it itself (peer recovery, a reply for a gone sender).
    var system: Bool?
    /// Nobody can answer it: its asker has been gone for a day
    /// (`subject_check_v1`). Folded out of the person's turn.
    var unanswerable: TendUnanswerable?

    enum CodingKeys: String, CodingKey {
        case action, body, decision, system, unanswerable
        case requestID = "request_id"
        case fromSessionID = "from_session_id"
        case fromPane = "from_pane"
        case fromAlias = "from_alias"
        case workspaceID = "workspace_id"
        case workID = "work_id"
        case createdAt = "created_at"
    }
}

extension TendHumanRequestEntry {
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        requestID = try values.decode(String.self, forKey: .requestID)
        action = try values.decodeIfPresent(String.self, forKey: .action)
        body = try values.decode(String.self, forKey: .body)
        decision = try values.decodeIfPresent(TendDecision.self, forKey: .decision)
        fromSessionID = try values.decode(String.self, forKey: .fromSessionID)
        fromPane = try values.decode(String.self, forKey: .fromPane)
        fromAlias = try values.decodeIfPresent(String.self, forKey: .fromAlias)
        workspaceID = try values.decodeIfPresent(String.self, forKey: .workspaceID)
        workID = try values.decodeIfPresent(String.self, forKey: .workID)
        createdAt = try values.decode(String.self, forKey: .createdAt)
        system = try values.decodeIfPresent(Bool.self, forKey: .system)
        // A shape this app cannot read leaves the item where it was.
        unanswerable = (try? values.decodeIfPresent(TendLenient<TendUnanswerable>.self, forKey: .unanswerable))??.value
    }
}

/// Why a question can no longer be answered and since when
/// (`subject_check_v1`).
struct TendUnanswerable: Decodable, Hashable, Sendable {
    enum Cause: Hashable, Sendable {
        /// The asking session ended.
        case ended
        /// Another agent started in its pane.
        case replaced
        /// Its pane is gone.
        case paneGone
        /// The agent exited; a shell is left in its pane.
        case agentExited
        case other(String)

        init(_ raw: String) {
            switch raw {
            case "ended": self = .ended
            case "replaced": self = .replaced
            case "pane_gone": self = .paneGone
            case "agent_exited": self = .agentExited
            default: self = .other(raw)
            }
        }

        /// Why, for people.
        var text: String {
            switch self {
            case .ended: String(localized: "Session ended")
            case .replaced: String(localized: "Replaced by another agent")
            case .paneGone: String(localized: "Pane gone")
            case .agentExited: String(localized: "Agent exited")
            case .other: String(localized: "Asker gone")
            }
        }
    }

    var cause: Cause
    /// The evidence episode; Keep and Close all name it.
    var episode: String
    var since: String
    var pane: String?

    enum CodingKeys: String, CodingKey { case cause, episode, since, pane }

    init(cause: Cause, episode: String, since: String, pane: String? = nil) {
        self.cause = cause
        self.episode = episode
        self.since = since
        self.pane = pane
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        cause = Cause(try values.decode(String.self, forKey: .cause))
        episode = try values.decode(String.self, forKey: .episode)
        since = try values.decode(String.self, forKey: .since)
        pane = try? values.decodeIfPresent(String.self, forKey: .pane)
    }
}

/// What `decision_close_unanswerable` closed and skipped.
struct TendCloseOutcome: Decodable, Hashable, Sendable {
    struct Skipped: Decodable, Hashable, Sendable {
        var requestID: String
        var reason: String

        enum CodingKeys: String, CodingKey {
            case reason
            case requestID = "request_id"
        }
    }

    var closed: [String] = []
    var skipped: [Skipped] = []

    enum CodingKeys: String, CodingKey { case closed, skipped }

    init(closed: [String] = [], skipped: [Skipped] = []) {
        self.closed = closed
        self.skipped = skipped
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        closed = (try? values.decodeIfPresent([String].self, forKey: .closed)) ?? []
        skipped = ((try? values.decodeIfPresent([TendLenient<Skipped>].self, forKey: .skipped)) ?? []).compactMap(\.value)
    }
}

/// An agent blocked in its pane. Home already shows these as Work cards in
/// Your turn, so the queue's copy is decoded but not drawn twice.
struct TendAgentWaitingEntry: Decodable, Hashable, Sendable {
    let sessionID: String
    let agentKind: String
    let pane: String?
    let state: String
    let message: String?
    let since: String

    enum CodingKeys: String, CodingKey {
        case pane, state, message, since
        case sessionID = "session_id"
        case agentKind = "agent_kind"
    }
}

enum TendPendingItem: Decodable, Hashable, Sendable {
    case humanRequest(TendHumanRequestEntry)
    case agentWaiting(TendAgentWaitingEntry)
    case review(TendReviewItem)

    private enum CodingKeys: String, CodingKey { case kind }

    init(from decoder: Decoder) throws {
        let kind = try decoder.container(keyedBy: CodingKeys.self).decode(String.self, forKey: .kind)
        switch kind {
        case "human_request": self = .humanRequest(try TendHumanRequestEntry(from: decoder))
        case "agent_waiting": self = .agentWaiting(try TendAgentWaitingEntry(from: decoder))
        case "review": self = .review(try TendReviewItem(from: decoder))
        default:
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "unknown queue item \(kind)"))
        }
    }
}

/// `decision_queue`'s answer. Entries this app cannot read are dropped, not
/// fatal; `unavailable` names sources tendd could not read.
struct TendPendingQueue: Decodable, Hashable, Sendable {
    var items: [TendPendingItem] = []
    var truncated = false
    var unavailable: [String] = []
    /// Entries tendd sent that this app could not read.
    var undecoded = 0
    /// How many open questions nobody can answer, exact even when their
    /// entries were cut (`subject_check_v1`; 0 from older daemons).
    var unanswerableCount = 0

    /// Every source was read in full and every entry decoded: what the
    /// queue leaves out is not pending.
    var isComplete: Bool { !truncated && unavailable.isEmpty && undecoded == 0 }

    enum CodingKeys: String, CodingKey {
        case items, truncated, unavailable
        case unanswerableCount = "unanswerable_count"
    }

    init(items: [TendPendingItem] = [], truncated: Bool = false, unavailable: [String] = [], unanswerableCount: Int = 0) {
        self.items = items
        self.truncated = truncated
        self.unavailable = unavailable
        self.unanswerableCount = unanswerableCount
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        let raw = (try? values.decodeIfPresent([TendLenient<TendPendingItem>].self, forKey: .items)) ?? []
        items = raw.compactMap(\.value)
        undecoded = raw.count - items.count
        truncated = (try? values.decodeIfPresent(Bool.self, forKey: .truncated)) ?? false
        unavailable = (try? values.decodeIfPresent([String].self, forKey: .unavailable)) ?? []
        unanswerableCount = (try? values.decodeIfPresent(Int.self, forKey: .unanswerableCount)) ?? 0
    }
}

/// What tendd established itself before a result reached the person: the
/// Work's declared checks, run by tendd (`work_verify_v1`). Unlike the
/// decision, nothing here is the agent's claim. Lenient: a field this app
/// does not read never hides the item.
struct TendVerification: Decodable, Hashable, Sendable {
    struct Check: Decodable, Hashable, Sendable {
        var command: String
        var name: String?
        var passed: Bool
        var exitCode: Int?
        var timedOut: Bool
        var tail: String

        enum CodingKeys: String, CodingKey {
            case command, name, passed, tail
            case exitCode = "exit_code"
            case timedOut = "timed_out"
        }

        init(command: String, name: String? = nil, passed: Bool, exitCode: Int? = nil, timedOut: Bool = false, tail: String = "") {
            self.command = command
            self.name = name
            self.passed = passed
            self.exitCode = exitCode
            self.timedOut = timedOut
            self.tail = tail
        }

        init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            command = try values.decode(String.self, forKey: .command)
            name = try? values.decodeIfPresent(String.self, forKey: .name)
            passed = (try? values.decodeIfPresent(Bool.self, forKey: .passed)) ?? false
            exitCode = try? values.decodeIfPresent(Int.self, forKey: .exitCode)
            timedOut = (try? values.decodeIfPresent(Bool.self, forKey: .timedOut)) ?? false
            tail = (try? values.decodeIfPresent(String.self, forKey: .tail)) ?? ""
        }

        /// `unit`, or the command when the check has no name.
        var label: String { name?.nilIfEmpty ?? command }

        /// How a failed check ended: `exit 2`, `timed out`, `killed`.
        var ending: String? {
            guard !passed else { return nil }
            if timedOut { return String(localized: "timed out") }
            if let exitCode { return String(localized: "exit \(exitCode)") }
            return String(localized: "killed")
        }
    }

    var passed: Bool
    var ranAt: String?
    var checks: [Check]

    enum CodingKeys: String, CodingKey {
        case passed, checks
        case ranAt = "ran_at"
    }

    init(passed: Bool, ranAt: String? = nil, checks: [Check]) {
        self.passed = passed
        self.ranAt = ranAt
        self.checks = checks
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        checks = ((try? values.decodeIfPresent([TendLenient<Check>].self, forKey: .checks)) ?? []).compactMap(\.value)
        passed = ((try? values.decodeIfPresent(Bool.self, forKey: .passed)) ?? false) && checks.allSatisfy(\.passed)
        ranAt = try? values.decodeIfPresent(String.self, forKey: .ranAt)
    }

    /// `tend ran 3/3 checks, all passed`.
    var headline: String {
        let ok = checks.filter(\.passed).count
        return checks.count == 1
            ? String(localized: "tend ran 1 check: \(ok == 1 ? String(localized: "passed") : String(localized: "failed"))")
            : String(localized: "tend ran \(checks.count) checks: \(ok) passed")
    }
}

extension TendReviewItem {
    /// The typed evidence, then the free-form anchor when it reads as
    /// evidence and is not already listed (as `tend review show` does).
    /// tendd also copies its checks into the evidence (marked `verified`)
    /// for clients that read only the decision; with the checks shown on
    /// their own, those copies are left out. An agent's own entry stays,
    /// even for the same command.
    var allEvidence: [TendEvidence] {
        let all = decision?.evidence ?? []
        let skip = verification?.checks.isEmpty == false ? (decision?.ranByTend ?? []) : []
        var evidence = all.enumerated().filter { !skip.contains($0.offset) }.map(\.element)
        if let anchor = anchor.flatMap(TendEvidence.fromAnchor), !evidence.contains(anchor) {
            evidence.append(anchor)
        }
        return evidence
    }
}

// MARK: - History

/// A human request and the answer it got.
struct TendHumanAnswerEntry: Decodable, Hashable, Sendable {
    let request: TendHumanRequestEntry
    /// `completed` for an answer; `declined`, `failed` or `blocked` when the
    /// person said so.
    let status: String
    let answer: String
    let answeredAt: String
    /// A person closed it because its asker was gone (Close all).
    var closure: TendQuestionClosure?

    enum CodingKeys: String, CodingKey {
        case status, answer, closure
        case answeredAt = "answered_at"
    }

    init(request: TendHumanRequestEntry, status: String, answer: String, answeredAt: String, closure: TendQuestionClosure? = nil) {
        self.request = request
        self.status = status
        self.answer = answer
        self.answeredAt = answeredAt
        self.closure = closure
    }

    init(from decoder: Decoder) throws {
        // The request's fields sit beside the answer's (serde flatten).
        request = try TendHumanRequestEntry(from: decoder)
        let values = try decoder.container(keyedBy: CodingKeys.self)
        status = try values.decode(String.self, forKey: .status)
        answer = try values.decode(String.self, forKey: .answer)
        answeredAt = try values.decode(String.self, forKey: .answeredAt)
        closure = (try? values.decodeIfPresent(TendLenient<TendQuestionClosure>.self, forKey: .closure))??.value
    }
}

/// Why a person closed a question: its asker was gone (Close all).
struct TendQuestionClosure: Decodable, Hashable, Sendable {
    var cause: TendUnanswerable.Cause
    /// `cli`, `web`, `mac` or `ios`.
    var surface: String

    enum CodingKeys: String, CodingKey { case cause, surface }

    init(cause: TendUnanswerable.Cause, surface: String) {
        self.cause = cause
        self.surface = surface
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        cause = TendUnanswerable.Cause((try? values.decode(String.self, forKey: .cause)) ?? "")
        surface = (try? values.decodeIfPresent(String.self, forKey: .surface)) ?? ""
    }

    /// `Closed: Pane gone`.
    var text: String { String(localized: "Closed, asker gone: \(cause.text)") }
}

/// An item tendd opened itself and closed without an answer, with why
/// (`decision_history_closures_v1`, only when asked for).
struct TendClosedByTendEntry: Decodable, Hashable, Sendable {
    let request: TendHumanRequestEntry
    /// `parent_missing`, `parent_closed`, `stale` or `collapsed`.
    let reason: String
    /// The reason in words, as `tend review cleanup` prints it.
    let reasonText: String
    let closedAt: String

    enum CodingKeys: String, CodingKey {
        case reason
        case reasonText = "reason_text"
        case closedAt = "closed_at"
    }

    init(request: TendHumanRequestEntry, reason: String, reasonText: String, closedAt: String) {
        self.request = request
        self.reason = reason
        self.reasonText = reasonText
        self.closedAt = closedAt
    }

    init(from decoder: Decoder) throws {
        request = try TendHumanRequestEntry(from: decoder)
        let values = try decoder.container(keyedBy: CodingKeys.self)
        reason = try values.decode(String.self, forKey: .reason)
        reasonText = (try? values.decodeIfPresent(String.self, forKey: .reasonText)) ?? reason
        closedAt = try values.decode(String.self, forKey: .closedAt)
    }

    /// Why, for people.
    var reasonDescription: String {
        switch reason {
        case "parent_missing": String(localized: "Closed by tend: the original request is gone")
        case "parent_closed": String(localized: "Closed by tend: the original request no longer waits")
        case "stale": String(localized: "Closed by tend: nobody is left to act on it")
        case "collapsed": String(localized: "Closed by tend: merged into another item")
        default: String(localized: "Closed by tend: \(reasonText)")
        }
    }
}

enum TendDecidedItem: Decodable, Hashable, Sendable {
    case review(TendReviewItem)
    case humanAnswer(TendHumanAnswerEntry)
    /// Only from a history read with `include_closures`.
    case closedByTend(TendClosedByTendEntry)

    private enum CodingKeys: String, CodingKey { case kind }

    init(from decoder: Decoder) throws {
        let kind = try decoder.container(keyedBy: CodingKeys.self).decode(String.self, forKey: .kind)
        switch kind {
        case "review": self = .review(try TendReviewItem(from: decoder))
        case "human_answer": self = .humanAnswer(try TendHumanAnswerEntry(from: decoder))
        case "closed_by_tend": self = .closedByTend(try TendClosedByTendEntry(from: decoder))
        default:
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "unknown decided item \(kind)"))
        }
    }
}

/// `decision_history`'s answer, newest first.
struct TendDecisionHistory: Decodable, Hashable, Sendable {
    var items: [TendDecidedItem] = []
    var truncated = false
    var unavailable: [String] = []
    /// Entries tendd sent that this app could not read.
    var undecoded = 0

    enum CodingKeys: String, CodingKey { case items, truncated, unavailable }

    init(items: [TendDecidedItem] = [], truncated: Bool = false, unavailable: [String] = []) {
        self.items = items
        self.truncated = truncated
        self.unavailable = unavailable
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        let raw = (try? values.decodeIfPresent([TendLenient<TendDecidedItem>].self, forKey: .items)) ?? []
        items = raw.compactMap(\.value)
        undecoded = raw.count - items.count
        truncated = (try? values.decodeIfPresent(Bool.self, forKey: .truncated)) ?? false
        unavailable = (try? values.decodeIfPresent([String].self, forKey: .unavailable)) ?? []
    }
}

/// Text the review store accepts: no control characters but newline and
/// tab, at most `maxBytes` of UTF-8, cut on a character boundary.
enum TendRecordText {
    static func bounded(_ text: String, maxBytes: Int) -> String {
        var scalars = String.UnicodeScalarView()
        for scalar in text.unicodeScalars
        where scalar == "\n" || scalar == "\t" || scalar.properties.generalCategory != .control {
            scalars.append(scalar)
        }
        let cleaned = String(scalars).trimmingCharacters(in: .whitespacesAndNewlines)
        guard cleaned.utf8.count > maxBytes else { return cleaned }
        var result = ""
        var used = 0
        let ellipsis = "…"
        for character in cleaned {
            let size = character.utf8.count
            if used + size > maxBytes - ellipsis.utf8.count { break }
            result.append(character)
            used += size
        }
        return result + ellipsis
    }
}

private extension String {
    var nilIfEmpty: String? { isEmpty ? nil : self }
}
