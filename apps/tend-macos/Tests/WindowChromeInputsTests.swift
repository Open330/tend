import Combine
import Foundation
import Testing
@testable import Tend

// The workbench chrome (root view, side bar, tab strips, status bar) draws
// from small published inputs instead of the whole AppModel: they change
// only when what the chrome shows does, and the side bar, tabs and status
// bar wait while no window is on screen.

private final class Publishes {
    private(set) var count = 0
    private var subscription: AnyCancellable?

    init<Object: ObservableObject>(_ object: Object) where Object.ObjectWillChangePublisher == ObservableObjectPublisher {
        subscription = object.objectWillChange.sink { [weak self] _ in self?.count += 1 }
    }
}

private func host(_ alias: String, local: Bool = false, activity: String = "2026-11-16T01:00:00Z") throws -> TendFleetHost {
    try JSONDecoder().decode(TendFleetHost.self, from: Data("""
    {"alias":"\(alias)","local":\(local),"mode":"control","state":"online",
     "remote":{"agents":[{"kind":"claude_code","agent_session_id":"\(alias)-1","state":"working",
       "last_activity_at":"\(activity)","rate_limit_5h_pct":42,"rate_limit_5h_resets_at":"2099-01-01T00:00:00Z"}],
       "panes":[]}}
    """.utf8))
}

@Suite @MainActor struct WindowChromeInputsTests {
    private func model() -> AppModel {
        AppModel(client: TendIPCClient(socketPath: "/tmp/tend-chrome-\(UUID().uuidString).sock"))
    }

    @Test func theSideBarTabsAndStatusBarWaitWhileNoWindowIsShown() throws {
        let model = model(), chrome = model.chrome
        let window = NSObject(), reader = ObjectIdentifier(window)
        chrome.setVisible(true, reader: reader)
        model.ingestExecutionSnapshotForTesting(TendExecutionSnapshot(hosts: [try host("mac", local: true)]))
        chrome.refresh()
        #expect(chrome.statusBar.value.hostCount == 1)

        chrome.setVisible(false, reader: reader)
        let statusBar = Publishes(chrome.statusBar), sidebar = Publishes(chrome.sidebar)
        model.ingestExecutionSnapshotForTesting(TendExecutionSnapshot(hosts: [try host("mac", local: true), try host("mini")]))
        model.isPresentingWorkStart = true
        chrome.refresh()
        #expect(statusBar.count == 0 && sidebar.count == 0, "hidden: nothing redraws")
        #expect(chrome.statusBar.value.hostCount == 1)
        #expect(chrome.content.value.isPresentingWorkStart, "the window's own sheets still follow")

        chrome.setVisible(true, reader: reader)
        #expect(chrome.statusBar.value.hostCount == 2, "shown again: caught up at once")
        chrome.removeReader(reader)
    }

    @Test func aHeartbeatThatMovesOnlyActivityRedrawsNoChrome() async throws {
        let model = model(), chrome = model.chrome
        let window = NSObject(), reader = ObjectIdentifier(window)
        chrome.setVisible(true, reader: reader)
        model.showHome()
        model.ingestExecutionSnapshotForTesting(TendExecutionSnapshot(hosts: [try host("mac", local: true)]))
        await model.rebuildProjectionsForTesting()
        chrome.refresh()
        #expect(chrome.sidebar.value.isListCollapsed)
        #expect(chrome.statusBar.value.usage.count == 1)
        let content = Publishes(chrome.content), statusBar = Publishes(chrome.statusBar), sidebar = Publishes(chrome.sidebar)
        model.ingestExecutionSnapshotForTesting(TendExecutionSnapshot(
            hosts: [try host("mac", local: true, activity: "2026-11-16T01:00:05Z")]
        ))
        await model.rebuildProjectionsForTesting()
        #expect(model.hostedAgents.first?.agent.lastActivityAt == "2026-11-16T01:00:05Z")
        chrome.refresh()
        #expect(content.count == 0 && statusBar.count == 0 && sidebar.count == 0)
        chrome.removeReader(reader)
    }

    @Test func tabLabelsFollowTheOpenTabs() throws {
        let model = model(), chrome = model.chrome
        let window = NSObject(), reader = ObjectIdentifier(window)
        chrome.setVisible(true, reader: reader)
        chrome.setOpenTabs([.workBoard, .inbox])
        #expect(chrome.tabStrip.value[.inbox]?.title == String(localized: "Inbox"))
        #expect(chrome.tabStrip.value[.ask] == nil)
        let tabs = Publishes(chrome.tabStrip)
        chrome.refresh()
        #expect(tabs.count == 0, "same tabs, same labels")
        chrome.removeReader(reader)
    }
}
