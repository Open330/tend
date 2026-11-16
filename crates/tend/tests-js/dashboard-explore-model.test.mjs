// Explore's sidebar model: every host's panes with their agents, the Mac's
// host tree and status groups, the filter, and the pane deep link.
import test from "node:test";
import assert from "node:assert/strict";
import { exploreSections, paneFromHash, paneHash, paneRows } from "../src/dashboard/web/explore-model.mjs";

const fleet = {
  hosts: [
    { alias: "mini", local: false, state: "offline", error: "ssh timed out", remote: { agents: [], panes: [] } },
    {
      alias: "mba",
      local: true,
      state: "online",
      tend_version: "0.10.21",
      remote: {
        agents: [
          { agent_session_id: "a", kind: "claude_code", pane: "%1", tmux_socket: "s", state: "working", last_prompt: "port the board" },
          { agent_session_id: "b", kind: "codex", pane: "%2", tmux_socket: "s", state: "waiting_input", last_prompt: "pick" },
          { agent_session_id: "c", kind: "claude_code", pane: "%9", tmux_socket: "s", state: "stopped" },
        ],
        panes: [
          { pane_id: "%1", socket: "s", session: "tend", window_index: "0", window_name: "impl", current_path: "/w/tend", current_command: "claude" },
          { pane_id: "%2", socket: "s", session: "tend", window_index: "1", window_name: "review", current_path: "/w/tend", current_command: "codex" },
          { pane_id: "%3", socket: "s", session: "ops", window_index: "0", window_name: "zsh", current_path: "/w/ops", current_command: "zsh" },
        ],
      },
    },
  ],
};

test("one row per pane, with its agent and status", () => {
  const rows = paneRows(fleet);
  assert.equal(rows.length, 3);
  const byPane = Object.fromEntries(rows.map((r) => [r.pane, r]));
  assert.equal(byPane["%1"].group, "active");
  assert.equal(byPane["%1"].title, "port the board");
  assert.equal(byPane["%2"].group, "attention");
  assert.equal(byPane["%3"].group, "shell");
  assert.equal(byPane["%3"].agent, null);
  assert.equal(byPane["%3"].repo, "ops");
});

test("the host tree: this host first, sessions inside, what needs you on top", () => {
  const model = exploreSections(fleet);
  assert.deepEqual(model.sections.map((s) => s.title), ["mba", "mini"]);
  assert.deepEqual(model.sections[0].sessions.map((s) => s.name), ["tend", "ops"]);
  assert.equal(model.sections[1].state, "offline");
  assert.deepEqual(model.attention.map((r) => r.pane), ["%2"]);
});

test("filters and status groups", () => {
  assert.equal(exploreSections(fleet, { show: "attention" }).total, 1);
  assert.equal(exploreSections(fleet, { show: "active" }).total, 2, "active includes what needs you");
  assert.equal(exploreSections(fleet, { query: "OPS" }).total, 1);
  const status = exploreSections(fleet, { grouping: "status" });
  assert.deepEqual(status.sections.map((s) => s.key), ["attention", "active", "shell"]);
});

test("the pane deep link round-trips, naming the host only for another one", () => {
  const [row] = paneRows(fleet).filter((r) => r.pane === "%2");
  assert.equal(paneHash(row), "#pane=%252&socket=s");
  assert.deepEqual(paneFromHash("#pane=%252&socket=s"), { pane: "%2", socket: "s", host: null });
  assert.equal(paneFromHash("#item=x"), null);
});
