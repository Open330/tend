// Push notifications (docs/proposals/IOS_APP.md, "Push (phase 2, wave 7)").
// The dashboard's host sends to Apple; a payload says only "something on
// this host may need you": its channel ties it to this pairing, and its
// `ref` names the newest item without naming it (HMAC under a key only the
// host and this phone hold). Nothing in a payload is trusted further: the
// item is read from the host before it is shown or decided.
import CryptoKit
import Foundation

/// What a registration gave this phone, kept in the Keychain with its
/// pairing: a notification from any other channel is not acted on.
struct PushBinding: Codable, Hashable, Sendable {
    let channel: String
    /// `ref_key`, decoded.
    let refKey: Data
    /// Whether the host lets this registration's notifications carry text.
    let content: Bool
}

/// The `tend` part of a notification, as sent; untrusted.
struct PushPayload: Equatable, Sendable {
    let channel: String
    let host: String
    let ref: String
    let category: String
    /// What the notification showed (empty without content).
    let body: String
    let subtitle: String

    /// From `UNNotificationContent.userInfo` plus what it displayed.
    init?(userInfo: [AnyHashable: Any], category: String, body: String, subtitle: String) {
        guard let tend = userInfo["tend"] as? [String: Any],
              (tend["v"] as? Int) == 1,
              let channel = tend["channel"] as? String, !channel.isEmpty,
              let host = tend["host"] as? String, !host.isEmpty,
              let ref = tend["ref"] as? String, ref.count == 32
        else { return nil }
        self.channel = channel
        self.host = host
        self.ref = ref.lowercased()
        self.category = category
        self.body = body
        self.subtitle = subtitle
    }
}

enum PushCategory {
    static let open = "TEND_OPEN"
    static let review = "TEND_REVIEW"
    static let question = "TEND_QUESTION"
    static let waiting = "TEND_WAITING"

    static let accept = "TEND_ACCEPT"
    static let sendBack = "TEND_SEND_BACK"
    static let answer = "TEND_ANSWER"
}

enum PushRef {
    /// The first 16 bytes of HMAC-SHA256(key, host NUL kind NUL id), as
    /// hex: how the host names an item in a payload.
    static func of(key: Data, host: String, kind: String, id: String) -> String {
        let message = Data("\(host)\u{0}\(kind)\u{0}\(id)".utf8)
        let mac = HMAC<SHA256>.authenticationCode(for: message, using: SymmetricKey(data: key))
        return Data(mac).prefix(16).map { String(format: "%02x", $0) }.joined()
    }

    /// The kind and id a decision is keyed by.
    static func key(_ decision: PendingDecision) -> (kind: String, id: String) {
        switch decision.kind {
        case .question(let entry): ("request", entry.requestID)
        case .result(let item): ("review", item.id)
        case .waiting(let entry): ("waiting", entry.sessionID)
        }
    }

    static func of(key: Data, _ decision: PendingDecision) -> String {
        let (kind, id) = Self.key(decision)
        return of(key: key, host: decision.host, kind: kind, id: id)
    }
}

/// The one line a notification with content shows, worded the same way on
/// the host (shared test vectors in apps/shared/Fixtures/push-vectors.json):
/// the first line not empty once its Markdown marks are removed and its
/// whitespace collapsed, at most 120 Unicode scalars.
enum CanonicalLine {
    static let limit = 120

    /// The same steps as the host's `canonical_line`, on Unicode scalars
    /// (Rust's `char`): lines split on LF with a trailing CR dropped; the
    /// first line not empty once cleaned; at most 120 scalars.
    static func of(_ text: String) -> String {
        let lines = text.unicodeScalars.split(separator: "\n", omittingEmptySubsequences: false).map { line -> [Unicode.Scalar] in
            var line = Array(line)
            if line.last == "\r" { line.removeLast() }
            return line
        }
        let line = lines.lazy.map(clean).first { !$0.isEmpty } ?? []
        guard line.count > limit else { return string(line) }
        return string(Array(line.prefix(limit - 1))) + "…"
    }

    private static func clean(_ line: [Unicode.Scalar]) -> [Unicode.Scalar] {
        var rest = line.drop { $0.properties.isWhitespace }
        // Repeated heading and quote marks, each with the spaces after it.
        while let first = rest.first, first == "#" || first == ">" {
            rest = rest.dropFirst().drop { $0 == " " }
        }
        // One list marker.
        for marker in ["- ", "* ", "+ "] where rest.starts(with: marker.unicodeScalars) {
            rest = rest.dropFirst(2)
            break
        }
        let kept = rest.filter { !["*", "_", "`", "~"].contains($0) }
        return Array(kept.split(whereSeparator: { $0.properties.isWhitespace }).joined(separator: [" "]))
    }

    private static func string(_ scalars: [Unicode.Scalar]) -> String {
        var view = String.UnicodeScalarView()
        view.append(contentsOf: scalars)
        return String(view)
    }

    /// What a decision's notification would show: the question's body, the
    /// review's summary, or the waiting agent's message.
    static func of(_ decision: PendingDecision) -> String {
        switch decision.kind {
        case .question(let entry): of(entry.body)
        case .result(let item): of(item.summary)
        case .waiting(let entry): of(entry.message ?? "")
        }
    }
}

/// Why a notification's button decided nothing; the item is opened instead.
enum PushRefusal: Error, Equatable {
    case otherChannel, contentOff, incompleteRead, notFound, ambiguous, otherKind, notDecidable, changed, hasOptions
}

/// The checks before a notification's button may decide: every one must
/// hold, or nothing is decided.
enum PushActionCheck {
    static func verify(
        _ payload: PushPayload,
        binding: PushBinding?,
        contentSwitch: Bool,
        hostAllowsContent: Bool,
        fresh section: HostQueue?
    ) -> Result<PendingDecision, PushRefusal> {
        guard let binding, binding.channel == payload.channel else { return .failure(.otherChannel) }
        guard contentSwitch, hostAllowsContent, binding.content else { return .failure(.contentOff) }
        guard let section, section.alias == payload.host,
              section.unavailable.isEmpty, section.queue.isComplete
        else { return .failure(.incompleteRead) }
        let matches = PendingDecision.from(section).filter { PushRef.of(key: binding.refKey, $0) == payload.ref }
        guard let decision = matches.first else { return .failure(.notFound) }
        guard matches.count == 1 else { return .failure(.ambiguous) }
        switch (payload.category, decision.kind) {
        case (PushCategory.review, .result), (PushCategory.question, .question): break
        default: return .failure(.otherKind)
        }
        guard decision.decidable else { return .failure(.notDecidable) }
        guard CanonicalLine.of(decision) == payload.body, label(decision) == payload.subtitle else {
            return .failure(.changed)
        }
        if case .question = decision.kind, decision.decision != nil { return .failure(.hasOptions) }
        return .success(decision)
    }

    /// The subtitle the host sends, from the same wire fields: who asked.
    static func label(_ decision: PendingDecision) -> String {
        switch decision.kind {
        case .question(let entry): entry.fromAlias ?? entry.fromPane
        case .result(let item): item.requestedBy.label ?? item.requestedBy.kind
        case .waiting(let entry): entry.agentKind
        }
    }
}
