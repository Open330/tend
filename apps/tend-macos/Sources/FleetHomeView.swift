// Home: check what needs you, inspect a unit of work, start more.
import SwiftUI

struct FleetHomeView: View {
    /// Not observed: Home redraws from `homeBoard`, which publishes only what
    /// it shows. Read the model's other state in actions, not in `body`.
    let model: AppModel
    @ObservedObject var homeBoard: FleetHomeBoard
    @Environment(\.colorScheme) private var colorScheme

    @State private var selectedID: String?
    @State private var showsHosts = false
    @State private var hostFilter: String?
    @State private var projectFilter: String?
    @State private var reviewed = FleetReviewMarks(defaults: .standard)
    @State private var showsReviewed = false
    @State private var migrating = false
    /// Reply drafts per unit, kept while the inspector closes or the unit's
    /// focus changes, each bound to the question it was written for.
    @State private var drafts: [String: FleetReplyDraft] = [:]
    @State private var approvalSeed: FleetApprovalSeed?
    @State private var quickLook: FleetQuickLookTarget?
    @State private var showsReview = false
    @State private var showsLeftovers = false
    /// The daemon's decision queue, read while Home is on screen.
    @StateObject private var decisionFeed = FleetDecisionFeed()
    /// Planned Work not started yet (`work_backlog_v1`).
    @StateObject private var backlogFeed = FleetBacklogFeed()
    /// Prompts and Work tendd sends when ready (`work_queue_v1`).
    @StateObject private var queueFeed = FleetQueueFeed()
    /// A commit or range from a decision's evidence, open in Changes.
    @State private var revisionTarget: FleetRevisionTarget?
    @AppStorage("home.review.scope") private var reviewScope: FleetReviewScope = .day
    @AppStorage(TendPreferences.homeAnimatesMovesKey) private var animatesMoves = true
    @AppStorage(TendPreferences.homeCardLayoutKey) private var cardLayout: FleetCardLayout = .emphasis
    @Namespace private var cardSpace
    /// Each unit's lane at the last update, to tell which cards moved.
    @State private var lastLanes: [String: FleetLane] = [:]
    /// Cards that just changed lane, with the lane they left; cleared a few
    /// seconds later.
    @State private var movedFrom: [String: FleetLane] = [:]

    /// `presentsReview` opens with the Review sheet up: for the off-screen
    /// design capture, which cannot press the toolbar button.
    init(model: AppModel, homeBoard: FleetHomeBoard, presentsReview: Bool = false) {
        self.model = model
        self.homeBoard = homeBoard
        _showsReview = State(initialValue: presentsReview)
    }

    private var allWorks: [FleetWork] {
        homeBoard.works
    }

    /// What hides a finished result: tendd's record of Looks Good when the
    /// daemon keeps one, else this Mac's own marks (an older tendd).
    private var marks: FleetReviewMarks {
        var marks = reviewed
        marks.recorded = decisionFeed.reviewedKeys
        return marks
    }

    /// Looks Good: recorded by tendd, which then hides the result. Against
    /// a tendd that cannot record, a local mark hides it as before.
    private func markReviewed(_ work: FleetWork) {
        if decisionFeed.reviewedKeys != nil {
            let feed = decisionFeed
            let client = model.client
            let works = allWorks
            Task { await feed.recordLooksGood(work, client: client, works: works) }
            return
        }
        reviewed.markReviewed(work)
        reviewed.save(to: .standard)
        guard let target = FleetBoard.reviewTarget(work) else { return }
        let key = FleetBoard.reviewKey(work)
        let summary = FleetDecisionRecord.summary(work)
        let client = model.client
        Task {
            do {
                try await client.recordReviewed(target: target, key: key, summary: summary)
            } catch {
                TendLog.app.warning("recording a review failed: \(error.localizedDescription, privacy: .public)")
            }
        }
    }

    /// Hands this Mac's old review marks to tendd once it keeps the record,
    /// then forgets them. A failure leaves them for the next try.
    private func migrateMarksIfNeeded() {
        guard decisionFeed.reviewedKeys != nil, model.isConnected, !allWorks.isEmpty,
              !UserDefaults.standard.bool(forKey: FleetReviewMarks.migratedKey) else { return }
        guard !reviewed.marks.isEmpty else {
            UserDefaults.standard.set(true, forKey: FleetReviewMarks.migratedKey)
            return
        }
        guard !migrating else { return }
        migrating = true
        let feed = decisionFeed
        let client = model.client
        let marks = reviewed
        let works = allWorks
        Task {
            defer { migrating = false }
            guard await feed.migrate(marks, works: works, client: client) else { return }
            reviewed.clear()
            reviewed.save(to: .standard)
            UserDefaults.standard.set(true, forKey: FleetReviewMarks.migratedKey)
        }
    }

    var body: some View {
        let _ = TendPerf.count("home.body")
        VStack(spacing: 0) {
            header
            Divider()
            board
        }
        .background(TendSurfacePalette.workspace(for: colorScheme).ignoresSafeArea())
        .background(TendWindowVisibilityReader(board: homeBoard))
        .sheet(item: $approvalSeed) { seed in
            FleetApprovalRuleSheet(model: model, seed: seed)
        }
        .sheet(item: $revisionTarget) { target in
            ChangesRevisionView(repo: target.repo, revision: target.revision)
        }
        // Both polls pause while the window is off screen and restart, with
        // a read at once, when it is shown again.
        .task(id: homeBoard.isVisible) {
            // Decisions made elsewhere (the CLI, an agent, another window)
            // leave the queue while Home stays open.
            guard homeBoard.isVisible else { return }
            while !Task.isCancelled {
                await decisionFeed.load(model.client, works: allWorks)
                try? await Task.sleep(for: FleetDecisionFeed.refreshEvery)
            }
        }
        .task(id: homeBoard.isVisible) {
            // Other hosts' review items, on their own loop: a slow host never
            // holds up this Mac's decisions.
            guard homeBoard.isVisible else { return }
            while !Task.isCancelled {
                await decisionFeed.loadRemote(model.client, hosts: model.fleetHosts)
                try? await Task.sleep(for: FleetDecisionFeed.remoteEvery)
            }
        }
        .task(id: homeBoard.isVisible) {
            // Work planned with `tend work plan` shows up while Home is open.
            guard homeBoard.isVisible else { return }
            while !Task.isCancelled {
                await backlogFeed.load(model.client)
                try? await Task.sleep(for: FleetBacklogFeed.refreshEvery)
            }
        }
        .task(id: homeBoard.isVisible) {
            queueFeed.isActive = homeBoard.isVisible
            guard homeBoard.isVisible else { return }
            while !Task.isCancelled {
                await queueFeed.load(model.client)
                try? await Task.sleep(for: FleetQueueFeed.refreshEvery)
            }
        }
        .sheet(isPresented: $showsReview) {
            FleetReviewView(
                model: model,
                feed: decisionFeed,
                decisions: { decisions },
                decisionCard: { decisionCard($0, openRevision: $1) },
                works: { allWorks },
                marks: Binding(get: { marks }, set: { _ in }),
                draft: draftBinding,
                submit: { submit(for: $0) },
                open: open,
                markReviewed: { markReviewed($0) },
                send: { text, target in try await send(text, to: target) },
                ask: { prompt in try await model.client.askOneShot(prompt) }
            )
        }
        .sheet(isPresented: $showsLeftovers) {
            FleetLeftoversView(model: model)
        }
        .sheet(item: $quickLook) { target in
            let work = allWorks.first { work in
                (work.agents + work.helpers).contains { $0.id == target.agentID }
            }
            FleetQuickLookView(
                model: model,
                target: target,
                work: work,
                draft: draftBinding(work?.id ?? ""),
                submit: { if let work { submit(for: work.id) } },
                openFull: open
            )
        }
    }

    // MARK: - Header

    private var header: some View {
        HStack(spacing: 12) {
            Text("Home")
                .font(.system(size: 17, weight: .semibold))
            Spacer(minLength: 8)
            TendSegmented(selection: $cardLayout, options: FleetCardLayout.allCases) { layout in
                Text(Image(systemName: layout.systemImage)).accessibilityLabel(layout.title)
            }
            .help("Card layout: status emphasis or compact list")
            reviewButton
            filterMenu
            hostsButton
            FleetAutomationButton(model: model) { approvalSeed = FleetApprovalSeed() }
            Button {
                model.presentWorkStart()
            } label: {
                Label("Start Work", systemImage: "play.fill")
            }
            .buttonStyle(.tendPrimary)
            .disabled(!homeBoard.isConnected || homeBoard.isStartingWork)
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 12)
    }

    /// Results the person asked for and has not reviewed, in the chosen
    /// range: the way to go through Finished quickly.
    private var reviewButton: some View {
        let count = FleetBoard.reviewQueue(allWorks, marks: marks, scope: reviewScope).count
        return Button {
            showsReview = true
        } label: {
            HStack(spacing: 5) {
                Image(systemName: "checklist")
                Text("Review")
                if count > 0 {
                    Text(verbatim: count > 99 ? "99+" : "\(count)")
                        .font(.system(size: 11, weight: .semibold))
                        .monospacedDigit()
                        .foregroundStyle(.white)
                        .lineLimit(1)
                        .fixedSize()
                        .padding(.horizontal, 5)
                        .background(Color.accentColor, in: Capsule())
                }
            }
        }
        .buttonStyle(.tendSecondary)
        .fixedSize()
        .keyboardShortcut("r", modifiers: [.command, .shift])
        .help("Review the results of work you asked for (⇧⌘R)")
    }

    /// Narrows the board to one host and/or one project.
    private var filterMenu: some View {
        let all = allWorks
        // Planned Work counts too: a project with only a plan can be picked.
        let planned = backlogFeed.shown(on: all)
        let hosts = Array(Set(all.flatMap(\.hostAliases) + (planned.isEmpty ? [] : [homeBoard.localHostAlias]))).sorted()
        let projects = Array(Set(all.compactMap(\.repo) + planned.compactMap(\.project))).sorted()
        let active = hostFilter != nil || projectFilter != nil
        return Menu {
            Picker("Host", selection: $hostFilter) {
                Text("All hosts").tag(String?.none)
                ForEach(hosts, id: \.self) { Text($0).tag(String?.some($0)) }
            }
            Picker("Project", selection: $projectFilter) {
                Text("All projects").tag(String?.none)
                ForEach(projects, id: \.self) { Text($0).tag(String?.some($0)) }
            }
            if active {
                Divider()
                Button("Clear Filters") {
                    hostFilter = nil
                    projectFilter = nil
                }
            }
        } label: {
            Label(filterLabel, systemImage: active ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease.circle")
        }
        .menuStyle(.borderlessButton)
        .fixedSize()
        .foregroundStyle(active ? Color.accentColor : Color.primary)
        .help("Show one host or one project")
    }

    private var filterLabel: String {
        switch (hostFilter, projectFilter) {
        case (nil, nil): String(localized: "Filter")
        case (let host?, nil): host
        case (nil, let project?): project
        case (let host?, let project?): "\(project) · \(host)"
        }
    }

    private var hostsButton: some View {
        let hosts = homeBoard.hosts
        let online = hosts.filter { $0.state == "online" }.count
        return Button {
            showsHosts.toggle()
        } label: {
            HStack(spacing: 6) {
                Circle()
                    .fill(online == hosts.count ? Color.green : Color.orange)
                    .frame(width: 7, height: 7)
                Text("Hosts \(online)/\(hosts.count)")
                    .monospacedDigit()
            }
        }
        .buttonStyle(.tendGhost)
        .help("Host connections")
        .popover(isPresented: $showsHosts, arrowEdge: .bottom) {
            FleetHostsPopover(hosts: hosts, select: { host in
                showsHosts = false
                model.select(.host(host.alias))
            }, showLeftovers: {
                showsHosts = false
                showsLeftovers = true
            })
        }
    }

    // MARK: - Board

    /// The decisions Home shows under the same host and project filters as
    /// the cards.
    private var decisions: [FleetDecision] {
        filteredDecisions.filter { !$0.isUnanswerable }
    }

    private var filteredDecisions: [FleetDecision] {
        // `homeBoard.decisionsRevision` redraws Home for a new operator message
        // or asker; the feed caches the merge itself.
        decisionFeed.decisions(model).filter { decision in
            (hostFilter == nil || decision.host.alias == hostFilter)
                && (projectFilter == nil || decision.repo == projectFilter)
        }
    }

    /// Your turn's Can't be answered group, under the same filters; nil
    /// when there is nothing in it (always from an older tendd).
    private var cantAnswerGroup: FleetCantAnswerGroup? {
        let folded = filteredDecisions.filter(\.isUnanswerable)
        let count = FleetCantAnswer.count(
            listed: folded,
            exact: decisionFeed.unanswerableCounts(localHost: homeBoard.localHostAlias),
            host: hostFilter,
            filtered: projectFilter != nil
        )
        // An emptied group still shows the last Close all's report.
        guard count > 0 || decisionFeed.closeAllResult != nil else { return nil }
        let feed = decisionFeed
        let model = model
        return FleetCantAnswerGroup(
            decisions: folded,
            count: count,
            canKeep: feed.canKeep,
            canCloseAll: feed.canCloseAll,
            busy: feed.busy,
            failures: feed.failures,
            result: feed.closeAllResult,
            keep: { decision in Task { _ = await feed.keep(decision, model: model) } },
            closeAll: { decisions in Task { await feed.closeAll(decisions, model: model) } },
            dashboard: feed.dashboard,
            dismissResult: { feed.closeAllResult = nil }
        )
    }

    /// Planned Work under the same filters as the cards: this Mac's backlog,
    /// by the project its plan starts in.
    private var backlog: [TendWorkRecord] {
        guard hostFilter == nil || hostFilter == homeBoard.localHostAlias else { return [] }
        return backlogFeed.shown(on: allWorks).filter { record in
            projectFilter == nil || record.project == projectFilter
        }
    }

    /// A decision card wired to the feed. `openRevision` is the sheet the
    /// card's host view presents Changes in.
    func decisionCard(_ decision: FleetDecision, openRevision: @escaping (FleetRevisionTarget) -> Void) -> FleetDecisionCard {
        let feed = decisionFeed
        let model = model
        let asker = decision.askerAgentID.flatMap(model.hostedAgent(id:))
        return FleetDecisionCard(
            decision: decision,
            busy: feed.busy.contains(decision.id),
            failure: feed.failures[decision.id],
            openRevision: openRevision,
            answer: { text in await feed.answer(decision, text: text, model: model) },
            decide: { disposition, note in await feed.decide(decision, disposition, note: note, model: model) },
            openAsker: asker.map { hosted in { open(hosted) } },
            dismiss: decision.isHumanRequest ? { await feed.dismiss(decision, model: model) } : nil,
            dashboard: feed.dashboard,
            draft: feed.drafts.row(for: decision.id)
        )
    }

    private var board: some View {
        let _ = TendPerf.count("home.board")
        let all = FleetBoard.filter(allWorks, host: hostFilter, project: projectFilter)
        let decisions = decisions
        let marks = marks
        // One pass: whether a unit is hidden decides both the list and the
        // "reviewed" count.
        let hidden = Set(all.lazy.filter { marks.hides($0) }.map(\.id))
        let works = showsReviewed ? all : all.filter { !hidden.contains($0.id) }
        let hiddenCount = hidden.count
        // A unit, or (picked from the compact list) a decision.
        let selected = all.first { $0.id == selectedID }
        let selectedDecision = selected == nil
            ? decisions.first { FleetTurnItem.decision($0).id == selectedID }
            : nil
        let shownID = selected?.id ?? selectedDecision.map { FleetTurnItem.decision($0).id }
        return GeometryReader { geometry in
            // Beside the board while all three lanes keep a readable width;
            // over the board in a narrower window.
            let floats = geometry.size.width < 1040
            ZStack(alignment: .trailing) {
                HStack(spacing: 0) {
                    lanes(works, decisions: decisions, reviewedCount: hiddenCount)
                    if shownID != nil, !floats {
                        Divider()
                        detailPanel(work: selected, decision: selectedDecision)
                            .frame(width: 400)
                            .transition(.move(edge: .trailing))
                    }
                }
                if shownID != nil, floats {
                    detailPanel(work: selected, decision: selectedDecision)
                        .frame(width: min(400, geometry.size.width - 40))
                        .background(.background)
                        .overlay(alignment: .leading) { Divider() }
                        .shadow(color: .black.opacity(0.18), radius: 14, x: -4)
                        .transition(.move(edge: .trailing))
                }
            }
        }
        .animation(.easeOut(duration: 0.18), value: shownID)
        // A decision opens in the panel only from the compact list; the
        // cards answer it in place.
        .onChange(of: cardLayout) { layout in
            if layout == .emphasis, selectedID?.hasPrefix("decision:") == true { selectedID = nil }
        }
        .onExitCommand { selectedID = nil }
        // Display filters hide units; they are not gone. Prune review marks
        // against the whole board.
        .onAppear { lastLanes = Dictionary(allWorks.map { ($0.id, $0.lane) }, uniquingKeysWith: { first, _ in first }) }
        .onChange(of: allWorks.map { "\($0.id)|\($0.lane.rawValue)" }) { _ in noteMoves() }
        .onChange(of: allWorks.map(\.id)) { ids in
            // Only a live board says a unit is gone; a disconnect is not.
            guard model.isConnected, !ids.isEmpty else { return }
            var pruned = reviewed
            pruned.prune(keeping: Set(ids))
            if pruned != reviewed { reviewed = pruned; reviewed.save(to: .standard) }
            migrateMarksIfNeeded()
        }
        .onChange(of: decisionFeed.reviewedKeys != nil) { _ in migrateMarksIfNeeded() }
    }

    /// Marks the cards that changed lane so they can show where they came
    /// from, and forgets the mark after a few seconds.
    private func noteMoves() {
        let works = allWorks
        let moves = FleetBoard.laneMoves(previous: lastLanes, current: works)
        lastLanes = Dictionary(works.map { ($0.id, $0.lane) }, uniquingKeysWith: { first, _ in first })
        guard animatesMoves, !moves.isEmpty else { return }
        movedFrom.merge(moves) { _, new in new }
        Task { @MainActor in
            try? await Task.sleep(nanoseconds: 4_000_000_000)
            // Not `withAnimation`: that animated the whole board for the
            // fade, every card redrawn each frame. Each card fades its own
            // mark (`FleetBoard.moveMarkAnimation`).
            for (id, lane) in moves where movedFrom[id] == lane { movedFrom[id] = nil }
        }
    }

    /// A draft for one of the unit's other waiting agents.
    private static func agentDraftKey(_ work: FleetWork, _ hosted: TendHostedAgent) -> String {
        "\(work.id)#\(hosted.id)"
    }

    @ViewBuilder
    private func detailPanel(work: FleetWork?, decision: FleetDecision?) -> some View {
        if let work {
            inspector(work)
        } else if let decision {
            let asker = decision.askerAgentID.flatMap(model.hostedAgent(id:))
            FleetDecisionDetail(
                decision: decision,
                card: decisionCard(decision) { revisionTarget = $0 },
                openAsker: asker.map { hosted in { open(hosted) } },
                close: { selectedID = nil }
            )
            .id(decision.id)
        }
    }

    private func inspector(_ work: FleetWork) -> some View {
        FleetWorkInspector(
            model: model,
            work: work,
            isReviewed: marks.hides(work),
            close: { selectedID = nil },
            open: peek,
            openWork: { identity in model.select(.work(identity)) },
            draft: draftBinding(work.id),
            submit: { submit(for: work.id) },
            approveLikeThis: {
                var seed = FleetApprovalSeed(agent: work.focus)
                if case .work(let identity) = work.source { seed.work = identity }
                approvalSeed = seed
            },
            markReviewed: {
                markReviewed(work)
                selectedID = nil
            },
            agentDraft: { draftBinding(Self.agentDraftKey(work, $0)) },
            submitAgent: { submit(for: Self.agentDraftKey(work, $0)) }
        )
        .id(work.id)
    }

    /// Sends a unit's draft. The board owns the send, so it outlives the
    /// inspector and a second submission waits for the first to finish.
    private func submit(for workID: String) {
        guard var draft = drafts[workID], let submission = draft.begin() else { return }
        drafts[workID] = draft
        Task {
            var failure: String?
            do {
                try await send(submission.text, to: submission.target)
            } catch {
                failure = error.localizedDescription
            }
            guard var current = drafts[workID] else { return }
            current.finish(submission, error: failure)
            drafts[workID] = current.isIdle ? nil : current
        }
    }

    /// Types `text` into the agent the draft was written for, after checking
    /// that the same session is still at its pane, at the same question.
    private func send(_ text: String, to target: FleetReplyTarget) async throws {
        guard let hosted = model.hostedAgent(id: target.agentID),
              let paneID = TendAgentAttentionCenter.paneIdentity(for: hosted),
              let current = model.executionSnapshot.watchPane(id: paneID),
              target.matches(current.agent) else {
            throw TendIPCError.server(String(localized: "The agent moved on. Open its pane to see where it is."))
        }
        if let blocker = TendNotificationActions.replyBlocker(current) { throw TendIPCError.server(blocker) }
        try await model.client.sendFleetPrompt(host: current.host, pane: current.pane, text: text)
        // The words went to the agent; tendd records them as the decision.
        let question = current.agent?.awaitingReply?.question
            ?? allWorks.first { ($0.agents + $0.helpers).contains { $0.id == hosted.id } }?.headline
        await FleetDecisionRecord.recordInstruction(
            client: model.client,
            target: model.homeReviewTarget(for: hosted),
            reply: target.kind,
            sessionID: target.sessionID,
            waitSince: target.stateEnteredAt,
            question: question,
            text: text
        )
    }

    @ViewBuilder
    private func lanes(_ works: [FleetWork], decisions: [FleetDecision], reviewedCount: Int) -> some View {
        Group {
            // A decision read that failed keeps the lanes, so its warning
            // shows instead of an ordinary empty Home.
            // The queue is this Mac's, like the backlog.
            let queueIsShown = hostFilter == nil || hostFilter == homeBoard.localHostAlias
            let queued = queueIsShown ? queueFeed.shown : []
            let queueProblem = queueIsShown ? queueFeed.problem : nil
            let problem = decisionFeed.problem == nil && backlogFeed.problem == nil && queueProblem == nil ? 0 : 1
            let backlog = backlog
            let cantAnswer = cantAnswerGroup
            if FleetBoard.showsEmptyBoard(visible: works.count + decisions.count + backlog.count + queued.count + problem + (cantAnswer == nil ? 0 : 1), reviewed: reviewedCount) {
                FleetEmptyBoard(
                    isConnected: homeBoard.isConnected,
                    newAgent: { model.presentNewAgent() },
                    startWork: { model.presentWorkStart() }
                )
            } else {
                HStack(alignment: .top, spacing: 12) {
                    // A failed backlog read shows its column with the reason.
                    if !backlog.isEmpty || backlogFeed.problem != nil {
                        FleetBacklogColumn(feed: backlogFeed, records: backlog) { record in
                            let feed = backlogFeed
                            let model = model
                            Task { await feed.start(record, model: model) }
                        }
                    }
                    // A failed or stuck queue shows its column with the reason.
                    if !queued.isEmpty || queueProblem != nil {
                        FleetQueueColumn(feed: queueFeed, items: queued, client: model.client)
                    }
                    ForEach(FleetLane.allCases) { lane in
                        FleetLaneColumn(
                            lane: lane,
                            works: works.filter { $0.lane == lane },
                            decisions: lane == .needsYou ? decisions : [],
                            decisionCard: { decisionCard($0) { revisionTarget = $0 } },
                            decisionProblem: lane == .needsYou ? decisionFeed.problem : nil,
                            cantAnswer: lane == .needsYou ? cantAnswer : nil,
                            selectedID: selectedID,
                            select: { id in selectedID = selectedID == id ? nil : id },
                            open: peek,
                            reviewedCount: lane == .finished ? reviewedCount : 0,
                            showsReviewed: $showsReviewed,
                            cardSpace: animatesMoves ? cardSpace : nil,
                            movedFrom: movedFrom,
                            layout: cardLayout
                        )
                    }
                }
                .environment(\.fleetCardEmphasis, cardLayout == .emphasis)
                .contentTransition(.identity)
                .animation(
                    animatesMoves ? FleetBoard.moveAnimation : nil,
                    value: works.map { "\($0.id)|\($0.lane.rawValue)" }
                )
                .padding(16)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            }
        }
    }

    private func draftBinding(_ workID: String) -> Binding<FleetReplyDraft> {
        Binding(
            get: { drafts[workID] ?? FleetReplyDraft() },
            set: { drafts[workID] = $0.isIdle ? nil : $0 }
        )
    }

    /// Open on the board: a quick look first, the full view from there.
    private func peek(_ hosted: TendHostedAgent) {
        quickLook = FleetQuickLookTarget(agentID: hosted.id)
    }

    /// An agent's pane when it has a live one, its agent editor otherwise.
    private func open(_ hosted: TendHostedAgent) {
        if let pane = TendAgentAttentionCenter.paneIdentity(for: hosted),
           model.executionSnapshot.watchPane(id: pane) != nil {
            model.selectWatchPane(pane)
        } else {
            model.select(.agent(hosted.id))
        }
    }
}

extension FleetHomeView: @MainActor Equatable {
    /// Home's inputs are its two objects; everything else it shows comes
    /// through their publishes or its own state. Without this, SwiftUI
    /// cannot compare the `@StateObject`/`@AppStorage` wrappers and
    /// re-evaluates Home whenever ContentView redraws (every model publish).
    static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.model === rhs.model && lhs.homeBoard === rhs.homeBoard
    }
}

// MARK: - Lanes and cards

struct FleetLaneColumn: View {
    let lane: FleetLane
    let works: [FleetWork]
    /// The daemon's decisions, in Your turn only, among the Work cards by
    /// how long each waits.
    var decisions: [FleetDecision] = []
    var decisionCard: ((FleetDecision) -> FleetDecisionCard)?
    /// Why the decisions may be incomplete (a source tendd could not read).
    var decisionProblem: String?
    /// Questions nobody can answer, folded at the end of Your turn and not
    /// in its count.
    var cantAnswer: FleetCantAnswerGroup?
    let selectedID: String?
    let select: (String) -> Void
    let open: (TendHostedAgent) -> Void
    var reviewedCount = 0
    @Binding var showsReviewed: Bool
    /// Set when cards should glide between lanes.
    var cardSpace: Namespace.ID?
    var movedFrom: [String: FleetLane] = [:]
    var layout: FleetCardLayout = .emphasis
    @State private var showsOlder = false
    @State private var showsProbable = false
    @State private var showsEmpty = false
    @Environment(\.colorScheme) private var colorScheme

    /// Where a card folds outside Your turn, each in exactly one row, so a
    /// row's count is what opening it shows: a possible helper first, then
    /// an empty session, then (in Finished) anything older than a day.
    private enum Fold { case probable, empty, older }

    private func fold(_ work: FleetWork) -> Fold? {
        guard lane != .needsYou else { return nil }
        if work.isProbableHelper { return .probable }
        if work.isEmptySession { return .empty }
        if lane == .finished, !FleetBoard.isRecent(work) { return .older }
        return nil
    }

    private var shown: [FleetWork] {
        works.filter { work in
            switch fold(work) {
            case .probable: showsProbable
            case .empty: showsEmpty
            case .older: showsOlder
            case nil: true
            }
        }
    }

    private var emptyCount: Int { works.filter { fold($0) == .empty }.count }
    private var probableCount: Int { works.filter { fold($0) == .probable }.count }
    private var olderCount: Int { works.filter { fold($0) == .older }.count }

    private var title: String { lane.title }

    private var subtitle: String {
        switch lane {
        case .needsYou: String(localized: "Decide or review")
        case .running: String(localized: "Agents are on it")
        case .finished: String(localized: "Ready for the next instruction")
        }
    }

    private var empty: String {
        switch lane {
        case .needsYou: String(localized: "Nothing needs you right now.")
        case .running: String(localized: "No agent is working.")
        case .finished: String(localized: "Nothing finished yet.")
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // The lane's state dot, its name and count. The hint sentence is
            // the header's help and accessibility hint, not a line that
            // repeats on every view (docs/proposals/MAC_DESIGN.md).
            HStack(alignment: .center, spacing: TendSpace.s) {
                Circle()
                    .fill(lane.tint)
                    .frame(width: 7, height: 7)
                    .accessibilityHidden(true)
                Text(title)
                    .font(TendType.section)
                    .foregroundStyle(TendInk.label(colorScheme))
                Text("\(works.count + decisions.count)")
                    .font(.system(size: 12, weight: .medium))
                    .monospacedDigit()
                    .foregroundStyle(TendInk.secondary(colorScheme))
                Spacer(minLength: 4)
            }
            .help(subtitle)
            .accessibilityElement(children: .combine)
            .accessibilityHint(subtitle)
            .padding(.horizontal, 12)
            .padding(.top, 10)
            .padding(.bottom, 8)
            ScrollView {
                LazyVStack(spacing: layout == .compact ? 2 : 8) {
                    if let decisionProblem {
                        Label(decisionProblem, systemImage: "exclamationmark.circle")
                            .font(TendType.meta)
                            .foregroundStyle(TendInk.state(.orange, colorScheme))
                            .lineLimit(3)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, 4)
                    }
                    if works.isEmpty && decisions.isEmpty {
                        Text(empty)
                            .font(TendType.detail)
                            .foregroundStyle(TendInk.secondary(colorScheme))
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 24)
                    }
                    if lane == .needsYou {
                        // Decisions and waiting units in one list, the
                        // longest wait first.
                        let groups = FleetBoard.byProject(FleetBoard.yourTurn(works: shown, decisions: decisionCard == nil ? [] : decisions))
                        ForEach(groups) { group in
                            if groups.count > 1 {
                                FleetProjectHeader(project: group.project, count: group.items.count, isSystem: group.isSystem)
                            }
                            ForEach(group.items) { item in
                                switch item {
                                case .decision(let decision):
                                    if layout == .compact {
                                        FleetCompactRow(decision: decision, isSelected: item.id == selectedID) {
                                            select(item.id)
                                        }
                                    } else {
                                        decisionCard?(decision)
                                    }
                                case .work(let work):
                                    workCard(work)
                                }
                            }
                        }
                        if let cantAnswer { cantAnswer }
                    } else {
                        let groups = FleetBoard.byProject(shown)
                        ForEach(groups) { group in
                            if groups.count > 1 {
                                FleetProjectHeader(project: group.project, count: group.works.count)
                            }
                            ForEach(group.works) { work in
                                workCard(work)
                            }
                        }
                    }
                    if emptyCount > 0 {
                        Button(showsEmpty
                            ? String(localized: "Hide empty sessions")
                            : String(localized: "\(emptyCount) empty sessions")) {
                            showsEmpty.toggle()
                        }
                        .buttonStyle(.tendGhost)
                        .controlSize(.small)
                        .padding(.top, 2)
                        .help("Agents that were opened but never given a task")
                    }
                    if probableCount > 0 {
                        Button(showsProbable
                            ? String(localized: "Hide possible helpers")
                            : String(localized: "\(probableCount) possible helper sessions")) {
                            showsProbable.toggle()
                        }
                        .buttonStyle(.tendGhost)
                        .controlSize(.small)
                        .padding(.top, 2)
                        .help("Sessions whose prompt hands over a tend request; who asked is not confirmed")
                    }
                    if olderCount > 0 {
                        Button(showsOlder ? String(localized: "Hide older") : String(localized: "\(olderCount) older")) {
                            showsOlder.toggle()
                        }
                        .buttonStyle(.tendGhost)
                        .controlSize(.small)
                        .padding(.top, 2)
                    }
                    if reviewedCount > 0 || (lane == .finished && showsReviewed) {
                        Button(showsReviewed ? String(localized: "Hide reviewed") : String(localized: "Show \(reviewedCount) reviewed")) {
                            showsReviewed.toggle()
                        }
                        .buttonStyle(.tendGhost)
                        .controlSize(.small)
                        .padding(.top, 2)
                    }
                }
                .padding(.horizontal, 8)
                .padding(.bottom, 10)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(TendSurface.lane(colorScheme), in: RoundedRectangle(cornerRadius: TendRadius.lane, style: .continuous))
    }

    @ViewBuilder
    private func workCard(_ work: FleetWork) -> some View {
        switch layout {
        case .emphasis:
            FleetWorkCard(
                work: work,
                isSelected: work.id == selectedID,
                select: { select(work.id) },
                movedFrom: movedFrom[work.id]
            )
            .modifier(FleetCardMotion(id: work.id, space: cardSpace))
        case .compact:
            FleetCompactRow(work: work, isSelected: work.id == selectedID, movedFrom: movedFrom[work.id]) {
                select(work.id)
            }
            .modifier(FleetCardMotion(id: work.id, space: cardSpace))
        }
    }
}

/// A project's name over its units in a lane.
struct FleetProjectHeader: View {
    let project: String?
    let count: Int
    /// tendd's own items rather than a project.
    var isSystem = false

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: isSystem ? "gearshape" : "folder")
                .font(.system(size: 10))
            Text(isSystem ? String(localized: "From tend itself") : project ?? String(localized: "No project"))
                .font(.system(size: 11, weight: .semibold))
                .lineLimit(1)
            Text("\(count)")
                .font(.system(size: 11))
                .monospacedDigit()
                .foregroundStyle(.tertiary)
            Spacer(minLength: 0)
        }
        .foregroundStyle(.secondary)
        .padding(.horizontal, 4)
        .padding(.top, 6)
        .accessibilityElement(children: .combine)
    }
}

/// One segment per stage, coloured by its state, with the current stage
/// named under it. The current stage's segment is also taller, so colour is
/// not the only cue, and VoiceOver reads name, state and count together.
struct FleetStageBar: View {
    let work: FleetWork
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        let current = work.currentStageIndex
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .center, spacing: 3) {
                ForEach(Array(work.stages.enumerated()), id: \.offset) { index, stage in
                    Capsule()
                        .fill(stage.state == .waiting ? TendInk.track(colorScheme) : stage.state.color)
                        .frame(height: index == current ? 6 : 4)
                }
            }
            .frame(height: 6)
            if let index = current {
                HStack {
                    // Inherits the row's secondary ink: a second .secondary
                    // here would dim it to 2:1.
                    Text(work.stages[index].name)
                        .fontWeight(.medium)
                    Spacer()
                    if work.stages.count > 1 {
                        Text("\(work.doneStageCount)/\(work.stages.count)")
                            .monospacedDigit()
                    }
                }
                .font(TendType.meta)
                .foregroundStyle(TendInk.secondary(colorScheme))
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(accessibilityText)
    }

    private var accessibilityText: String {
        guard let index = work.currentStageIndex else {
            return work.stages.map(\.state.label).joined(separator: ", ")
        }
        let stage = work.stages[index]
        return work.stages.count > 1
            ? "\(stage.name), \(stage.state.label), \(work.doneStageCount)/\(work.stages.count)"
            : "\(stage.name), \(stage.state.label)"
    }
}

extension FleetWorkStage.State {
    var color: Color {
        switch self {
        case .done: .green
        case .running: .blue
        case .needsYou: .orange
        case .waiting: Color.secondary.opacity(0.25)
        }
    }

    var label: String {
        switch self {
        case .done: String(localized: "Done")
        case .running: String(localized: "Working")
        case .needsYou: String(localized: "Your turn")
        case .waiting: String(localized: "Waiting")
        }
    }
}

// MARK: - Inspector

private struct FleetWorkInspector: View {
    @ObservedObject var model: AppModel
    let work: FleetWork
    let isReviewed: Bool
    let close: () -> Void
    let open: (TendHostedAgent) -> Void
    let openWork: (TendWorkIdentity) -> Void
    @Binding var draft: FleetReplyDraft
    let submit: () -> Void
    let approveLikeThis: () -> Void
    let markReviewed: () -> Void
    /// Drafts and sends for the unit's other waiting agents, kept by the
    /// board like the unit's own.
    let agentDraft: (TendHostedAgent) -> Binding<FleetReplyDraft>
    let submitAgent: (TendHostedAgent) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .top, spacing: 8) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(work.title)
                        .font(.system(size: 16, weight: .semibold))
                        .fixedSize(horizontal: false, vertical: true)
                    Text([work.repo, work.hostAliases.joined(separator: ", ")].compactMap { $0 }.joined(separator: " · "))
                        .font(TendType.meta)
                        .foregroundStyle(.secondary)
                }
                Spacer(minLength: 4)
                Button(action: close) {
                    Image(systemName: "xmark")
                }
                .buttonStyle(.tendGhost)
                .help("Close")
                .keyboardShortcut(.cancelAction)
            }
            .padding(16)
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    now
                    section("Stages") {
                        VStack(alignment: .leading, spacing: 0) {
                            ForEach(Array(work.stages.enumerated()), id: \.offset) { index, stage in
                                FleetStageRow(stage: stage, isLast: index == work.stages.count - 1)
                            }
                        }
                    }
                    section("Agents") {
                        VStack(alignment: .leading, spacing: 8) {
                            ForEach(work.agents) { hosted in agentRow(hosted) }
                        }
                    }
                    if !work.helpers.isEmpty {
                        section("Helpers") {
                            VStack(alignment: .leading, spacing: 8) {
                                Text("Sessions other agents started to help with this work.")
                                    .font(TendType.meta)
                                    .foregroundStyle(.secondary)
                                ForEach(work.helpers) { hosted in agentRow(hosted) }
                            }
                        }
                    }
                }
                .padding(16)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }

    @ViewBuilder
    private var now: some View {
        let tint: Color = switch work.lane {
        case .needsYou: .orange
        case .running: .blue
        case .finished: .green
        }
        VStack(alignment: .leading, spacing: 10) {
            Text(laneTitle)
                .font(.system(size: 11, weight: .bold))
                .foregroundStyle(tint)
            Text(work.headline)
                .font(.system(size: 13.5, weight: work.lane == .needsYou ? .semibold : .regular))
                .fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
            if work.lane == .needsYou, let focus = work.focus {
                FleetAskedContext(agent: focus.agent)
            }
            if work.handledLimit {
                Label("Rate limited · the limit policy will resume it", systemImage: "arrow.triangle.2.circlepath")
                    .font(TendType.meta)
                    .foregroundStyle(.secondary)
            }
            if work.lane == .needsYou, let focus = work.focus, focus.pane != nil,
               ["waiting_input", "waiting_choice"].contains(focus.agent.state) {
                // The screen decides: option buttons for a choice, the answer
                // field for a question that takes words.
                FleetChoicePanel(model: model, hosted: focus) {
                    if FleetBoard.replyTarget(for: work) != nil || !draft.isIdle {
                        FleetReplyField(work: work, draft: $draft, submit: submit)
                    }
                }
                .id(focus.id)
            } else if FleetBoard.replyTarget(for: work) != nil || !draft.isIdle {
                FleetReplyField(work: work, draft: $draft, submit: submit)
            }
            let others = FleetBoard.waiting(work).filter { $0.id != work.focus?.id }
            if work.lane == .needsYou, !others.isEmpty {
                // One unit, several agents asking: each answers on its own.
                VStack(alignment: .leading, spacing: 10) {
                    Text("Also waiting on you")
                        .font(.system(size: 11, weight: .bold))
                        .foregroundStyle(.orange)
                    ForEach(others) { hosted in
                        FleetWaitingAgent(model: model, hosted: hosted, open: open) {
                            FleetReplyField(
                                work: work,
                                draft: agentDraft(hosted),
                                submit: { submitAgent(hosted) },
                                agent: hosted
                            )
                        }
                    }
                }
                .padding(.top, 4)
            }
            HStack(spacing: 8) {
                if let focus = work.focus {
                    Button(paneButtonTitle) { open(focus) }
                        .buttonStyle(FleetBoard.reply(for: work) == nil ? .tendPrimary : .tendSecondary)
                }
                if case .work(let identity) = work.source {
                    Button("Work Details") { openWork(identity) }
                        .buttonStyle(.tendSecondary)
                }
                if FleetBoard.choosesInPane(work), work.focus.map({ FleetApprovalRules.supportedKinds.contains($0.agent.kind) }) == true {
                    Button("Always Approve This Command…", action: approveLikeThis)
                        .buttonStyle(.tendGhost)
                        .help("Write a rule so Tend approves this command on its own")
                }
                if work.lane == .finished && !isReviewed {
                    Spacer(minLength: 0)
                    Button(action: markReviewed) {
                        Label("Looks Good", systemImage: "checkmark")
                    }
                    .buttonStyle(.tendGhost)
                    .help("Set this aside until it changes again")
                }
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(tint.opacity(0.1), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 9, style: .continuous).strokeBorder(tint.opacity(0.45)))
    }

    private func agentRow(_ hosted: TendHostedAgent) -> some View {
        HStack(spacing: 8) {
            AgentStatusTag(status: hosted.agent.status, showsLabel: false)
            TendAgentLogo(kind: hosted.agent.kind, size: 18)
            VStack(alignment: .leading, spacing: 1) {
                Text(FleetBoard.owner(hosted))
                    .font(TendType.body.weight(.medium))
                Text(hosted.agent.status.label())
                    .font(TendType.meta)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 4)
            Button("Open") { open(hosted) }
                .buttonStyle(.tendSecondary)
                .controlSize(.small)
        }
    }

    private var paneButtonTitle: String {
        if FleetBoard.choosesInPane(work) { return String(localized: "Choose in Pane") }
        if work.lane == .needsYou && FleetBoard.reply(for: work) == nil { return String(localized: "Answer in Pane") }
        return String(localized: "Open Pane")
    }


    private var laneTitle: String {
        switch work.lane {
        case .needsYou: String(localized: "Needs your decision")
        case .running: String(localized: "Now")
        case .finished: String(localized: "Last result")
        }
    }

    private func section<Content: View>(_ title: LocalizedStringKey, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title)
                .font(.system(size: 11, weight: .bold))
                .foregroundStyle(.secondary)
                .textCase(.uppercase)
            content()
        }
    }
}

/// Answer an agent's question, or send a finished unit back to work. The
/// draft is the board's, bound to the question it was written for; shared
/// by the inspector and the quick look.
struct FleetReplyField: View {
    let work: FleetWork
    @Binding var draft: FleetReplyDraft
    let submit: () -> Void
    /// The agent the field answers when it is not the unit's reply target:
    /// another agent in the unit waiting for words.
    var agent: TendHostedAgent?

    private var currentTarget: FleetReplyTarget? {
        guard let agent else { return FleetBoard.replyTarget(for: work) }
        let answers: Bool
        if case .asked = agent.agent.status { answers = true } else { answers = agent.agent.state == "waiting_input" }
        return answers ? FleetReplyTarget(kind: .answer, agent: agent) : nil
    }

    /// Text written for a question the unit has moved past.
    private var isStale: Bool {
        guard let target = draft.target, draft.hasText else { return false }
        return target != currentTarget
    }

    var body: some View {
        let kind = (isStale ? nil : draft.target?.kind) ?? currentTarget?.kind ?? .answer
        VStack(alignment: .leading, spacing: 6) {
            if isStale, let target = draft.target {
                VStack(alignment: .leading, spacing: 6) {
                    Label("Written for an earlier question from \(target.owner). It was not sent.", systemImage: "exclamationmark.triangle.fill")
                        .font(TendType.meta)
                        .foregroundStyle(.orange)
                        .fixedSize(horizontal: false, vertical: true)
                    HStack(spacing: 8) {
                        if let current = currentTarget {
                            Button(String(localized: "Use for \(current.owner)")) {
                                draft.retarget(current)
                            }
                            .buttonStyle(.tendSecondary)
                            .controlSize(.small)
                        }
                        Button("Discard", role: .destructive) {
                            draft.discard()
                        }
                        .buttonStyle(.tendGhost)
                        .controlSize(.small)
                    }
                    .disabled(draft.isSending)
                }
            }
            HStack(alignment: .bottom, spacing: 8) {
                TextField(
                    kind == .answer ? String(localized: "Your answer") : String(localized: "Next instruction"),
                    text: Binding(
                        get: { draft.text },
                        set: { draft.edit($0, current: currentTarget) }
                    ),
                    axis: .vertical
                )
                .textFieldStyle(.roundedBorder)
                .lineLimit(1...6)
                .disabled(draft.isSending)
                .onSubmit(send)
                Button(kind == .answer ? String(localized: "Answer") : String(localized: "Send Back"), action: send)
                    .buttonStyle(.tendPrimary)
                    .disabled(draft.isSending || isStale || draft.target == nil || !draft.hasText)
            }
            if draft.isSending {
                ProgressView().controlSize(.small)
            } else if let failure = draft.failure {
                Label(failure, systemImage: "exclamationmark.circle.fill")
                    .font(TendType.meta)
                    .foregroundStyle(.red)
            } else if draft.sent {
                Label("Sent", systemImage: "checkmark.circle.fill")
                    .font(TendType.meta)
                    .foregroundStyle(.green)
            }
        }
    }

    private func send() {
        guard !isStale else { return }
        submit()
    }
}

private struct FleetStageRow: View {
    let stage: FleetWorkStage
    let isLast: Bool

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            VStack(spacing: 0) {
                Circle()
                    .strokeBorder(stage.state == .waiting ? Color.secondary.opacity(0.4) : stage.state.color, lineWidth: 2)
                    .background(Circle().fill(stage.state == .done ? stage.state.color : Color.clear))
                    .frame(width: 12, height: 12)
                    .padding(.top, 3)
                if !isLast {
                    Rectangle()
                        .fill(Color.secondary.opacity(0.25))
                        .frame(width: 2)
                        .frame(maxHeight: .infinity)
                }
            }
            .frame(width: 12)
            VStack(alignment: .leading, spacing: 1) {
                Text(stage.name).fontWeight(.semibold)
                HStack(spacing: 4) {
                    if let kind = stage.kind {
                        TendAgentLogo(kind: kind, size: 12)
                    }
                    Text(stage.owner)
                }
                .font(TendType.meta)
                .foregroundStyle(.secondary)
            }
            .padding(.bottom, isLast ? 0 : 10)
            Spacer(minLength: 4)
            Text(stage.state.label)
                .font(TendType.meta)
                .foregroundStyle(.secondary)
        }
        .fixedSize(horizontal: false, vertical: true)
    }
}

// MARK: - Hosts and empty board

private struct FleetHostsPopover: View {
    let hosts: [TendFleetHostIdentity]
    let select: (TendFleetHostIdentity) -> Void
    var showLeftovers: () -> Void = {}

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Hosts")
                .font(.headline)
            if hosts.isEmpty {
                Text("No hosts yet.")
                    .foregroundStyle(.secondary)
            }
            ForEach(hosts, id: \.alias) { host in
                Button {
                    select(host)
                } label: {
                    HStack(spacing: 8) {
                        Circle()
                            .fill(fleetHostColor(host.state))
                            .frame(width: 7, height: 7)
                        Text(host.alias).fontWeight(.medium)
                            .lineLimit(1)
                        Spacer(minLength: 12)
                        Text(fleetHostStateLabel(host.state))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .fixedSize(horizontal: true, vertical: false)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            Divider()
            Button("Leftovers…", action: showLeftovers)
                .buttonStyle(.tendGhost)
                .help("Worktrees tend created on each host, and what keeps them")
        }
        .padding(14)
        .frame(width: 260)
    }
}

private struct FleetEmptyBoard: View {
    let isConnected: Bool
    let newAgent: () -> Void
    let startWork: () -> Void

    var body: some View {
        VStack(spacing: 12) {
            Image(systemName: "rectangle.3.group")
                .font(.system(size: 38))
                .foregroundStyle(.secondary)
            Text("No work yet")
                .font(.title3.weight(.semibold))
            Text("Start a Work from a template, or start one agent. Each shows up here as a unit of work.")
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 380)
            HStack {
                Button("Start Work", action: startWork)
                    .buttonStyle(.tendPrimary)
                Button("New Agent", action: newAgent)
                    .buttonStyle(.tendSecondary)
            }
            .disabled(!isConnected)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(32)
    }
}


/// Each host a unit runs on: its icon (the one set for the menu bar) and
/// its name.
struct FleetHostBadges: View {
    let hosts: [TendFleetHostIdentity]

    var body: some View {
        HStack(spacing: 6) {
            ForEach(hosts, id: \.alias) { host in
                let icon = TendHostIcons.icon(for: host.alias, local: host.local)
                HStack(spacing: 3) {
                    TendGlyphFace(value: icon, gradient: TendMenuBarAgentAppearance(identity: host.alias, symbol: icon, provider: "").gradient, size: 14, glyphSize: 7)
                    Text(verbatim: host.alias)
                        .lineLimit(1)
                }
                .help(host.local ? String(localized: "\(host.alias) (this Mac)") : host.alias)
            }
        }
    }
}

/// Lets a card glide from its old lane to its new one: the same id in one
/// namespace across the lanes. Off when the setting is.
struct FleetCardMotion: ViewModifier {
    let id: String
    let space: Namespace.ID?

    func body(content: Content) -> some View {
        if let space {
            content
                .matchedGeometryEffect(id: id, in: space)
                .transition(.opacity.combined(with: .scale(scale: 0.96)))
        } else {
            content
        }
    }
}
