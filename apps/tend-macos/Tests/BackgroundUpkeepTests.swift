import Foundation
import Testing
@testable import Tend

// Upkeep that runs whether or not anyone looks: reconciliation slows down
// with no window on screen, and decision drafts do not outlive their
// decisions.

@Suite @MainActor struct BackgroundUpkeepTests {
    @Test func reconciliationSlowsOnlyWithNoWindowAndALivePushStream() {
        let model = AppModel(client: TendIPCClient(socketPath: "/tmp/tend-upkeep-\(UUID().uuidString).sock"))
        let now = Date()
        model.markReconciledForTesting(at: now)
        let stream = model.fleetStreamOpened()
        model.fleetStreamReady(stream)
        #expect(model.reconciliationDue(at: now + 15, windowOnScreen: false), "not connected yet: every 15 s")
        model.setConnectedForTesting()
        #expect(model.reconciliationDue(at: now + 15, windowOnScreen: true), "a window on screen: every 15 s")
        #expect(!model.reconciliationDue(at: now + 15, windowOnScreen: false))
        #expect(!model.reconciliationDue(at: now + 45, windowOnScreen: false))
        #expect(model.reconciliationDue(at: now + AppModel.backgroundReconciliation, windowOnScreen: false))
        model.fleetStreamEnded(stream)
        #expect(model.reconciliationDue(at: now + 15, windowOnScreen: false), "no push stream: every 15 s")
    }

    @Test func onlyTheCurrentPushStreamCountsAsLive() {
        let model = AppModel(client: TendIPCClient(socketPath: "/tmp/tend-upkeep-\(UUID().uuidString).sock"))
        let old = model.fleetStreamOpened()
        model.fleetStreamEnded(old)
        model.fleetStreamReady(old)
        #expect(!model.fleetStreamLive, "a hello that arrives after its stream ended")
        let first = model.fleetStreamOpened()
        let second = model.fleetStreamOpened()
        model.fleetStreamReady(second)
        model.fleetStreamEnded(first)
        #expect(model.fleetStreamLive, "an older attempt's cleanup leaves the newer stream live")
        model.fleetStreamReady(first)
        #expect(model.fleetStreamLive)
        model.fleetStreamEnded(second)
        #expect(!model.fleetStreamLive)
    }

    @Test func aLateHelloCannotReplaceANewerDaemonsIdentity() {
        let model = AppModel(client: TendIPCClient(socketPath: "/tmp/tend-upkeep-\(UUID().uuidString).sock"))
        let a = model.fleetStreamOpened()
        let b = model.fleetStreamOpened()
        model.fleetStreamHello(TendDaemonHello(bootID: "B", generation: 2), attempt: b)
        #expect(model.localDaemonBootID == "B")
        model.fleetStreamHello(TendDaemonHello(bootID: "A", generation: 1), attempt: a)
        #expect(model.localDaemonBootID == "B")
        #expect(model.localDaemonGeneration == 2)
        model.fleetStreamEnded(b)
        model.fleetStreamHello(TendDaemonHello(bootID: "B2", generation: 3), attempt: b)
        #expect(model.localDaemonBootID == "B", "nor a hello after its own stream ended")
    }

    @Test func draftsOfDecisionsThatAreGoneArePruned() {
        let drafts = FleetDecisionFeed().drafts
        let a = drafts.row(for: "a")
        _ = drafts.row(for: "b")
        _ = drafts.row(for: "c")
        drafts.binding(for: "c").wrappedValue = "typing"
        drafts.prune(keeping: ["a"])
        #expect(drafts.ids == ["a", "c"], "the open answer stays while its decision is briefly missing")
        #expect(drafts.row(for: "a") === a, "a kept card keeps its row")
        drafts.binding(for: "c").wrappedValue = nil
        drafts.prune(keeping: ["a"])
        #expect(drafts.ids == ["a"])
    }
}
