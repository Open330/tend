// Explore → Messages under a minimal fake DOM: the reader fetches what its
// link and filters ask for, shows bodies only when the server included
// them, opens threads across scope, never looks up another host's ids,
// and leaves nothing private behind on lost access or destroy.
import test from "node:test";
import assert from "node:assert/strict";

class Node {
  constructor(tag) {
    Object.assign(this, { tag, children: [], listeners: {}, dataset: {}, attributes: {}, hidden: false, textContent: "", className: "", value: "", disabled: false, scrollTop: 0, title: "", tabIndex: 0 });
    const node = this;
    this.classList = { add: (c) => (node.className += ` ${c}`), toggle() {}, contains: (c) => node.className.split(" ").includes(c) };
  }
  get firstChild() { return this.children[0] || null; }
  append(...nodes) { this.children.push(...nodes.filter((n) => n != null)); }
  replaceChildren(...nodes) { this.children = nodes.filter((n) => n != null); }
  addEventListener(type, fn) { (this.listeners[type] ||= []).push(fn); }
  setAttribute(name, value) { this.attributes[name] = String(value); }
  getAttribute(name) { return this.attributes[name]; }
  focus() {}
  fire(type, event = {}) { for (const fn of this.listeners[type] || []) fn({ preventDefault() {}, stopPropagation() {}, ...event }); }
  click() { this.fire("click"); }
  get text() { return [this.textContent, ...this.children.map((c) => (typeof c === "string" ? c : c.text))].join(" "); }
}
const find = (node, pred) => {
  if (!node || typeof node !== "object") return null;
  if (pred(node)) return node;
  for (const child of node.children || []) {
    const hit = find(child, pred);
    if (hit) return hit;
  }
  return null;
};
const findAll = (node, pred, out = []) => {
  if (!node || typeof node !== "object") return out;
  if (pred(node)) out.push(node);
  for (const child of node.children || []) findAll(child, pred, out);
  return out;
};
const hasClass = (cls) => (n) => String(n.className || n.attributes?.class || "").split(" ").includes(cls);

globalThis.document = { activeElement: null, createElement: (tag) => new Node(tag), createElementNS: (_ns, tag) => new Node(tag) };
const location = { pathname: "/explore/messages", hash: "", origin: "http://dash" };
const setUrl = (url) => {
  const at = url.indexOf("#");
  location.pathname = at < 0 ? url : url.slice(0, at);
  location.hash = at < 0 ? "" : url.slice(at);
};
globalThis.window = { location, history: { replaceState: (_s, _t, url) => setUrl(url), pushState: (_s, _t, url) => setUrl(url) } };
globalThis.history = window.history;
let copied = null;
Object.defineProperty(globalThis, "navigator", { value: { clipboard: { writeText: async (text) => { copied = text; } } }, configurable: true });

const { stringsFor } = await import("../src/dashboard/web/home-strings.mjs");
const { mountExplore } = await import("../src/dashboard/web/explore.mjs");
const S = stringsFor("en");
const el = (tag, cls, text) => { const n = new Node(tag); if (cls) n.className = cls; if (text != null) n.textContent = text; return n; };

const reads = [];
let answer = null;
let gate = null;
const deps = {
  S,
  el,
  icon: (name) => el("span", `icon i-${name}`),
  button: (label, onClick, cls) => { const b = el("button", cls, label); b.addEventListener("click", onClick); return b; },
  logo: () => null,
  hostBadge: (alias) => el("span", null, alias),
  when: () => "",
  read: async (path) => {
    reads.push(path);
    if (path.startsWith("/api/panes/")) return { text: "" };
    if (gate) await gate;
    return answer(path);
  },
  write: async () => ({}),
  authHeaders: () => ({}),
  canWrite: () => true,
  access: () => ({ mode: "token", login: { available: true, signed_in: false } }),
  fleetSettled: () => true,
};
globalThis.fetch = async () => ({ status: 404, body: null });

const room = { host: "tmux", socket: "/tmp/tmux-1/default", window_id: "@1" };
const agent = (pane, alias) => ({ agent_kind: "codex", agent_session_id: `s-${pane}`, alias, pane, socket: "/tmp/tmux-1/default", room, tmux_session_name: "tend", window_name: "work" });
const request = (id, extra = {}) => ({
  id,
  from: agent("%1", "impl"),
  to: agent("%2", "review"),
  kind: "review",
  status: "completed",
  created_at: "2026-11-01T00:00:00Z",
  thread_id: "ENG-1/review",
  body: `secret body ${id}`,
  reply: { at: "2026-11-01T00:00:30Z", status: "completed", body: `secret reply ${id}`, artifacts: ["a.txt"] },
  updates: [{ author: agent("%2", "review"), at: "2026-11-01T00:00:10Z", body: `secret update ${id}` }],
  ...extra,
});
const payload = (requests, details = true, more = {}) => ({ requests, details_included: details, pagination: { total: requests.length, limit: 500, has_more: false, ...more } });
const fleet = { hosts: [
  { alias: "mba", local: true, state: "online", remote: { agents: [], panes: [
    { pane_id: "%1", socket: "/tmp/tmux-1/default", session: "tend", window_index: "0", window_name: "work", current_command: "zsh" },
  ] } },
  { alias: "mini", local: false, state: "online", remote: { agents: [], panes: [] } },
] };
const settle = () => new Promise((r) => setTimeout(r, 15));
const lastQuery = () => new URLSearchParams(reads.filter((p) => p.startsWith("/api/collaboration")).at(-1).split("?")[1]);

function mount(url) {
  setUrl(url);
  reads.length = 0;
  const root = new Node("div");
  const view = mountExplore(root, deps);
  return { root, view };
}

test("the reader loads the last week, lists messages and opens one with its updates, reply and artifacts", async () => {
  answer = () => payload([request("req_1"), request("req_2", { created_at: "2026-11-01T01:00:00Z", status: "queued", reply: null })]);
  const { root, view } = mount("/explore/messages");
  view.update(fleet);
  await settle();
  const query = lastQuery();
  assert.equal(query.get("since"), "7d");
  assert.equal(query.get("limit"), "500");
  const rows = findAll(root, hasClass("mx-row"));
  assert.deepEqual(rows.map((r) => r.dataset.id), ["req_2", "req_1"], "newest first");
  rows[1].click();
  assert.match(location.hash, /open=req_1/);
  const detail = find(root, hasClass("mx-detail"));
  for (const word of ["secret body req_1", "secret update req_1", "secret reply req_1", "a.txt"]) assert.ok(detail.text.includes(word), word);
  // Copy link names this host, so another daemon's req_1 is never it.
  find(detail, (n) => n.tag === "button" && n.textContent === S.copyLink).click();
  await settle();
  assert.equal(copied, "http://dash/explore/messages#message=req_1&host=mba");
  // A participant whose pane is live here opens it in Explore.
  const open = find(detail, (n) => n.tag === "a" && n.textContent === S.openPane);
  assert.equal(open.href, "/explore#pane=%251&socket=%2Ftmp%2Ftmux-1%2Fdefault");
  view.destroy();
});

test("without operator credentials no body is shown, only the sign-in", async () => {
  answer = () => payload([{ ...request("req_1"), body: undefined, reply: { at: "2026-11-01T00:00:30Z", status: "completed" }, updates: [] }], false);
  const { root, view } = mount("/explore/messages#open=req_1");
  view.update(fleet);
  await settle();
  assert.ok(!root.text.includes("secret"));
  assert.ok(root.text.includes(S.previewLocked));
  assert.ok(root.text.includes(S.detailsLocked));
  const signIn = find(root, (n) => n.tag === "a" && n.textContent === S.signIn);
  assert.match(signIn.href, /^\/auth\/login\?return_to=%2Fexplore%2Fmessages/);
  view.destroy();
});

test("filters and search ask the server again and land in the address", async () => {
  answer = () => payload([request("req_1")]);
  const { root, view } = mount("/explore/messages");
  view.update(fleet);
  await settle();
  const kind = findAll(root, (n) => n.tag === "select")[2];
  kind.value = "task";
  kind.fire("change");
  await settle();
  assert.equal(lastQuery().get("kind"), "task");
  assert.match(location.hash, /kind=task/);
  const range = findAll(root, (n) => n.tag === "select")[0];
  range.value = "custom";
  range.fire("change");
  await settle();
  assert.equal(lastQuery().get("kind"), "task", "an unfinished custom period asks nothing");
  assert.ok(root.text.includes(S.periodMissing));
  view.destroy();
});

test("a thread link opens its whole retained history, outside the scope", async () => {
  answer = () => payload([request("req_1")]);
  const { root, view } = mount("/explore/messages#session=tend&socket=%2Ftmp%2Ftmux-1%2Fdefault&pane=%251&open=req_1");
  view.update(fleet);
  await settle();
  assert.equal(lastQuery().get("session"), "tend");
  assert.ok(root.text.includes(`${S.scopedTo} `));
  find(root, (n) => n.tag === "button" && n.textContent === "ENG-1/review").click();
  await settle();
  const query = lastQuery();
  assert.equal(query.get("thread"), "ENG-1/review");
  assert.equal(query.get("since"), "all");
  assert.equal(query.get("session"), null);
  assert.equal(location.hash, "#thread=ENG-1%2Freview");
  view.destroy();
});

test("another host's message is never looked up here", async () => {
  answer = () => payload([request("req_1")]);
  const { root, view } = mount("/explore/messages#message=req_1&host=mini");
  view.update(fleet);
  await settle();
  assert.equal(reads.filter((p) => p.startsWith("/api/collaboration")).length, 0);
  assert.ok(root.text.includes(S.messagesRemote({ host: "mini" })));
  assert.ok(!root.text.includes("secret"));
  view.destroy();
});

test("losing access clears the bodies; a late answer after destroy draws nothing", async () => {
  answer = () => payload([request("req_1")]);
  const { root, view } = mount("/explore/messages#open=req_1");
  view.update(fleet);
  await settle();
  assert.ok(root.text.includes("secret body req_1"));
  answer = () => { throw Object.assign(new Error("signed in without operator access"), { status: 403, auth: true }); };
  find(root, (n) => n.tag === "button" && n.textContent === S.refresh).click();
  await settle();
  assert.ok(!root.text.includes("secret"), "nothing private stays");
  assert.ok(root.text.includes(S.messagesOperatorOnly));
  // A request in flight when the shell drops Explore.
  answer = () => payload([request("req_9")]);
  let release;
  gate = new Promise((r) => (release = r));
  find(root, (n) => n.tag === "button" && n.textContent === S.refresh).click();
  view.destroy();
  release();
  gate = null;
  await settle();
  assert.equal(root.children.length, 0);
  assert.ok(!root.text.includes("req_9"));
});

test("a credential that narrows takes the bodies with it at once", async () => {
  answer = () => payload([request("req_1")]);
  let authorized = true;
  const before = deps.access;
  deps.access = () => ({ mode: "token", write_authorized: authorized, login: {} });
  const { root, view } = mount("/explore/messages#open=req_1");
  view.update(fleet);
  await settle();
  assert.ok(root.text.includes("secret body req_1"));
  authorized = false;
  let release;
  gate = new Promise((r) => (release = r));
  view.update(fleet);
  assert.ok(!root.text.includes("secret"), "gone before the next answer");
  answer = () => payload([{ ...request("req_1"), body: undefined, reply: null, updates: [] }], false);
  release();
  gate = null;
  await settle();
  assert.ok(!root.text.includes("secret"));
  assert.ok(root.text.includes(S.detailsLocked));
  deps.access = before;
  view.destroy();
});

test("a downgrade while the privileged answer is still on its way never shows it", async () => {
  let authorized = true;
  const before = deps.access;
  deps.access = () => ({ mode: "token", write_authorized: authorized, login: {} });
  // The first answer (read with operator access) is held back.
  let release;
  gate = new Promise((r) => (release = r));
  answer = () => payload([request("req_1")]);
  const { root, view } = mount("/explore/messages#open=req_1");
  view.update(fleet);
  const issued = () => reads.filter((p) => p.startsWith("/api/collaboration")).length;
  assert.equal(issued(), 1);
  // Access narrows while it is pending; the same query is asked again.
  authorized = false;
  view.update(fleet);
  assert.equal(issued(), 2, "the credential's own request, not a wait for the old one");
  // Both answers land: the old one carries bodies, the new one does not.
  let calls = 0;
  answer = () => (calls++ === 0
    ? payload([request("req_1")])
    : payload([{ ...request("req_1"), body: undefined, reply: null, updates: [] }], false));
  release();
  gate = null;
  await settle();
  assert.ok(!root.text.includes("secret"), "the privileged answer was dropped");
  assert.ok(root.text.includes(S.detailsLocked));
  deps.access = before;
  view.destroy();
});

for (const [name, pick] of [
  ["search", (root) => find(root, (n) => n.tag === "input" && n.placeholder === S.searchMessages)],
  ["Work", (root) => find(root, (n) => n.tag === "input" && n.attributes.list === "mx-work-options")],
  ["thread", (root) => find(root, (n) => n.tag === "input" && n.attributes.list === "mx-thread-options")],
]) {
  test(`a pending ${name} edit does not bring the address back to Messages`, async () => {
    answer = () => payload([request("req_1")]);
    // Leaving for Panes through the switch.
    let { root, view } = mount("/explore/messages");
    view.update(fleet);
    await settle();
    let input = pick(root);
    input.value = "typed";
    input.fire("input");
    find(root, (n) => n.dataset?.mode === "panes").click();
    await new Promise((r) => setTimeout(r, 400));
    assert.equal(location.pathname, "/explore");
    assert.equal(location.hash, "");
    view.update(fleet);
    assert.equal(location.pathname, "/explore", "a later poll stays on Panes");
    assert.ok(find(root, hasClass("ex-list")));
    view.destroy();
    // Leaving Explore for another page of the shell.
    ({ root, view } = mount("/explore/messages"));
    view.update(fleet);
    await settle();
    input = pick(root);
    input.value = "typed";
    input.fire("input");
    setUrl("/settings");
    view.suspend();
    await new Promise((r) => setTimeout(r, 400));
    assert.equal(location.pathname, "/settings");
    view.destroy();
  });
}

test("Panes | Messages switches views, and a pane opens Messages scoped to it", async () => {
  answer = () => payload([request("req_1")]);
  const { root, view } = mount("/explore#pane=%251&socket=%2Ftmp%2Ftmux-1%2Fdefault");
  view.update(fleet);
  await settle();
  assert.ok(find(root, hasClass("ex-output")), "the pane is open");
  find(root, (n) => n.tag === "button" && n.textContent === S.paneMessages && n.className.includes("small")).click();
  await settle();
  assert.equal(location.pathname, "/explore/messages");
  assert.equal(location.hash, "#session=tend&socket=%2Ftmp%2Ftmux-1%2Fdefault&pane=%251");
  assert.ok(find(root, hasClass("mx-main")));
  assert.equal(lastQuery().get("session"), "tend");
  // Graph: drawn with SVG nodes; an edge narrows the sequence to it.
  find(root, (n) => n.dataset?.tab === "graph").click();
  const edge = find(root, hasClass("mx-edge-group"));
  assert.ok(edge, "an edge is drawn");
  edge.fire("click");
  assert.ok(find(root, (n) => n.tag === "button" && n.textContent === S.showAllMessages && !n.hidden));
  // Back to Panes through the switch.
  find(root, (n) => n.dataset?.mode === "panes").click();
  assert.equal(location.pathname, "/explore");
  assert.ok(find(root, hasClass("ex-list")));
  view.destroy();
});
