// Explore's sidebar model: every host's panes with their agents, the Mac's
// host tree and status groups, the filter, and the pane deep link.
import test from "node:test";
import assert from "node:assert/strict";
import {
  DEFAULT_EXPLORE_SORT,
  EXPLORE_SORTS,
  comparePanes,
  exploreSections,
  exploreSortOf,
  paneFromHash,
  paneHash,
  paneMatches,
  paneRows,
} from "../src/dashboard/web/explore-model.mjs";

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
  assert.deepEqual(model.sections[0].sessions.map((s) => s.name), ["ops", "tend"], "topology: sessions by name");
  assert.equal(model.sections[1].state, "offline");
  assert.deepEqual(model.attention.map((r) => r.pane), ["%2"]);
});

test("filters and status groups", () => {
  assert.equal(exploreSections(fleet, { show: "attention" }).total, 1);
  assert.equal(exploreSections(fleet, { show: "active" }).total, 1, "active is the working panes, as on the Mac");
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

test("two tmux servers with the same session and window names stay apart", () => {
  const twin = { hosts: [{ alias: "mba", local: true, state: "online", remote: { agents: [], panes: [
    { pane_id: "%1", socket: "alpha", session: "tend", window_id: "@1", window_index: "0", window_name: "main" },
    { pane_id: "%1", socket: "beta", session: "tend", window_id: "@1", window_index: "0", window_name: "main" },
  ] } }] };
  const [host] = exploreSections(twin).sections;
  assert.equal(host.sessions.length, 2);
  assert.deepEqual(host.sessions.map((s) => s.socket), ["alpha", "beta"]);
  assert.notEqual(host.sessions[0].windows[0].key, host.sessions[1].windows[0].key);
});

// Three hosts, agents whose three times give three different orders:
//   %1 (mba)  prompt 10:00, update 10:05
//   %2 (mba)  prompt 10:08, update 10:01
//   %1 (mini) prompt 09:00, update 10:10
// and shell panes, which have no time.
const timed = {
  hosts: [
    { alias: "mini", local: false, state: "online", remote: {
      agents: [{ agent_session_id: "c", kind: "codex", pane: "%1", tmux_socket: "s", state: "idle", last_prompt_at: "2026-11-19T09:00:00Z", last_activity_at: "2026-11-19T10:10:00.250Z" }],
      panes: [{ pane_id: "%1", socket: "s", session: "a", window_index: "0", pane_index: "0", window_name: "main" }],
    } },
    { alias: "air", local: false, state: "online", remote: { agents: [], panes: [
      { pane_id: "%7", socket: "s", session: "x", window_index: "0", pane_index: "1", window_name: "zsh" },
      { pane_id: "%8", socket: "s", session: "x", window_index: "0", pane_index: "0", window_name: "zsh" },
    ] } },
    { alias: "mba", local: true, state: "online", remote: {
      agents: [
        { agent_session_id: "a", kind: "claude_code", pane: "%1", tmux_socket: "s", state: "working", ai_title: "Port the board", last_prompt_at: "2026-11-19T10:00:00Z", last_activity_at: "2026-11-19T10:05:00Z" },
        { agent_session_id: "b-77", kind: "codex", pane: "%2", tmux_socket: "s", state: "working", last_prompt: "secret words", last_prompt_at: "2026-11-19T10:08:00+00:00", last_activity_at: "2026-11-19T10:01:00Z" },
      ],
      panes: [
        { pane_id: "%4", socket: "s", session: "tend", window_index: "10", pane_index: "0", window_name: "w10", current_command: "zsh" },
        { pane_id: "%1", socket: "s", session: "tend", window_index: "0", pane_index: "0", window_name: "impl", current_command: "claude", current_path: "/w/tend" },
        { pane_id: "%2", socket: "s", session: "tend", window_index: "1", pane_index: "0", window_name: "review", agent_alias: "critic", current_command: "codex" },
        { pane_id: "%5", socket: "s", session: "tend", window_index: "2", pane_index: "0", window_name: "w2", current_command: "zsh" },
        { pane_id: "%3", socket: "s", session: "Ops", window_index: "0", pane_index: "0", window_name: "zsh", current_command: "zsh", current_path: "/srv/Deploy" },
      ],
    } },
  ],
};
const flat = (sort) => exploreSections(timed, { grouping: "status", sort }).sections.flatMap((s) => s.rows).map((r) => `${r.host}${r.pane}`);
const tree = (sort) => exploreSections(timed, { sort }).sections.map((h) => [h.title, h.sessions.map((s) => [s.name, s.windows.map((w) => w.name)])]);
const ordered = (sort) => paneRows(timed).sort((a, b) => comparePanes(a, b, sort)).map((r) => `${r.host}${r.pane}`);

test("the Mac's four orders, topology by default", () => {
  assert.deepEqual(EXPLORE_SORTS, ["topology", "recent", "myPrompt", "agentActivity"]);
  assert.equal(DEFAULT_EXPLORE_SORT, "topology");
  assert.equal(exploreSortOf("recent"), "recent");
  assert.equal(exploreSortOf("attention"), "topology", "anything else stored is the default");
  assert.equal(exploreSortOf(null), "topology");
  assert.equal(exploreSections(timed).sort, "topology");
});

test("topology: this host first, then alias; session by name; window and pane by index", () => {
  assert.deepEqual(ordered("topology"), ["mba%3", "mba%1", "mba%2", "mba%5", "mba%4", "air%8", "air%7", "mini%1"]);
  assert.deepEqual(tree("topology"), [
    ["mba", [["Ops", ["zsh"]], ["tend", ["impl", "review", "w2", "w10"]]]],
    ["air", [["x", ["zsh"]]]],
    ["mini", [["a", ["main"]]]],
  ]);
  const [air] = exploreSections(timed).sections.filter((h) => h.title === "air");
  assert.deepEqual(air.sessions[0].windows[0].rows.map((r) => r.pane), ["%8", "%7"], "pane index 0 before 1");
});

test("latest activity: the later of your prompt and the agent's update, newest first", () => {
  assert.deepEqual(ordered("recent"), ["mini%1", "mba%2", "mba%1", "mba%3", "mba%5", "mba%4", "air%8", "air%7"]);
  assert.deepEqual(tree("recent"), [
    ["mini", [["a", ["main"]]]],
    ["mba", [["tend", ["review", "impl", "w2", "w10"]], ["Ops", ["zsh"]]]],
    ["air", [["x", ["zsh"]]]],
  ]);
});

test("my latest prompt: by your prompt alone", () => {
  assert.deepEqual(ordered("myPrompt"), ["mba%2", "mba%1", "mini%1", "mba%3", "mba%5", "mba%4", "air%8", "air%7"]);
  assert.deepEqual(tree("myPrompt").map(([host]) => host), ["mba", "mini", "air"]);
  assert.deepEqual(tree("myPrompt")[0][1][0][1], ["review", "impl", "w2", "w10"]);
});

test("agent latest update: by the agent's update alone", () => {
  assert.deepEqual(ordered("agentActivity"), ["mini%1", "mba%1", "mba%2", "mba%3", "mba%5", "mba%4", "air%8", "air%7"]);
  assert.deepEqual(tree("agentActivity").map(([host]) => host), ["mini", "mba", "air"]);
  assert.deepEqual(tree("agentActivity")[1][1][0][1], ["impl", "review", "w2", "w10"]);
  assert.deepEqual(flat("agentActivity").slice(0, 2), ["mba%1", "mba%2"], "a status group keeps the order (both working)");
});

test("equal or missing times fall back to the topology order", () => {
  const row = (host, pane, at, extra = {}) => ({ host, pane, local: false, session: "s", windowIndex: "0", paneIndex: "0", agent: { promptAt: at, activityAt: at }, ...extra });
  const same = "2026-11-19T10:00:00Z";
  assert.ok(comparePanes(row("b", "%1", same), row("a", "%1", same), "recent") > 0, "same time: by host alias");
  assert.ok(comparePanes(row("b", "%1", same), row("a", "%1", same, { local: true }), "recent") > 0, "same time: this host first");
  assert.ok(comparePanes(row("a", "%10", same), row("a", "%9", same), "recent") > 0, "pane ids by number");
  assert.ok(comparePanes(row("a", "%1", "not a date"), row("b", "%1", null), "recent") < 0, "an unreadable time is no time");
  assert.ok(comparePanes(row("a", "%1", "2026-11-19T10:00:00"), row("b", "%1", null), "recent") < 0, "a time without its zone is no time");
  assert.ok(comparePanes(row("z", "%1", same), row("a", "%1", null), "recent") < 0, "any time before none");
  assert.ok(comparePanes(row("z", "%1", same), row("a", "%1", null), "topology") > 0, "topology ignores time");
});

test("search: host, session, window name, pane id, command, path, agent alias, title and session id, any case", () => {
  const hits = (query) => paneRows(timed).filter((r) => paneMatches(r, query)).map((r) => `${r.host}${r.pane}`).sort();
  assert.deepEqual(hits("MINI"), ["mini%1"], "host alias");
  assert.deepEqual(hits("ops"), ["mba%3"], "session, ignoring case");
  assert.deepEqual(hits("Review"), ["mba%2"], "window name");
  assert.deepEqual(hits("%7"), ["air%7"], "pane id");
  assert.deepEqual(hits("claude"), ["mba%1"], "command");
  assert.deepEqual(hits("/srv/deploy"), ["mba%3"], "path");
  assert.deepEqual(hits("critic"), ["mba%2"], "agent alias");
  assert.deepEqual(hits("port the"), ["mba%1"], "agent title");
  assert.deepEqual(hits("b-77"), ["mba%2"], "agent session id");
  assert.deepEqual(hits("secret"), [], "not the prompt text");
  assert.equal(hits("").length, 8, "an empty query matches every pane");
});

test("a pane's match keeps its session and window, and only them", () => {
  const model = exploreSections(timed, { query: "critic" });
  assert.deepEqual(model.sections.map((h) => h.title), ["mba"], "hosts without a match go");
  assert.deepEqual(model.sections[0].sessions.map((s) => [s.name, s.windows.map((w) => w.name)]), [["tend", ["review"]]]);
  assert.equal(exploreSections(timed, { query: "tend" }).total, 4, "a session's name matches every pane in it");
});

test("the count and the empty states", () => {
  const all = exploreSections(timed);
  assert.deepEqual([all.total, all.all, all.filtering, all.empty], [8, 8, false, null]);
  const some = exploreSections(timed, { query: "zsh" });
  assert.deepEqual([some.total, some.all, some.filtering], [5, 8, true]);
  const none = exploreSections(timed, { query: "nothing like this" });
  assert.equal(none.empty, "noMatch");
  assert.deepEqual(none.sections, []);
  assert.equal(exploreSections(timed, { show: "attention" }).empty, "noMatch", "a status filter too");
  assert.equal(exploreSections(fleet, { show: "attention" }).sections.some((h) => h.title === "mini"), false, "a pane-less host goes under a filter");
  assert.equal(exploreSections({ hosts: [{ alias: "mba", local: true, remote: { agents: [], panes: [] } }] }).empty, "none");
  assert.equal(exploreSections(null).empty, "none");
});

test("what needs you: errors first, then the waiting, and only without a status filter", () => {
  const waiting = { hosts: [{ alias: "mba", local: true, remote: {
    agents: [
      { agent_session_id: "w", kind: "codex", pane: "%1", state: "waiting_input" },
      { agent_session_id: "e", kind: "codex", pane: "%2", state: "error" },
    ],
    panes: [{ pane_id: "%1", session: "a", window_index: "0" }, { pane_id: "%2", session: "a", window_index: "1" }],
  } }] };
  assert.deepEqual(exploreSections(waiting).attention.map((r) => r.pane), ["%2", "%1"]);
  assert.deepEqual(exploreSections(waiting, { show: "attention" }).attention, []);
});

test("needing attention is the board's one rule; an agent that asked groups as idle", async () => {
  const { boardAttention } = await import("../src/dashboard/web/home-board.mjs");
  const later = new Date(Date.now() + 3600_000).toISOString();
  const capped = {
    hosts: [{ alias: "mba", local: true, state: "online", remote: {
      agents: [
        { agent_session_id: "cap", kind: "codex", pane: "%1", tmux_socket: "s", state: "idle", rate_limit_scope: "five_hour", rate_limited_until: later },
        { agent_session_id: "ask", kind: "claude_code", pane: "%2", tmux_socket: "s", state: "idle", awaiting_reply: { question: "Ship?" } },
      ],
      panes: ["%1", "%2"].map((id, i) => ({ pane_id: id, socket: "s", session: "w", window_index: "0", window_name: "w", pane_index: String(i) })),
    } }],
  };
  // Without the board, a cap counts, as on a Mac whose daemon sent no rules.
  let rows = paneRows(capped);
  assert.deepEqual(rows.map((r) => [r.agent.session, r.group, r.attention]), [["cap", "limited", true], ["ask", "idle", true]]);
  // The board says a limit policy covers the cap: it keeps working.
  const board = { lanes: { running: { groups: [{ items: [{ agents: [{ host: "mba", session_id: "cap", needs_attention: false }], helpers: [], focus: null }] }] } } };
  rows = paneRows(capped, boardAttention(board));
  assert.equal(rows[0].attention, false);
  assert.equal(rows[1].attention, true, "an agent the board does not list falls back to the rule");
  assert.deepEqual(exploreSections(capped, { attention: boardAttention(board) }).attention.map((r) => r.agent.session), ["ask"]);
  assert.equal(boardAttention(null), null);
});
