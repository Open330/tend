import Foundation
import Testing
@testable import Tend

private func agent(_ fields: [String: Any]) throws -> TendAgent {
    var wire: [String: Any] = ["kind": "claude_code", "agent_session_id": "s-sdk", "state": "error"]
    wire.merge(fields) { _, new in new }
    return try JSONDecoder().decode(TendAgent.self, from: JSONSerialization.data(withJSONObject: wire))
}

/// The agent detail says each fact once: with the request card shown, the
/// Summary tab does not repeat the card's message or its latest prompt.
@Test func agentDetailSummaryDoesNotRepeatTheRequestCard() throws {
    let failed = try agent(["last_response": "500 from the codegen service", "last_prompt": "regenerate the client"])
    #expect(InboxAgentRequestCard.waitingText(failed) == "500 from the codegen service")
    #expect(FleetAgentDetailView.summaryBelowCard(failed, cardShown: true) == nil)
    #expect(FleetAgentDetailView.summaryBelowCard(failed, cardShown: false) == "500 from the codegen service")

    // The card shows the notification; the reply it stopped at is new here.
    let asking = try agent(["state": "waiting_input", "last_notification": "Pick a schema",
                            "last_response": "Two schemas fit; see the diff."])
    #expect(FleetAgentDetailView.summaryBelowCard(asking, cardShown: true) == "Two schemas fit; see the diff.")

    // Only a prompt is retained: the card's Latest prompt already says it.
    let promptOnly = try agent(["last_prompt": "regenerate the client"])
    #expect(FleetAgentDetailView.summaryBelowCard(promptOnly, cardShown: true) == nil)
    #expect(FleetAgentDetailView.summaryBelowCard(promptOnly, cardShown: false) == "regenerate the client")
}
