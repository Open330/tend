// A review item as tendd sends it (`review_list`, the decision queue).
// Shared by Tend for Mac and the iOS app.
import Foundation

// MARK: - Review items (explicit requests)

enum TendReviewDisposition: String, Sendable {
    case accepted, returned, withdrawn
}

/// An explicit request that one result be looked at (`tend review`).
struct TendReviewItem: Decodable, Hashable, Identifiable, Sendable {
    struct Target: Decodable, Hashable, Sendable {
        let kind: String
        let provider: String?
        let sessionID: String?
        let workspace: String?
        let work: String?
        let alias: String?
        let requestID: String?

        enum CodingKeys: String, CodingKey {
            case kind, provider, workspace, work, alias
            case sessionID = "session_id"
            case requestID = "request_id"
        }

        var text: String {
            switch kind {
            case "work":
                let base = [workspace, work].compactMap { $0 }.joined(separator: "/")
                return alias.map { "\(base) · \($0)" } ?? base
            case "session":
                return [provider, sessionID.map { String($0.prefix(8)) }].compactMap { $0 }.joined(separator: " ")
            default:
                return requestID ?? kind
            }
        }
    }

    struct Actor: Decodable, Hashable, Sendable {
        let kind: String
        let label: String?
    }

    let id: String
    let target: Target
    let generation: UInt64
    let summary: String
    let anchor: String?
    let requestedAt: String?
    let requestedBy: Actor
    /// What the asker wants decided (`review_decision_v1`); nil from an
    /// older daemon or when they gave none.
    var decision: TendDecision? = nil
    /// Set once someone decided the item (`decision_history`).
    var disposition: Disposition? = nil
    var idempotencyKey: String? = nil
    /// What tendd ran itself (the Work's declared checks) before the item
    /// reached the person; nil when it ran nothing.
    var verification: TendVerification? = nil

    struct Disposition: Decodable, Hashable, Sendable {
        let kind: String
        let by: Actor
        let at: String
        let note: String?
    }

    enum CodingKeys: String, CodingKey {
        case id, target, generation, summary, anchor, decision, disposition, verification
        case requestedAt = "requested_at"
        case requestedBy = "requested_by"
        case idempotencyKey = "idempotency_key"
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        id = try values.decode(String.self, forKey: .id)
        target = try values.decode(Target.self, forKey: .target)
        generation = try values.decode(UInt64.self, forKey: .generation)
        summary = try values.decode(String.self, forKey: .summary)
        anchor = try values.decodeIfPresent(String.self, forKey: .anchor)
        requestedAt = try values.decodeIfPresent(String.self, forKey: .requestedAt)
        requestedBy = try values.decode(Actor.self, forKey: .requestedBy)
        decision = try values.decodeIfPresent(TendDecision.self, forKey: .decision)
        disposition = try values.decodeIfPresent(Disposition.self, forKey: .disposition)
        idempotencyKey = try values.decodeIfPresent(String.self, forKey: .idempotencyKey)
        // A verification record this app cannot read leaves the item
        // visible, without its checks.
        verification = (try? values.decodeIfPresent(TendLenient<TendVerification>.self, forKey: .verification))?.value
    }

    /// Who asked, as they declared it (not verified).
    var askedBy: String {
        let who = requestedBy.label ?? (requestedBy.kind == "agent" ? String(localized: "an agent") : String(localized: "a person"))
        return String(localized: "Asked by \(who)")
    }
}

