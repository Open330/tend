import Foundation

/// A field decoded when it has the expected shape and nil otherwise.
struct TendLenient<Value: Decodable & Sendable>: Decodable, Sendable {
    let value: Value?

    init(from decoder: Decoder) throws {
        value = try? Value(from: decoder)
    }
}
