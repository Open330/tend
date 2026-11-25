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
const layoutButtons = ["emphasis", "compact"].map((layout) => {
  const node = new Node("button");
  node.dataset.layout = layout;
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
  querySelectorAll: (selector) => (selector === "#review-mode button" ? modeButtons : selector === "#layout-toggle button" ? layoutButtons : []),
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
    const reply = path.includes("close-unanswerable")
      ? { closed: ["rq_a"], skipped: [{ request_id: "rq_b", reason: "no longer unanswerable" }] }
      : path.includes("/keep")
        ? { request_id: "rq_a", episode: "ep_a" }
        : { request_id: "req_9", status: "completed" };
    return { ok: true, status: 200, json: async () => reply };
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

test("a question offers Answer… and Decline… on the card; a reason is optional (apps/shared/Design/CARDS.md)", () => {
  const card = find(lanes(), (n) => n.dataset?.id === "req_9");
  const labels = [];
  const walk = (n) => {
    if (n.tag === "button") labels.push(n.text.trim());
    for (const c of n.children || []) if (typeof c !== "string") walk(c);
  };
  walk(card);
  assert.ok(labels.includes("Answer…") && labels.includes("Decline…"), labels.join(" | "));
  assert.ok(labels.indexOf("Answer…") < labels.indexOf("Decline…"), "Answer… comes first");
  find(card, (n) => n.tag === "button" && n.text.includes("Decline…")).click();
  const fresh = find(lanes(), (n) => n.dataset?.id === "req_9");
  const area = find(fresh, (n) => n.tag === "textarea");
  assert.equal(area.placeholder, "Reason (optional)");
  const decline = find(fresh, (n) => n.tag === "button" && n.text.trim() === "Decline");
  assert.equal(decline.disabled, false, "Decline needs no reason");
  find(fresh, (n) => n.tag === "button" && n.text.includes("Cancel")).click();
  assert.equal(posts.length, 0, "nothing was sent");
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

test("without /api/access (the Windows app), the board alone offers Looks Good and nothing more", async () => {
  serverBoard = {
    hosts: [{ alias: "mba", local: true, state: "online" }],
    agent_count: 1,
    projects: ["tend"],
    decision_hosts: [{ alias: "mba", local: true, control: true, decides_reviews: true }],
    backlog: [],
    lanes: {
      needs_you: { count: 0, groups: [] },
      running: { count: 0, groups: [] },
      finished: { count: 1, groups: [{ project: "tend", system: false, items: [
        { type: "unit", id: "agent:mba:e", lane: "finished", title: "write docs", repo: "tend", headline: "Docs written.",
          review_key: "rk1", stages: [{ name: "claude", state: "done" }], shown_time: recent, agents: [], helpers: [],
          focus: { host: "mba", local: true }, host_aliases: ["mba"], agent_kinds: ["claude_code"], recent: true,
          reviewed: false, requested_by_person: true },
      ] }] },
    },
  };
  const dialog = document.getElementById("review-dialog");
  dialog.open = true;
  // The digest's mark is an icon button named by its title.
  const looksGood = () => find(document.getElementById("review-list"),
    (n) => n.tag === "button" && (/Looks Good/.test(n.text) || n.title === "Looks Good"));
  try {
    status.home = 200;
    status.access = 403;
    await home.refresh();
    document.getElementById("review-button").click();
    const good = looksGood();
    assert.ok(good, "Looks Good is offered from the board read alone");
    assert.equal(document.getElementById("start-work").hidden, true, "Start Work stays hidden");
    assert.equal(document.getElementById("rail-explore").hidden, true, "Explore stays hidden");
    posts.length = 0;
    good.click();
    await settle();
    assert.equal(posts.length, 1);
    assert.equal(posts[0].path, "/api/home/reviewed");
    assert.deepEqual(JSON.parse(posts[0].init.body), { id: "agent:mba:e", review_key: "rk1" });
    // No board (an older daemon, or a credential that may only decide): no mark.
    status.home = 404;
    serverBoard = null;
    await home.refresh();
    assert.equal(looksGood(), null, "no board, no Looks Good");
  } finally {
    dialog.open = false;
    status.access = 200;
    status.home = 404;
    serverBoard = null;
    posts.length = 0;
    await home.refresh();
  }
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

test("one review id on two hosts: a link and a click open the named host's, and decide it there", async () => {
  const review = (host, local) => ({
    type: "decision", id: local ? "review:rv_same" : `review:${host}:rv_same`, host, local, actionable: true, since: recent,
    entry: { kind: "review", id: "rv_same", summary: `result on ${host}`, requested_at: recent, target: { kind: "session", session_id: "s" } },
  });
  serverBoard = {
    hosts: [{ alias: "mba", local: true, state: "online" }, { alias: "mini", local: false, state: "online" }],
    decision_hosts: [
      { alias: "mba", local: true, control: true, decides_reviews: true },
      { alias: "mini", local: false, control: true, decides_reviews: true },
    ],
    backlog: [],
    lanes: {
      needs_you: { count: 2, groups: [{ project: null, system: false, items: [review("mba", true), review("mini", false)] }] },
      running: { groups: [] }, finished: { groups: [] },
    },
  };
  status.home = 200;
  layoutButtons[1].click();
  // Tend for Mac's link names the host that holds the item.
  window.location.hash = "#item=rv_same&host=mini";
  await home.refresh();
  const inspector = () => document.getElementById("inspector");
  assert.match(inspector().text, /result on mini/);
  assert.doesNotMatch(inspector().text, /result on mba/);
  find(inspector(), (n) => n.tag === "button" && n.text.includes("Accept")).click();
  await settle();
  assert.equal(posts.at(-1).path, "/api/decisions/reviews/rv_same");
  assert.equal(JSON.parse(posts.at(-1).init.body).host, "mini", "decided on the host the link named");
  // The other host's row opens that host's item.
  find(lanes(), (n) => n.dataset?.id === "rv_same" && n.dataset?.host === "mba").click();
  assert.match(inspector().text, /result on mba/);
  assert.doesNotMatch(inspector().text, /result on mini/);
  layoutButtons[0].click();
  window.location.hash = "";
  status.home = 404;
  serverBoard = null;
  posts.length = 0;
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
  // recorded as sent back. Home's filters do not narrow Review, nor whom
  // its replies reach.
  document.getElementById("filter-project").fire("change", { target: { value: "elsewhere" } });
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
  // A radio owns its arrows, and a focused button its Return: neither marks.
  const marks = posts.filter((p) => p.path === "/api/home/reviewed").length;
  dialog.fire("keydown", { key: "ArrowRight", target: { tagName: "BUTTON", getAttribute: (name) => (name === "role" ? "radio" : null) } });
  dialog.fire("keydown", { key: "ArrowRight", target: { tagName: "BUTTON", getAttribute: () => null } });
  await settle();
  assert.equal(posts.filter((p) => p.path === "/api/home/reviewed").length, marks, "no mark from a control's own keys");
  document.getElementById("filter-project").fire("change", { target: { value: "" } });
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

test("the workbench's old links open Explore; Home's own stay (legacy-link.mjs has every case)", () => {
  assert.deepEqual(home.legacyLink("/", "", "#pane=%251&socket=s"), { token: null, href: "/explore#pane=%251&socket=s" });
  assert.deepEqual(home.legacyLink("/", "", "#item=req_9"), { token: null, href: "/#item=req_9" });
  assert.deepEqual(home.legacyLink("/home", "", "#pane=%251"), { token: null, href: "/home#pane=%251" });
  assert.equal(home.paneLink({ pane: "%1", socket: "s" }), "/explore#pane=%251&socket=s");
});

test("Steer this Work sends the next instruction to every agent of the Work, operator-only", async () => {
  const agent = (host, local, pane) => ({ host, local, kind: "codex", pane, socket: "s", state: "working", session_id: `${host}${pane}`, owner: `codex · ${host}` });
  const work = (agents) => ({
    type: "unit", id: "work:tend/fix-login", source: { kind: "work", workspace: "tend", work: "fix-login" },
    lane: "running", title: "fix-login", repo: "tend", headline: "Reading the diff",
    stages: [{ name: "Implement", state: "running", kind: "codex" }], shown_time: recent, host_aliases: ["mba"], agent_kinds: ["codex"],
    agents, helpers: [], recent: true, focus: agents[0],
  });
  const boardWith = (unit, decisionHosts = []) => ({
    hosts: [{ alias: "mba", local: true, state: "online" }, { alias: "mini", local: false, state: "online" }],
    decision_hosts: decisionHosts, backlog: [],
    lanes: { needs_you: { groups: [] }, running: { count: 1, groups: [{ project: "tend", items: [unit] }] }, finished: { groups: [] } },
  });
  const inspector = () => document.getElementById("inspector");
  const steerArea = () => find(inspector(), (n) => n.tag === "textarea" && n.dataset?.key === "steer:work:tend/fix-login");
  const steerSend = () => find(inspector(), (n) => n.tag === "button" && /Send to \d/.test(n.text));
  try {
    serverBoard = boardWith(work([agent("mba", true, "%4"), agent("mba", true, "%5")]));
    status.home = 200;
    await home.refresh();
    find(lanes(), (n) => n.dataset?.id === "work:tend/fix-login").click();
    assert.match(inspector().text, /Steer this Work/);
    assert.match(steerSend().text, /Send to 2/, "the Mac's count: the Work's agents");
    assert.equal(steerSend().disabled, true, "nothing to send yet");
    steerArea().value = "  rebase on main, then rerun the tests  ";
    steerArea().fire("input");
    let release;
    postGate = new Promise((resolve) => (release = resolve));
    posts.length = 0;
    steerSend().click();
    await settle();
    assert.equal(posts.length, 1);
    assert.equal(posts[0].path, "/api/work-control/prompt");
    assert.equal(posts[0].init.headers["X-Tend-Operator"], "1");
    assert.deepEqual(JSON.parse(posts[0].init.body), {
      identity: { workspace_id: "tend", work_id: "fix-login" }, text: "rebase on main, then rerun the tests", submit: true,
    });
    assert.equal(steerArea().disabled, true, "frozen while sending");
    release();
    postGate = null;
    await settle();
    assert.equal(steerArea().value, "", "the sent draft clears");
    assert.match(inspector().text, /Sent to/, "the result line");

    // A host in observe mode is never steered, as on the Mac.
    serverBoard = boardWith(work([agent("mba", true, "%4"), agent("mini", false, "%2")]), [
      { alias: "mba", local: true, control: true }, { alias: "mini", local: false, control: false },
    ]);
    await home.refresh();
    assert.match(inspector().text, /registered in observe mode/);
    steerArea().value = "go";
    steerArea().fire("input");
    assert.equal(steerSend().disabled, true);

    // A Work whose agents are all on another host: the route reaches only
    // this host's panes, so nothing can be sent from here.
    serverBoard = boardWith(work([agent("mini", false, "%2"), agent("mini", false, "%3")]), [
      { alias: "mba", local: true, control: true }, { alias: "mini", local: false, control: true },
    ]);
    await home.refresh();
    assert.match(steerSend().text, /Send to 0/);
    steerArea().value = "go";
    steerArea().fire("input");
    assert.equal(steerSend().disabled, true, "no agent here to send to");
    assert.match(inspector().text, /steer the 2 on other hosts from there/);
    posts.length = 0;
    steerSend().click();
    await settle();
    assert.equal(posts.length, 0, "nothing posted");

    // A mixed Work, this host and a control-mode host: the count is this
    // host's agents, the notice names the others, and the send goes through
    // this host's Work control (which reaches only its own panes).
    serverBoard = boardWith(work([agent("mba", true, "%4"), agent("mini", false, "%2")]), [
      { alias: "mba", local: true, control: true }, { alias: "mini", local: false, control: true },
    ]);
    await home.refresh();
    assert.match(steerSend().text, /Send to 1/, "this host's agents only");
    assert.match(inspector().text, /This reaches the agents on this host; steer the 1 on other hosts from there\./);
    assert.doesNotMatch(inspector().text, /observe mode/);
    steerArea().value = "ship it";
    steerArea().fire("input");
    assert.equal(steerSend().disabled, false);
    posts.length = 0;
    steerSend().click();
    await settle();
    assert.equal(posts.length, 1, "one request, to this host's daemon");
    assert.equal(posts[0].path, "/api/work-control/prompt");
    assert.deepEqual(JSON.parse(posts[0].init.body), { identity: { workspace_id: "tend", work_id: "fix-login" }, text: "ship it", submit: true });
    assert.equal(JSON.parse(posts[0].init.body).host, undefined, "never addressed to the other host");

    // Not an operator: no composer.
    status.access = 403;
    await home.refresh();
    assert.equal(steerArea(), null, "Steer is operator-only");
  } finally {
    postGate = null;
    status.access = 200;
    status.home = 404;
    serverBoard = null;
    posts.length = 0;
    await home.refresh();
  }
});

test("sign-in returns to Home, and to the linked card", () => {
  assert.equal(home.homeReturnPath(""), "/home");
  assert.equal(home.homeReturnPath("#item=a b"), "/home#item=a%20b");
  assert.equal(home.homeReturnPath("#item=rv_1&host=mini"), "/home#item=rv_1&host=mini");
  // A collaboration link comes back to the message, never with a token.
  assert.equal(home.homeReturnPath("#message=req_1&host=mba&token=t", "/explore/messages"), "/explore/messages#message=req_1&host=mba");
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

test("consecutive list items share one list, as a browser's elements see it", () => {
  // Browser elements have tagName, not this fake's `tag`: hide it.
  const make = document.createElement;
  document.createElement = (tag) => {
    const node = make(tag);
    Object.defineProperty(node, "tag", { get: () => undefined, configurable: true });
    node.tagName = tag.toUpperCase();
    return node;
  };
  try {
    const blocks = home.markdown("1. First\n2. Second\n\n- a\n- b");
    assert.equal(blocks.length, 2);
    assert.deepEqual(blocks.map((b) => [b.tagName, b.children.length]), [["OL", 2], ["UL", 2]]);
  } finally {
    document.createElement = make;
  }
});

test("questions nobody can answer fold out of Your turn; Keep and Close all send the listed pairs", async () => {
  const old = new Date(Date.now() - 4 * 86400_000).toISOString();
  const since = new Date(Date.now() - 2 * 86400_000).toISOString();
  const gone = (id, cause, episode) => ({
    type: "decision", id: `human:mba:${id}`, host: "mba", local: true, actionable: true, since: old,
    entry: { kind: "human_request", request_id: id, body: `Deploy ${id}?`, created_at: old, from_session_id: "s",
      unanswerable: { cause, episode, since } },
  });
  const board = (closeUnanswerable) => ({
    hosts: [{ alias: "mba", local: true, state: "online" }],
    agent_count: 0,
    projects: [],
    decision_hosts: [{ alias: "mba", local: true, control: true, decides_reviews: true, subject_check: true,
      close_unanswerable: closeUnanswerable, truncated: true, unanswerable_count: 3 }],
    backlog: [],
    lanes: {
      needs_you: { count: 3, groups: [{ project: null, system: false, items: [
        { type: "decision", id: "human:mba:req_9", host: "mba", local: true, actionable: true, since: recent,
          entry: { kind: "human_request", request_id: "req_9", body: "Ship it?", created_at: recent } },
        gone("rq_a", "pane_gone", "ep_a"),
        gone("rq_b", "replaced", "ep_b"),
      ] }] },
      running: { count: 0, groups: [] },
      finished: { count: 0, groups: [] },
    },
  });
  const yourTurn = () => lanes().children.find((n) => n.className.includes("needsYou"));
  serverBoard = board(false);
  status.home = 200;
  await home.refresh();
  let section = yourTurn();
  assert.match(section.text, /Your turn 1 Decide or review/, "the count leaves them out");
  assert.equal(document.getElementById("rail-badge").textContent, "1");
  assert.match(section.text, /Can't be answered \(3\)/, "the exact count, not just the listed two");
  assert.doesNotMatch(section.text, /Deploy rq_a/, "collapsed by default");
  find(section, (n) => n.tag === "button" && n.text.includes("Can't be answered")).click();
  section = yourTurn();
  assert.match(section.text, /Deploy rq_a\?/);
  assert.match(section.text, /Pane gone · since/);
  assert.match(section.text, /Replaced by another agent/);
  assert.match(section.text, /1 more not listed/);
  assert.equal(find(section, (n) => n.tag === "button" && n.text.includes("Close all")), null, "hidden while the switch is off");
  const row = find(section, (n) => n.dataset?.id === "rq_a");
  find(row, (n) => n.tag === "button" && n.text.includes("Keep")).click();
  await settle();
  assert.equal(posts.at(-1).path, "/api/decisions/keep");
  assert.deepEqual(JSON.parse(posts.at(-1).init.body), { request_id: "rq_a", episode: "ep_a" });

  serverBoard = board(true);
  await home.refresh();
  section = yourTurn();
  find(section, (n) => n.tag === "button" && n.text.includes("Close all")).click();
  section = yourTurn();
  assert.match(section.text, /Close 2 questions\? Each asking agent gets a reply that you dismissed it\. This can't be undone\./);
  // That it is final is said in the dialog itself, on its own line, and
  // the button that closes says how many, styled as destructive.
  assert.ok(find(section, (n) => n.tag === "p" && n.className === "undo" && n.text.includes("This can't be undone.")));
  assert.match(find(section, (n) => n.tag === "button" && n.text.includes("Close 2")).className, /danger/);
  const sent = posts.length;
  find(section, (n) => n.tag === "button" && n.text.includes("Close 2")).click();
  await settle();
  assert.equal(posts.length, sent + 1, "one request after one confirmation");
  assert.equal(posts.at(-1).path, "/api/decisions/close-unanswerable");
  assert.deepEqual(JSON.parse(posts.at(-1).init.body), {
    items: [{ request_id: "rq_a", episode: "ep_a" }, { request_id: "rq_b", episode: "ep_b" }],
    by_label: "web",
  });
  assert.match(yourTurn().text, /Closed 1 question\. Skipped 1: rq_b \(no longer unanswerable\)/);
  // Both rows now leave the fold: rq_a was closed; rq_b is live again.
  serverBoard.lanes.needs_you.groups = [];
  serverBoard.decision_hosts[0].unanswerable_count = 0;
  await home.refresh();
  assert.doesNotMatch(yourTurn().text, /Can't be answered/, "the empty group is gone");
  assert.match(yourTurn().text, /Skipped 1: rq_b \(no longer unanswerable\)/,
    "the result must remain visible when the refreshed fold is empty");
  find(yourTurn(), (n) => n.tag === "button" && n.text.includes("Dismiss")).click();
  assert.doesNotMatch(yourTurn().text, /Skipped 1/, "Dismiss clears it");
  status.home = 404;
  serverBoard = null;
  await home.refresh();
});
