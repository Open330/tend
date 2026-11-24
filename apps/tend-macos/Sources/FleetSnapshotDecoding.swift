import Foundation

/// A Fleet snapshot is about half a megabyte, and each Fleet event (several
/// a minute) fetches all of it, although an event usually changes one host.
/// Each host's JSON is decoded only when its bytes differ from the last
/// snapshot's; an unchanged host reuses the value decoded then. Anything the
/// scan does not recognise returns nil, and the caller decodes the whole
/// response as before.
struct TendFleetHostDecodeCache {
    private var previous: [(bytes: Data, host: TendFleetHost)] = []
    /// Hosts decoded by the last call (the rest were reused); for tests.
    private(set) var lastDecodedCount = 0

    /// The hosts of a `fleet_snapshot` response, or nil when `response` is
    /// not a successful one in the expected shape.
    mutating func hosts(fromResponse response: Data) throws -> [TendFleetHost]? {
        guard let scan = TendFleetSnapshotScanner.scan(response) else { return nil }
        let decoder = JSONDecoder()
        // Everything but the hosts' contents is checked by the decoder the
        // full path uses: the response with an empty host list must decode
        // and say ok, or the full decode runs and reports what it reports.
        var envelope = Data(response[response.startIndex..<scan.array.lowerBound])
        envelope.append(contentsOf: "[]".utf8)
        envelope.append(response[scan.array.upperBound...])
        guard let checked = try? decoder.decode(TendIPCResponse.self, from: envelope),
              checked.ok, checked.fleet != nil else { return nil }
        let slices = scan.hosts
        var hosts: [TendFleetHost] = []
        var cached: [(bytes: Data, host: TendFleetHost)] = []
        hosts.reserveCapacity(slices.count)
        cached.reserveCapacity(slices.count)
        var decoded = 0
        for (index, range) in slices.enumerated() {
            let bytes = response.subdata(in: range)
            let host: TendFleetHost
            // Hosts keep their order, so the same position is checked
            // first; a host added or removed falls back to a search.
            if previous.indices.contains(index), previous[index].bytes == bytes {
                host = previous[index].host
                TendPerf.count("fleet.host.reused")
            } else if let match = previous.first(where: { $0.bytes == bytes }) {
                host = match.host
                TendPerf.count("fleet.host.reused")
            } else {
                host = try decoder.decode(TendFleetHost.self, from: bytes)
                decoded += 1
                TendPerf.count("fleet.host.decoded")
            }
            hosts.append(host)
            cached.append((bytes, host))
        }
        previous = cached
        lastDecodedCount = decoded
        return hosts
    }

    mutating func reset() {
        previous = []
    }
}

/// Finds the byte range of each element of `fleet.hosts` in a response
/// line without decoding it: strings are skipped with their escapes, and
/// nesting is tracked by brackets. Only `{"ok":true, …, "fleet":{…,
/// "hosts":[…]}}` is accepted; anything else returns nil.
enum TendFleetSnapshotScanner {
    struct Scan {
        /// The `hosts` array, brackets included.
        let array: Range<Data.Index>
        /// Each element of it.
        let hosts: [Range<Data.Index>]
    }

    static func scan(_ data: Data) -> Scan? {
        let found = data.withUnsafeBytes { raw -> (array: Range<Int>, hosts: [Range<Int>])? in
            guard let base = raw.baseAddress?.assumingMemoryBound(to: UInt8.self) else { return nil }
            return Scanner(bytes: UnsafeBufferPointer(start: base, count: raw.count)).response()
        }
        guard let found else { return nil }
        let shift = { (range: Range<Int>) in (data.startIndex + range.lowerBound)..<(data.startIndex + range.upperBound) }
        return Scan(array: shift(found.array), hosts: found.hosts.map(shift))
    }

    static func hostSlices(in data: Data) -> [Range<Data.Index>]? {
        scan(data)?.hosts
    }

    private final class Scanner {
        let bytes: UnsafeBufferPointer<UInt8>
        private(set) var at = 0

        init(bytes: UnsafeBufferPointer<UInt8>) {
            self.bytes = bytes
        }

        func response() -> (array: Range<Int>, hosts: [Range<Int>])? {
            var ok = false
            var hosts: (array: Range<Int>, hosts: [Range<Int>])?
            let parsed = object { key in
                switch key {
                case "ok":
                    ok = literal("true")
                    return ok || skipValue()
                case "fleet":
                    var found: (array: Range<Int>, hosts: [Range<Int>])?
                    let parsed = object { key in
                        guard key == "hosts" else { return skipValue() }
                        let start = at
                        guard let elements = array() else { return false }
                        found = (start..<at, elements)
                        return true
                    }
                    hosts = found
                    return parsed
                default:
                    return skipValue()
                }
            }
            skipWhitespace()
            guard parsed, ok, at == bytes.count else { return nil }
            return hosts
        }

        /// Calls `member` with each key, positioned at its value; `member`
        /// consumes the value and returns false to fail.
        func object(_ member: (String) -> Bool) -> Bool {
            skipWhitespace()
            guard consume(UInt8(ascii: "{")) else { return false }
            skipWhitespace()
            if consume(UInt8(ascii: "}")) { return true }
            // A repeated key is left to the full decoder, whose choice of
            // which one counts is the one that matters.
            var seen = Set<String>()
            while true {
                skipWhitespace()
                guard let key = string(), seen.insert(key).inserted else { return false }
                skipWhitespace()
                guard consume(UInt8(ascii: ":")) else { return false }
                skipWhitespace()
                guard member(key) else { return false }
                skipWhitespace()
                if consume(UInt8(ascii: ",")) { continue }
                return consume(UInt8(ascii: "}"))
            }
        }

        /// The ranges of each element of an array.
        func array() -> [Range<Int>]? {
            skipWhitespace()
            guard consume(UInt8(ascii: "[")) else { return nil }
            var elements: [Range<Int>] = []
            skipWhitespace()
            if consume(UInt8(ascii: "]")) { return elements }
            while true {
                skipWhitespace()
                let start = at
                guard skipValue() else { return nil }
                elements.append(start..<at)
                skipWhitespace()
                if consume(UInt8(ascii: ",")) { continue }
                return consume(UInt8(ascii: "]")) ? elements : nil
            }
        }

        /// Skips one value of any kind.
        func skipValue() -> Bool {
            skipWhitespace()
            guard at < bytes.count else { return false }
            switch bytes[at] {
            case UInt8(ascii: "\""):
                return skipString()
            case UInt8(ascii: "{"), UInt8(ascii: "["):
                // Strings are skipped whole, so brackets inside them never
                // count; the closing bracket must match the opening one.
                var stack: [UInt8] = []
                while at < bytes.count {
                    let byte = bytes[at]
                    switch byte {
                    case UInt8(ascii: "\""):
                        guard skipString() else { return false }
                        continue
                    case UInt8(ascii: "{"):
                        stack.append(UInt8(ascii: "}"))
                    case UInt8(ascii: "["):
                        stack.append(UInt8(ascii: "]"))
                    case UInt8(ascii: "}"), UInt8(ascii: "]"):
                        guard stack.popLast() == byte else { return false }
                        if stack.isEmpty {
                            at += 1
                            return true
                        }
                    default:
                        break
                    }
                    at += 1
                }
                return false
            default:
                // A number or a literal: up to the next delimiter.
                let start = at
                while at < bytes.count {
                    switch bytes[at] {
                    case UInt8(ascii: ","), UInt8(ascii: "}"), UInt8(ascii: "]"),
                         UInt8(ascii: " "), UInt8(ascii: "\n"), UInt8(ascii: "\r"), UInt8(ascii: "\t"):
                        return at > start
                    default:
                        at += 1
                    }
                }
                return at > start
            }
        }

        func literal(_ word: StaticString) -> Bool {
            let count = word.utf8CodeUnitCount
            guard at + count <= bytes.count else { return false }
            let matches = word.withUTF8Buffer { expected in
                (0..<count).allSatisfy { bytes[at + $0] == expected[$0] }
            }
            guard matches else { return false }
            // The literal must end here, not merely start the next token.
            let end = at + count
            if end < bytes.count {
                switch bytes[end] {
                case UInt8(ascii: ","), UInt8(ascii: "}"), UInt8(ascii: "]"),
                     UInt8(ascii: " "), UInt8(ascii: "\n"), UInt8(ascii: "\r"), UInt8(ascii: "\t"):
                    break
                default:
                    return false
                }
            }
            at = end
            return true
        }

        /// A key: its raw bytes between the quotes. Keys with escapes never
        /// match the plain names looked for, which is the safe outcome.
        func string() -> String? {
            let start = at
            guard skipString() else { return nil }
            return String(decoding: UnsafeBufferPointer(rebasing: bytes[(start + 1)..<(at - 1)]), as: UTF8.self)
        }

        func skipString() -> Bool {
            guard consume(UInt8(ascii: "\"")) else { return false }
            while at < bytes.count {
                switch bytes[at] {
                case UInt8(ascii: "\\"):
                    at += 2
                case UInt8(ascii: "\""):
                    at += 1
                    return true
                default:
                    at += 1
                }
            }
            return false
        }

        func skipWhitespace() {
            while at < bytes.count {
                switch bytes[at] {
                case UInt8(ascii: " "), UInt8(ascii: "\n"), UInt8(ascii: "\r"), UInt8(ascii: "\t"):
                    at += 1
                default:
                    return
                }
            }
        }

        func consume(_ byte: UInt8) -> Bool {
            guard at < bytes.count, bytes[at] == byte else { return false }
            at += 1
            return true
        }
    }
}
