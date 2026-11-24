import Foundation
import Testing
@testable import Tend

private func decodingHost(_ alias: String, prompt: String = "hello", activity: String = "2026-11-24T00:00:00Z") -> [String: Any] {
    [
        "alias": alias, "local": alias == "local", "mode": "control", "state": "online",
        "latency_ms": 12, "labels": ["role": "build"],
        "remote": [
            "panes": [[
                "pane_id": "%1", "session_id": "$1", "session": "main", "window_id": "@1",
                "window_name": "w", "window_index": "0", "pane_index": "1",
                "current_command": "zsh", "title": "", "current_path": "/tmp", "socket": "default",
            ]],
            "agents": [[
                "kind": "codex", "agent_session_id": "\(alias)-agent", "state": "working", "pane": "%1",
                "last_prompt": prompt, "last_activity_at": activity,
            ]],
        ],
    ]
}

private func fleetResponse(_ hosts: [[String: Any]], extra: [String: Any] = [:]) throws -> Data {
    var object: [String: Any] = ["ok": true, "protocol": 6, "fleet": ["generated_at": "now", "hosts": hosts]]
    object.merge(extra) { $1 }
    // Sorted keys: tendd writes each host the same way every time.
    var data = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
    data.append(0x0A) // A response line keeps its newline.
    return data
}

private func fullDecode(_ data: Data) throws -> [TendFleetHost] {
    try #require(JSONDecoder().decode(TendIPCResponse.self, from: data).fleet).hosts
}

@Test func fleetHostCacheDecodesTheSameHostsAsTheFullResponse() throws {
    // Brackets, quotes, escapes and non-ASCII inside strings never end a host.
    let tricky = "a \"quoted\" ]} {[ \\ back\\slash \u{1F600} 한국어 \n line"
    let data = try fleetResponse([decodingHost("local", prompt: tricky), decodingHost("rtzr"), decodingHost("mini", prompt: "}")])
    var cache = TendFleetHostDecodeCache()
    let hosts = try #require(try cache.hosts(fromResponse: data))
    #expect(hosts == (try fullDecode(data)))
    #expect(hosts.first?.remote?.agents.first?.lastPrompt == tricky)
    #expect(cache.lastDecodedCount == 3)

    // Whitespace between tokens (a pretty-printed response) scans too.
    let pretty = try JSONSerialization.data(withJSONObject: JSONSerialization.jsonObject(with: data), options: [.prettyPrinted])
    var other = TendFleetHostDecodeCache()
    #expect(try other.hosts(fromResponse: pretty) == hosts)
}

@Test func fleetHostCacheDecodesOnlyTheHostsThatChanged() throws {
    var cache = TendFleetHostDecodeCache()
    let first = try fleetResponse([decodingHost("local"), decodingHost("rtzr"), decodingHost("mini")])
    _ = try cache.hosts(fromResponse: first)
    #expect(cache.lastDecodedCount == 3)

    // One host's agent moved on: only that host is decoded again.
    let second = try fleetResponse([decodingHost("local"), decodingHost("rtzr", activity: "2026-11-24T00:00:05Z"), decodingHost("mini")])
    let hosts = try #require(try cache.hosts(fromResponse: second))
    #expect(cache.lastDecodedCount == 1)
    #expect(hosts == (try fullDecode(second)))

    // A host removed and the rest reordered: still reused, by content.
    let third = try fleetResponse([decodingHost("mini"), decodingHost("local")])
    let reordered = try #require(try cache.hosts(fromResponse: third))
    #expect(cache.lastDecodedCount == 0)
    #expect(reordered.map(\.alias) == ["mini", "local"])
    #expect(reordered == (try fullDecode(third)))
}

@Test func fleetHostCacheLeavesAnythingUnexpectedToTheFullDecode() throws {
    var cache = TendFleetHostDecodeCache()
    // A failure, a missing or null fleet, or malformed JSON: nil, so the
    // caller decodes the whole response and reports what tendd said.
    let failure = try JSONSerialization.data(withJSONObject: ["ok": false, "error": "nope", "fleet": ["hosts": []]])
    #expect(try cache.hosts(fromResponse: failure) == nil)
    let missing = try JSONSerialization.data(withJSONObject: ["ok": true, "protocol": 6])
    #expect(try cache.hosts(fromResponse: missing) == nil)
    let null = Data(#"{"ok":true,"fleet":null}"#.utf8)
    #expect(try cache.hosts(fromResponse: null) == nil)
    let truncated = try fleetResponse([decodingHost("local")]).dropLast(10)
    #expect(try cache.hosts(fromResponse: Data(truncated)) == nil)
    let trailing = Data(#"{"ok":true,"fleet":{"hosts":[]}} x"#.utf8)
    #expect(try cache.hosts(fromResponse: trailing) == nil)
    let mismatched = Data(#"{"ok":true,"fleet":{"hosts":[{"alias":"a"]}}"#.utf8)
    #expect(try cache.hosts(fromResponse: mismatched) == nil)
    let okString = Data(#"{"ok":"true","fleet":{"hosts":[]}}"#.utf8)
    #expect(try cache.hosts(fromResponse: okString) == nil)
    // Other members, before or after, are skipped.
    let extra = try fleetResponse([decodingHost("local")], extra: ["capabilities": ["a", "b"], "z": [["x": [1, 2.5e3, -1, true, false, NSNull()]]]])
    #expect(try cache.hosts(fromResponse: extra)?.map(\.alias) == ["local"])
    // An empty fleet is a fleet.
    #expect(try cache.hosts(fromResponse: fleetResponse([])) == [])
}

@Test func fleetHostCacheDeclinesWhatTheFullDecoderWouldRejectOrReadDifferently() throws {
    var cache = TendFleetHostDecodeCache()
    let host = String(decoding: try JSONSerialization.data(withJSONObject: decodingHost("local"), options: [.sortedKeys]), as: UTF8.self)
    let cases = [
        // Malformed members the scan only skips.
        #"{"ok":true,"z":tru,"fleet":{"hosts":[HOST]}}"#,
        #"{"ok":true,"z":{"a" 1},"fleet":{"hosts":[HOST]}}"#,
        // Valid JSON, but a known field of the wrong type.
        #"{"ok":true,"capabilities":false,"fleet":{"hosts":[HOST]}}"#,
        // Repeated keys: the full decoder's choice is the one that counts.
        #"{"ok":false,"ok":true,"fleet":{"hosts":[HOST]}}"#,
        #"{"ok":true,"fleet":{"hosts":[]},"fleet":{"hosts":[HOST]}}"#,
        #"{"ok":true,"fleet":{"hosts":[],"hosts":[HOST]}}"#,
        // An escaped key the decoder reads as a repeat.
        #"{"ok":true,"fleet":{"h\u006fsts":[],"hosts":[HOST]}}"#,
        #"{"ok":true,"fl\u0065et":{"hosts":[]},"fleet":{"hosts":[HOST]}}"#,
    ]
    for json in cases {
        #expect(try cache.hosts(fromResponse: Data(json.replacingOccurrences(of: "HOST", with: host).utf8)) == nil, "\(json)")
    }
    // Nesting past the scan's conservative cap is left to the full decoder.
    let deep = String(repeating: "[", count: 100) + String(repeating: "]", count: 100)
    let nested = Data(#"{"ok":true,"fleet":{"hosts":[{"alias":"a","local":false,"mode":"control","state":"online","x":DEEP}]}}"#
        .replacingOccurrences(of: "DEEP", with: deep).utf8)
    #expect(try cache.hosts(fromResponse: nested) == nil)
    // A host the decoder rejects is an error, as in the full decode.
    let badHost = Data(#"{"ok":true,"fleet":{"hosts":[{"alias":1}]}}"#.utf8)
    #expect(throws: (any Error).self) { try cache.hosts(fromResponse: badHost) }
}

@Test func fleetSnapshotRequestKeepsDecodingErrorsAndRejections() async throws {
    let responses = LockedResponses([
        Data(#"{"ok":true,"capabilities":false,"fleet":{"hosts":[]}}"#.utf8),
        Data(#"{"ok":false,"error":"no fleet here","fleet":{"hosts":[]}}"#.utf8),
    ])
    let client = TendIPCClient(socketPath: "/tmp/unused-fleet-cache.sock") { _, payload in
        let request = try JSONSerialization.jsonObject(with: payload) as? [String: Any]
        if request?["kind"] as? String == "hello" {
            return try JSONSerialization.data(withJSONObject: [
                "ok": true, "protocol": 6, "min_protocol": 1, "max_protocol": 6,
                "capabilities": ["session_bytes_v1", "session_attachment_identity_v1", "work_control_v1", "fleet_v1"],
            ])
        }
        return responses.next()
    }
    try await client.hello()
    await #expect(throws: DecodingError.self) { _ = try await client.executionSnapshot() }
    do {
        _ = try await client.executionSnapshot()
        Issue.record("a rejected reply must stay rejected")
    } catch TendIPCError.server(let message) {
        #expect(message == "no fleet here")
    }
}

@Test func fleetSnapshotRequestDecodesThroughTheHostCache() async throws {
    let local = decodingHost("local")
    let remote = decodingHost("rtzr")
    let responses = LockedResponses([
        try fleetResponse([local, remote]),
        try fleetResponse([local, decodingHost("rtzr", prompt: "next")]),
    ])
    let client = TendIPCClient(socketPath: "/tmp/unused-fleet-cache.sock") { _, payload in
        let request = try JSONSerialization.jsonObject(with: payload) as? [String: Any]
        if request?["kind"] as? String == "hello" {
            return try JSONSerialization.data(withJSONObject: [
                "ok": true, "protocol": 6, "min_protocol": 1, "max_protocol": 6, "capabilities": ["session_bytes_v1", "session_attachment_identity_v1", "work_control_v1", "fleet_v1"],
            ])
        }
        return responses.next()
    }
    try await client.hello()
    let first = try await client.executionSnapshot()
    #expect(first.hosts.map(\.alias) == ["local", "rtzr"])
    let second = try await client.executionSnapshot()
    #expect(second.hostedAgents.first { $0.host.alias == "rtzr" }?.agent.lastPrompt == "next")
    #expect(second.hosts[0] == first.hosts[0])
}

private final class LockedResponses: @unchecked Sendable {
    private let lock = NSLock()
    private var queue: [Data]

    init(_ queue: [Data]) { self.queue = queue }

    func next() -> Data {
        lock.withLock { queue.removeFirst() }
    }
}
