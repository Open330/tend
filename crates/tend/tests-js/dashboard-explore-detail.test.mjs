// Explore's session and window detail (the Mac's FleetSessionDetailView and
// FleetWindowDetailView): what a selection resolves to, the window's
// fallback to its session, the deep links, and the workspace filter.
import test from "node:test";
import assert from "node:assert/strict";
import {
  exploreDetail,
  exploreHash,
  exploreSections,
  inWorkspace,
  sameSocket,
  selectionFromHash,
  sessionSelection,
  windowSelection,
  workspaceFromHash,
} from "../src/dashboard/web/explore-model.mjs";

const pane = (id, socket, session, index, name, extra = {}) => ({
  pane_id: id, socket, session, session_id: `$${session}`, window_id: `@${index}`, window_index: String(index), window_name: name, pane_index: "0", current_command: "zsh", ...extra,
});
// Two tmux servers, each with a session "tend" and a window @1, and a
// second host with the same names again.
const fleet = {
  hosts: [
    {
      alias: "mba", local: true, state: "online",
      remote: {
        agents: [
          { agent_session_id: "a", kind: "claude_code", pane: "%1", tmux_socket: "/tmp/tmux-501/main", state: "working", last_response: "Ported the board.", last_activity_at: "2026-11-19T10:00:00Z" },
          { agent_session_id: "b", kind: "codex", pane: "%2", tmux_socket: "/tmp/tmux-501/main", state: "waiting_input", last_prompt: "pick one", subagents: [{}, {}], workload: { process_count: 3 } },
          { agent_session_id: "c", kind: "codex", pane: "%1", tmux_socket: "/tmp/tmux-501/other", state: "idle", recap: "Reviewed the diff." },
        ],
        panes: [
          pane("%1", "/tmp/tmux-501/main", "tend", 1, "impl", { tend: { workspace_id: "tend", work_id: "fix-login" } }),
          pane("%2", "/tmp/tmux-501/main", "tend", 1, "impl", { pane_index: "1", tend: { workspace_id: "tend", work_id: "fix-login" } }),
          pane("%3", "/tmp/tmux-501/main", "tend", 2, "shell"),
          pane("%1", "/tmp/tmux-501/other", "tend", 1, "review"),
          pane("%4", "/tmp/tmux-501/main", "ops", 1, "logs"),
        ],
      },
    },
    { alias: "mini", local: false, state: "online", remote: { agents: [], panes: [pane("%1", "/tmp/tmux-501/main", "tend", 1, "remote")] } },
  ],
};

test("a session's detail: its host, window and pane counts, and the panes by window", () => {
  const detail = exploreDetail(fleet, { kind: "session", session: "tend", socket: "/tmp/tmux-501/main", host: null });
  assert.equal(detail.kind, "session");
  assert.equal(detail.host, "mba");
  assert.equal(detail.name, "tend");
  assert.equal(detail.windowCount, 2);
  assert.equal(detail.paneCount, 3);
  assert.deepEqual(detail.windows.map((w) => w.name), ["impl", "shell"]);
  assert.equal(detail.windows[0].work, "tend / fix-login", "one Work in the window");
  // What needs you first, then the working (the Mac's panePriority).
  assert.deepEqual(detail.windows[0].cards.map((c) => c.pane), ["%2", "%1"]);
  assert.equal(detail.windows[0].cards[1].latest, "Ported the board.", "the latest response");
  assert.equal(detail.windows[0].cards[0].latest, "pick one", "or the prompt");
  assert.equal(detail.windows[1].cards[0].state, null, "a shell pane");
});

test("a window's detail: its panes and the Mac's metrics", () => {
  const detail = exploreDetail(fleet, { kind: "window", window: "@1", session: "tend", socket: "main" });
  assert.equal(detail.kind, "window");
  assert.equal(detail.name, "impl");
  assert.equal(detail.paneCount, 2);
  assert.deepEqual(detail.metrics, { agents: 2, working: 1, attention: 1, subagents: 2, processes: 3 });
  assert.equal(detail.socket, "/tmp/tmux-501/main");
});

test("identical names on another tmux server or another host do not collide", () => {
  const other = exploreDetail(fleet, { kind: "window", window: "@1", session: "tend", socket: "/tmp/tmux-501/other" });
  assert.equal(other.name, "review");
  assert.equal(other.cards[0].latest, "Reviewed the diff.");
  const remote = exploreDetail(fleet, { kind: "session", session: "tend", socket: "main", host: "mini" });
  assert.equal(remote.host, "mini");
  assert.equal(remote.local, false);
  assert.equal(remote.windows[0].name, "remote");
  const main = exploreDetail(fleet, { kind: "session", session: "tend", socket: "main" });
  assert.equal(main.host, "mba", "no host: this host's");
  assert.notEqual(main.key, remote.key);
  assert.notEqual(main.key, exploreDetail(fleet, { kind: "session", session: "tend", socket: "other" }).key);
});

test("a window as session:index, as main.js accepted", () => {
  const detail = exploreDetail(fleet, { kind: "window", window: "tend:2", session: null, socket: "main" });
  assert.equal(detail.kind, "window");
  assert.equal(detail.name, "shell");
});

test("a window that is gone falls back to its session", () => {
  const byId = exploreDetail(fleet, { kind: "window", window: "@9", session: "tend", socket: "main" });
  assert.equal(byId.kind, "session");
  assert.equal(byId.fallback, true);
  const byIndex = exploreDetail(fleet, { kind: "window", window: "ops:7", session: null, socket: null });
  assert.equal(byIndex.kind, "session");
  assert.equal(byIndex.name, "ops");
});

test("a session that is gone says so", () => {
  assert.equal(exploreDetail(fleet, { kind: "session", session: "nope", socket: null }).kind, "gone");
  assert.equal(exploreDetail(fleet, { kind: "session", session: "tend", host: "elsewhere" }).kind, "gone");
  assert.equal(exploreDetail(fleet, { kind: "pane", pane: "%1" }), null);
});

test("deep links: session and window, with the host and socket, round-trip", () => {
  const sections = exploreSections(fleet).sections;
  const mba = sections.find((s) => s.title === "mba");
  const session = mba.sessions.find((s) => s.name === "tend" && s.socket.endsWith("main"));
  const window = session.windows[0];
  const sessionLink = exploreHash(sessionSelection(mba, session));
  assert.equal(sessionLink, "#session=tend&socket=%2Ftmp%2Ftmux-501%2Fmain");
  assert.equal(exploreDetail(fleet, selectionFromHash(sessionLink)).key, exploreDetail(fleet, sessionSelection(mba, session)).key);
  const windowLink = exploreHash(windowSelection(mba, session, window));
  assert.equal(windowLink, "#window=%401&session=tend&socket=%2Ftmp%2Ftmux-501%2Fmain");
  assert.equal(exploreDetail(fleet, selectionFromHash(windowLink)).name, "impl");
  const mini = sections.find((s) => s.title === "mini");
  const remoteLink = exploreHash(sessionSelection(mini, mini.sessions[0]));
  assert.match(remoteLink, /&host=mini$/);
  assert.equal(exploreDetail(fleet, selectionFromHash(remoteLink)).host, "mini");
});

test("the address reads a pane over a window over a session", () => {
  assert.equal(selectionFromHash("#session=a&window=@1&pane=%251").kind, "pane");
  assert.equal(selectionFromHash("#session=a&window=@1").kind, "window");
  assert.equal(selectionFromHash("#session=a").kind, "session");
  assert.equal(selectionFromHash("#workspace=a"), null);
  assert.equal(selectionFromHash(""), null);
});

test("the workspace filter: its session, or the managed workspace, on its own socket", () => {
  assert.deepEqual(workspaceFromHash("#workspace=tend&wsocket=main&socket=other"), { workspace: "tend", socket: "main" });
  const model = exploreSections(fleet, { workspace: { workspace: "tend", socket: "main" } });
  assert.equal(model.empty, null);
  assert.equal(model.filtering, true);
  assert.deepEqual(model.sections.map((s) => s.title), ["mba", "mini"]);
  assert.deepEqual(model.sections[0].sessions.map((s) => s.socket), ["/tmp/tmux-501/main"], "not the other server's tend");
  assert.equal(inWorkspace({ session: "x", workspaceId: "tend", socket: "/tmp/tmux-501/main" }, { workspace: "tend", socket: null }), true);
});

test("a workspace no live session matches says so, never everything", () => {
  const model = exploreSections(fleet, { workspace: { workspace: "gone", socket: null } });
  assert.equal(model.empty, "noWorkspace");
  assert.equal(model.total, 0);
  assert.deepEqual(model.sections, []);
});

test("a short socket name matches the Fleet's socket path", () => {
  assert.equal(sameSocket("/tmp/tmux-501/main", "main"), true);
  assert.equal(sameSocket("/tmp/tmux-501/main", "/tmp/tmux-501/main"), true);
  assert.equal(sameSocket("/tmp/tmux-501/main", "other"), false);
  assert.equal(sameSocket("/tmp/tmux-501/main", null), true);
});
