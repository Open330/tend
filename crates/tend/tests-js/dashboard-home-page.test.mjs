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
  get text() { return [this.textContent, ...this.children.map((c) => (typeof c === "string" ? c : c.text))].join(" "); }
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
const modeButtons = ["digest", "cards", "inbox"].map((mode) => {
  const node = new Node("button");
  node.dataset.mode = mode;
  return node;
});
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
  // Review's mode switch, so a test can pick a mode the way a click does.
  querySelectorAll: (selector) => (selector === "#review-mode button" ? modeButtons : []),
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
  // The words went; tendd records them as the Mac's decision.
  const record = posts.find((p) => p.path === "/api/home/instruction");
  assert.ok(record, "the answer is recorded");
  assert.deepEqual(JSON.parse(record.init.body), {
    id: "work:ws/job", session_id: "first", state_entered_at: "t1", kind: "answer", text: "ANSWER FOR FIRST", question: "Which way?",
  });
  status.home = 404;
  serverBoard = null;
  await home.refresh();
});

test("Review goes through results three ways, with Later and Send Back", async () => {
  const older = new Date(Date.now() - 120_000).toISOString();
  const result = (id, title, at, extra = {}) => ({
    type: "unit", id, lane: "finished", title, repo: "tend", headline: `${title} done.`,
    stages: [{ name: "claude", state: "done" }], shown_time: at, agents: [], helpers: [], host_aliases: ["mba"], agent_kinds: ["claude_code"],
    recent: true, reviewed: false, requested_by_person: true, review_key: `home:${id}`,
    focus: { host: "mba", local: true, pane: id === "agent:mba:a" ? "%7" : "%8", socket: "s", state: "idle", session_id: id.slice(-1), state_entered_at: at, owner: "claude · mba" },
    review: { asked: `Please ${title}`, result: `# Report\n${title} is done. All tests pass.`, result_truncated: false },
    ...extra,
  });
  serverBoard = {
    hosts: [{ alias: "mba", local: true, state: "online" }], decision_hosts: [], backlog: [],
    lanes: { needs_you: { groups: [] }, running: { groups: [] }, finished: { groups: [
      { project: "tend", items: [result("agent:mba:a", "fix toast", recent), result("agent:mba:b", "write docs", older)] },
    ] } },
  };
  status.home = 200;
  await home.refresh();
  const list = document.getElementById("review-list");
  const dialog = document.getElementById("review-dialog");
  dialog.open = true;
  document.getElementById("review-button").click();
  // The digest, the Mac's default: by project, each result in a line.
  assert.match(list.text, /tend 2/);
  assert.match(list.text, /fix toast Report fix toast is done\. All tests pass\./, "the result's digest, Markdown marks dropped");
  assert.match(list.text, /Mark Project Reviewed/);
  // Cards: one at a time, what was asked and what came back.
  modeButtons[1].click();
  assert.equal(stored.get("tend.home.review.mode"), "cards");
  assert.match(list.text, /1 of 2/);
  assert.match(list.text, /fix toast/);
  assert.match(list.text, /Asked Please fix toast/);
  assert.match(list.text, /Agent's report +Report +fix toast is done\. All tests pass\./, "the report as Markdown");
  assert.ok(find(list, (n) => n.tag === "h2" && n.text.includes("Report")), "a heading, not a literal #");
  // H puts it off: the next result comes up, the first goes last.
  dialog.fire("keydown", { key: "h" });
  assert.match(list.text, /write docs/);
  assert.doesNotMatch(list.text, /fix toast/);
  // R opens Send Back: the reply goes to that agent's pane, then is
  // recorded as sent back.
  dialog.fire("keydown", { key: "r" });
  const area = find(list, (n) => n.tag === "textarea");
  assert.ok(area, "Send Back's reply field");
  area.value = "also the README";
  area.fire("input");
  find(list, (n) => n.tag === "button" && n.text.includes("Send Back") && n.className.includes("primary")).click();
  await settle();
  const sent = posts.findLast((p) => p.path.startsWith("/api/panes/"));
  assert.equal(sent.path, "/api/panes/%258/prompt");
  const record = JSON.parse(posts.findLast((p) => p.path === "/api/home/instruction").init.body);
  assert.equal(record.kind, "send_back");
  assert.equal(record.id, "agent:mba:b");
  assert.equal(record.text, "also the README");
  // Inbox: the list and the selected result; J moves, E marks it.
  modeButtons[2].click();
  assert.match(list.text, /Mark All Reviewed…/);
  assert.match(list.text, /J\/K Move/);
  dialog.fire("keydown", { key: "j" });
  dialog.fire("keydown", { key: "e" });
  await settle();
  const mark = JSON.parse(posts.findLast((p) => p.path === "/api/home/reviewed").init.body);
  assert.equal(mark.id, "agent:mba:a", "J moved to the result put off last");
  // Typing in a field is left alone.
  const before = posts.length;
  dialog.fire("keydown", { key: "e", target: { tagName: "TEXTAREA" } });
  await settle();
  assert.equal(posts.length, before);
  dialog.fire("close");
  modeButtons[0].click();
  status.home = 404;
  serverBoard = null;
  await home.refresh();
});

test("the workbench's old links still open the workbench; Home's own do not", () => {
  assert.equal(home.legacyTarget("/", "#pane=%251&socket=s"), "/workbench#pane=%251&socket=s");
  assert.equal(home.legacyTarget("/", "#workspace=tend"), "/workbench#workspace=tend");
  assert.equal(home.legacyTarget("/", "#item=req_9"), null);
  assert.equal(home.legacyTarget("/home", "#pane=%251"), null);
  assert.equal(home.paneLink({ pane: "%1", socket: "s" }), "/explore#pane=%251&socket=s");
});

test("sign-in returns to Home, and to the linked card", () => {
  assert.equal(home.homeReturnPath(""), "/home");
  assert.equal(home.homeReturnPath("#item=a b"), "/home#item=a%20b");
});

test("the report's Markdown becomes nodes, never markup", () => {
  const blocks = home.markdown("# Done\n\nRan **all** tests with `cargo test`.\n- one\n- two <b>x</b>\n```\nlet a = 1;\n```");
  assert.deepEqual(blocks.map((b) => b.tag), ["h2", "p", "ul", "pre"]);
  assert.ok(find(blocks[1], (n) => n.tag === "strong" && n.textContent === "all"));
  assert.ok(find(blocks[1], (n) => n.tag === "code" && n.textContent === "cargo test"));
  assert.equal(blocks[2].children.length, 2);
  assert.ok(blocks[2].children[1].children.includes("two <b>x</b>"), "angle brackets stay text");
  assert.equal(blocks[3].children[0].textContent, "let a = 1;");
});
