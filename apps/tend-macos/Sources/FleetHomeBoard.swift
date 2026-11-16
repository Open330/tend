import Combine
import Foundation

/// Everything Home's board draws from the app model, published only when
/// what Home shows changes. Home observes this instead of the whole
/// `AppModel`, whose other publishes (the Inbox's progress, Ask, a telemetry
/// field of a remote agent, the workspace revision) used to re-evaluate the
/// entire board.
@MainActor
final class FleetHomeBoard: ObservableObject {
    /// Home's units. Replaced when a unit changes in a way Home can show
    /// (`TendProjectionEquality.sameOnHome`), not for each agent heartbeat.
    @Published private(set) var works: [FleetWork] = []
    /// Every host, as the header's Hosts button and its popover name them.
    @Published private(set) var hosts: [TendFleetHostIdentity] = []
    @Published private(set) var isConnected = false
    @Published private(set) var isStartingWork = false
    /// Bumped when Home's decisions may have changed outside the decision
    /// feed: a new operator message, or an asking agent's id or checkout.
    @Published private(set) var decisionsRevision: UInt64 = 0
    /// Ticks every `clockPeriod` so what Home works out from the clock (the
    /// cards' "5 min ago", Finished units folding away after a day) keeps
    /// moving on a quiet board. Home used to get this for free from the
    /// model's constant publishes.
    @Published private(set) var clock: UInt64 = 0
    static let clockPeriod: TimeInterval = 15

    var localHostAlias: String {
        hosts.first(where: \.local)?.alias ?? "local"
    }

    private var subscriptions: [AnyCancellable] = []
    private var perfSubscription: AnyCancellable?

    init() {
        perfSubscription = TendPerf.observe(self, "board.publish")
    }

    /// Follows the model's inputs Home reads. `@Published` sends the new
    /// value before it is stored, so each input is mapped from the value
    /// itself.
    func follow(_ model: AppModel) {
        subscriptions = [
            model.$connectionState.map { $0 == .connected }.removeDuplicates()
                .sink { [weak self] in self?.setConnected($0) },
            model.$isStartingWork.removeDuplicates()
                .sink { [weak self] in self?.setStartingWork($0) },
            model.$executionSnapshot.map { $0.hosts.map(\.identity) }.removeDuplicates()
                .sink { [weak self] in self?.setHosts($0) },
            model.$operatorMessages.dropFirst()
                .sink { [weak self] _ in self?.decisionsChanged() },
            Timer.publish(every: Self.clockPeriod, tolerance: 2, on: .main, in: .common).autoconnect()
                .sink { [weak self] _ in self?.clock &+= 1 },
        ]
    }

    func setWorks(_ works: [FleetWork]) {
        TendPerf.count("board.works")
        self.works = works
    }

    func setHosts(_ hosts: [TendFleetHostIdentity]) {
        if self.hosts != hosts { self.hosts = hosts }
    }

    func setConnected(_ connected: Bool) {
        if isConnected != connected { isConnected = connected }
    }

    func setStartingWork(_ starting: Bool) {
        if isStartingWork != starting { isStartingWork = starting }
    }

    func decisionsChanged() {
        decisionsRevision &+= 1
    }
}
