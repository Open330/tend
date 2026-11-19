// Explore → Messages as data: the links it reads and writes, the query it
// sends, the scope a pane or session gives it, the host check that keeps
// and another daemon's ids out. Old links: dashboard-legacy-link.test.mjs.
import test from "node:test";
import assert from "node:assert/strict";

import {
  collaborationParams,
  customRangeError,
  hostCheck,
  mergeCollaborationRequests,
  messagesFromHash,
  messagesHash,
  messagesLink,
  messagesReturnPath,
  paneMessagesState,
  paneResolver,
  participantPaneLink,
  readerState,
  safeExternalHref,
  sameQuery,
  scopeReaderRequests,
  sessionMessagesState,
} from "../src/dashboard/web/messages-model.mjs";
import {
  collaborationStatusClass,
  conversationText,
  graphLayout,
  projectCollaboration,
  requestsTouchingPane,
  sequenceEvents,
  sequenceLayout,
} from "../src/dashboard/web/collaboration-model.mjs";

const fleet = {
  hosts: [
    { alias: "mba", local: true, remote: { panes: [
      { pane_id: "%1", socket: "/tmp/tmux-1/default", session: "tend" },
      { pane_id: "%2", socket: "/tmp/tmux-1/default", session: "web~view~1" },
      { pane_id: "%9", socket: "/tmp/tmux-1/default", session: "a" },
      { pane_id: "%9", socket: "/tmp/tmux-1/other", session: "b" },
    ] } },
    { alias: "mini", local: false, remote: { panes: [{ pane_id: "%1", socket: "/tmp/tmux-1/default", session: "tend" }] } },
  ],
};
const room = { host: "tmux", socket: "/tmp/tmux-1/default", window_id: "@1" };
const agent = (pane, session, extra = {}) => ({ agent_kind: "codex", agent_session_id: `s-${pane}`, pane, socket: "/tmp/tmux-1/default", room, tmux_session_name: session, ...extra });

test("a link round-trips the reader: scope, host, filters and tab, defaults left out", () => {
  const state = readerState({
    message: "req_1",
    host: "mini",
    scope: { session: "tend", socket: "/tmp/tmux-1/default", pane: "%1" },
    tab: "graph",
    kind: "review",
    status: "queued",
    work: "ws/ENG-1",
    q: "deploy",
    room,
  });
  const hash = messagesHash(state);
  const back = messagesFromHash(hash);
  assert.deepEqual(back, { ...state, range: "all", open: "req_1" });
  // A message link reaches all retained history; it says nothing more.
  assert.equal(messagesHash(readerState({ message: "req_1" })), "#message=req_1");
  assert.equal(messagesFromHash("#message=req_1").range, "all");
  assert.equal(messagesFromHash("").range, "7d");
  assert.equal(messagesHash(readerState()), "");
  assert.equal(messagesLink({ thread: "T 1" }), "/explore/messages#thread=T+1");
});

test("a link's unknown values and keys are ignored, never sent", () => {
  const state = messagesFromHash("#kind=bogus&status=nope&tab=evil&range=forever&room=%7Bbad&token=secret&since=x");
  assert.equal(state.kind, "");
  assert.equal(state.status, "");
  assert.equal(state.tab, "messages");
  assert.equal(state.range, "7d");
  assert.equal(state.room, null);
  assert.equal(state.since, "", "a period only with range=custom");
  assert.ok(!messagesHash(state).includes("secret"));
  assert.equal(messagesReturnPath("#message=a&token=secret"), "/explore/messages#message=a");
});

test("the query carries every filter, the scope and the next page", () => {
  const { params } = collaborationParams(readerState({
    range: "24h",
    q: "deploy",
    message: "req_1",
    work: "ws/ENG-1",
    thread: "t",
    kind: "task",
    status: "blocked",
    room,
    scope: { session: "tend~view~2", socket: "/tmp/tmux-1/default", pane: "%1" },
  }), { append: true, pagination: { next_cursor: "123:req_0" } });
  assert.deepEqual(Object.fromEntries(params), {
    limit: "500",
    since: "24h",
    q: "deploy",
    id: "req_1",
    workspace: "ws",
    work: "ENG-1",
    thread: "t",
    kind: "task",
    status: "blocked",
    room: JSON.stringify(room),
    session: "tend",
    socket: "/tmp/tmux-1/default",
    cursor: "123:req_0",
  });
  // A workspace scope without a session filters by workspace.
  const workspace = collaborationParams(readerState({ scope: { workspace: "webapp" } })).params;
  assert.equal(workspace.get("workspace"), "webapp");
  assert.equal(workspace.get("session"), null);
  assert.equal(collaborationParams(readerState(), { append: true, pagination: { next_offset: 500 } }).params.get("offset"), "500");
});

test("a custom period is asked for only when it is one", () => {
  assert.equal(customRangeError(readerState({ range: "custom" })), "missing");
  assert.equal(customRangeError(readerState({ range: "custom", since: "2026-11-02T10:00", until: "2026-11-01T10:00" })), "order");
  assert.deepEqual(collaborationParams(readerState({ range: "custom", since: "2026-11-02T10:00" })), { params: null, error: "missing" });
  const ok = collaborationParams(readerState({ range: "custom", since: "2026-11-01T10:00", until: "2026-11-02T10:00" })).params;
  assert.equal(ok.get("since"), new Date("2026-11-01T10:00").toISOString());
  assert.equal(ok.get("until"), new Date("2026-11-02T10:00").toISOString());
});

test("a message id is looked up only on the host its link names", () => {
  assert.equal(hostCheck(readerState(), fleet), "local");
  assert.equal(hostCheck(readerState({ host: "mba" }), fleet), "local");
  assert.equal(hostCheck(readerState({ host: "mini" }), fleet), "remote");
  assert.equal(hostCheck(readerState({ host: "elsewhere" }), fleet), "remote");
  assert.equal(hostCheck(readerState({ host: "mba" }), null), "unknown");
});

test("a pane or session in Explore gives the reader its scope, host kept", () => {
  const local = paneMessagesState({ host: "mba", local: true, pane: "%1", socket: "/tmp/tmux-1/default", session: "tend" });
  assert.equal(messagesHash(local), "#session=tend&socket=%2Ftmp%2Ftmux-1%2Fdefault&pane=%251");
  const remote = paneMessagesState({ host: "mini", local: false, pane: "%1", socket: null, session: "tend" });
  assert.equal(messagesHash(remote), "#host=mini&session=tend&pane=%251");
  assert.equal(messagesHash(sessionMessagesState({ host: "mba", local: true, session: "tend", socket: "s" })), "#session=tend&socket=s");
});

test("scope keeps a session's history, a closed pane's too, and narrows to one pane", () => {
  const requests = [
    { id: "a", from: agent("%1", "tend"), to: agent("%2", "web") },
    { id: "b", from: agent("%7", "tend"), to: { console: true } }, // %7 has closed
    { id: "c", from: agent("%3", "other"), to: agent("%4", "other") },
    { id: "d", from: agent("%1", "tend", { socket: "/tmp/tmux-1/other" }), to: { console: true } },
  ];
  const resolve = paneResolver(fleet);
  assert.deepEqual(resolve("%2", null), { session: "web", socket: "default" });
  assert.equal(resolve("%9", null), null, "ambiguous across tmux servers");
  assert.deepEqual(resolve("%9", "/tmp/tmux-1/other"), { session: "b", socket: "other" });
  const session = scopeReaderRequests(requests, { session: "tend", socket: "/tmp/tmux-1/default" }, resolve).map((r) => r.id);
  assert.deepEqual(session, ["a", "b"]);
  const pane = scopeReaderRequests(requests, { session: "tend", socket: "/tmp/tmux-1/default", pane: "%1" }, resolve).map((r) => r.id);
  assert.deepEqual(pane, ["a"], "the same pane id on another tmux server is not this pane");
  assert.equal(scopeReaderRequests(requests, null).length, 4);
  assert.deepEqual(requestsTouchingPane(requests, "%1").map((r) => r.id), ["a", "d"]);
});

test("a participant opens its pane in Explore only while it is live here", () => {
  assert.equal(participantPaneLink(agent("%1", "tend"), fleet), "/explore#pane=%251&socket=%2Ftmp%2Ftmux-1%2Fdefault");
  assert.equal(participantPaneLink(agent("%7", "tend"), fleet), null);
  assert.equal(participantPaneLink({ console: true }, fleet), null);
});

test("pages merge by id, oldest first; same-query changes do not refetch", () => {
  const merged = mergeCollaborationRequests(
    [{ id: "b", created_at: "2026-11-02" }, { id: "a", created_at: "2026-11-01", status: "queued" }],
    [{ id: "a", created_at: "2026-11-01", status: "completed" }, { id: "c", created_at: "2026-10-30" }],
  );
  assert.deepEqual(merged.map((r) => [r.id, r.status]), [["c", undefined], ["a", "completed"], ["b", undefined]]);
  assert.ok(sameQuery(readerState({ tab: "graph", open: "x" }), readerState()));
  assert.ok(!sameQuery(readerState({ kind: "task" }), readerState()));
  assert.equal(safeExternalHref("javascript:alert(1)", "http://h"), "");
  assert.equal(safeExternalHref("/x", "http://h"), "http://h/x");
});

test("graph and sequence layouts: one path per direction, replies after requests", () => {
  const a = agent("%1", "tend");
  const b = agent("%2", "web");
  const requests = [
    { id: "r1", from: a, to: b, kind: "review", status: "completed", created_at: "2026-11-01T00:00:00Z", reply: { at: "2026-11-01T00:00:00Z", status: "completed" } },
    { id: "r2", from: b, to: a, kind: "task", status: "queued", created_at: "2026-11-01T00:01:00Z" },
  ];
  const layout = graphLayout(projectCollaboration(requests));
  assert.equal(layout.edges.length, 2);
  assert.ok(layout.edges[0].path.startsWith("M "));
  assert.notEqual(layout.edges[0].path, layout.edges[1].path, "opposite directions bend apart");
  assert.ok(layout.edges.find((e) => e.edge.replyCount)?.replyPath);
  assert.ok(layout.width >= 560 && layout.height >= 320);
  assert.deepEqual(sequenceEvents(requests).map((e) => [e.request.id, e.reply]), [["r1", false], ["r1", true], ["r2", false]]);
  const capped = sequenceLayout(requests, { cap: 1 });
  assert.equal(capped.total, 2);
  assert.deepEqual(capped.visible.map((r) => r.id), ["r2"]);
  assert.equal(capped.rows.length, 1);
  assert.equal(collaborationStatusClass("expired"), "danger");
  assert.equal(collaborationStatusClass("claimed"), "pending");
  assert.equal(conversationText({ body: "q", updates: [{ body: "u" }], reply: { body: "r" } }), "q\n\nu\n\nr");
});
