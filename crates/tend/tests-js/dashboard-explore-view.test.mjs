// Explore's view under a minimal fake DOM: a draft stays with its pane,
// a pending send freezes it, and destroy leaves nothing private behind.
import test from "node:test";
import assert from "node:assert/strict";

class Node {
  constructor(tag) {
    Object.assign(this, { tag, children: [], listeners: {}, dataset: {}, attributes: {}, hidden: false, textContent: "", className: "", value: "", disabled: false, scrollTop: 0, scrollHeight: 0, clientHeight: 0 });
    const node = this;
    this.classList = { add: (c) => (node.className += ` ${c}`), toggle() {}, contains: () => false };
  }
  append(...nodes) { this.children.push(...nodes.filter(Boolean)); }
  replaceChildren(...nodes) { this.children = nodes.filter(Boolean); }
  addEventListener(type, fn) { (this.listeners[type] ||= []).push(fn); }
  setAttribute(name, value) { this.attributes[name] = value; }
  fire(type, event = {}) { for (const fn of this.listeners[type] || []) fn({ preventDefault() {}, stopPropagation() {}, ...event }); }
  click() { this.fire("click"); }
  get text() { return [this.textContent, ...this.children.map((c) => c.text)].join(" "); }
}
const find = (node, pred) => {
  if (pred(node)) return node;
  for (const child of node.children || []) {
    const hit = find(child, pred);
    if (hit) return hit;
  }
  return null;
};
globalThis.document = { activeElement: null, createElement: (tag) => new Node(tag) };
globalThis.window = { location: { pathname: "/explore", hash: "" } };
globalThis.history = { replaceState() {} };

const { mountExplore } = await import("../src/dashboard/web/explore.mjs");
const posts = [];
let gate = null;
const el = (tag, cls, text) => { const n = new Node(tag); if (cls) n.className = cls; if (text != null) n.textContent = text; return n; };
const deps = {
  S: new Proxy(
    {
      stateLabels: {},
      statusGroups: {},
      exploreSorts: { topology: "Topology", recent: "Latest activity", myPrompt: "My latest prompt", agentActivity: "Agent latest update" },
      exploreMenuHelp: (group, order) => `by ${group}, ordered by ${order}`,
      showAllCount: (n) => `Show all ${n}`,
      sessionMeta: (v) => `${v.host} · ${v.windows} windows · ${v.panes} panes`,
      windowMeta: (v) => `${v.host} · ${v.panes} panes`,
      windowCardMeta: (v) => `#${v.index} · ${v.panes} panes`,
      workspaceFilter: (v) => `Workspace ${v.workspace}`,
      noWorkspaceMatch: (v) => `No live session matches workspace ${v.workspace}.`,
    },
    { get: (t, k) => (k in t ? t[k] : String(k)) },
  ),
  el,
  icon: () => el("span"),
  button: (label, onClick, cls) => { const b = el("button", cls, label); b.addEventListener("click", onClick); return b; },
  logo: () => null,
  hostBadge: (alias) => el("span", null, alias),
  when: () => "",
  read: async () => ({ text: "" }),
  write: async (path, body) => { posts.push({ path, body }); if (gate) await gate; return {}; },
  authHeaders: () => ({}),
  canWrite: () => true,
};
// No live stream in a unit test: an unsupported route falls back to polling.
globalThis.fetch = async () => ({ status: 404, body: null });
const fleet = { hosts: [
  { alias: "mba", local: true, state: "online", remote: { agents: [], panes: [
    { pane_id: "%1", socket: "s", session: "a", window_index: "0", window_name: "one", current_command: "zsh" },
    { pane_id: "%2", socket: "s", session: "a", window_index: "1", window_name: "two", current_command: "zsh" },
  ] } },
  { alias: "mini", local: false, state: "online", remote: { agents: [], panes: [
    { pane_id: "%1", socket: "s", session: "a", window_index: "0", window_name: "one", current_command: "zsh" },
  ] } },
] };
const root = new Node("div");
const view = mountExplore(root, deps);
view.update(fleet);
const settle = () => new Promise((r) => setTimeout(r, 10));
const row = (pane) => find(root, (n) => n.dataset?.id === `mba:s:${pane}`);
const composer = () => find(root, (n) => n.tag === "textarea");

test("a draft stays with the pane it was written for", async () => {
  row("%1").click();
  composer().value = "for pane one";
  composer().fire("input");
  row("%2").click();
  assert.equal(composer().value, "", "pane two starts empty");
  composer().fire("keydown", { key: "Enter" });
  await settle();
  assert.equal(posts.length, 0, "nothing sent to pane two");
  row("%1").click();
  assert.equal(composer().value, "for pane one");
});

test("a pending send freezes the field and clears only what it sent", async () => {
  let release;
  gate = new Promise((r) => (release = r));
  composer().fire("keydown", { key: "Enter" });
  await settle();
  assert.equal(posts.at(-1).path, "/api/panes/%251/prompt");
  assert.equal(composer().disabled, true, "frozen while sending");
  release();
  gate = null;
  await settle();
  assert.equal(composer().value, "");
  assert.equal(composer().disabled, false);
});

test("switching panes while a send is pending leaves the new pane usable", async () => {
  row("%1").click();
  composer().value = "again";
  composer().fire("input");
  let release;
  gate = new Promise((r) => (release = r));
  composer().fire("keydown", { key: "Enter" });
  await settle();
  row("%2").click();
  assert.equal(composer().disabled, false, "pane two is not the one sending");
  release();
  gate = null;
  await settle();
  composer().value = "for two";
  composer().fire("input");
  assert.equal(find(root, (n) => n.tag === "button" && n.text.includes("send"))?.disabled, false);
});

test("two panes sending at once: each stays locked until its own answer", async () => {
  let releaseOne;
  let releaseTwo;
  row("%1").click();
  composer().value = "one";
  composer().fire("input");
  gate = new Promise((r) => (releaseOne = r));
  composer().fire("keydown", { key: "Enter" });
  await settle();
  row("%2").click();
  composer().value = "two";
  composer().fire("input");
  gate = new Promise((r) => (releaseTwo = r));
  composer().fire("keydown", { key: "Enter" });
  await settle();
  row("%1").click();
  assert.equal(composer().disabled, true, "pane one is still sending");
  const before = posts.length;
  composer().fire("keydown", { key: "Enter" });
  await settle();
  assert.equal(posts.length, before, "no second copy of pane one's instruction");
  gate = null;
  releaseOne();
  releaseTwo();
  await settle();
  assert.equal(composer().disabled, false);
  assert.equal(composer().value, "", "pane one's sent text is gone from the field on screen");
  row("%2").click();
  assert.equal(composer().value, "", "and pane two's");
});

test("a pane link names the host: the same pane id on another host opens that one", async () => {
  row("%1").click();
  window.location.hash = "#pane=%251&socket=s&host=mini";
  view.update(fleet);
  assert.ok(find(root, (n) => n.textContent === "mini"), "mini's pane is open");
  assert.equal(composer(), null, "another host's pane has no composer here");
  window.location.hash = "#pane=%251&socket=s";
  view.update(fleet);
  assert.ok(composer(), "back on this host's pane");
  window.location.hash = "";
});

test("destroy leaves nothing private behind", () => {
  view.destroy();
  assert.equal(root.children.length, 0);
  view.update(fleet);
  assert.equal(root.children.length, 0, "no later update draws it back");
});

test("the order is kept per browser under tend.explore.sort, and redraws the list", () => {
  const stored = new Map([["tend.explore.sort", "myPrompt"]]);
  globalThis.localStorage = { getItem: (k) => stored.get(k) ?? null, setItem: (k, v) => stored.set(k, String(v)) };
  const at = (prompt, update) => ({ last_prompt_at: prompt, last_activity_at: update });
  const timed = { hosts: [{ alias: "mba", local: true, state: "online", remote: {
    agents: [
      { agent_session_id: "a", kind: "codex", pane: "%1", state: "idle", ...at("2026-11-19T10:00:00Z", "2026-11-19T10:09:00Z") },
      { agent_session_id: "b", kind: "codex", pane: "%2", state: "idle", ...at("2026-11-19T10:05:00Z", "2026-11-19T10:01:00Z") },
    ],
    panes: [
      { pane_id: "%1", session: "a", window_index: "0", window_name: "one" },
      { pane_id: "%2", session: "a", window_index: "1", window_name: "two" },
    ],
  } }] };
  const box = new Node("div");
  const explore = mountExplore(box, deps);
  explore.update(timed);
  const sort = find(box, (n) => n.tag === "select" && n.className === "ex-sort");
  assert.equal(sort.value, "myPrompt", "the stored order comes back");
  const panes = () => {
    const out = [];
    const walk = (n) => { if (n.dataset?.id) out.push(n.dataset.id.split(":").pop()); for (const c of n.children || []) walk(c); };
    walk(find(box, (n) => n.className === "ex-list"));
    return out;
  };
  assert.deepEqual(panes(), ["%2", "%1"], "my latest prompt: pane two's window first");
  sort.value = "agentActivity";
  sort.fire("change");
  assert.equal(stored.get("tend.explore.sort"), "agentActivity");
  assert.deepEqual(panes(), ["%1", "%2"]);
  stored.set("tend.explore.sort", "bogus");
  const again = new Node("div");
  mountExplore(again, deps).update(timed);
  assert.equal(find(again, (n) => n.className === "ex-sort").value, "topology", "an unknown order is the default");
  globalThis.localStorage = { getItem() { throw new Error("blocked"); }, setItem() { throw new Error("blocked"); } };
  const blocked = new Node("div");
  const view3 = mountExplore(blocked, deps);
  view3.update(timed);
  const select = find(blocked, (n) => n.className === "ex-sort");
  select.value = "recent";
  assert.doesNotThrow(() => select.fire("change"), "storage refused: still sorts");
  delete globalThis.localStorage;
});

test("the empty states: nothing detected, nothing matching, and the count", () => {
  const box = new Node("div");
  const explore = mountExplore(box, deps);
  explore.update({ hosts: [{ alias: "mba", local: true, state: "online", remote: { agents: [], panes: [] } }] });
  assert.match(find(box, (n) => n.className === "ex-list").text, /noPanesDetected/);
  explore.update(fleet);
  const search = find(box, (n) => n.tag === "input");
  search.value = "no such pane";
  search.fire("input");
  const list = find(box, (n) => n.className === "ex-list");
  assert.match(list.text, /noMatchingPanes/);
  assert.equal(find(list, (n) => n.dataset?.id), null, "no rows");
  assert.equal(find(box, (n) => n.className === "ex-count").textContent, "0/3", "shown/all while filtering");
  search.value = "";
  search.fire("input");
  assert.equal(find(box, (n) => n.className === "ex-count").textContent, "3");
});

test("a session or window row opens its detail; the chevron only folds it", () => {
  const box = new Node("div");
  const two = { hosts: [
    { alias: "mba", local: true, state: "online", remote: { agents: [
      { agent_session_id: "a", kind: "codex", pane: "%1", tmux_socket: "s", state: "working", last_response: "Halfway there." },
    ], panes: [
      { pane_id: "%1", socket: "s", session: "a", window_id: "@1", window_index: "0", window_name: "one", current_command: "codex" },
      { pane_id: "%2", socket: "s", session: "a", window_id: "@2", window_index: "1", window_name: "two", current_command: "zsh" },
    ] } },
  ] };
  const replaced = [];
  globalThis.history = { replaceState: (_s, _t, url) => replaced.push(url) };
  const explore = mountExplore(box, deps);
  explore.update(two);
  const list = () => find(box, (n) => n.className === "ex-list");
  const viewer = () => box.children[1];
  const sessionRow = () => find(list(), (n) => n.className.startsWith("ex-session"));
  // The chevron folds the session's windows away, and opens nothing.
  find(sessionRow(), (n) => n.className === "ex-fold").click();
  assert.equal(find(list(), (n) => n.className.startsWith("ex-window")), null, "folded");
  assert.match(viewer().text, /selectPane/, "nothing opened");
  find(sessionRow(), (n) => n.className === "ex-fold").click();
  // The row opens the session's detail, with its address.
  find(sessionRow(), (n) => n.className === "ex-pick").click();
  assert.equal(replaced.at(-1), "/explore#session=a&socket=s");
  assert.match(viewer().text, /mba · 2 windows · 2 panes/);
  assert.match(viewer().text, /Halfway there\./, "a pane card with its latest activity");
  assert.match(sessionRow().className, /selected/);
  // A window card's header opens that window's detail; a pane card opens the pane.
  find(viewer(), (n) => n.className === "ex-window-head").click();
  assert.equal(replaced.at(-1), "/explore#window=%401&session=a&socket=s");
  assert.match(viewer().text, /mba · 1 panes/);
  find(viewer(), (n) => n.className === "ex-card-head").click();
  assert.equal(replaced.at(-1), "/explore#pane=%251&socket=s");
  assert.ok(find(viewer(), (n) => n.tag === "pre"), "the pane's output");
  explore.destroy();
  globalThis.history = { replaceState() {} };
});

test("a full-path pane link opens that server's pane, not one whose socket shares the file name", () => {
  const box = new Node("div");
  const twins = { hosts: [{ alias: "mba", local: true, state: "online", remote: { agents: [], panes: [
    { pane_id: "%1", socket: "/tmp/a/default", session: "tend", window_index: "0", window_name: "on-a", current_command: "zsh" },
    { pane_id: "%1", socket: "/tmp/b/default", session: "tend", window_index: "0", window_name: "on-b", current_command: "zsh" },
  ] } }] };
  for (const [socket, name] of [["/tmp/b/default", "on-b"], ["/tmp/a/default", "on-a"]]) {
    window.location.hash = `#pane=%251&socket=${encodeURIComponent(socket)}`;
    const explore = mountExplore(box, deps);
    explore.update(twins);
    const head = find(box.children[1], (n) => n.className === "ex-pane-head");
    assert.ok(head, "the pane opened");
    assert.match(head.text, new RegExp(`0:${name}`), `the pane on ${socket}`);
    explore.destroy();
  }
  window.location.hash = "";
});

test("a deep link opens a window, falls back to its session, and a workspace filters", () => {
  const box = new Node("div");
  window.location.hash = "#window=%409&session=a&socket=s";
  const explore = mountExplore(box, deps);
  explore.update(fleet);
  assert.match(box.children[1].text, /windowFellBack/, "the window is gone: its session");
  assert.match(box.children[1].text, /mba · 2 windows · 2 panes/);
  window.location.hash = "#workspace=nope&wsocket=s";
  explore.update(fleet);
  assert.match(find(box, (n) => n.className === "ex-list").text, /No live session matches workspace nope\./);
  assert.match(find(box, (n) => n.className === "ex-scope").text, /Workspace nope/);
  window.location.hash = "#workspace=a&wsocket=s";
  explore.update(fleet);
  assert.ok(find(box, (n) => n.dataset?.id === "mba:s:%1"), "the workspace's panes");
  window.location.hash = "";
  explore.destroy();
});
