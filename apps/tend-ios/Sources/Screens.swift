// The screens: Your turn (what needs you, per host), Decided, and the
// device's settings. iPhone gets them as tabs; iPad gets Tend for Mac's
// sidebar (Your turn, Decided, then the hosts) beside the selected one.
import SwiftUI
import UIKit

struct YourTurnView: View {
    /// Only this host's section; nil for every host.
    var host: String?
    @Environment(AppModel.self) private var model

    private var sections: [HostQueue] {
        model.hosts.filter { host == nil || $0.alias == host }
    }

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 14) {
                if let outcome = model.outcome {
                    Label(outcome, systemImage: "checkmark.circle.fill")
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(TendColor.done)
                        .accessibilityIdentifier("outcome")
                }
                if let problem = model.problem {
                    Label(problem, systemImage: "exclamationmark.triangle")
                        .font(.footnote)
                        .foregroundStyle(TendColor.error)
                        .accessibilityIdentifier("problem")
                }
                if !model.loaded {
                    ProgressView().frame(maxWidth: .infinity).padding(.top, 40)
                } else if AppModel.allClear(sections) {
                    empty
                }
                ForEach(sections) { section in
                    HostSectionView(section: section, showsHeader: host == nil)
                }
            }
            .padding(16)
            .frame(maxWidth: 760)
            .frame(maxWidth: .infinity)
        }
        .background(Color(uiColor: .systemGroupedBackground))
        .refreshable { await model.refresh() }
        .navigationTitle(host ?? String(localized: "Your turn"))
    }

    private var empty: some View {
        VStack(spacing: 12) {
            TendMark(waiting: false, error: false, working: true, done: true)
                .frame(width: 56, height: 56)
            Text("Nothing needs you").font(.headline)
            Text("Questions your agents ask and results they file for review appear here.")
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity)
        .padding(.top, 48)
        .accessibilityIdentifier("emptyQueue")
    }
}

struct HostSectionView: View {
    let section: HostQueue
    var showsHeader = true

    var body: some View {
        let decisions = PendingDecision.from(section)
        VStack(alignment: .leading, spacing: 10) {
            if showsHeader, !decisions.isEmpty || !section.unavailable.isEmpty || !section.queue.isComplete {
                HStack(spacing: 6) {
                    Image(systemName: "server.rack")
                    Text(verbatim: section.alias).font(.subheadline.weight(.semibold))
                    if section.local { Text("this dashboard").foregroundStyle(.secondary) }
                    Spacer()
                    if !decisions.isEmpty {
                        Text("\(decisions.count)").monospacedDigit().foregroundStyle(.secondary)
                    }
                }
                .font(.subheadline)
            }
            ForEach(section.unavailable, id: \.self) { reason in
                Label(String(localized: "Not everything could be read: \(reason)"), systemImage: "exclamationmark.triangle")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            if section.queue.truncated {
                Text("Showing the first \(decisions.count); more are waiting.")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            if section.queue.undecoded > 0 {
                Label(String(localized: "\(section.queue.undecoded) more need you in a form this app cannot show yet. Open the dashboard in a browser, or update the app."), systemImage: "exclamationmark.triangle")
                    .font(.footnote)
                    .foregroundStyle(TendColor.waiting)
                    .accessibilityIdentifier("undecoded")
            }

            ForEach(decisions) { decision in
                DecisionCard(decision: decision)
            }
        }
    }
}

struct DecidedView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        List {
            if !model.historyLoaded {
                ProgressView().frame(maxWidth: .infinity)
            } else if model.past.isEmpty, model.history.allSatisfy({ $0.history.isComplete && $0.unavailable.isEmpty }) {
                Text("Nothing decided yet.").foregroundStyle(.secondary)
            }
            ForEach(model.history.filter { !$0.history.isComplete || !$0.unavailable.isEmpty }) { section in
                Label(historyGap(section), systemImage: "exclamationmark.triangle")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            ForEach(model.past) { past in
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 6) {
                        Chip(text: past.outcome, color: past.isPositive ? TendColor.done : TendColor.error)
                            .accessibilityIdentifier("pastOutcome")
                        Text(verbatim: past.host).font(.caption).foregroundStyle(.secondary)
                        Spacer()
                        if let at = past.at {
                            Text(at, format: .relative(presentation: .numeric, unitsStyle: .abbreviated))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                    Text(verbatim: past.title).font(.subheadline.weight(.semibold))
                        .accessibilityIdentifier("pastTitle")
                    if let text = past.text {
                        Text(verbatim: text).font(.subheadline).foregroundStyle(.secondary)
                    }
                    if let by = past.by {
                        Text("by \(by)").font(.caption).foregroundStyle(.secondary)
                    }
                }
                .padding(.vertical, 2)
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("pastDecision")
            }
        }
        .refreshable { await model.refreshHistory() }
        .task { await model.refreshHistory() }
        .navigationTitle(String(localized: "Decided"))
    }
}

private func historyGap(_ section: HostHistory) -> String {
    var parts = section.unavailable + section.history.unavailable
    if section.history.truncated { parts.append(String(localized: "only the latest are shown")) }
    if section.history.undecoded > 0 {
        parts.append(String(localized: "\(section.history.undecoded) this app cannot show"))
    }
    return String(localized: "\(section.alias): \(parts.joined(separator: "; "))")
}

struct DeviceSettingsView: View {
    @Environment(AppModel.self) private var model
    @State private var confirming = false

    var body: some View {
        Form {
            if let pairing = model.pairing {
                Section(String(localized: "Paired with")) {
                    LabeledContent(String(localized: "Dashboard"), value: pairing.origin.display)
                    LabeledContent(String(localized: "This device"), value: pairing.device.name)
                }
                Section {
                    Text("This device's key can read and answer what needs you on every host the dashboard covers, and nothing else. It stops working after 30 days unused, 90 days after pairing, or when removed under access ▸ Devices.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                if let problem = model.signOutProblem {
                    Section {
                        Label(problem, systemImage: "exclamationmark.triangle")
                            .foregroundStyle(TendColor.error)
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
        .navigationTitle(String(localized: "Settings"))
    }
}

enum SidebarItem: Hashable {
    case yourTurn, decided, host(String), settings
}

struct RootView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.horizontalSizeClass) private var sizeClass
    @Environment(\.scenePhase) private var scenePhase
    @State private var selection: SidebarItem? = .yourTurn

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
        // Re-read when the app comes forward, and every 20 s while it is.
        .task(id: TaskKey(paired: model.pairing != nil, active: scenePhase == .active)) {
            guard model.pairing != nil, scenePhase == .active else { return }
            while !Task.isCancelled {
                await model.refresh()
                try? await Task.sleep(nanoseconds: 20_000_000_000)
            }
        }
    }

    private struct TaskKey: Hashable {
        let paired: Bool
        let active: Bool
    }

    private var tabs: some View {
        TabView {
            NavigationStack { YourTurnView() }
                .tabItem { Label(String(localized: "Your turn"), systemImage: "tray.full") }
                .badge(model.actionableCount)
            NavigationStack { DecidedView() }
                .tabItem { Label(String(localized: "Decided"), systemImage: "checkmark.seal") }
            NavigationStack { DeviceSettingsView() }
                .tabItem { Label(String(localized: "Settings"), systemImage: "gearshape") }
        }
    }

    private var split: some View {
        NavigationSplitView {
            List(selection: $selection) {
                Label(String(localized: "Your turn"), systemImage: "tray.full")
                    .badge(model.actionableCount)
                    .tag(SidebarItem.yourTurn)
                    .accessibilityIdentifier("sidebarYourTurn")
                Label(String(localized: "Decided"), systemImage: "checkmark.seal")
                    .tag(SidebarItem.decided)
                    .accessibilityIdentifier("sidebarDecided")
                Section(String(localized: "Hosts")) {
                    ForEach(model.hosts) { host in
                        Label {
                            Text(verbatim: host.alias)
                        } icon: {
                            Image(systemName: host.unavailable.isEmpty ? "server.rack" : "exclamationmark.triangle")
                        }
                        .badge(PendingDecision.from(host).count)
                        .tag(SidebarItem.host(host.alias))
                    }
                }
                Label(String(localized: "Settings"), systemImage: "gearshape")
                    .tag(SidebarItem.settings)
                    .accessibilityIdentifier("sidebarSettings")
            }
            .navigationTitle("Tend")
        } detail: {
            NavigationStack {
                switch selection ?? .yourTurn {
                case .yourTurn: YourTurnView()
                case .decided: DecidedView()
                case .host(let alias): YourTurnView(host: alias)
                case .settings: DeviceSettingsView()
                }
            }
        }
    }
}

@main
struct TendApp: App {
    @State private var model = AppModel()

    var body: some Scene {
        WindowGroup {
            RootView().environment(model)
        }
    }
}

extension TendDecisionHistory {
    /// Every source was read in full and every entry decoded.
    var isComplete: Bool { !truncated && unavailable.isEmpty && undecoded == 0 }
}
