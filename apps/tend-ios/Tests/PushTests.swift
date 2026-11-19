import XCTest
@testable import Tend

/// Notifications on the phone: what a payload may name, how an item is
/// found from its `ref`, and every reason a notification's button decides
/// nothing.
final class PushTests: XCTestCase {
    private let key = Data((0..<32).map { UInt8($0) })
    private var binding: PushBinding { PushBinding(channel: "ch_1", refKey: key, content: true) }

    private func section(_ items: String, truncated: Bool = false, unavailable: String = "") throws -> HostQueue {
        let json = #"{"hosts":[{"alias":"mini","local":false,"control":true,"decides_reviews":true,"covers":[],"unavailable":[\#(unavailable)],"queue":{"items":[\#(items)],"truncated":\#(truncated),"unavailable":[]}}]}"#
        struct Hosts: Decodable { let hosts: [HostQueue] }
        return try JSONDecoder().decode(Hosts.self, from: Data(json.utf8)).hosts[0]
    }

    private let review = #"{"kind":"review","id":"rev_1","target":{"kind":"agent","label":"impl"},"generation":1,"summary":"**Merge** the retry path now?\nmore detail","requested_by":{"kind":"agent","label":"impl"},"requested_at":"2026-11-19T10:00:00Z"}"#
    private let question = #"{"kind":"human_request","request_id":"req_1","body":"Ship it behind a flag?","from_session_id":"s1","from_pane":"%7","from_alias":"lead","created_at":"2026-11-19T10:00:00Z"}"#
    private let choice = #"{"kind":"human_request","request_id":"req_2","body":"Which one?","decision":{"question":"Which one?","options":[{"label":"a"},{"label":"b"}]},"from_session_id":"s1","from_pane":"%7","created_at":"2026-11-19T10:00:00Z"}"#

    private func payload(_ ref: String, category: String, body: String, subtitle: String, channel: String = "ch_1") -> PushPayload {
        PushPayload(userInfo: ["tend": ["v": 1, "channel": channel, "host": "mini", "ref": ref]], category: category, body: body, subtitle: subtitle)!
    }

    private func ref(_ kind: String, _ id: String) -> String {
        PushRef.of(key: key, host: "mini", kind: kind, id: id)
    }

    func testAPayloadNamesOnlyAChannelAHostAndARef() {
        XCTAssertNil(PushPayload(userInfo: [:], category: "", body: "", subtitle: ""))
        XCTAssertNil(PushPayload(userInfo: ["tend": ["v": 2, "channel": "c", "host": "h", "ref": String(repeating: "a", count: 32)]], category: "", body: "", subtitle: ""))
        XCTAssertNil(PushPayload(userInfo: ["tend": ["v": 1, "channel": "c", "host": "h", "ref": "short"]], category: "", body: "", subtitle: ""))
        let parsed = PushPayload(userInfo: ["tend": ["v": 1, "channel": "c", "host": "h", "ref": String(repeating: "A", count: 32)]], category: "TEND_OPEN", body: "", subtitle: "")
        XCTAssertEqual(parsed?.ref, String(repeating: "a", count: 32))
    }

    func testTheCanonicalLine() {
        XCTAssertEqual(CanonicalLine.of("\n\n## **Merge**  the `retry`  path\nsecond"), "Merge the retry path")
        XCTAssertEqual(CanonicalLine.of("> - _quoted_ item"), "quoted item")
        let long = String(repeating: "word ", count: 40)
        let line = CanonicalLine.of(long)
        XCTAssertEqual(line.count, 120)
        XCTAssertTrue(line.hasSuffix("…"))
    }

    /// The vectors the host's tests use too, when the file is there.
    func testTheSharedVectors() throws {
        guard let url = Bundle(for: Self.self).url(forResource: "push-vectors", withExtension: "json") else {
            throw XCTSkip("apps/shared/Fixtures/push-vectors.json is not in this checkout yet")
        }
        struct Vectors: Decodable {
            struct Ref: Decodable { let key_hex: String; let host: String; let kind: String; let id: String; let ref: String }
            struct Line: Decodable { let text: String; let line: String }
            let ref: [Ref]
            let canonical: [Line]
        }
        let vectors = try JSONDecoder().decode(Vectors.self, from: Data(contentsOf: url))
        for v in vectors.ref {
            let bytes = stride(from: 0, to: v.key_hex.count, by: 2).map {
                UInt8(v.key_hex[v.key_hex.index(v.key_hex.startIndex, offsetBy: $0)...].prefix(2), radix: 16)!
            }
            XCTAssertEqual(PushRef.of(key: Data(bytes), host: v.host, kind: v.kind, id: v.id), v.ref, "\(v.kind) \(v.id)")
        }
        for v in vectors.canonical {
            XCTAssertEqual(CanonicalLine.of(v.text), v.line, v.text)
        }
    }

    func testAButtonDecidesOnlyWhatTheNotificationShowed() throws {
        let fresh = try section(review)
        let shown = payload(ref("review", "rev_1"), category: PushCategory.review, body: "Merge the retry path now?", subtitle: "impl")
        func check(_ p: PushPayload, binding: PushBinding? = nil, contentSwitch: Bool = true, hostAllows: Bool = true, fresh: HostQueue?) -> Result<PendingDecision, PushRefusal> {
            PushActionCheck.verify(p, binding: binding ?? self.binding, contentSwitch: contentSwitch, hostAllowsContent: hostAllows, fresh: fresh)
        }
        guard case .success(let decision) = check(shown, fresh: fresh) else { return XCTFail("the shown review was refused") }
        XCTAssertEqual(PushRef.key(decision).id, "rev_1")

        XCTAssertEqual(check(payload(shown.ref, category: shown.category, body: shown.body, subtitle: shown.subtitle, channel: "ch_old"), fresh: fresh), .failure(.otherChannel))
        XCTAssertEqual(check(shown, contentSwitch: false, fresh: fresh), .failure(.contentOff))
        XCTAssertEqual(check(shown, hostAllows: false, fresh: fresh), .failure(.contentOff))
        XCTAssertEqual(check(shown, binding: PushBinding(channel: "ch_1", refKey: key, content: false), fresh: fresh), .failure(.contentOff))
        XCTAssertEqual(check(shown, fresh: nil), .failure(.incompleteRead))
        XCTAssertEqual(check(shown, fresh: try section(review, truncated: true)), .failure(.incompleteRead))
        XCTAssertEqual(check(shown, fresh: try section(review, unavailable: #""reviews""#)), .failure(.incompleteRead))
        XCTAssertEqual(check(shown, fresh: try section(question)), .failure(.notFound))
        XCTAssertEqual(check(payload(shown.ref, category: PushCategory.question, body: shown.body, subtitle: shown.subtitle), fresh: fresh), .failure(.otherKind))
        XCTAssertEqual(check(payload(shown.ref, category: shown.category, body: "Something else", subtitle: shown.subtitle), fresh: fresh), .failure(.changed))
        XCTAssertEqual(check(payload(shown.ref, category: shown.category, body: shown.body, subtitle: "someone"), fresh: fresh), .failure(.changed))
        // The same item twice: never guess which one.
        XCTAssertEqual(check(shown, fresh: try section(review + "," + review)), .failure(.ambiguous))
    }

    func testAQuestionWithOptionsAlwaysOpensTheApp() throws {
        let shown = payload(ref("request", "req_2"), category: PushCategory.question, body: "Which one?", subtitle: "%7")
        XCTAssertEqual(PushActionCheck.verify(shown, binding: binding, contentSwitch: true, hostAllowsContent: true, fresh: try section(choice)), .failure(.hasOptions))
        let plain = payload(ref("request", "req_1"), category: PushCategory.question, body: "Ship it behind a flag?", subtitle: "lead")
        if case .failure(let refusal) = PushActionCheck.verify(plain, binding: binding, contentSwitch: true, hostAllowsContent: true, fresh: try section(question)) {
            XCTFail("a plain question was refused: \(refusal)")
        }
    }
}
