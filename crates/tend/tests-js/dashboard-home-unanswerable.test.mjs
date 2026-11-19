// Home's "Can't be answered" group (docs/proposals/STALE_QUESTIONS.md):
// questions whose asker is gone leave Your turn and its count, the group's
// count is exact under truncation, Keep and Close all name exact pairs, Close
// all exists only where the host allows it, and an older daemon's queue
// reads exactly as before.
import test from "node:test";
import assert from "node:assert/strict";

import { buildBoard, cantAnswerGroup, closeAllPayload, fromServerBoard, keepPayload, laneCounts, unanswerableOf } from "../src/dashboard/web/home-board.mjs";
import { decidedCard, hostSections } from "../src/dashboard/web/needs-model.mjs";

const now = Date.parse("2026-11-19T12:00:00Z");

const live = { kind: "human_request", request_id: "rq_live", body: "Merge #12?", from_session_id: "s1", from_pane: "%3", created_at: "2026-11-19T11:00:00Z" };
const gone = (id, cause, episode, created = "2026-11-10T11:00:00Z") => ({
  kind: "human_request",
  request_id: id,
  body: `Deploy ${id}?`,
  from_session_id: `s_${id}`,
  from_pane: "%42",
  created_at: created,
  unanswerable: { cause, episode, since: "2026-11-16T00:00:00Z", pane: "%42" },
});

function decisions({ subjectCheck = true, closeUnanswerable = false, count = 2, truncated = false, remote = null } = {}) {
  const hosts = [
    {
      alias: "mba",
      local: true,
      control: true,
      decides_reviews: true,
      subject_check: subjectCheck,
      close_unanswerable: closeUnanswerable,
      queue: { items: [live, gone("rq_a", "pane_gone", "ep_a"), gone("rq_b", "ended", "ep_b")], truncated, unanswerable_count: count },
    },
  ];
  if (remote) hosts.push(remote);
  return { hosts };
}

test("folded questions leave Your turn and its count, into their own group", () => {
  const board = buildBoard({ decisions: decisions(), now });
  const yourTurn = board.lanes.needsYou;
  assert.equal(yourTurn.count, 1);
  assert.deepEqual(yourTurn.groups.flatMap((g) => g.units).map((u) => u.id), ["rq_live"]);
  assert.equal(laneCounts(board).needsYou, 1, "badges read the lane count");
  assert.equal(yourTurn.cantAnswer.count, 2);
  assert.deepEqual(yourTurn.cantAnswer.units.map((u) => [u.id, u.unanswerable.cause, u.unanswerable.since]), [
    ["rq_a", "pane_gone", "2026-11-16T00:00:00Z"],
    ["rq_b", "ended", "2026-11-16T00:00:00Z"],
  ]);
});

test("the group's count is the host's exact count when the list was cut", () => {
  const board = buildBoard({ decisions: decisions({ count: 7, truncated: true }), now });
  assert.equal(board.lanes.needsYou.cantAnswer.count, 7);
  assert.equal(board.lanes.needsYou.cantAnswer.units.length, 2);
  // Under a project filter only what is listed can be counted.
  const filtered = cantAnswerGroup(board.lanes.needsYou.cantAnswer.units, decisions({ count: 7 }).hosts, { project: "x" });
  assert.equal(filtered.count, 2);
  // Another host's filter leaves this host's count out.
  assert.equal(cantAnswerGroup([], decisions({ count: 7 }).hosts, { host: "other" }).count, 0);
});

test("Keep sends the request and its episode; Close all exactly the listed pairs", () => {
  const group = buildBoard({ decisions: decisions({ closeUnanswerable: true }), now }).lanes.needsYou.cantAnswer;
  assert.ok(group.units.every((u) => u.keepable));
  assert.deepEqual(keepPayload(group.units[0]), { request_id: "rq_a", episode: "ep_a" });
  assert.deepEqual(group.closeAll, {
    items: [
      { request_id: "rq_a", episode: "ep_a" },
      { request_id: "rq_b", episode: "ep_b" },
    ],
    by_label: "web",
  });
  assert.deepEqual(closeAllPayload(group.units.slice(1)).items, [{ request_id: "rq_b", episode: "ep_b" }]);
});

test("Close all is hidden while the host's switch is off, and never offered for another host", () => {
  const off = buildBoard({ decisions: decisions({ closeUnanswerable: false }), now }).lanes.needsYou.cantAnswer;
  assert.equal(off.closeAll, null);
  assert.ok(off.units.every((u) => u.keepable), "Keep works either way");
  const remote = {
    alias: "mini",
    local: false,
    control: true,
    subject_check: true,
    close_unanswerable: true,
    queue: { items: [gone("rq_r", "agent_exited", "ep_r")], unanswerable_count: 1 },
  };
  const group = buildBoard({ decisions: decisions({ closeUnanswerable: false, remote }), now }).lanes.needsYou.cantAnswer;
  assert.equal(group.count, 3);
  const theirs = group.units.find((u) => u.id === "rq_r");
  assert.equal(theirs.keepable, false, "Keep acts on this host only");
  assert.equal(theirs.closable, false);
  assert.equal(group.closeAll, null);
  assert.equal(group.remote, true);
});

test("an older daemon's queue reads exactly as before", () => {
  const old = {
    hosts: [{ alias: "mba", local: true, control: true, queue: { items: [live, { ...gone("rq_a", "pane_gone", "ep_a"), unanswerable: undefined }] } }],
  };
  const board = buildBoard({ decisions: old, now });
  assert.equal(board.lanes.needsYou.count, 2);
  assert.equal(board.lanes.needsYou.cantAnswer.count, 0);
  assert.equal(unanswerableOf({ unanswerable: { cause: "ended" } }), null, "no episode, nothing to act on");
});

test("the daemon's board folds the same way, with its host flags", () => {
  const item = (entry) => ({ type: "decision", id: `human:mba:${entry.request_id}`, host: "mba", local: true, actionable: true, since: entry.created_at, entry });
  const server = {
    hosts: [{ alias: "mba", local: true, state: "online" }],
    decision_hosts: [{ alias: "mba", local: true, control: true, subject_check: true, close_unanswerable: true, unanswerable_count: 4, truncated: true }],
    lanes: {
      needs_you: { count: 3, groups: [{ project: null, items: [item(live), item(gone("rq_a", "replaced", "ep_a"))] }] },
      running: { groups: [] },
      finished: { groups: [] },
    },
  };
  const board = fromServerBoard(server, { now });
  assert.equal(board.lanes.needsYou.count, 1);
  assert.equal(board.lanes.needsYou.cantAnswer.count, 4);
  assert.deepEqual(board.lanes.needsYou.cantAnswer.closeAll.items, [{ request_id: "rq_a", episode: "ep_a" }]);
  // Without the flags (an older daemon's board) nothing can be kept or closed.
  server.decision_hosts = [{ alias: "mba", local: true, control: true }];
  const older = fromServerBoard(server, { now }).lanes.needsYou.cantAnswer;
  assert.equal(older.count, 1);
  assert.equal(older.closeAll, null);
  assert.equal(older.units[0].keepable, false);
});

test("history: tend's own closures and a person's close say why", () => {
  const history = {
    hosts: [
      {
        alias: "mba",
        local: true,
        history: {
          items: [
            { kind: "closed_by_tend", request_id: "rq_sys", body: "Recover?", from_session_id: "tend:peer-recovery", from_pane: "", created_at: "2026-11-17T00:00:00Z", system: true, reason: "stale", reason_text: "stale: nobody left to act on it", closed_by: "daemon", closed_at: "2026-11-19T10:00:00Z" },
            { kind: "human_answer", request_id: "rq_a", body: "Deploy?", from_session_id: "s", from_pane: "%4", created_at: "2026-11-10T00:00:00Z", status: "declined", answer: "dismissed", answered_at: "2026-11-19T11:00:00Z", closure: { reason: "subject_gone", cause: "pane_gone", episode: "ep_a", by: "person", surface: "web", at: "2026-11-19T11:00:00Z" } },
          ],
        },
      },
    ],
  };
  const finished = buildBoard({ history, now }).lanes.finished.groups.flatMap((g) => g.units);
  const closed = finished.find((u) => u.outcome === "closedByTend");
  assert.equal(closed.reason, "stale");
  assert.equal(closed.reasonText, "stale: nobody left to act on it");
  const declined = finished.find((u) => u.outcome === "declined");
  assert.deepEqual(declined.closure, { cause: "pane_gone", surface: "web" });
  assert.equal(decidedCard(history.hosts[0].history.items[0], now).label, "closed by tend");
});

test("the Needs page lists folded questions last, marked", () => {
  const sections = hostSections(decisions(), now);
  const cards = sections[0].cards;
  assert.equal(cards[0].id, "rq_live");
  assert.deepEqual(cards.slice(1).map((c) => [c.id, c.label]), [
    ["rq_a", "can't be answered"],
    ["rq_b", "can't be answered"],
  ]);
});
