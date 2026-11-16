import Combine
import Foundation
import Testing
@testable import Tend

// The App's scenes observe TendAppCommandState, not the whole AppModel:
// a model publish that changes none of the menu's four states must not
// reach them.

@Suite @MainActor struct AppCommandStateTests {
    @Test func followsTheMenuStatesAndIgnoresOtherPublishes() {
        let model = AppModel()
        let state = TendAppCommandState(model: model)
        var publishes = 0
        let watch = state.objectWillChange.sink { publishes += 1 }
        defer { watch.cancel() }
        #expect(!state.isConnected)

        model.setConnectedForTesting()
        #expect(state.isConnected)
        model.isStartingAgent = true
        #expect(state.isStartingAgent)
        let afterChanges = publishes
        #expect(afterChanges == 2)

        // Unrelated publishes, and the same values again, publish nothing.
        model.isStartingAgent = true
        model.setConnectedForTesting()
        model.objectWillChange.send()
        #expect(publishes == afterChanges)

        model.isStartingAgent = false
        #expect(!state.isStartingAgent)
        #expect(publishes == afterChanges + 1)
    }
}
