// Home's Queued column: prompts and new Work tendd will send on its own once
// their target is ready (`work_queue_v1`), in queue order. The person can
// reorder, edit, cancel or start an item now, and settles an attempt whose
// outcome tendd could not know.
import Combine
import SwiftUI

extension Notification.Name {
    /// Posted after the Start Work sheet queued a new Work, so an open Home
    /// reads the queue at once instead of at its next poll.
    static let tendWorkQueued = Notification.Name("dev.tend.workQueued")
}

// MARK: - Wording

extension TendQueueItem.Wait {
    /// Why it has not gone yet, for people.
    var text: String {
        switch self {
        case .agentBusy: String(localized: "Waiting for the agent to finish its turn")
        case .agentNeedsYou: String(localized: "Waiting: the agent needs you first")
        case .agentRateLimited: String(localized: "Waiting: the agent is at its usage limit")
        case .agentError: String(localized: "Waiting: the agent shows an error")
        case .agentMissing: String(localized: "Waiting: the agent is not running")
        case .agentAmbiguous: String(localized: "Waiting: the agent shows in more than one pane")
        case .workAmbiguous: String(localized: "Waiting: the Work has several agents; name one")
        case .draining: String(localized: "Waiting: the agent's input is paused")
        case .reserved: String(localized: "Waiting: an earlier queued prompt to this agent has not been picked up")
        case .behind: String(localized: "Waiting behind an earlier item for the same agent")
        case .noSlot: String(localized: "Waiting for a free slot: too many agents are busy")
        case .launching: String(localized: "Waiting: another queued Work is starting")
        case .retryLater: String(localized: "Will try again shortly")
        case .storeUnsaved: String(localized: "Paused: tendd could not save the queue")
        default: String(localized: "Waiting")
        }
    }
}

extension TendQueueItem.FailReason {
    var text: String? {
        switch self {
        case .targetGone: String(localized: "The agent went away")
        case .targetNotWorker: String(localized: "The target is not a worker agent")
        case .paneRefusedInput: String(localized: "The pane would not take the text")
        case .launchRefused: String(localized: "tendd refused to start it")
        case .identityInUse: String(localized: "That Work ID is already in use")
        case .notLocal: String(localized: "The target is not on this Mac")
        default: nil
        }
    }
}

extension TendQueueItem {
    /// Where it goes.
    var targetText: String {
        switch target {
        case .agent(let sessionID, let label):
            let name = label?.trimmingCharacters(in: .whitespaces).nonEmpty ?? String(sessionID.prefix(8))
            return String(localized: "Agent \(name)")
        case .work(let identity, let alias):
            let work = identity.workspaceID.isEmpty ? identity.workID : "\(identity.workspaceID)/\(identity.workID)"
            return [work, alias].compactMap { $0?.nonEmpty }.joined(separator: " · ")
        case .newWork(let work, _, let cwd, let pipeline):
            let folder = cwd.map { ($0 as NSString).lastPathComponent }
            return ([String(localized: "New Work")] + [work, folder, pipeline].compactMap { $0?.nonEmpty })
                .joined(separator: " · ")
        case .other:
            return String(localized: "Unknown target")
        }
    }

    /// The short status word on the row.
    var statusText: String {
        switch state {
        case .waiting: String(localized: "Waiting")
        case .dispatching:
            if case .newWork = target { String(localized: "Starting") } else { String(localized: "Sending") }
        case .dispatched: String(localized: "Sent")
        case .uncertain: String(localized: "Uncertain")
        case .failed: String(localized: "Failed")
        case .cancelled: String(localized: "Cancelled")
        case .other: String(localized: "Unknown")
        }
    }

    /// The line under the title: why it waits, what went wrong, or the
    /// warning on an attempt that may have gone.
    var detailText: String? {
        switch state {
        case .waiting: wait?.text
        case .uncertain: String(localized: "May already have been sent — check the agent")
        case .failed(let reason, let detail): reason.text ?? detail.nonEmpty
        default: nil
        }
    }
}

private extension String {
    var nonEmpty: String? { isEmpty ? nil : self }
}

// MARK: - Feed

/// Reads the queue while Home is on screen and makes the person's changes.
@MainActor
final class FleetQueueFeed: ObservableObject {
    /// nil while the daemon has no queue to offer (no `work_queue_v1`).
    @Published private(set) var queue: TendWorkQueue?
    @Published private(set) var problem: String?
    /// The item a change is being made to.
    @Published private(set) var busy: String?
    @Published private(set) var failures: [String: String] = [:]

    static let refreshEvery: Duration = .seconds(10)
    /// Bumped by every read, so one that started earlier never overwrites
    /// a later answer (a read after a change races the poll).
    private var generation = 0
    private weak var client: TendIPCClient?
    /// Home is on screen and polling. Off screen, a queued add is not read
    /// now: Home reads the queue at once when it is shown again.
    var isActive = false
    private var subscriptions: [AnyCancellable] = []

    init(center: NotificationCenter = .default) {
        if let perf = TendPerf.observe(self, "queue.publish") { subscriptions.append(perf) }
        center.publisher(for: .tendWorkQueued)
            .sink { [weak self] _ in
                Task { @MainActor [weak self] in
                    guard let self, self.isActive, let client = self.client else { return }
                    await self.load(client)
                }
            }
            .store(in: &subscriptions)
    }

    /// What Home lists: the unfinished items, in order.
    var shown: [TendQueueItem] { queue?.pending ?? [] }

    func load(_ client: TendIPCClient) async {
        self.client = client
        await load { try await client.workQueue() }
    }

    /// One read through `read` (nil: the daemon has no queue).
    func load(_ read: () async throws -> TendWorkQueue?) async {
        TendPerf.count("queue.poll")
        generation &+= 1
        let started = generation
        do {
            let queue = try await read()
            guard started == generation else { return }
            apply(queue)
        } catch is CancellationError {
            return
        } catch {
            guard started == generation else { return }
            setProblem(String(localized: "Could not read the queue: \(error.localizedDescription)"))
        }
    }

    /// Takes a read: publishes only what changed, so an unchanged queue
    /// polled every few seconds does not redraw Home.
    func apply(_ queue: TendWorkQueue?) {
        if self.queue != queue { self.queue = queue }
        let warning: String? = if let broken = queue?.broken {
            String(localized: "tendd could not read the queue; it is read-only: \(broken)")
        } else if queue?.unsaved == true {
            String(localized: "tendd could not save the queue; nothing new goes until it can")
        } else {
            nil
        }
        setProblem(warning)
        // Drop errors for items that left the queue.
        let ids = Set(shown.map(\.id))
        let kept = failures.filter { ids.contains($0.key) }
        if kept.count != failures.count { failures = kept }
    }

    private func setProblem(_ problem: String?) {
        if self.problem != problem { self.problem = problem }
    }

    func perform(_ change: TendQueueChange, on item: TendQueueItem, client: TendIPCClient) async {
        guard busy == nil else { return }
        busy = item.id
        if failures[item.id] != nil { failures[item.id] = nil }
        do {
            try await client.makeWorkQueueClient().change(change, on: item)
        } catch {
            failures[item.id] = error.localizedDescription
        }
        busy = nil
        await load(client)
    }
}

// MARK: - Views

/// The Queued column, shown only while something is queued.
struct FleetQueueColumn: View {
    @ObservedObject var feed: FleetQueueFeed
    let items: [TendQueueItem]
    let client: TendIPCClient

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text("Queued")
                    .font(.system(size: 13, weight: .bold))
                Text("\(items.count)")
                    .font(.system(size: 12, weight: .semibold))
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
                Spacer(minLength: 4)
                Text("Goes when its target is ready")
                    .font(TendType.meta)
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
            }
            .padding(.horizontal, 12)
            .padding(.top, 10)
            .padding(.bottom, 8)
            ScrollView {
                LazyVStack(spacing: 8) {
                    if let problem = feed.problem {
                        Label(problem, systemImage: "exclamationmark.circle")
                            .font(TendType.meta)
                            .foregroundStyle(.orange)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    ForEach(Array(items.enumerated()), id: \.element.id) { index, item in
                        FleetQueueCard(
                            item: item,
                            place: item.position ?? index,
                            isFirst: index == 0,
                            isLast: index == items.count - 1,
                            working: feed.busy == item.id,
                            busy: feed.busy != nil,
                            failure: feed.failures[item.id],
                            perform: { change in
                                let feed = feed
                                let client = client
                                Task { await feed.perform(change, on: item, client: client) }
                            }
                        )
                    }
                }
                .padding(.horizontal, 8)
                .padding(.bottom, 10)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(TendTheme.panelFill, in: RoundedRectangle(cornerRadius: 9, style: .continuous))
    }
}

struct FleetQueueCard: View {
    /// A change that could deliver the prompt twice, asked on the card
    /// before it is made.
    enum Confirmation: Equatable {
        case sendAgain, notSent, remove

        var message: String {
            switch self {
            case .sendAgain:
                String(localized: "It may already have reached the agent. Sending it again can deliver it twice.")
            case .notSent:
                String(localized: "If it did reach the agent, it will arrive twice when it goes again.")
            case .remove:
                String(localized: "It may already have reached the agent. Removing it lets tendd send that agent new prompts.")
            }
        }

        var action: String {
            switch self {
            case .sendAgain: String(localized: "Send Again")
            case .notSent: String(localized: "Mark Not Sent")
            case .remove: String(localized: "Remove")
            }
        }

        var change: TendQueueChange {
            switch self {
            case .sendAgain: .runNow(allowDuplicate: true)
            case .notSent: .resolve(sent: false, ackDuplicateRisk: true)
            case .remove: .remove(ackDuplicateRisk: true)
            }
        }
    }

    let item: TendQueueItem
    /// Its 0-based place among the unfinished items.
    let place: Int
    var isFirst = false
    var isLast = false
    var working = false
    var busy = false
    var failure: String?
    let perform: (TendQueueChange) -> Void
    @Environment(\.colorScheme) private var colorScheme
    @State private var confirming: Confirmation?
    @State private var editing = false
    @State private var draft = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text(verbatim: item.displayTitle)
                    .font(.system(size: 13, weight: .semibold))
                    .lineLimit(2)
                    .frame(maxWidth: .infinity, alignment: .leading)
                statusChip
            }
            Text(verbatim: item.targetText)
                .font(TendType.meta)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
            if let detail = item.detailText {
                if item.isUncertain {
                    Label(detail, systemImage: "exclamationmark.triangle.fill")
                        .font(TendType.meta)
                        .foregroundStyle(.orange)
                } else {
                    Text(verbatim: detail)
                        .font(TendType.meta)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
            }
            if let confirming {
                confirmation(confirming)
            } else {
                actions
            }
            if let failure {
                Label(failure, systemImage: "exclamationmark.circle")
                    .font(TendType.meta)
                    .foregroundStyle(.red)
                    .lineLimit(3)
                    .textSelection(.enabled)
            }
        }
        .padding(12)
        .background(TendSurfacePalette.workspace(for: colorScheme), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .strokeBorder(item.isUncertain ? Color.orange.opacity(0.6) : TendTheme.border(colorScheme), lineWidth: 1)
        )
        .popover(isPresented: $editing, arrowEdge: .trailing) { editor }
    }

    private var statusChip: some View {
        Text(verbatim: item.statusText)
            .font(.system(size: 10, weight: .semibold))
            .padding(.horizontal, 5)
            .padding(.vertical, 1)
            .background((item.isUncertain ? Color.orange : Color.secondary).opacity(0.15), in: Capsule())
            .foregroundStyle(item.isUncertain ? Color.orange : Color.secondary)
    }

    @ViewBuilder
    private var actions: some View {
        HStack(spacing: 6) {
            switch item.state {
            case .waiting:
                Button { perform(.move(to: max(0, place - 1))) } label: { Image(systemName: "chevron.up") }
                    .buttonStyle(.tendIcon)
                    .disabled(busy || isFirst)
                    .help(String(localized: "Move up"))
                Button { perform(.move(to: place + 1)) } label: { Image(systemName: "chevron.down") }
                    .buttonStyle(.tendIcon)
                    .disabled(busy || isLast)
                    .help(String(localized: "Move down"))
                Spacer(minLength: 4)
                progress
                Button("Start Now") { perform(.runNow(allowDuplicate: false)) }
                    .buttonStyle(.tendSecondary)
                    .disabled(busy)
                    .help(String(localized: "Send it now, ahead of the queue, once its target is safe to use"))
                Menu {
                    Button("Edit Prompt…") {
                        draft = item.prompt
                        editing = true
                    }
                    Button("Cancel") { perform(.cancel) }
                    Divider()
                    Button("Remove", role: .destructive) { perform(.remove(ackDuplicateRisk: false)) }
                } label: {
                    Image(systemName: "ellipsis")
                }
                .menuStyle(.borderlessButton)
                .menuIndicator(.hidden)
                .fixedSize()
                .disabled(busy)
            case .uncertain:
                Button("Mark as Sent") { perform(.resolve(sent: true, ackDuplicateRisk: false)) }
                    .buttonStyle(.tendSecondary)
                    .disabled(busy)
                    .help(String(localized: "It reached the agent: finish it"))
                Button("Not Sent…") { confirming = .notSent }
                    .buttonStyle(.tendSecondary)
                    .disabled(busy)
                    .help(String(localized: "It did not reach the agent: queue it again"))
                Spacer(minLength: 4)
                progress
                Menu {
                    Button("Send Again (may duplicate)…") { confirming = .sendAgain }
                    Divider()
                    Button("Remove…", role: .destructive) { confirming = .remove }
                } label: {
                    Image(systemName: "ellipsis")
                }
                .menuStyle(.borderlessButton)
                .menuIndicator(.hidden)
                .fixedSize()
                .disabled(busy)
            case .dispatching:
                ProgressView().controlSize(.small).scaleEffect(0.7)
                Spacer(minLength: 4)
            default:
                Spacer(minLength: 4)
                progress
                Button("Remove", role: .destructive) { perform(.remove(ackDuplicateRisk: false)) }
                    .buttonStyle(.tendSecondary)
                    .disabled(busy)
            }
        }
        .font(TendType.meta)
    }

    @ViewBuilder
    private var progress: some View {
        if working { ProgressView().controlSize(.small).scaleEffect(0.7) }
    }

    private func confirmation(_ confirmation: Confirmation) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(verbatim: confirmation.message)
                .font(TendType.meta)
                .foregroundStyle(.orange)
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 6) {
                Spacer(minLength: 4)
                Button("Keep") { confirming = nil }
                    .buttonStyle(.tendSecondary)
                Button(confirmation.action) {
                    confirming = nil
                    perform(confirmation.change)
                }
                .buttonStyle(.tendPrimary)
                .disabled(busy)
            }
        }
    }

    private var editor: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Edit Prompt")
                .font(.system(size: 13, weight: .semibold))
            TextEditor(text: $draft)
                .font(TendType.body)
                .frame(width: 380, height: 160)
                .overlay(
                    RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .strokeBorder(TendTheme.border(colorScheme), lineWidth: 1)
                )
            HStack {
                Spacer()
                Button("Cancel") { editing = false }
                    .keyboardShortcut(.cancelAction)
                Button("Save") {
                    editing = false
                    perform(.edit(prompt: draft))
                }
                .buttonStyle(.tendPrimary)
                .keyboardShortcut(.defaultAction)
                .disabled(draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || draft == item.prompt)
            }
        }
        .padding(14)
    }
}
