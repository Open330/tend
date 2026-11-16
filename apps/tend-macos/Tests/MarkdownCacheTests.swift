import Foundation
import Testing
@testable import Tend

@Suite struct MarkdownCacheTests {
    @Test func aSourceIsParsedOnceAndGivesTheSameText() {
        let source = "# Plan \(UUID().uuidString)\n\nFirst **step**.\n\n- one\n- two\n\n```\ncode\n```"
        #expect(!TendMarkdownText.isCached(source))
        let first = TendMarkdownText.attributedString(markdown: source)
        #expect(TendMarkdownText.isCached(source))
        let second = TendMarkdownText.attributedString(markdown: source)
        #expect(first == second)
        #expect(String(first.characters).contains("• one\n• two"))
        // A different source is its own entry, even when it differs only
        // in Unicode normalization (é composed vs decomposed).
        let composed = "caf\u{E9} \(UUID().uuidString)", decomposed = composed.replacingOccurrences(of: "\u{E9}", with: "e\u{301}")
        _ = TendMarkdownText.attributedString(markdown: composed)
        #expect(!TendMarkdownText.isCached(decomposed))
    }

    @Test func cachedRedrawsCostLessThanParsing() {
        let source = (0..<40).map { "Paragraph \($0) with `code`, *emphasis* and a [link](https://example.com).\n" }
            .joined(separator: "\n") + UUID().uuidString
        let clock = ContinuousClock()
        let cold = clock.measure { _ = TendMarkdownText.attributedString(markdown: source) }
        let warm = clock.measure {
            for _ in 0..<100 { _ = TendMarkdownText.attributedString(markdown: source) }
        }
        print("markdown: first parse \(cold), 100 cached reads \(warm)")
        #expect(warm < cold * 100)
    }
}
