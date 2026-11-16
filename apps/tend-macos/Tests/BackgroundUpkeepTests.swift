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
        #expect(model.reconciliationDue(at: now + 15, windowOnScreen: false), "no push stream: every 15 s")
        model.setFleetStreamLiveForTesting(true)
        #expect(model.reconciliationDue(at: now + 15, windowOnScreen: true), "a window on screen: every 15 s")
        #expect(!model.reconciliationDue(at: now + 15, windowOnScreen: false))
        #expect(!model.reconciliationDue(at: now + 45, windowOnScreen: false))
        #expect(model.reconciliationDue(at: now + AppModel.backgroundReconciliation, windowOnScreen: false))
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
