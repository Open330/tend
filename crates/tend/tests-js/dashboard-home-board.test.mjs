// Home's board model: lanes, order, project groups and what each card may
// do, from the daemon's real documents (apps/shared/Fixtures, recorded from
// a scratch tendd) plus a Fleet snapshot.
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { buildBoard, cantAnswerGroup, evidenceChip, fromServerBoard, groupByProject, hostHue, isPaneless, needsAttention, productOf, TEND_PROJECT } from "../src/dashboard/web/home-board.mjs";

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
  // apps/shared/Design/COUNTS.md: a folded card still counts.
  assert.equal(board.lanes.finished.count, 2);
  assert.equal(board.yourTurn, yourTurn.length, "the badge is Your turn's number");
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

test("an agent's title and line follow the Mac's solo projection", async () => {
  const { promptTitle } = await import("../src/dashboard/web/home-board.mjs");
  assert.equal(promptTitle("<task-notification><summary>Build finished\nmore</summary></task-notification>"), "⟳ Build finished");
  assert.deepEqual(promptTitle("<task-notification>x</task-notification>"), { key: "backgroundTask" });
  assert.deepEqual(promptTitle("[tend:reply] Completed reply from codex@%13 (abc) is ready. Read it"), { key: "replyReady", who: "codex@%13" });
  assert.equal(promptTitle("[tend:review] Review from codex@%39. Read one: x"), "Review from codex@%39");
  const board = buildBoard({
    fleet: { hosts: [{ alias: "mba", local: true, state: "online", remote: {
      agents: [{ agent_session_id: "a", kind: "codex", pane: "%1", tmux_socket: "s", cwd: "/w/old", state: "working", last_prompt: "new task", recap: "the previous turn's recap", state_entered_at: at(1) }],
      panes: [{ pane_id: "%1", socket: "s", current_path: "/w/moved" }],
    } }] },
    now: NOW,
  });
  const unit = board.lanes.running.groups[0].units[0];
  assert.equal(unit.title, "new task", "the prompt, not the recap, is the title");
  assert.deepEqual(unit.headline, { text: "the previous turn's recap" }, "the recap is the status line");
  assert.equal(unit.project, "moved", "the pane's current path wins over the recorded cwd");
  const untitled = buildBoard({ fleet: { hosts: [{ alias: "mba", local: true, remote: { agents: [{ agent_session_id: "b", kind: "claude_code", pane: "%2", state: "working" }] } }] }, now: NOW });
  assert.deepEqual(untitled.lanes.running.groups[0].units[0].title, { key: "kindOnHost", kind: "claude", host: "mba" });
});

test("the daemon's board: a null focus stays null, and Review ignores Home's filters", async () => {
  const { fromServerBoard, reviewQueue, replyTarget, sameTarget } = await import("../src/dashboard/web/home-board.mjs");
  const unit = (id, repo, extra = {}) => ({
    type: "unit", id, lane: "finished", title: id, repo, headline: "done", stages: [{ name: "claude", state: "done" }],
    shown_time: new Date().toISOString(), agents: [{ host: "mba", local: true, pane: "%2", socket: "s", state: "idle", session_id: "x" }],
    helpers: [], focus: null, host_aliases: ["mba"], agent_kinds: ["claude_code"], recent: true, requested_by_person: true, review_key: `home:${id}`, ...extra,
  });
  const server = { hosts: [{ alias: "mba", local: true }], lanes: { needs_you: { groups: [] }, running: { groups: [] }, finished: { groups: [
    { project: "a", items: [unit("u1", "a")] }, { project: "b", items: [unit("u2", "b")] },
  ] } } };
  const board = fromServerBoard(server, { filter: { project: "a" } });
  const shown = board.lanes.finished.groups.flatMap((g) => g.units);
  assert.equal(shown.length, 1);
  assert.equal(shown[0].pane, null, "no stand-in agent for a null focus");
  assert.equal(replyTarget(shown[0]), null, "nobody to answer");
  assert.equal(reviewQueue(fromServerBoard(server, {})).length, 2, "Review counts every finished result");
  // A reply target is the agent at one question.
  const focused = fromServerBoard({ ...server, lanes: { ...server.lanes, finished: { groups: [{ project: "a", items: [unit("u3", "a", {
    focus: { host: "mba", local: true, pane: "%2", socket: "s", state: "idle", session_id: "x", state_entered_at: "t1", owner: "claude · mba" },
  })] }] } } }).lanes.finished.groups[0].units[0];
  const target = replyTarget(focused);
  assert.equal(target.kind, "sendBack");
  assert.equal(target.pane, "%2");
  assert.ok(sameTarget(target, { ...target }));
  assert.ok(!sameTarget(target, { ...target, since: "t2" }), "the same agent's next turn is another target");
  assert.ok(!sameTarget(target, { ...target, session: "y", pane: "%3" }), "another agent is another target");
});

test("a waiting choice or an error is answered in the pane, not by text", async () => {
  const { replyTarget } = await import("../src/dashboard/web/home-board.mjs");
  const focus = (state) => ({ host: "mba", pane: "%1", state, session: "s", remote: false });
  assert.equal(replyTarget({ lane: "needsYou", focus: focus("waiting_choice") }), null);
  assert.equal(replyTarget({ lane: "needsYou", focus: focus("error") }), null);
  assert.equal(replyTarget({ lane: "needsYou", focus: focus("waiting_input") }).kind, "answer");
  assert.equal(replyTarget({ lane: "running", focus: focus("working") }), null);
});

test("Review's Later, project groups and digest follow the Mac", async () => {
  const { withLater, byProject, digest } = await import("../src/dashboard/web/home-board.mjs");
  const units = [{ id: "a", project: "tend" }, { id: "b", project: "" }, { id: "c", project: "tend" }];
  assert.deepEqual(withLater(units, ["a"]).map((u) => u.id), ["b", "c", "a"]);
  assert.deepEqual(withLater(units, ["c", "a", "gone"]).map((u) => u.id), ["b", "c", "a"], "put off in order; gone ones drop");
  assert.deepEqual(byProject(units).map((g) => [g.project, g.units.map((u) => u.id)]), [["tend", ["a", "c"]], [null, ["b"]]]);
  assert.equal(digest("# Fix it\n\n- one\n---\n`two`", 600), "Fix it one two`");
  assert.equal(digest("  \n---\n"), null);
  assert.equal(digest(`${"a".repeat(40)}. ${"b ".repeat(40)}`, 50), `${"a".repeat(40)}.`);
  assert.equal(digest("word word word", 9), "word…");
});

test("waiting on input or a choice, or an error, needs the operator", () => {
  assert.equal(needsAttention({ state: "waiting_input" }), true);
  assert.equal(needsAttention({ state: "waiting_choice" }), true);
  assert.equal(needsAttention({ state: "error" }), true);
  assert.equal(needsAttention({ state: "working" }), false);
  assert.equal(needsAttention(null), false);
});

test("a registry row without a pane is headless", () => {
  assert.equal(isPaneless({ pane: "%1" }), false);
  assert.equal(isPaneless({}), true);
  assert.equal(isPaneless(null), true);
});

test("an asker tendd does not name is marked, not named", () => {
  const queue = {
    hosts: [{
      alias: "local", local: true, control: true, decides_reviews: true,
      queue: { items: [
        { kind: "human_request", request_id: "anon", body: "who asks?", created_at: at(3) },
        { kind: "review", id: "rv_alias", summary: "named", requested_at: at(2), target: { kind: "session", session_id: "x", alias: "an agent" } },
      ] },
    }],
  };
  const units = buildBoard({ decisions: queue, fleet, now: NOW }).lanes.needsYou.groups.flatMap((g) => g.units);
  const anon = units.find((u) => u.id === "anon");
  assert.equal(anon.asker, null);
  assert.equal(anon.askerMissing, true, "the page says \"an agent\" in its language");
  const named = units.find((u) => u.id === "rv_alias");
  assert.equal(named.asker, "an agent", "a real alias is kept as written");
  assert.ok(!named.askerMissing);
});

test("the daemon's counts are shown as given; a filter shows N of the total; the badge stays whole", () => {
  const unit = (id, lane, repo, extra = {}) => ({
    type: "unit", id, lane, repo, title: id, since: new Date(NOW - 60_000).toISOString(),
    agents: [], helpers: [], stages: [], host_aliases: ["mba"], recent: true, ...extra,
  });
  const server = {
    hosts: [{ alias: "mba", local: true, state: "online" }],
    lanes: {
      needs_you: { count: 2, groups: [{ project: "a", items: [unit("y1", "needs_you", "a")] }, { project: "b", items: [unit("y2", "needs_you", "b")] }] },
      running: { count: 1, groups: [{ project: "a", items: [unit("r1", "running", "a")] }] },
      finished: { count: 3, groups: [{ project: "a", items: [unit("f1", "finished", "a"), unit("f2", "finished", "a", { reviewed: true })] }, { project: "b", items: [unit("f3", "finished", "b")] }] },
    },
    counts: { your_turn: 2, actionable: 0, cant_answer: 0, in_progress: 1, finished: 2, reviewed: 1, coverage: "partial" },
    decision_hosts: [],
  };
  const history = { hosts: [{ alias: "mba", local: true, history: { items: [
    { kind: "human_answer", request_id: "old", body: "done?", answer: "yes", status: "completed", answered_at: new Date(NOW - 120_000).toISOString() },
  ] } }] };
  const whole = fromServerBoard(server, { history, now: NOW });
  assert.equal(whole.lanes.needsYou.count, 2);
  assert.equal(whole.lanes.finished.count, 2, "tendd's number: not reviewed, history never counted");
  assert.equal(whole.lanes.finished.total, undefined);
  assert.equal(whole.yourTurn, 2);
  assert.equal(whole.partial, true);
  const narrowed = fromServerBoard(server, { history, now: NOW, filter: { project: "a" } });
  assert.equal(narrowed.lanes.needsYou.count, 1);
  assert.equal(narrowed.lanes.needsYou.total, 2, "the header says 1 of 2");
  assert.equal(narrowed.lanes.finished.count, 1, "f1 only: f2 is reviewed, history is not counted");
  assert.equal(narrowed.yourTurn, 2, "the badge is never narrowed");
});

test("Can't be answered takes each host's floor, summed (2/1, 1/4, 1/none = 7)", () => {
  const gone = (host, id) => ({ id, host, unanswerable: true, remote: host !== "a" });
  const units = [gone("a", "a1"), gone("a", "a2"), gone("b", "b1"), gone("c", "c1")];
  const sections = [
    { alias: "a", queue: { unanswerable_count: 1 } },
    { alias: "b", queue: { unanswerable_count: 4 } },
    { alias: "c", queue: {} },
  ];
  assert.equal(cantAnswerGroup(units, sections).count, 7);
  assert.equal(cantAnswerGroup(units, sections, { host: "b" }).count, 4);
});

test("an older daemon's board still says N of M under a filter, and partial from its sections", () => {
  const unit = (id, repo) => ({ type: "unit", id, lane: "running", repo, title: id, since: new Date(NOW - 60_000).toISOString(), agents: [], helpers: [], stages: [], host_aliases: ["mba"], recent: true });
  const server = {
    hosts: [{ alias: "mba", local: true, state: "online" }],
    lanes: { needs_you: { groups: [] }, running: { groups: [{ project: "a", items: [unit("r1", "a")] }, { project: "b", items: [unit("r2", "b")] }] }, finished: { groups: [] } },
    decision_hosts: [{ alias: "mba", local: true, truncated: true, unavailable: [] }],
  };
  const board = fromServerBoard(server, { now: NOW, filter: { project: "a" } });
  assert.equal(board.lanes.running.count, 1);
  assert.equal(board.lanes.running.total, 2);
  assert.equal(board.partial, true, "a cut queue: not all there is");
});
