// A quiet heads-up before a rate limit runs out: when an account's 5-hour
// or 7-day window crosses a threshold, Tend says so once for that window.
import Foundation

enum TendUsageAlerts {
    static let enabledKey = "tend.notifications.usage"
    static let notifiedKey = "tend.notifications.usageNotified"
    /// The usage levels' own steps: yellow from 80%, urgent from 95%.
    static let thresholds: [Double] = [80, 95]
    static let urgent = 95.0
    /// A reading older than this is not news: a reconnect or restart that
    /// shows an agent's last figure from an hour ago does not alert.
    static let freshness: TimeInterval = 30 * 60
    /// With no reset time to tell windows apart, a fresh reading this low
    /// means the window has rolled over and may alert again.
    static let rearmBelow = 50.0

    struct Alert: Equatable, Sendable {
        /// The keys this alert answers for, one per host: see `keys`.
        let key: String
        let provider: String
        let hostAliases: [String]
        let kind: TendUsageWindowKind
        let percent: Double
        let threshold: Double
        let resetsAt: Date?
    }

    /// What the alerts read from one agent: everything `TendUsageGroup`
    /// and `due` look at, and nothing they don't. Agents reporting neither
    /// a window nor a cap leave it out; the order is the snapshot's, since
    /// it settles ties between equally fresh readings.
    ///
    /// The activity time only picks the freshest reading and says whether a
    /// reading is fresh, so it is kept as its rank among the inputs (equal
    /// times share one) and that flag: a working agent's heartbeat that
    /// moves its time without passing another's changes nothing here.
    struct Input: Equatable, Sendable {
        let hostAlias: String
        let hostLocal: Bool
        let provider: String
        let fiveHour: Double?
        let fiveHourResetsAt: String?
        let sevenDay: Double?
        let sevenDayResetsAt: String?
        let capScope: String?
        let cappedUntil: String?
        fileprivate(set) var activityRank = 0
        fileprivate(set) var fresh = false

        private init?(_ hosted: TendHostedAgent) {
            let agent = hosted.agent
            guard agent.rateLimit5hPercent != nil || agent.rateLimit7dPercent != nil || agent.rateLimitScope != nil
            else { return nil }
            hostAlias = hosted.host.alias
            hostLocal = hosted.host.local
            provider = agent.kind
            fiveHour = agent.rateLimit5hPercent
            fiveHourResetsAt = agent.rateLimit5hResetsAt
            sevenDay = agent.rateLimit7dPercent
            sevenDayResetsAt = agent.rateLimit7dResetsAt
            capScope = agent.rateLimitScope
            cappedUntil = agent.rateLimitedUntil
        }

        var reportsWindow: Bool { fiveHour != nil || sevenDay != nil }

        /// The inputs of `agents` at `now`, and each one's activity time.
        static func inputs(_ agents: [TendHostedAgent], now: Date) -> (inputs: [Input], activity: [Date?]) {
            var inputs: [Input] = []
            var activity: [Date?] = []
            for hosted in agents {
                guard var input = Input(hosted) else { continue }
                let sampled = TendUsageTimestamp.parse(hosted.agent.lastActivityAt)
                input.fresh = sampled.map { now.timeIntervalSince($0) <= freshness } ?? false
                inputs.append(input)
                activity.append(sampled)
            }
            // Ranked as `TendUsageGroup` compares them: no time is oldest.
            let distinct = Set(activity.map { $0 ?? .distantPast }).sorted()
            let rank = Dictionary(uniqueKeysWithValues: distinct.enumerated().map { ($1, $0) })
            for index in inputs.indices { inputs[index].activityRank = rank[activity[index] ?? .distantPast] ?? 0 }
            return (inputs, activity)
        }

        /// The first instant after `date` at which the same inputs can give
        /// a different answer: a window's reset passing, a cap lifting, or
        /// a fresh reading going stale. Those are the only ways `due`
        /// depends on the time; nil when none lies ahead.
        static func nextChange(_ inputs: [Input], activity: [Date?], after date: Date) -> Date? {
            var next: Date?
            func consider(_ instant: Date?) {
                guard let instant, instant > date else { return }
                if next.map({ instant < $0 }) ?? true { next = instant }
            }
            for (input, sampled) in zip(inputs, activity) {
                if input.fiveHour != nil { consider(TendUsageTimestamp.parse(input.fiveHourResetsAt)) }
                if input.sevenDay != nil { consider(TendUsageTimestamp.parse(input.sevenDayResetsAt)) }
                if input.reportsWindow, input.fresh, let sampled { consider(sampled.addingTimeInterval(freshness)) }
                if input.capScope != nil { consider(TendUsageTimestamp.parse(input.cappedUntil)) }
            }
            return next
        }
    }

    static func isEnabled(_ defaults: UserDefaults = .standard) -> Bool {
        defaults.object(forKey: enabledKey) as? Bool ?? true
    }

    /// One window as seen from each of its hosts: provider, host, kind and
    /// the exact reset (or "none"). Accounts are inferred, not known (see
    /// `TendUsageGroup`), so a key never speaks for another host: two
    /// accounts that merely share a reset never silence each other, while a
    /// host joining a group that already announced finds its partner's key
    /// and stays quiet, and one leaving keeps the rest.
    static func cycles(_ group: TendUsageGroup, _ window: TendUsageWindow) -> [String] {
        let reset = window.resetsAt.map { String(Int($0.timeIntervalSince1970)) } ?? "none"
        return group.hostAliases.map { [group.provider, $0, window.kind.rawValue, reset].joined(separator: "\u{1F}") }
    }

    static func key(cycle: String, threshold: Double) -> String {
        cycle + "\u{1F}" + String(Int(threshold))
    }

    /// The alerts due now: per window with a fresh reading, the highest
    /// threshold it has crossed that none of its hosts announced in this
    /// window yet. An account already capped is left to the limit
    /// notification. `crossed` lists every key the alert covers, every host
    /// and lower thresholds included, so a jump from 79% to 96% announces
    /// 95% only and a figure that falls back does not announce again.
    /// Within one call a key is announced once. `rearmed` lists reset-less
    /// cycles whose fresh reading fell low enough to count as a new window;
    /// `silent` lists keys to record without announcing, for hosts that
    /// joined a window another host already announced.
    static func due(
        groups: [TendUsageGroup],
        notified: Set<String>,
        now: Date
    ) -> (alerts: [(alert: Alert, crossed: [String])], rearmed: [String], silent: [String]) {
        var alerts: [(Alert, [String])] = []
        var rearmed: [String] = []
        var silent: [String] = []
        var seen = notified
        for group in groups where group.capped.isEmpty {
            for window in group.windows {
                guard let sampled = window.sampledAt, now.timeIntervalSince(sampled) <= freshness else { continue }
                let cycles = cycles(group, window)
                if window.resetsAt == nil, window.percent < rearmBelow {
                    rearmed.append(contentsOf: cycles)
                    continue
                }
                // A threshold any host announced in this window covers every
                // host now in the group, whatever the current figure: pass it
                // on, so it outlives the host that announced it.
                for threshold in thresholds {
                    let keys = cycles.map { key(cycle: $0, threshold: threshold) }
                    guard keys.contains(where: seen.contains) else { continue }
                    let missing = keys.filter { !seen.contains($0) }
                    silent.append(contentsOf: missing)
                    seen.formUnion(missing)
                }
                let passed = thresholds.filter { window.percent >= $0 }
                guard let top = passed.max() else { continue }
                let crossed = cycles.flatMap { cycle in passed.map { key(cycle: cycle, threshold: $0) } }
                guard !cycles.contains(where: { seen.contains(key(cycle: $0, threshold: top)) }) else {
                    let missing = crossed.filter { !seen.contains($0) }
                    silent.append(contentsOf: missing)
                    seen.formUnion(missing)
                    continue
                }
                seen.formUnion(crossed)
                alerts.append((
                    Alert(
                        key: key(cycle: cycles.joined(separator: "+"), threshold: top),
                        provider: group.provider,
                        hostAliases: group.hostAliases,
                        kind: window.kind,
                        percent: window.percent,
                        threshold: top,
                        resetsAt: window.resetsAt
                    ),
                    crossed
                ))
            }
        }
        return (alerts, rearmed, silent)
    }

    static func notification(for alert: Alert, sound: Bool) -> TendAgentNotification {
        let provider = TendUsageFormat.providerName(alert.provider)
        let window = alert.kind == .fiveHour ? String(localized: "5-hour limit") : String(localized: "weekly limit")
        let percent = Int(alert.percent.rounded())
        let reset = alert.resetsAt.map {
            String(localized: "Resets \($0.formatted(date: .omitted, time: .shortened)).")
        }
        return TendAgentNotification(
            identifier: "usage.\(alert.key)",
            title: String(localized: "\(provider) has used \(percent)% of its \(window)"),
            subtitle: alert.hostAliases.joined(separator: ", "),
            body: reset,
            pane: nil,
            sound: sound && alert.threshold >= urgent,
            category: nil
        )
    }

    /// Announced keys with when they were announced, pruned after eight
    /// days: no window outlives a week.
    static func loadNotified(_ defaults: UserDefaults) -> [String: Date] {
        guard let data = defaults.data(forKey: notifiedKey),
              let stored = try? JSONDecoder().decode([String: Date].self, from: data) else { return [:] }
        return stored
    }

    static func saveNotified(_ notified: [String: Date], to defaults: UserDefaults, now: Date) {
        let kept = pruned(notified, now: now)
        if let data = try? JSONEncoder().encode(kept) { defaults.set(data, forKey: notifiedKey) }
    }

    static let receiptLifetime: TimeInterval = 8 * 24 * 3600

    static func pruned(_ notified: [String: Date], now: Date) -> [String: Date] {
        notified.filter { now.timeIntervalSince($0.value) < receiptLifetime }
    }

    /// When a receipt made at `date` is pruned by the next save.
    static func expiry(_ date: Date) -> Date { date.addingTimeInterval(receiptLifetime) }
}
