import test from "node:test";
import assert from "node:assert/strict";

import { agentCard, cardCount, finishedAgentSections, finishedSections, progressSections } from "../src/dashboard/web/home-model.mjs";

const now = Date.parse("2026-11-15T12:00:00Z");
const at = (minutesAgo) => new Date(now - minutesAgo * 60_000).toISOString();

const fleet = {
  hosts: [
    { alias: "mini", local: false, state: "online", remote: { agents: [
      { agent_session_id: "r1", kind: "codex", pane: "%9", tmux_socket: "/tmp/s", state: "working", last_prompt: "remote work", state_entered_at: at(5) },
    ] } },
    { alias: "home", local: true, state: "online", remote: { agents: [
      { agent_session_id: "a", kind: "claude_code", pane: "rmux:%1", tmux_socket: "\\\\.\\pipe\\rmux-1", tmux_session: "tend", cwd: "F:\\workspace\\tend", state: "working", last_prompt: "older\nsecond line", state_entered_at: at(30) },
      { agent_session_id: "b", kind: "codex", pane: "rmux:%2", state: "starting", state_entered_at: at(1) },
      { agent_session_id: "c", kind: "claude_code", pane: "rmux:%3", state: "waiting_input", last_prompt: "asks", state_entered_at: at(2) },
      { agent_session_id: "d", kind: "claude_code", pane: "rmux:%4", state: "idle", last_prompt: "done task", last_response: "All green\nmore", state_entered_at: at(10) },
      { agent_session_id: "e", kind: "claude_code", pane: "rmux:%5", state: "idle", state_entered_at: at(3) },
      { agent_session_id: "f", kind: "claude_code", pane: "rmux:%6", state: "idle", last_prompt: "yesterday", state_entered_at: at(60 * 30) },
      { agent_session_id: "g", kind: "claude_code", state: "working", last_prompt: "headless" },
      { agent_session_id: "h", kind: "claude_code", pane: "rmux:%7", state: "stopped", last_prompt: "gone" },
    ] } },
    { alias: "rtzr", local: false, state: "offline", error: "ssh: timeout" },
  ],
};

test("In progress: working and starting agents per host, this host first, newest first", () => {
  const sections = progressSections(fleet, now);
  assert.deepEqual(sections.map((s) => s.alias), ["home", "mini", "rtzr"]);
  assert.deepEqual(sections[0].cards.map((c) => c.id), ["home:b", "home:a"]);
  assert.deepEqual(sections[1].cards.map((c) => c.id), ["mini:r1"]);
  assert.deepEqual(sections[2].notes, ["offline: ssh: timeout"]);
  assert.equal(cardCount(sections), 3);
});

test("an agent card names the task, the repo and where it runs; only this host's panes link", () => {
  const [home, mini] = progressSections(fleet, now);
  const card = home.cards[1];
  assert.equal(card.title, "older");
  assert.equal(card.meta, "claude_code · tend · tend · %1 · 30m");
  assert.equal(card.pane, "rmux:%1");
  assert.equal(card.socket, "\\\\.\\pipe\\rmux-1");
  assert.equal(card.actionable, false);
  assert.equal(mini.cards[0].pane, null);
  assert.equal(mini.cards[0].remote, true);
  assert.equal(agentCard({ state: "working" }, { alias: "x", local: true }, now).title, "working");
});

test("Finished: idle agents that did something recently, with what they said", () => {
  const [home] = finishedAgentSections(fleet, now);
  assert.deepEqual(home.cards.map((c) => c.id), ["home:d"]);
  assert.equal(home.cards[0].outcome, "All green");
});

test("Finished merges decided items and finished agents under one heading per host", () => {
  const history = [{ alias: "home", local: true, covers: [], notes: ["more than shown"], cards: [{ id: "rv1" }] }];
  const merged = finishedSections(history, finishedAgentSections(fleet, now));
  assert.deepEqual(merged.map((s) => s.alias), ["home", "mini", "rtzr"]);
  assert.deepEqual(merged[0].cards.map((c) => c.id), ["rv1", "home:d"]);
  assert.deepEqual(merged[0].notes, ["more than shown"]);
});
