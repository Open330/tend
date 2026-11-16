import Foundation
import Testing
@testable import Tend

// Usage alerts as the attention center posts them from fleet snapshots:
// which reading counts as freshest, the thresholds, and when a window
// starts over. These pin the behaviour end to end, through `ingest`.

@MainActor
private final class UsagePoster: TendNotificationPosting {
    var posted: [TendAgentNotification] = []
    func post(_ notification: TendAgentNotification) { posted.append(notification) }
    func removeDelivered(identifiers: [String]) {}
    func setDockBadge(_ count: Int) {}
}

private let start = Date(timeIntervalSince1970: 1_800_000_000)

private func stamp(_ date: Date) -> String {
    ISO8601DateFormatter().string(from: date)
}

/// One agent's usage fields on the wire. Times are absolute, so a later
/// ingest of the same fixture is the same snapshot seen later.
private struct UsageFixture {
    var session = "s-1"
    var kind = "claude_code"
    var activity: Date? = start
    var fiveHour: Double? = nil
    var fiveHourReset: Date? = start.addingTimeInterval(3 * 3600 + 17 * 60 + 23)
    var sevenDay: Double? = nil
    var sevenDayReset: Date? = start.addingTimeInterval(4 * 86_400 + 41)
    var cappedUntil: Date? = nil
    var cost: Double? = nil

    var wire: [String: Any] {
        var wire: [String: Any] = [
            "kind": kind, "agent_session_id": session, "state": "working",
            "state_entered_at": "2026-09-23T10:00:00Z",
        ]
        if let activity { wire["last_activity_at"] = stamp(activity) }
        if let fiveHour { wire["rate_limit_5h_pct"] = fiveHour }
        if let fiveHourReset, fiveHour != nil { wire["rate_limit_5h_resets_at"] = stamp(fiveHourReset) }
        if let sevenDay { wire["rate_limit_7d_pct"] = sevenDay }
        if let sevenDayReset, sevenDay != nil { wire["rate_limit_7d_resets_at"] = stamp(sevenDayReset) }
        if let cappedUntil {
            wire["rate_limit_scope"] = "five_hour"
            wire["rate_limited_until"] = stamp(cappedUntil)
        }
        if let cost { wire["cost_usd"] = cost }
        return wire
    }
}

private func snapshot(_ hosts: [(alias: String, agents: [UsageFixture])]) throws -> TendExecutionSnapshot {
    let decoded = try hosts.map { host in
        try JSONDecoder().decode(TendFleetHost.self, from: JSONSerialization.data(withJSONObject: [
            "alias": host.alias, "local": host.alias == "local", "mode": "control", "state": "online",
            "remote": ["agents": host.agents.map(\.wire), "panes": [[String: Any]]()],
        ]))
    }
    return TendExecutionSnapshot(hosts: decoded)
}

@MainActor
private final class UsageHarness {
    let defaults = UserDefaults(suiteName: "tend.usage-alerts.tests.\(UUID().uuidString)")!
    let poster = UsagePoster()
    var clock = start
    private(set) var center: TendAgentAttentionCenter!

    init() {
        center = TendAgentAttentionCenter(
            defaults: defaults, poster: poster,
            settings: { TendNotificationSettings() }, now: { [unowned self] in self.clock }
        )
    }

    func ingest(_ agents: [UsageFixture], host: String = "local") throws {
        try center.ingest(snapshot([(host, agents)]))
    }

    func ingest(hosts: [(alias: String, agents: [UsageFixture])]) throws {
        try center.ingest(snapshot(hosts))
    }

    /// The alerts posted since the last call.
    func take() -> [TendAgentNotification] {
        defer { poster.posted.removeAll() }
        return poster.posted
    }

    func advance(_ seconds: TimeInterval) { clock += seconds }
}

private func threshold(_ notification: TendAgentNotification) -> String {
    String(notification.identifier.split(separator: "\u{1F}").last ?? "")
}

@MainActor
@Suite struct UsageAlertIngestTests {
    @Test func usageAlertUsesTheMostRecentlyActiveAgentsReading() throws {
        let h = UsageHarness()
        // Two sessions of one account on one host: the stale one says 97%, the
        // one that just reported says 82%. The fresh figure wins.
        try h.ingest([
            UsageFixture(session: "old", activity: start.addingTimeInterval(-20 * 60), fiveHour: 97),
            UsageFixture(session: "new", activity: start.addingTimeInterval(-60), fiveHour: 82),
        ])
        let first = h.take()
        #expect(first.count == 1)
        #expect(first.first.map(threshold) == "80")
        #expect(first.first?.title.contains("82%") == true)

        // Now the 97% session is the one that reported last.
        h.advance(60)
        try h.ingest([
            UsageFixture(session: "old", activity: start.addingTimeInterval(30), fiveHour: 97),
            UsageFixture(session: "new", activity: start.addingTimeInterval(-60), fiveHour: 82),
        ])
        let second = h.take()
        #expect(second.map(threshold) == ["95"])
        #expect(second.first?.title.contains("97%") == true)
    }

    @Test func usageAlertTakesTheFreshestReadingAcrossHostsOfOneAccount() throws {
        let h = UsageHarness()
        // The same account on two Macs (matching resets): the local reading is
        // older and higher, the remote one fresher. One alert names both hosts.
        try h.ingest(hosts: [
            ("local", [UsageFixture(activity: start.addingTimeInterval(-15 * 60), fiveHour: 96)]),
            ("mini", [UsageFixture(session: "m-1", activity: start.addingTimeInterval(-30), fiveHour: 85)]),
        ])
        let posted = h.take()
        #expect(posted.count == 1)
        #expect(posted.first.map(threshold) == "80")
        #expect(posted.first?.subtitle == "local, mini")
        #expect(posted.first?.title.contains("85%") == true)
        // A later identical reading does not announce again.
        h.advance(30)
        try h.ingest(hosts: [
            ("local", [UsageFixture(activity: start.addingTimeInterval(-15 * 60), fiveHour: 96)]),
            ("mini", [UsageFixture(session: "m-1", activity: start.addingTimeInterval(-30), fiveHour: 85)]),
        ])
        #expect(h.take().isEmpty)
    }

    @Test func usageThresholdsAnnounceOnceEachAndSoundOnlyWhenUrgent() throws {
        let h = UsageHarness()
        try h.ingest([UsageFixture(fiveHour: 79)])
        #expect(h.take().isEmpty)
        try h.ingest([UsageFixture(fiveHour: 82)])
        let yellow = h.take()
        #expect(yellow.map(threshold) == ["80"])
        #expect(yellow.first?.sound == false)
        try h.ingest([UsageFixture(fiveHour: 82)])
        try h.ingest([UsageFixture(fiveHour: 88)])
        #expect(h.take().isEmpty)
        try h.ingest([UsageFixture(fiveHour: 96)])
        let urgent = h.take()
        #expect(urgent.map(threshold) == ["95"])
        #expect(urgent.first?.sound == true)
        // Falling back and rising again within the window stays quiet.
        try h.ingest([UsageFixture(fiveHour: 70)])
        try h.ingest([UsageFixture(fiveHour: 97)])
        #expect(h.take().isEmpty)
        #expect(TendUsageAlerts.loadNotified(h.defaults).count == 2)
    }

    @Test func usageJumpPastBothThresholdsAnnouncesOnlyTheTop() throws {
        let h = UsageHarness()
        try h.ingest([UsageFixture(fiveHour: 50, sevenDay: 96)])
        let posted = h.take()
        #expect(posted.map(threshold) == ["95"])
        #expect(posted.first?.identifier.contains("seven_day") == true)
    }

    @Test func usageWindowRollsOverAtItsResetAndANewResetAlertsAgain() throws {
        let h = UsageHarness()
        let reset = start.addingTimeInterval(20 * 60 + 7)
        try h.ingest([UsageFixture(fiveHour: 85, fiveHourReset: reset)])
        #expect(h.take().map(threshold) == ["80"])
        // The same figure after its reset has passed says nothing any more.
        h.advance(21 * 60)
        try h.ingest([UsageFixture(fiveHour: 85, fiveHourReset: reset)])
        #expect(h.take().isEmpty)
        // The next window, with its own reset, announces again.
        try h.ingest([UsageFixture(
            activity: h.clock, fiveHour: 85, fiveHourReset: reset.addingTimeInterval(5 * 3600)
        )])
        #expect(h.take().map(threshold) == ["80"])
    }

    @Test func usageWithoutAResetRearmsOnlyAfterALowFreshReading() throws {
        let h = UsageHarness()
        try h.ingest([UsageFixture(fiveHour: 85, fiveHourReset: nil)])
        #expect(h.take().map(threshold) == ["80"])
        try h.ingest([UsageFixture(fiveHour: 60, fiveHourReset: nil)])
        try h.ingest([UsageFixture(fiveHour: 85, fiveHourReset: nil)])
        #expect(h.take().isEmpty)
        try h.ingest([UsageFixture(fiveHour: 20, fiveHourReset: nil)])
        try h.ingest([UsageFixture(fiveHour: 85, fiveHourReset: nil)])
        #expect(h.take().map(threshold) == ["80"])
    }

    @Test func usageReadingTooOldToBeNewsDoesNotAlertEvenLater() throws {
        let h = UsageHarness()
        try h.ingest([UsageFixture(activity: start.addingTimeInterval(-31 * 60), fiveHour: 90)])
        #expect(h.take().isEmpty)
        // A fresh reading under the threshold, which then ages past freshness:
        // nothing to announce either way.
        try h.ingest([UsageFixture(activity: start, fiveHour: 70)])
        h.advance(40 * 60)
        try h.ingest([UsageFixture(activity: start, fiveHour: 70)])
        #expect(h.take().isEmpty)
    }

    @Test func usageAlertWaitsForACapToLiftWithNoNewReading() throws {
        let h = UsageHarness()
        // Capped: the limit notification speaks for the account, not usage.
        let fixture = UsageFixture(sevenDay: 85, cappedUntil: start.addingTimeInterval(10 * 60))
        try h.ingest([fixture])
        #expect(h.take().isEmpty)
        // The identical snapshot once the cap has lifted (the reading is 11
        // minutes old, still fresh): the 7-day window is now due.
        h.advance(11 * 60)
        try h.ingest([fixture])
        #expect(h.take().map(threshold) == ["80"])
    }

    @Test func usageAlertsTurnedBackOnAnnounceTheCurrentReading() throws {
        let h = UsageHarness()
        h.defaults.set(false, forKey: TendUsageAlerts.enabledKey)
        try h.ingest([UsageFixture(fiveHour: 85)])
        #expect(h.take().isEmpty)
        h.defaults.set(true, forKey: TendUsageAlerts.enabledKey)
        try h.ingest([UsageFixture(fiveHour: 85)])
        #expect(h.take().map(threshold) == ["80"])
    }

    @Test func usageAlertsSurviveARelaunchWithoutRepeating() throws {
        let h = UsageHarness()
        try h.ingest([UsageFixture(fiveHour: 85)])
        #expect(h.take().count == 1)
        let relaunched = TendAgentAttentionCenter(
            defaults: h.defaults, poster: h.poster,
            settings: { TendNotificationSettings() }, now: { start }
        )
        try relaunched.ingest(snapshot([("local", [UsageFixture(fiveHour: 85)])]))
        #expect(h.take().isEmpty)
    }

    @Test func unchangedUsageIsNotReCheckedUntilTimeCouldChangeTheAnswer() throws {
        TendPerf.enableForTesting()
        func checks() -> Int { TendPerf.snapshot()["usage.checked"] ?? 0 }
        let h = UsageHarness()
        let quiet = UsageFixture(fiveHour: 60)
        try h.ingest([quiet])
        let first = checks()
        for _ in 0..<10 {
            h.advance(5)
            try h.ingest([quiet])
        }
        #expect(checks() == first)
        // A changed reading is checked, and announces.
        try h.ingest([UsageFixture(fiveHour: 85)])
        #expect(checks() == first + 1)
        #expect(h.take().map(threshold) == ["80"])
        // Once announced, the same reading is quiet again.
        try h.ingest([UsageFixture(fiveHour: 85)])
        let settled = checks()
        try h.ingest([UsageFixture(fiveHour: 85)])
        #expect(checks() == settled)
        // Past its freshness the reading is checked once more.
        h.advance(TendUsageAlerts.freshness + 1)
        try h.ingest([UsageFixture(fiveHour: 85)])
        #expect(checks() == settled + 1)
        #expect(h.take().isEmpty)
    }

    @Test func aRearmThatRemovesNothingIsQuietToo() throws {
        TendPerf.enableForTesting()
        func checks() -> Int { TendPerf.snapshot()["usage.checked"] ?? 0 }
        let h = UsageHarness()
        try h.ingest([UsageFixture(fiveHour: 85, fiveHourReset: nil)])
        #expect(h.take().map(threshold) == ["80"])
        try h.ingest([UsageFixture(fiveHour: 20, fiveHourReset: nil)])
        #expect(TendUsageAlerts.loadNotified(h.defaults).isEmpty)
        try h.ingest([UsageFixture(fiveHour: 20, fiveHourReset: nil)])
        let settled = checks()
        try h.ingest([UsageFixture(fiveHour: 20, fiveHourReset: nil)])
        #expect(checks() == settled)
        try h.ingest([UsageFixture(fiveHour: 85, fiveHourReset: nil)])
        #expect(h.take().map(threshold) == ["80"])
    }
}
