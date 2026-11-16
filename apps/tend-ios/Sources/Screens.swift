// The app's shell, as Tend for Mac lays it out: Home first, then the
// hosts, then Settings. iPad gets the Mac's sidebar beside the selected
// screen; iPhone gets them as tabs.
import SwiftUI
import UIKit

/// One host's part of Your turn: its heading, what could not be read, and
/// its cards.
struct HostSectionView: View {
    let section: HostQueue

    var body: some View {
        let decisions = PendingDecision.from(section)
        VStack(alignment: .leading, spacing: 10) {
            ForEach(section.unavailable, id: \.self) { reason in
                Label(String(localized: "\(section.alias): not everything could be read: \(reason)"), systemImage: "exclamationmark.triangle")
                    .font(TendType.meta)
                    .foregroundStyle(.secondary)
            }
            if section.queue.truncated {
                Text("\(section.alias): showing the first \(decisions.count); more are waiting.")
                    .font(TendType.meta)
                    .foregroundStyle(.secondary)
            }
            if section.queue.undecoded > 0 {
                Label(String(localized: "\(section.queue.undecoded) more need you in a form this app cannot show yet. Open the dashboard in a browser, or update the app."), systemImage: "exclamationmark.triangle")
                    .font(TendType.meta)
                    .foregroundStyle(.orange)
                    .accessibilityIdentifier("undecoded")
            }
            ForEach(decisions) { decision in
                DecisionCard(decision: decision)
            }
        }
    }
}

struct DeviceSettingsView: View {
    @Environment(AppModel.self) private var model
    @State private var confirming = false
    @State private var repairing = false

    var body: some View {
        Form {
            if let pairing = model.pairing {
                Section(String(localized: "Paired with")) {
                    LabeledContent(String(localized: "Dashboard"), value: pairing.origin.display)
                    LabeledContent(String(localized: "This device"), value: pairing.device.name)
                    LabeledContent(String(localized: "Access")) {
                        Text(model.operates ? String(localized: "Decide, hosts and new work") : String(localized: "Decide only"))
                            .accessibilityIdentifier("scopeLabel")
                    }
                }
                Section {
                    Text(model.operates ? Self.operateText : Self.decideText)
                        .font(TendType.meta)
                        .foregroundStyle(.secondary)
                    if !model.operates {
                        Button(String(localized: "Pair again with hosts and new work…")) { repairing = true }
                            .accessibilityIdentifier("repair")
                            .confirmationDialog(String(localized: "Pair this device again?"), isPresented: $repairing) {
                                Button(String(localized: "Pair again")) {
                                    let address = pairing.origin.display
                                    let name = pairing.device.name
                                    Task {
                                        await model.signOut()
                                        if model.pairing == nil {
                                            model.startPairing(address: address, name: name, scope: .operate)
                                        }
                                    }
                                }
                            } message: {
                                Text("This device signs out, then asks the dashboard again, for hosts and new work. An operator approves the new code on the dashboard and may still allow deciding only.")
                            }
                    }
                }
                if let problem = model.signOutProblem {
                    Section {
                        Label(problem, systemImage: "exclamationmark.triangle")
                            .foregroundStyle(.red)
                            .accessibilityIdentifier("signOutProblem")
                        Button(String(localized: "Forget this device here only"), role: .destructive) {
                            model.forgetLocally()
                        }
                        .accessibilityIdentifier("forgetLocally")
                    } footer: {
                        Text("Then remove it under access ▸ Devices on the dashboard; until then, its key keeps working there.")
                    }
                }
                Section {
                    Button(String(localized: "Sign out"), role: .destructive) { confirming = true }
                        .accessibilityIdentifier("signOut")
                        .confirmationDialog(String(localized: "Unpair this device?"), isPresented: $confirming) {
                            Button(String(localized: "Sign out"), role: .destructive) {
                                Task { await model.signOut() }
                            }
                        } message: {
                            Text("The dashboard forgets this device's key. To use it again, pair it again.")
                        }
                }
            }
        }
        .scrollContentBackground(.hidden)
        .background(TendSurface.panel)
        .navigationTitle(String(localized: "Settings"))
    }

    private static let decideText = String(localized: "This device's key can read and answer what needs you on every host the dashboard covers, and nothing else. It stops working after 30 days unused, 90 days after pairing, or when removed under access ▸ Devices.")
    private static let operateText = String(localized: "This device's key can also see every host and agent, read their recent output, and start new work, which runs agents on your hosts. Starting work asks for Face ID or your passcode. It stops working after 30 days unused, 90 days after pairing, or when removed or limited to deciding under access ▸ Devices.")
}

enum SidebarItem: Hashable {
    case home, host(String), settings
}

struct RootView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.horizontalSizeClass) private var sizeClass
    @Environment(\.scenePhase) private var scenePhase
    @State private var selection: SidebarItem? = .home

    var body: some View {
        Group {
            if model.pairing == nil {
                PairingView()
            } else if sizeClass == .regular {
                split
            } else {
                tabs
            }
        }
        // Re-read when the app comes forward, and every 10 s while it is.
        .task(id: TaskKey(paired: model.pairing != nil, active: scenePhase == .active)) {
            guard model.pairing != nil, scenePhase == .active else { return }
            await model.refreshDevice()
            await model.refreshHistory()
            var round = 0
            while !Task.isCancelled {
                await model.refresh()
                round += 1
                if round % 6 == 0 { await model.refreshHistory() }
                try? await Task.sleep(nanoseconds: 10_000_000_000)
            }
        }
    }

    private struct TaskKey: Hashable {
        let paired: Bool
        let active: Bool
    }

    private var tabs: some View {
        TabView {
            NavigationStack {
                HomeView().withAgentDestinations()
            }
            .tabItem { Label(String(localized: "Home"), systemImage: "house") }
            .badge(model.actionableCount)
            NavigationStack {
                HostsView().withAgentDestinations()
            }
            .tabItem { Label(String(localized: "Hosts"), systemImage: "server.rack") }
            NavigationStack { DeviceSettingsView() }
                .tabItem { Label(String(localized: "Settings"), systemImage: "gearshape") }
        }
    }

    private var split: some View {
        NavigationSplitView {
            List(selection: $selection) {
                Label(String(localized: "Home"), systemImage: "house")
                    .badge(model.actionableCount)
                    .tag(SidebarItem.home)
                    .accessibilityIdentifier("sidebarHome")
                Section(String(localized: "Hosts")) {
                    if let fleet = model.visibleFleet {
                        ForEach(fleet.ordered) { host in
                            HostRow(host: host)
                                .accessibilityIdentifier("sidebarHost")
                                .tag(SidebarItem.host(host.alias))
                        }
                    } else {
                        ForEach(model.hosts) { section in
                            HStack(spacing: 10) {
                                HostBadge(alias: section.alias, local: section.local, state: section.unavailable.isEmpty ? "online" : "degraded", size: 24)
                                Text(verbatim: section.alias)
                            }
                            .badge(PendingDecision.from(section).count)
                        }
                    }
                }
                Label(String(localized: "Settings"), systemImage: "gearshape")
                    .tag(SidebarItem.settings)
                    .accessibilityIdentifier("sidebarSettings")
            }
            .scrollContentBackground(.hidden)
            .background(TendSurface.sideBar)
            .navigationTitle("Tend")
        } detail: {
            NavigationStack {
                switch selection ?? .home {
                case .home: HomeView().withAgentDestinations()
                case .host(let alias): HostDetailView(alias: alias).withAgentDestinations()
                case .settings: DeviceSettingsView()
                }
            }
        }
        .navigationSplitViewStyle(.prominentDetail)
    }
}

@main
struct TendApp: App {
    @State private var model = AppModel()

    var body: some Scene {
        WindowGroup {
            // As the host of the unit tests the app shows nothing and polls
            // nothing: its own reads would hit the tests' stubs and race them
            // for the Keychain. (UI tests launch the app separately, without
            // this variable.)
            if ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] != nil {
                Color.clear
            } else {
                RootView().environment(model)
            }
        }
    }
}

extension TendDecisionHistory {
    /// Every source was read in full and every entry decoded.
    var isComplete: Bool { !truncated && unavailable.isEmpty && undecoded == 0 }
}
