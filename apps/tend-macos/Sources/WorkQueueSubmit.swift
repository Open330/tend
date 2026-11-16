// What the Start Work sheet's Add to Queue sends, and the one-at-a-time
// rule between queueing and starting. Kept out of the view so both are
// testable.
import Combine
import Foundation

/// The Start Work form as Add to Queue reads it.
struct WorkQueueForm: Equatable, Sendable {
    var prompt = ""
    /// The Work ID typed in the form; blank lets tendd make one.
    var work = ""
    /// The ID the form previews and routes by (the typed one, else the one
    /// made from the task).
    var routeKey = ""
    var workspace = ""
    /// The explicit pipeline choice; blank is the route default.
    var pipeline = ""
    var folder = ""
    var external = ""
    var skill = ""
    var context = ""

    enum Plan: Equatable, Sendable {
        case ready(TendQueueNewWork)
        /// No task to send.
        case noPrompt
        /// Neither a folder nor a route that provides one with a known
        /// workspace.
        case noFolder
        /// The folder is not an absolute path.
        case relativeFolder
    }

    /// The new Work to queue, routed the way the form previews it: tendd
    /// gives a queued Work its own reserved id, so the routes match
    /// `route_key`, and the pipeline and workspace the preview shows are
    /// sent explicitly.
    func plan(options: TendWorkOptions?) -> Plan {
        let prompt = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !prompt.isEmpty else { return .noPrompt }
        let routeKey = routeKey.trimmingCharacters(in: .whitespacesAndNewlines)
        let route = options?.route(matching: routeKey)

        let typedWorkspace = workspace.trimmingCharacters(in: .whitespacesAndNewlines)
        let routeWorkspace = route?.workspace?.trimmingCharacters(in: .whitespacesAndNewlines)
        let workspace: String? = if !typedWorkspace.isEmpty {
            typedWorkspace
        } else if let routeWorkspace, !routeWorkspace.isEmpty, !routeWorkspace.contains("{{") {
            routeWorkspace
        } else {
            nil
        }

        let explicitPipeline = pipeline.trimmingCharacters(in: .whitespacesAndNewlines)
        let pipeline: String? = explicitPipeline.isEmpty
            ? options?.defaultPipeline(for: routeKey)?.name
            : explicitPipeline

        let typedFolder = folder.trimmingCharacters(in: .whitespacesAndNewlines)
        let cwd: String?
        if !typedFolder.isEmpty {
            let expanded = (typedFolder as NSString).expandingTildeInPath
            guard expanded.hasPrefix("/") else { return .relativeFolder }
            cwd = expanded
        } else if let route, route.worktree || route.prepare || route.cwd?.isEmpty == false, workspace != nil {
            // The route provides the directory itself.
            cwd = nil
        } else {
            return .noFolder
        }

        let typedWork = work.trimmingCharacters(in: .whitespacesAndNewlines)
        return .ready(TendQueueNewWork(
            prompt: self.prompt,
            work: typedWork.isEmpty ? nil : typedWork,
            routeKey: routeKey.isEmpty ? nil : routeKey,
            workspace: workspace,
            cwd: cwd,
            pipeline: pipeline,
            external: external,
            skill: skill,
            context: context
        ))
    }
}

/// Runs the sheet's two submissions one at a time, and keeps a queued
/// add's request key across retries of the same intent.
@MainActor
final class WorkStartSubmitter: ObservableObject {
    @Published private(set) var isQueueing = false
    @Published private(set) var isStarting = false
    @Published private(set) var queueError: String?
    /// The add last sent and not known to have landed: its key is reused
    /// while the request stays the same, so tendd never adds it twice.
    private(set) var pendingAdd: TendQueueNewWork?

    var isBusy: Bool { isQueueing || isStarting }

    /// The request to send for `draft`: the pending one's key when it is a
    /// retry of the same request, else a fresh one.
    func request(for draft: TendQueueNewWork) -> TendQueueNewWork {
        guard let pendingAdd, pendingAdd.sameRequest(as: draft) else { return draft }
        return pendingAdd
    }

    /// Queues `draft` unless anything is already being submitted (`busy`
    /// covers a start the app tracks elsewhere). True once tendd has it.
    func queue(
        _ draft: TendQueueNewWork,
        busy: Bool = false,
        add: (TendQueueNewWork) async throws -> Void
    ) async -> Bool {
        guard !isBusy, !busy else { return false }
        let request = request(for: draft)
        pendingAdd = request
        isQueueing = true
        if queueError != nil { queueError = nil }
        defer { isQueueing = false }
        do {
            try await add(request)
            pendingAdd = nil
            return true
        } catch {
            queueError = Self.message(for: error)
            return false
        }
    }

    /// Starts the Work unless anything is already being submitted.
    func start(busy: Bool = false, launch: () async -> Bool) async -> Bool {
        guard !isBusy, !busy else { return false }
        isStarting = true
        defer { isStarting = false }
        return await launch()
    }

    func showQueueError(_ message: String?) {
        if queueError != message { queueError = message }
    }

    /// tendd's own refusal is final; anything else (a dropped connection,
    /// a timeout, an unreadable reply) may have come after tendd added it.
    static func message(for error: Error) -> String {
        if let ipc = error as? TendIPCError, case .server = ipc { return error.localizedDescription }
        return error.localizedDescription + " "
            + String(localized: "It may have been queued; Add to Queue again will not add it twice.")
    }
}
