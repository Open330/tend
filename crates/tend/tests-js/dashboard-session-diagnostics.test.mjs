import assert from "node:assert/strict";
import test from "node:test";
import { baseSessionName } from "../src/dashboard/web/session-model.mjs";
import { projectCollaboration, scopeCollaborationRequests } from "../src/dashboard/web/collaboration-model.mjs";

test("a numeric client-view suffix names its base session; other names stay", () => {
  assert.equal(baseSessionName("webapp~view~123"), "webapp");
  assert.equal(baseSessionName("webapp~view~12~view~34"), "webapp");
  assert.equal(baseSessionName("webapp~view~beta"), "webapp~view~beta");
});

test("retained collaboration rooms have a session group and client views share its scope", () => {
  const participant = (session, pane) => ({ tmux_session_name: session, pane, agent_session_id: pane, agent_kind: "codex", room: { host: "rmux", socket: "/tmp/rmux/default", window_id: "@1" } });
  const messages = [
    { id: "webapp", from: participant("webapp~view~123", "%1"), to: participant("webapp", "%2") },
    { id: "junia", from: participant("junia", "%3"), to: participant("junia", "%4") },
  ];
  assert.deepEqual(scopeCollaborationRequests(messages, { session: "webapp", socket: "default" }).map((request) => request.id), ["webapp"]);
  const projection = projectCollaboration(messages.slice(0, 1));
  assert.equal(projection.rooms[0].session, "webapp");
  assert.ok(!projection.rooms[0].label.includes("~view~"));
});
