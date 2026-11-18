// Start Work, as on the Mac (`WorkStartView`): say what needs to get done,
// where, and which team; the agents work out the rest. It goes through the
// dashboard's device route `POST /api/device/work`, which mints the Work id
// (so it only ever starts new work), takes a project only from its own
// list, and checks `allow_work_start`. The app asks for Face ID or the
// passcode every time: the agents it starts can do anything on that host.
import LocalAuthentication
import SwiftUI

struct StartWorkView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var task = ""
    @State private var host = ""
    @State private var folder = ""
    @State private var pipeline = ""
    /// One per sheet: a retry after a lost answer sends the same key, so
    /// the dashboard never starts the same request twice.
    @State private var idempotencyKey = UUID().uuidString
    @State private var options: WorkOptions?
    @State private var optionsProblem: String?
    @State private var starting = false
    @State private var problem: String?
    @FocusState private var taskFocused: Bool

    /// Hosts that can take work: online, in control mode.
    private var hosts: [FleetHost] {
        (model.visibleFleet?.ordered ?? []).filter { $0.online && ($0.local || $0.control) }
    }

    private var canStart: Bool {
        model.canStartWork
            && !task.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !host.isEmpty
            && !starting
            && options?.allowed == true
            && !(options?.pipelines.isEmpty ?? true)
            && !pipeline.isEmpty
            && !folder.isEmpty
    }

    var body: some View {
        NavigationStack {
            Form {
                if !model.operates {
                    Section { DecideOnlyNotice() }
                        .listRowInsets(EdgeInsets())
                        .listRowBackground(Color.clear)
                } else {
                    if let options, !options.allowed {
                        Section {
                            WorkStartOffNotice(dashboard: model.pairing?.origin.display ?? "")
                        }
                        .listRowInsets(EdgeInsets())
                        .listRowBackground(Color.clear)
                    }
                    Section {
                        ZStack(alignment: .topLeading) {
                            if task.isEmpty {
                                Text("What needs to get done? For example: fix the login timeout on slow networks and add a test.")
                                    .font(TendType.body)
                                    .foregroundStyle(TendInk.tertiary)
                                    .padding(.top, 8)
                                    .padding(.leading, 4)
                                    .allowsHitTesting(false)
                            }
                            TextEditor(text: $task)
                                .font(TendType.body)
                                .frame(minHeight: 140)
                                .focused($taskFocused)
                                .accessibilityIdentifier("workTask")
                        }
                    } header: {
                        Text("Say what needs to get done. The agents work out the rest.")
                            .textCase(nil)
                            .font(TendType.meta)
                    }
                    Section(String(localized: "Where")) {
                        Picker(String(localized: "Host"), selection: $host) {
                            ForEach(hosts) { host in
                                Text(verbatim: host.local ? String(localized: "\(host.alias) (dashboard host)") : host.alias)
                                    .tag(host.alias)
                            }
                        }
                        .accessibilityIdentifier("workHost")
                        if let projects = options?.projects {
                            if projects.isEmpty {
                                Text("No agent has worked on \(host) yet, so there is no project to start in. Start one there first.")
                                    .font(TendType.meta)
                                    .foregroundStyle(TendInk.secondary)
                            } else {
                                Picker(String(localized: "Project"), selection: $folder) {
                                    ForEach(projects, id: \.self) { path in
                                        Text(verbatim: (path as NSString).lastPathComponent).tag(path)
                                    }
                                }
                                .accessibilityIdentifier("workProject")
                                Text(verbatim: folder).font(TendType.mono).foregroundStyle(TendInk.secondary).lineLimit(1).truncationMode(.middle)
                            }
                        }
                    }
                    Section(String(localized: "Team")) {
                        if let options, !options.allowed {
                            Label(String(localized: "Turned off on this dashboard"), systemImage: "lock")
                                .font(TendType.meta)
                                .foregroundStyle(TendInk.secondary)
                        } else if let options {
                            if options.pipelines.isEmpty {
                                Label(String(localized: "\(host) has no pipeline configured. Add one to its tend config to start work there."), systemImage: "exclamationmark.triangle")
                                    .font(TendType.meta)
                                    .foregroundStyle(TendPalette.orange)
                            } else {
                                Picker(String(localized: "Pipeline"), selection: $pipeline) {
                                    ForEach(options.pipelines) { pipeline in
                                        Text(verbatim: pipeline.name).tag(pipeline.name)
                                    }
                                }
                                .accessibilityIdentifier("workPipeline")
                                if let description = options.pipelines.first(where: { $0.name == pipeline })?.description {
                                    Text(verbatim: description).font(TendType.meta).foregroundStyle(TendInk.secondary)
                                }
                            }
                        } else if let optionsProblem {
                            Label(optionsProblem, systemImage: "exclamationmark.triangle").font(TendType.meta).foregroundStyle(TendInk.secondary)
                        } else {
                            ProgressView()
                        }
                    }
                    if let problem {
                        Section {
                            Label(problem, systemImage: "exclamationmark.circle")
                                .foregroundStyle(TendPalette.red)
                                .accessibilityIdentifier("workProblem")
                        }
                    }
                }
            }
            .scrollContentBackground(.hidden)
            .background(TendSurface.panel)
            .navigationTitle(String(localized: "Start Work"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(String(localized: "Cancel")) { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    if starting {
                        ProgressView()
                    } else {
                        Button(String(localized: "Start")) { Task { await start() } }
                            .disabled(!canStart)
                            .accessibilityIdentifier("workStart")
                    }
                }
            }
            .onAppear {
                if host.isEmpty { host = hosts.first?.alias ?? "" }
                taskFocused = true
            }
            .task(id: host) { await loadOptions() }
            // The device lost Start Work (narrowed, or the dashboard stopped
            // offering it) while the sheet was open.
            .onChange(of: model.canStartWork) { _, allowed in
                if !allowed { dismiss() }
            }
        }
    }

    private func loadOptions() async {
        guard model.operates, !host.isEmpty else { return }
        options = nil
        optionsProblem = nil
        do {
            let loaded = try await model.workOptions(host: host)
            options = loaded
            if !loaded.projects.contains(folder) { folder = loaded.projects.first ?? "" }
            if !loaded.pipelines.contains(where: { $0.name == pipeline }) {
                pipeline = loaded.pipelines.first?.name ?? ""
            }
        } catch {
            optionsProblem = error.localizedDescription
        }
    }

    private func start() async {
        problem = nil
        let pairing = model.pairing
        guard await Self.confirmOwner(reason: String(localized: "Start agents on \(host)")) else {
            problem = String(localized: "Not started: Face ID or the passcode is needed to start work.")
            return
        }
        // Face ID takes a while: check again that this device, this host and
        // these choices still hold before anything is sent.
        guard canStart, model.pairing == pairing,
              hosts.contains(where: { $0.alias == host }),
              options?.projects.contains(folder) == true,
              options?.pipelines.contains(where: { $0.name == pipeline }) == true
        else {
            problem = String(localized: "Not started: what this device may do changed. Check and try again.")
            return
        }
        starting = true
        defer { starting = false }
        let request = StartWorkRequest(
            idempotencyKey: idempotencyKey,
            host: host,
            project: folder,
            pipeline: pipeline,
            request: task.trimmingCharacters(in: .whitespacesAndNewlines)
        )
        do {
            let started = try await model.startWork(request)
            switch started.state {
            case "started": dismiss()
            case "unknown": problem = String(localized: "It may have started: the host did not answer in time. Check \(host) before trying again.")
            default: problem = String(localized: "\(host) did not start it.")
            }
        } catch {
            problem = error.localizedDescription
        }
    }

    /// Face ID, Touch ID or the passcode. UI tests run on a simulator with
    /// no enrolled biometrics and no passcode: they pass `-TendSkipOwnerCheck`.
    static func confirmOwner(reason: String) async -> Bool {
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("-TendSkipOwnerCheck") { return true }
        #endif
        let context = LAContext()
        var error: NSError?
        guard context.canEvaluatePolicy(.deviceOwnerAuthentication, error: &error) else { return false }
        return (try? await context.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: reason)) ?? false
    }
}

/// Why Start can never be tapped here, who can change it, and what still
/// works.
struct WorkStartOffNotice: View {
    let dashboard: String

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Label(String(localized: "Starting work is turned off"), systemImage: "lock")
                .font(TendType.body.weight(.semibold))
            Text("The dashboard at \(dashboard) does not start work for anyone, from the web or from this device. Whoever runs that host can turn it on: set `allow_work_start = true` under `[dashboard]` in its tend config, then restart tendd.")
                .font(TendType.meta)
                .foregroundStyle(TendInk.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Text("Deciding, and watching hosts and agents, keep working.")
                .font(TendType.meta)
                .foregroundStyle(TendInk.tertiary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .tendCard()
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("workStartOff")
    }
}
