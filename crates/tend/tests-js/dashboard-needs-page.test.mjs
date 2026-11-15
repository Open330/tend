// The "Needs you" page module itself, under a minimal fake DOM: the model
// tests cannot catch a missing import or a render path that throws.
import test from "node:test";
import assert from "node:assert/strict";

class Node {
  constructor(tag) {
    this.tag = tag;
    this.children = [];
    this.listeners = {};
    this.dataset = {};
    this.attributes = {};
    this.hidden = false;
    this.textContent = "";
    this.className = "";
  }
  append(...nodes) { this.children.push(...nodes); }
  replaceChildren(...nodes) { this.children = nodes; }
  addEventListener(type, fn) { (this.listeners[type] ||= []).push(fn); }
  setAttribute(name, value) { this.attributes[name] = value; }
  querySelectorAll() { return []; }
  click() { for (const fn of this.listeners.click || []) fn(); }
  get text() { return [this.textContent, ...this.children.map((c) => c.text)].join(" "); }
}

const nodes = new Map();
globalThis.document = {
  hidden: false,
  activeElement: null,
  getElementById(id) {
    if (!nodes.has(id)) nodes.set(id, new Node("div"));
    return nodes.get(id);
  },
  createElement: (tag) => new Node(tag),
  querySelectorAll: () => [],
  addEventListener() {},
};
globalThis.window = { location: { hash: "" } };
globalThis.localStorage = { getItem: () => null };
globalThis.setInterval = () => 0;
const answered = { kind: "human_answer", request_id: "rq_1", body: "Merge?", status: "completed", answer: "yes", answered_at: "2026-11-12T11:00:00Z" };
const recent = new Date(Date.now() - 60_000).toISOString();
const fleet = {
  hosts: [
    { alias: "mba", local: true, state: "online", remote: { agents: [
      { agent_session_id: "w1", kind: "codex", pane: "%4", state: "working", last_prompt: "port the board", state_entered_at: recent },
      { agent_session_id: "i1", kind: "claude_code", pane: "%5", state: "idle", last_prompt: "fix the toast", last_response: "Done: toast fixed", state_entered_at: recent },
    ] } },
  ],
};
let fleetStatus = 200;
globalThis.fetch = async (path) => path.includes("/api/fleet") && fleetStatus !== 200 ? ({
  ok: false,
  status: fleetStatus,
  json: async () => null,
}) : ({
  ok: true,
  status: 200,
  json: async () =>
    path.includes("/api/fleet")
      ? fleet
      : path.includes("history")
      ? { covers: "mba", hosts: [{ alias: "mba", local: true, history: { items: [answered] } }, { alias: "mini", local: false, unavailable: ["host is offline"], history: { items: [] } }] }
      : { hosts: [{ alias: "mba", local: true, control: true, decides_reviews: true, queue: { items: [] } }] },
});

const settle = () => new Promise((resolve) => setTimeout(resolve, 20));

test("the page renders the queue and every host's history", async () => {
  await import("../src/dashboard/web/needs.mjs");
  await settle();
  assert.equal(document.getElementById("status").textContent, "");
  document.getElementById("tab-history").click();
  await settle();
  assert.equal(document.getElementById("status").textContent, "", "no error on the Decided tab");
  const history = document.getElementById("history").text;
  assert.match(history, /Merge\?/);
  assert.match(history, /mini/);
  assert.match(history, /unavailable: host is offline/);
  assert.equal(document.getElementById("covers").textContent, "Covers mba, mini.");
  assert.match(history, /Done: toast fixed/, "an agent that finished is in Finished");
});

test("In progress lists the working agents", async () => {
  document.getElementById("tab-progress").click();
  await settle();
  assert.equal(document.getElementById("status").textContent, "", "no error on the In progress tab");
  assert.equal(document.getElementById("progress").hidden, false);
  assert.equal(document.getElementById("history").hidden, true);
  const progress = document.getElementById("progress").text;
  assert.match(progress, /port the board/);
  assert.doesNotMatch(progress, /fix the toast/);
});

test("a sign-in lost on the Fleet read drops the page like any other read", async () => {
  fleetStatus = 401;
  document.getElementById("tab-history").click();
  await settle();
  assert.equal(document.getElementById("history").children.length, 0, "no private history left");
  assert.equal(document.getElementById("login").hidden, false, "sign-in offered");
  fleetStatus = 200;
});
