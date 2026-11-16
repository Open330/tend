// Home's page module under a minimal fake DOM: it renders the lanes, keeps
// the token out of the address bar, keeps the page on a refused optional
// read, drops everything private on a lost sign-in, and answers through
// the audited decision route with the operator header.
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
    this.value = "";
    this.disabled = false;
    const node = this;
    this.classList = {
      add: (c) => (node.className = `${node.className} ${c}`.trim()),
      toggle: (c, on) => {
        const set = new Set(node.className.split(" ").filter(Boolean));
        if (on ?? !set.has(c)) set.add(c);
        else set.delete(c);
        node.className = [...set].join(" ");
      },
      contains: (c) => node.className.split(" ").includes(c),
    };
  }
  append(...nodes) { this.children.push(...nodes); }
  replaceChildren(...nodes) { this.children = nodes; }
  addEventListener(type, fn) { (this.listeners[type] ||= []).push(fn); }
  setAttribute(name, value) { this.attributes[name] = value; }
  getAttribute(name) { return this.attributes[name]; }
  contains() { return false; }
  closest() { return null; }
  focus() {}
  scrollIntoView() {}
  click() { for (const fn of this.listeners.click || []) fn({ stopPropagation() {}, preventDefault() {} }); }
  fire(type, event = {}) { for (const fn of this.listeners[type] || []) fn({ preventDefault() {}, stopPropagation() {}, ...event }); }
  get text() { return [this.textContent, ...this.children.map((c) => c.text)].join(" "); }
}

function find(node, pred) {
  if (pred(node)) return node;
  for (const child of node.children || []) {
    const hit = find(child, pred);
    if (hit) return hit;
  }
  return null;
}

const nodes = new Map();
const dummy = new Node("div");
globalThis.document = {
  hidden: false,
  activeElement: null,
  documentElement: new Node("html"),
  title: "",
  getElementById(id) {
    if (!nodes.has(id)) nodes.set(id, new Node("div"));
    return nodes.get(id);
  },
  createElement: (tag) => new Node(tag),
  querySelector: (selector) => (selector === ".statusbar" ? dummy : null),
  querySelectorAll: () => [],
  addEventListener() {},
};
let replaced = null;
globalThis.window = { location: { hash: "#token=t0ken&item=req_9", pathname: "/home", search: "" } };
globalThis.history = { replaceState: (_s, _t, url) => { replaced = url; window.location.hash = url.includes("#") ? url.slice(url.indexOf("#")) : ""; } };
const stored = new Map();
globalThis.localStorage = { getItem: (k) => stored.get(k) ?? null, setItem: (k, v) => stored.set(k, v) };
globalThis.setInterval = () => 0;
globalThis.CSS = { escape: (s) => s };
Object.defineProperty(globalThis, "navigator", { value: { languages: ["en-US"] }, configurable: true });

const recent = new Date(Date.now() - 60_000).toISOString();
const queue = {
  hosts: [{ alias: "mba", local: true, control: true, decides_reviews: true, queue: { items: [
    { kind: "human_request", request_id: "req_9", body: "Ship it?", created_at: recent, from_alias: "impl" },
  ] } }],
};
const fleet = { hosts: [{ alias: "mba", local: true, state: "online", remote: { agents: [
  { agent_session_id: "w1", kind: "codex", pane: "%4", state: "working", last_prompt: "port the board", state_entered_at: recent },
] } }] };
// An older daemon (or a proxy that does not forward it) has no board read:
// the page lays out the queue and the Fleet snapshot itself.
const status = { home: 404, decisions: 200, fleet: 200, history: 200, access: 200 };
let serverBoard = null;
const posts = [];
let postGate = null;
globalThis.fetch = async (path, init = {}) => {
  if (init.method === "POST") {
    posts.push({ path, init });
    if (postGate) await postGate;
    return { ok: true, status: 200, json: async () => ({ request_id: "req_9", status: "completed" }) };
  }
  const name = path.includes("/api/home") ? "home" : path.includes("history") ? "history" : path.includes("decisions") ? "decisions" : path.includes("fleet") ? "fleet" : "access";
  const code = status[name];
  const body = { home: serverBoard, decisions: queue, fleet, history: { hosts: [] }, access: { mode: "token", write_authorized: true, capabilities: { work_start: false }, login: { role: "operator" } } }[name];
  return { ok: code === 200, status: code, json: async () => body };
};

const settle = () => new Promise((resolve) => setTimeout(resolve, 20));
const home = await import("../src/dashboard/web/home.mjs");
await settle();
const lanes = () => document.getElementById("lanes");

test("the token moves from the fragment into storage, out of the address bar", () => {
  assert.equal(stored.get("tend.token"), "t0ken");
  assert.equal(replaced, "/home#item=req_9");
});

test("the three lanes render with the Mac's words and counts", () => {
  const sections = lanes().children;
  assert.equal(sections.length, 3);
  assert.match(sections[0].text, /Your turn 1 Decide or review/);
  assert.match(sections[0].text, /Ship it\?/);
  assert.match(sections[1].text, /In progress 1/);
  assert.match(sections[1].text, /port the board/);
  assert.match(sections[2].text, /Nothing finished yet\./);
  assert.equal(document.getElementById("conn-label").textContent, "tendd");
  assert.equal(document.getElementById("rail-badge").textContent, "1");
});

test("answering sends the audited route with the operator header", async () => {
  const card = find(lanes(), (n) => n.dataset?.id === "req_9");
  find(card, (n) => n.tag === "button" && n.text.includes("Answer…")).click();
  const fresh = find(lanes(), (n) => n.dataset?.id === "req_9");
  const area = find(fresh, (n) => n.tag === "textarea");
  assert.ok(area, "the composer opened");
  area.value = "yes, behind a flag";
  area.fire("input");
  find(fresh, (n) => n.tag === "button" && n.text.includes("Send")).click();
  await settle();
  assert.equal(posts.length, 1);
  assert.equal(posts[0].path, "/api/decisions/requests/req_9");
  assert.equal(posts[0].init.headers["X-Tend-Operator"], "1");
  assert.equal(posts[0].init.headers.Authorization, "Bearer t0ken");
  assert.deepEqual(JSON.parse(posts[0].init.body), { answer: "yes, behind a flag" });
});

test("a refused optional read (a proxy or a paired device) keeps the page", async () => {
  status.fleet = 403;
  status.access = 403;
  await home.refresh();
  assert.equal(document.getElementById("login").hidden, true, "no sign-in prompt");
  assert.match(lanes().children[0].text, /Ship it\?/, "Your turn stays");
  assert.match(lanes().children[1].text, /No agent is working\./);
  status.fleet = 200;
  status.access = 200;
});

test("a credential that may only decide (a paired device: 401 on the Fleet read) keeps the queue", async () => {
  status.fleet = 401;
  status.access = 401;
  await home.refresh();
  assert.equal(document.getElementById("login").hidden, true, "not signed out");
  assert.match(lanes().children[0].text, /Ship it\?/);
  assert.doesNotMatch(lanes().text, /port the board/, "nothing from the refused read");
  status.fleet = 200;
  status.access = 200;
  await home.refresh();
});

test("what a narrowed credential may no longer see goes at once, even under a draft", async () => {
  await home.refresh();
  assert.match(lanes().text, /port the board/);
  // Someone is typing in a composer.
  const typing = { value: "half an answer", closest: (sel) => (sel === ".composer" ? {} : null) };
  document.activeElement = typing;
  status.fleet = 403;
  await home.refresh();
  assert.doesNotMatch(lanes().text, /port the board/, "the Fleet data left the page");
  document.activeElement = null;
  status.fleet = 200;
  await home.refresh();
});

test("a lost sign-in drops everything private and offers sign-in back to the card", async () => {
  status.decisions = 401;
  await home.refresh();
  assert.equal(lanes().children.length, 0);
  const login = document.getElementById("login");
  assert.equal(login.hidden, false);
  assert.equal(login.href, `/auth/login?return_to=${encodeURIComponent("/home#item=req_9")}`);
  assert.equal(document.getElementById("hosts-list").children.length, 0, "no host names left");
  assert.equal(document.getElementById("filter-host").children.length, 0, "no filter options left");
  assert.equal(document.getElementById("start-work").hidden, true);
  assert.equal(document.getElementById("review-list").children.length, 0, "no Review contents left");
  assert.equal(document.getElementById("review-button").hidden, true);
  status.decisions = 200;
  await home.refresh();
  assert.equal(login.hidden, true, "back once the queue reads again");
  assert.equal(lanes().children.length, 3);
});

test("the daemon's board, when it has one, is what the page draws", async () => {
  serverBoard = {
    hosts: [{ alias: "mba", local: true, state: "online" }],
    agent_count: 2,
    projects: ["tend"],
    decision_hosts: [{ alias: "mba", local: true, control: true, decides_reviews: true }],
    backlog: [{ workspace: "tend", work: "next-up", title: "next-up", line: "Plan the release", priority: 1 }],
    lanes: {
      needs_you: { count: 1, groups: [{ project: null, system: false, items: [
        { type: "decision", id: "human:mba:req_9", host: "mba", local: true, actionable: true, since: recent,
          entry: { kind: "human_request", request_id: "req_9", body: "Ship it?", created_at: recent } },
      ] }] },
      running: { count: 1, groups: [{ project: "tend", system: false, items: [
        { type: "unit", id: "work:tend/fix-login", lane: "running", title: "fix-login", repo: "tend",
          headline: { key: "waiting_on", owner: "codex · mba", text: "Reading the diff" },
          stages: [{ name: "Implement", state: "done", kind: "claude_code" }, { name: "Review", state: "running", kind: "codex" }],
          shown_time: recent, agents: [], helpers: [], focus: { host: "mba", local: true, kind: "codex", pane: "%4" },
          host_aliases: ["mba"], agent_kinds: ["claude_code", "codex"], recent: true },
      ] }] },
      finished: { count: 2, groups: [{ project: "tend", system: false, items: [
        { type: "unit", id: "agent:mba:d", lane: "finished", title: "fix toast", repo: "tend", headline: "Toast fixed.",
          stages: [{ name: "claude", state: "done" }], shown_time: recent, agents: [], helpers: [], focus: { host: "mba", local: true },
          host_aliases: ["mba"], agent_kinds: ["claude_code"], recent: true, reviewed: true, requested_by_person: true },
        { type: "unit", id: "agent:mba:e", lane: "finished", title: "write docs", repo: "tend", headline: "Docs written.",
          stages: [{ name: "claude", state: "done" }], shown_time: recent, agents: [], helpers: [], focus: { host: "mba", local: true },
          host_aliases: ["mba"], agent_kinds: ["claude_code"], recent: true, reviewed: false, requested_by_person: true },
      ] }] },
    },
  };
  status.home = 200;
  await home.refresh();
  const text = lanes().text;
  assert.match(text, /Backlog 1 Planned, not started/);
  assert.match(text, /fix-login/);
  assert.match(text, /Waiting on codex · mba: Reading the diff/, "a key the daemon sent, worded here");
  assert.match(text, /Implement|Review/);
  assert.match(text, /write docs/);
  assert.doesNotMatch(text, /fix toast/, "a reviewed unit hides");
  assert.match(text, /Show 1 reviewed/);
  assert.equal(document.getElementById("review-count").textContent, "1", "Review counts what a person asked for and nobody marked");
  // A board read that fails now draws what the fallback reads, not the
  // board from before.
  status.home = 503;
  await home.refresh();
  assert.doesNotMatch(lanes().text, /write docs/, "no leftover board");
  assert.match(lanes().text, /Ship it\?/, "the fallback queue");
  status.home = 404;
  serverBoard = null;
  await home.refresh();
});

test("a reply goes only to the agent and question it was written for", async () => {
  const unitAt = (session, since) => ({
    type: "unit", id: "work:ws/job", lane: "needs_you", title: "job", repo: "tend", headline: "Which way?",
    stages: [{ name: "impl", state: "needs_you" }], shown_time: recent, host_aliases: ["mba"], agent_kinds: ["claude_code"],
    agents: [], helpers: [], recent: true,
    focus: { host: "mba", local: true, pane: "%1", socket: "s", state: "waiting_input", session_id: session, state_entered_at: since, owner: `claude · mba` },
  });
  const boardWith = (unit) => ({
    hosts: [{ alias: "mba", local: true, state: "online" }], decision_hosts: [], backlog: [],
    lanes: { needs_you: { count: 1, groups: [{ project: "tend", items: [unit] }] }, running: { groups: [] }, finished: { groups: [] } },
  });
  serverBoard = boardWith(unitAt("first", "t1"));
  status.home = 200;
  await home.refresh();
  find(lanes(), (n) => n.dataset?.id === "work:ws/job").click();
  const inspector = document.getElementById("inspector");
  const area = find(inspector, (n) => n.tag === "textarea");
  assert.ok(area, "a reply field for the waiting agent");
  area.value = "ANSWER FOR FIRST";
  area.fire("input");
  const send = find(inspector, (n) => n.tag === "button" && n.text.includes("Answer"));
  // Another session takes the same pane while the field keeps focus: the
  // page does not redraw under the cursor, but the old Send must not send.
  document.activeElement = { value: area.value, closest: (sel) => (sel === ".composer" ? {} : null) };
  serverBoard = boardWith(unitAt("second", "t2"));
  await home.refresh();
  const before = posts.length;
  send.click();
  await settle();
  assert.equal(posts.length, before, "nothing typed to the replacement session");
  document.activeElement = null;
  // The same agent at the same question: sent, and the draft frozen while
  // the answer is pending.
  serverBoard = boardWith(unitAt("first", "t1"));
  await home.refresh();
  const area2 = find(document.getElementById("inspector"), (n) => n.tag === "textarea");
  assert.equal(area2?.value, "ANSWER FOR FIRST", "the draft kept its own target");
  let release;
  postGate = new Promise((resolve) => (release = resolve));
  find(document.getElementById("inspector"), (n) => n.tag === "button" && n.text.includes("Answer")).click();
  await settle();
  assert.equal(posts.at(-1).path, "/api/panes/%251/prompt");
  assert.equal(find(document.getElementById("inspector"), (n) => n.tag === "textarea")?.disabled, true, "frozen while sending");
  release();
  postGate = null;
  await settle();
  status.home = 404;
  serverBoard = null;
  await home.refresh();
});

test("sign-in returns to Home, and to the linked card", () => {
  assert.equal(home.homeReturnPath(""), "/home");
  assert.equal(home.homeReturnPath("#item=a b"), "/home#item=a%20b");
});
