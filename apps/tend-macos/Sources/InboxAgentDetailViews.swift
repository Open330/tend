import SwiftUI

/// Why an agent sits in the Inbox: the state it is stuck in, the operator
/// requests it still owes, and its latest prompt and reply. Where it runs is
/// the detail screen's Execution location, said there once.
/// Shown at the top of the agent detail so a "Needs attention" selection
/// explains itself without leaving the Inbox.
struct InboxAgentRequestCard: View {
    let participant: TendHostedAgent
    let requests: [TendOperatorMessage]
    let openInLiveWatch: () -> Void

    /// The normalized status, as the header and every list say it (an idle
    /// agent owing a reply reads "Asked you", not "Idle").
    private var status: TendAgentStatus { participant.agent.status }

    /// The most specific text about what the agent is waiting on: its last
    /// notification (the input request itself), then the reply it stopped at,
    /// then its recap.
    private var waitingText: String? { Self.waitingText(participant.agent) }

    static func waitingText(_ agent: TendAgent) -> String? {
        nonEmpty(agent.lastNotification) ?? nonEmpty(agent.lastResponse) ?? nonEmpty(agent.recap)
    }

    /// `workspace › work` for the agent's Work, from its group or its pane.
    static func workLabel(_ participant: TendHostedAgent, work: TendWorkGroup?) -> String? {
        if let work {
            return "\(work.workspaceID) › \(work.workID)"
        }
        guard let identity = participant.pane?.workIdentity else { return nil }
        return "\(identity.workspaceID) › \(identity.workID)"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                // A cap is said by the header's badge, which explains it and
                // offers the switch.
                if !status.isLimited {
                    Label(status.label(), systemImage: status.symbol)
                        .font(.headline)
                        .foregroundStyle(status.color)
                }
                if let since = participant.agent.stateEnteredAt {
                    Text("since \(compactTimestamp(since))")
                        .font(TendType.detail)
                        .foregroundStyle(.secondary)
                }
                Spacer(minLength: 8)
                Button(action: openInLiveWatch) {
                    Label("Open in Live Watch", systemImage: "rectangle.on.rectangle")
                }
                .buttonStyle(.tendPrimary)
                .disabled(participant.pane == nil)
                .help(
                    participant.pane == nil
                        ? "This agent is not attached to a pane"
                        : "Follow this agent's pane in Live Watch"
                )
            }

            if let waitingText {
                // In full: the Summary tab no longer repeats it.
                MarkdownContent(source: waitingText, font: .body)
            } else {
                Text("No request text was retained for this agent.")
                    .font(TendType.meta)
                    .foregroundStyle(.secondary)
            }

            if !requests.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Open requests")
                        .font(TendType.detail.weight(.semibold))
                        .foregroundStyle(.secondary)
                    ForEach(requests) { message in
                        InboxAgentRequestRow(message: message)
                    }
                }
            }

            if let prompt = nonEmpty(participant.agent.lastPrompt) {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Latest prompt")
                        .font(TendType.detail.weight(.semibold))
                        .foregroundStyle(.secondary)
                    MarkdownContent(source: prompt, lineLimit: 4)
                }
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(TendTheme.panelFill, in: RoundedRectangle(cornerRadius: TendTheme.panelRadius, style: .continuous))
    }
}

/// One operator request the agent still owes, or whose reply the operator
/// has not read yet.
private struct InboxAgentRequestRow: View {
    let message: TendOperatorMessage

    private var request: TendCollaborationRequest { message.request }

    private var statusText: String {
        if let reply = request.reply { return reply.status.capitalized }
        return request.status.capitalized
    }

    private var statusColor: Color {
        let status = request.reply?.status ?? request.status
        switch status {
        case "completed": return .green
        case "blocked", "declined", "failed": return .red
        case "claimed": return .blue
        default: return .secondary
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 7) {
                Text(request.kind.capitalized)
                    .font(TendType.detail.weight(.semibold))
                Text(statusText)
                    .font(TendType.meta.weight(.medium))
                    .foregroundStyle(statusColor)
                if message.hasUnreadReply {
                    Label("New reply", systemImage: "arrowshape.turn.up.left.fill")
                        .font(TendType.meta)
                        .foregroundStyle(.orange)
                }
                Spacer(minLength: 4)
                Text(compactTimestamp(message.activityAt))
                    .font(TendType.monoSmall)
                    .foregroundStyle(.secondary)
            }
            MarkdownContent(source: request.body, lineLimit: 3)
            if let reply = request.reply, !reply.body.isEmpty {
                MarkdownContent(source: reply.body, lineLimit: 3)
                    .padding(.leading, 10)
                    .overlay(alignment: .leading) {
                        RoundedRectangle(cornerRadius: 1)
                            .fill(statusColor.opacity(0.5))
                            .frame(width: 2)
                    }
            }
        }
        .padding(9)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: 8))
    }
}

private func nonEmpty(_ value: String?) -> String? {
    guard let value, !value.isEmpty else { return nil }
    return value
}

/// "2026-09-03T10:15:42Z" → "2026-09-03 10:15".
private func compactTimestamp(_ value: String) -> String {
    String(value.replacingOccurrences(of: "T", with: " ").prefix(16))
}
