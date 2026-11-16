import Combine
import Foundation
import os

/// Debug counters for idle-CPU work: view body evaluations, model publishes,
/// projections built, and feed polls. Off unless the app starts with
/// `TEND_PERF_COUNTERS=<seconds>` in its environment or the
/// `-TendPerfCounters <seconds>` launch argument; then the running totals
/// are printed to standard error every that many seconds (default 15).
/// When off, `count` is one load and a branch.
enum TendPerf {
    nonisolated(unsafe) private(set) static var isEnabled = interval(
        environment: ProcessInfo.processInfo.environment,
        arguments: ProcessInfo.processInfo.arguments
    ) != nil
    private static let counts = OSAllocatedUnfairLock(initialState: [String: Int]())

    /// The dump period the environment or arguments ask for, nil when off.
    static func interval(environment: [String: String], arguments: [String]) -> TimeInterval? {
        var raw = environment["TEND_PERF_COUNTERS"]
        if let index = arguments.firstIndex(of: "-TendPerfCounters") {
            raw = arguments.indices.contains(index + 1) ? arguments[index + 1] : "15"
        }
        guard let raw, !raw.isEmpty, raw != "0" else { return nil }
        return max(1, TimeInterval(raw) ?? 15)
    }

    @inline(__always)
    static func count(_ name: StaticString) {
        guard isEnabled else { return }
        let key = name.description
        counts.withLock { $0[key, default: 0] += 1 }
    }

    static func count(dynamic name: String) {
        guard isEnabled else { return }
        counts.withLock { $0[name, default: 0] += 1 }
    }

    static func snapshot() -> [String: Int] {
        counts.withLock { $0 }
    }

    /// Tests turn the counters on and start from zero.
    static func enableForTesting() {
        isEnabled = true
        counts.withLock { $0.removeAll() }
    }

    /// Counts `object`'s publishes as `name`; nil while the counters are off.
    static func observe<Object: ObservableObject>(_ object: Object, _ name: String) -> AnyCancellable?
    where Object.ObjectWillChangePublisher == ObservableObjectPublisher {
        guard isEnabled else { return nil }
        return object.objectWillChange.sink { _ in count(dynamic: name) }
    }

    static func reset() {
        counts.withLock { $0.removeAll() }
    }

    /// Prints the totals every `interval` seconds while the app runs.
    static func startDumping() {
        guard let interval = interval(
            environment: ProcessInfo.processInfo.environment,
            arguments: ProcessInfo.processInfo.arguments
        ) else { return }
        let start = Date()
        Timer.scheduledTimer(withTimeInterval: interval, repeats: true) { _ in
            let elapsed = Int(Date().timeIntervalSince(start))
            let line = snapshot().sorted { $0.key < $1.key }
                .map { "\($0.key)=\($0.value)" }
                .joined(separator: " ")
            FileHandle.standardError.write(Data("[tend-perf] t=\(elapsed)s \(line)\n".utf8))
        }
    }
}

extension AppModel {
    /// Counts every AppModel publish, and the ones Home and the sidebar
    /// react to by property, so a run shows what keeps the views busy.
    func installPerfCounters() {
        func watch<Value>(_ publisher: Published<Value>.Publisher, _ name: String) {
            let key = "model.publish.\(name)"
            perfSubscriptions.append(publisher.dropFirst().sink { _ in TendPerf.count(dynamic: key) })
        }
        perfSubscriptions.append(objectWillChange.sink { _ in TendPerf.count("model.publish.all") })
        watch($sessions, "sessions")
        watch($pipelineRuns, "pipelineRuns")
        watch($executionSnapshot, "executionSnapshot")
        watch($workGroups, "workGroups")
        watch($hostedAgents, "hostedAgents")
        watch($homeWorks, "homeWorks")
        watch($workspaceRevision, "workspaceRevision")
        watch($connectionState, "connectionState")
        watch($operatorMessages, "operatorMessages")
        watch($mailboxRevisions, "mailboxRevisions")
        watch($isRefreshingInbox, "isRefreshingInbox")
        watch($inboxError, "inboxError")
        watch($inboxHostFailures, "inboxHostFailures")
        watch($askEntries, "askEntries")
        watch($askConversations, "askConversations")
        watch($sidebarSelection, "sidebarSelection")
    }
}
