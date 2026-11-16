// Home's board model: lanes, order, project groups and what each card may
// do, from the daemon's real documents (apps/shared/Fixtures, recorded from
// a scratch tendd) plus a Fleet snapshot.
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { buildBoard, evidenceChip, groupByProject, hostHue, productOf, TEND_PROJECT } from "../src/dashboard/web/home-board.mjs";

const fixtures = join(dirname(fileURLToPath(import.meta.url)), "../../../apps/shared/Fixtures");
const load = (name) => JSON.parse(readFileSync(join(fixtures, name), "utf8"));
const decisions = load("decisions.json");
const decisionsFleet = load("decisions-fleet.json");
const NOW = Date.parse("2026-11-15T09:00:00Z");
const at = (minutes) => new Date(NOW - minutes * 60_000).toISOString();

const fleet = {
  hosts: [
    {
      alias: "local",
      local: true,
      state: "online",
      remote: {
        agents: [
          { agent_session_id: "ios-asker-1794730400", kind: "claude_code", pane: "%5", tmux_socket: "s", cwd: "/w/tend", state: "working", last_prompt: "migrate storage", state_entered_at: at(30) },
          { agent_session_id: "w2", kind: "codex", pane: "%6", tmux_socket: "s", state: "working", last_prompt: "port the board", state_entered_at: at(2) },
          { agent_session_id: "w3", kind: "claude_code", pane: "%7", tmux_socket: "s", cwd: "/w/site", state: "waiting_input", last_prompt: "pick a font", state_entered_at: at(5) },
          { agent_session_id: "d1", kind: "claude_code", pane: "%8", tmux_socket: "s", cwd: "/w/tend", state: "idle", last_prompt: "fix toast", last_response: "Done: toast fixed\nmore", state_entered_at: at(10) },
          { agent_session_id: "d2", kind: "claude_code", pane: "%9", tmux_socket: "s", cwd: "/w/tend", state: "idle", last_prompt: "old work", last_response: "old", state_entered_at: at(60 * 30) },
          { agent_session_id: "h1", kind: "claude_code", state: "working", last_prompt: "headless" },
          { agent_session_id: "s1", kind: "claude_code", pane: "%10", state: "stopped", last_prompt: "gone" },
        ],
        panes: [{ pane_id: "%6", socket: "s", current_path: "/w/site" }],
      },
    },
    { alias: "mini", local: false, state: "offline", error: "ssh timed out", remote: null },
  ],
};

test("lanes: the queue in Your turn, working agents in progress, idle ones finished", () => {
  const board = buildBoard({ decisions, fleet, now: NOW });
  const yourTurn = board.lanes.needsYou.groups.flatMap((g) => g.units);
  assert.equal(yourTurn.length, decisions.hosts[0].queue.items.length);
  assert.ok(yourTurn.every((u) => u.type === "decision"));
  const running = board.lanes.running.groups.flatMap((g) => g.units);
  assert.deepEqual(running.map((u) => u.title), ["port the board", "migrate storage"], "newest first; no headless, waiting or stopped agents");
  const finished = board.lanes.finished.groups.flatMap((g) => g.units);
  assert.deepEqual(finished.map((u) => u.title), ["fix toast"]);
  assert.equal(finished[0].headline.text, "Done: toast fixed", "the last result's first line");
  assert.equal(board.lanes.finished.older.length, 1, "older than a day folds away");
  assert.equal(board.lanes.finished.count, 1);
});

test("a question takes its asker's project; an agent without cwd takes its pane's path", () => {
  const board = buildBoard({ decisions, fleet, now: NOW });
  const question = board.lanes.needsYou.groups.flatMap((g) => g.units).find((u) => u.decision === "question");
  assert.equal(question.project, "tend");
  assert.equal(question.agentKind, "claude_code");
  const codex = board.lanes.running.groups.flatMap((g) => g.units).find((u) => u.title === "port the board");
  assert.equal(codex.project, "site");
  assert.equal(codex.product, "openai");
});

test("Your turn: the longest wait first, tend's own asks last", () => {
  const queue = {
    hosts: [{
      alias: "local", local: true, control: true, decides_reviews: true,
      queue: { items: [
        { kind: "human_request", request_id: "sys", body: "peer recovery", system: true, created_at: at(90), from_session_id: "tend:peer-recovery" },
        { kind: "human_request", request_id: "new", body: "newer", created_at: at(1) },
        { kind: "review", id: "rv", summary: "old result", requested_at: at(40), target: { kind: "session", session_id: "x" } },
        { kind: "agent_waiting", session_id: "w3", agent_kind: "claude_code", pane: "%7", tmux_socket: "s", state: "waiting_input", message: "Which font?", since: at(5) },
      ] },
    }],
  };
  const board = buildBoard({ decisions: queue, fleet, now: NOW });
  const groups = board.lanes.needsYou.groups;
  assert.equal(groups.at(-1).project, TEND_PROJECT, "tend's own group is last");
  const order = groups.flatMap((g) => g.units).map((u) => u.id);
  assert.equal(order.at(-1), "sys");
  const site = groups.find((g) => g.project === "site").units[0];
  assert.equal(site.type, "work", "a waiting agent is a unit in Your turn");
  assert.equal(site.headline.text, "Which font?");
  assert.equal(site.stages[0].state, "needsYou");
  const noProject = groups.find((g) => g.project === "").units.map((u) => u.id);
  assert.deepEqual(noProject, ["rv", "new"], "older first within a project");
  assert.ok(!board.lanes.running.groups.flatMap((g) => g.units).some((u) => u.id === "local:w3"), "never also in progress");
});

test("remote cards: observe-only hosts cannot act; results only where the host decides them", () => {
  const board = buildBoard({ decisions: decisionsFleet, now: NOW });
  const units = board.lanes.needsYou.groups.flatMap((g) => g.units);
  for (const unit of units.filter((u) => u.remote && u.type === "decision")) {
    const host = decisionsFleet.hosts.find((h) => h.alias === unit.host);
    const expected = host.control !== false && (unit.decision === "question" || host.decides_reviews === true);
    assert.equal(unit.actionable, expected, `${unit.host} ${unit.decision}`);
  }
  for (const unit of units.filter((u) => u.remote && u.type === "work")) {
    assert.equal(unit.pane, null, "another host's pane does not open here");
  }
  assert.ok(board.lanes.needsYou.notes.some((n) => n.key === "observeOnly"));
  assert.ok(units.every((u) => u !== null));
});

test("history lands in Finished with what was decided", () => {
  const history = load("decisions-history.json");
  const board = buildBoard({ history: { hosts: [{ alias: "local", local: true, history: history.history }] }, now: Date.parse("2026-11-15T09:00:00Z"), window: 365 * 86400_000 });
  const decided = board.lanes.finished.groups.flatMap((g) => g.units).filter((u) => u.type === "decided");
  assert.ok(decided.length >= 2);
  assert.ok(decided.some((u) => u.outcome === "accepted"));
  assert.ok(decided.some((u) => u.outcome === "answered"));
});

test("filters narrow every lane to one host or project", () => {
  const board = buildBoard({ decisions, fleet, now: NOW, filter: { project: "site" } });
  assert.equal(board.lanes.needsYou.count, 0);
  assert.deepEqual(board.lanes.running.groups.flatMap((g) => g.units).map((u) => u.title), ["port the board"]);
  assert.deepEqual(board.projects, ["site", "tend"]);
  const hosts = buildBoard({ decisions, fleet, now: NOW, filter: { host: "mini" } });
  assert.equal(hosts.lanes.running.count, 0);
  assert.deepEqual(hosts.hosts.map((h) => [h.alias, h.state]), [["local", "online"], ["mini", "offline"]]);
});

test("groups: a project sits where its first unit is, tend's own last", () => {
  const groups = groupByProject([{ project: TEND_PROJECT }, { project: "b" }, { project: "" }, { project: "a" }, { project: "b" }]);
  assert.deepEqual(groups.map((g) => g.project), ["b", "", "a", TEND_PROJECT]);
  assert.equal(groups[0].units.length, 2);
});

test("evidence chips link only http(s) and name a pull request like the Mac", () => {
  assert.deepEqual(evidenceChip({ kind: "pull_request", url: "https://github.com/Open330/tend/pull/12" }), { icon: "pr", text: "Open330/tend#12", href: "https://github.com/Open330/tend/pull/12" });
  assert.equal(evidenceChip({ kind: "pull_request", url: "javascript:alert(1)" }).href, null);
  assert.equal(evidenceChip({ kind: "test", command: "cargo test", passed: false }).tone, "fail");
  assert.equal(evidenceChip({ kind: "nope" }), null);
});

test("host identity colour is FNV-1a of the alias modulo 8, like TendTheme", () => {
  assert.ok(hostHue("jiun-mini") >= 0 && hostHue("jiun-mini") < 8);
  assert.equal(hostHue("a"), 0xe40c292c % 8);
  assert.equal(productOf("claude_code"), "claude");
  assert.equal(productOf("codex"), "openai");
  assert.equal(productOf("gemini"), null);
});
