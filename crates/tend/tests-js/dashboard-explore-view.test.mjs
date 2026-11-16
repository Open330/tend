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
  S: new Proxy({ stateLabels: {}, statusGroups: {} }, { get: (t, k) => (k in t ? t[k] : String(k)) }),
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
const fleet = { hosts: [{ alias: "mba", local: true, state: "online", remote: { agents: [], panes: [
  { pane_id: "%1", socket: "s", session: "a", window_index: "0", window_name: "one", current_command: "zsh" },
  { pane_id: "%2", socket: "s", session: "a", window_index: "1", window_name: "two", current_command: "zsh" },
] } }] };
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

test("destroy leaves nothing private behind", () => {
  view.destroy();
  assert.equal(root.children.length, 0);
  view.update(fleet);
  assert.equal(root.children.length, 0, "no later update draws it back");
});
