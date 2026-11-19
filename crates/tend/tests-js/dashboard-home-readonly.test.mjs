// The shell for someone who is not an operator, under a minimal fake DOM:
// a visitor whose reads answer 401 goes to /welcome with a clean return
// address; one who may read (public_read, auth off, a viewer) gets Explore
// read-only, without ever asking for pane output, and Home's operator
// notice with Upgrade with token; coming down from an operator's view,
// nothing private stays.
import test from "node:test";
import assert from "node:assert/strict";

class Node {
  constructor(tag) {
    Object.assign(this, { tag, children: [], listeners: {}, dataset: {}, attributes: {}, hidden: false, textContent: "", className: "", value: "", disabled: false, scrollTop: 0, scrollHeight: 0, clientHeight: 0 });
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
  append(...nodes) { this.children.push(...nodes.filter(Boolean)); }
  replaceChildren(...nodes) { this.children = nodes.filter(Boolean); }
  addEventListener(type, fn) { (this.listeners[type] ||= []).push(fn); }
  setAttribute(name, value) { this.attributes[name] = value; }
  getAttribute(name) { return this.attributes[name]; }
  removeAttribute(name) { delete this.attributes[name]; }
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
const $ = (id) => {
  if (!nodes.has(id)) nodes.set(id, new Node("div"));
  return nodes.get(id);
};
const rail = ["home", "explore", "settings"].map((route) => {
  const node = $(`rail-${route}`);
  node.dataset.route = route;
  return node;
});
const dummy = new Node("div");
globalThis.document = {
  hidden: false,
  activeElement: null,
  documentElement: new Node("html"),
  title: "",
  getElementById: $,
  createElement: (tag) => new Node(tag),
  querySelector: (selector) => (selector === ".statusbar" ? dummy : null),
  querySelectorAll: (selector) => (selector === ".rail-item[data-route]" ? rail : []),
  addEventListener() {},
};
const replaced = [];
let reloads = 0;
globalThis.window = {
  location: {
    pathname: "/",
    search: "",
    hash: "#token=t0ken&item=req_9",
    origin: "https://tend.example",
    replace: (url) => replaced.push(url),
    reload: () => (reloads += 1),
  },
};
const setAddress = (url) => {
  const at = new URL(url, "https://tend.example");
  Object.assign(window.location, { pathname: at.pathname, search: at.search, hash: at.hash });
};
globalThis.history = { replaceState: (_s, _t, url) => setAddress(url), pushState: (_s, _t, url) => setAddress(url) };
const stored = new Map();
globalThis.localStorage = { getItem: (k) => stored.get(k) ?? null, setItem: (k, v) => stored.set(k, v), removeItem: (k) => stored.delete(k) };
globalThis.setInterval = () => 0;
globalThis.clearInterval = () => {};
Object.defineProperty(globalThis, "navigator", { value: { languages: ["en-US"] }, configurable: true });

const recent = new Date(Date.now() - 60_000).toISOString();
const fleet = { hosts: [{ alias: "mba", local: true, state: "online", remote: {
  agents: [{ agent_session_id: "w1", kind: "codex", pane: "%1", socket: "s", state: "working", last_prompt: "port the board", state_entered_at: recent }],
  panes: [{ pane_id: "%1", socket: "s", session: "a", window_index: "0", window_name: "one", current_command: "codex" }],
} }] };
const queue = { hosts: [{ alias: "mba", local: true, control: true, decides_reviews: true, queue: { items: [
  { kind: "human_request", request_id: "req_9", body: "Ship it?", created_at: recent, from_alias: "impl" },
] } }] };
const ACCESS = {
  operator: { mode: "token", write_available: true, write_authorized: true, capabilities: {}, login: { available: true, signed_in: true, role: "operator", email: "op@x" } },
  publicRead: { mode: "public_read", write_available: true, write_authorized: false, capabilities: {}, login: { available: false, signed_in: false, role: "none" } },
  viewer: { mode: "token", write_available: true, write_authorized: false, capabilities: {}, login: { available: true, signed_in: true, role: "viewer", email: "viewer@x", enrollment: true } },
  viewerNoEnroll: { mode: "token", write_available: true, write_authorized: false, capabilities: {}, login: { available: true, signed_in: true, role: "viewer", email: "viewer@x", enrollment: false } },
  noAuth: { mode: "read_only", write_available: false, write_authorized: false, capabilities: {}, login: { available: false, signed_in: false, role: "none" } },
};
// Who is asking: each principal's answers, as the server's auth layers give them.
let principal = "anonymous";
const answers = {
  // auth = "token", nobody signed in: every read is 401.
  anonymous: { access: [401], fleet: [401], decisions: [401], home: [401], history: [401] },
  publicRead: { access: [200, ACCESS.publicRead], fleet: [200, fleet], decisions: [401], home: [401], history: [401] },
  viewer: { access: [200, ACCESS.viewer], fleet: [200, fleet], decisions: [403], home: [403], history: [403] },
  viewerNoEnroll: { access: [200, ACCESS.viewerNoEnroll], fleet: [200, fleet], decisions: [403], home: [403], history: [403] },
  noAuth: { access: [200, ACCESS.noAuth], fleet: [200, fleet], decisions: [403], home: [403], history: [403] },
  operator: { access: [200, ACCESS.operator], fleet: [200, fleet], decisions: [200, queue], home: [404], history: [200, { hosts: [] }] },
};
const fetched = [];
const posts = [];
let enrollAnswer = [200, { redirect: "/home" }];
globalThis.fetch = async (path, init = {}) => {
  fetched.push({ path, init });
  if (init.method === "POST") {
    posts.push({ path, init });
    const [code, body] = path === "/auth/enroll" ? enrollAnswer : [200, {}];
    return { ok: code === 200, status: code, json: async () => body };
  }
  const name = path.includes("/api/home") ? "home" : path.includes("history") ? "history" : path.includes("decisions") ? "decisions" : path.includes("fleet") ? "fleet" : path.includes("access") ? "access" : "other";
  // A token this browser holds makes the anonymous public_read visitor an operator.
  const who = principal === "publicRead" && init.headers?.Authorization === "Bearer good" ? "operator" : principal;
  const [code, body] = answers[who][name] || [404];
  return { ok: code === 200, status: code, body: null, json: async () => body };
};

const settle = () => new Promise((resolve) => setTimeout(resolve, 20));
const home = await import("../src/dashboard/web/home.mjs");
await settle();
const notice = () => $("operator-notice");
const exploreView = () => $("view-explore");

test("a visitor whose reads answer 401 at / goes to /welcome, with a return address and no token", () => {
  assert.equal(stored.get("tend.token"), "t0ken", "the link's token was kept first");
  assert.equal(replaced.length, 1);
  assert.equal(replaced[0], `/welcome?return_to=${encodeURIComponent("/home#item=req_9")}`);
  assert.doesNotMatch(decodeURIComponent(replaced[0]), /token/);
  assert.equal($("lanes").children.length, 0);
});

test("the return address keeps what each page names, never a credential", () => {
  assert.equal(home.welcomeHref("/", ""), `/welcome?return_to=${encodeURIComponent("/home")}`);
  assert.equal(home.shellReturnPath("/explore", "#pane=%251&socket=s&token=x"), "/explore#pane=%251&socket=s");
  assert.equal(home.shellReturnPath("/explore", "#pane=%252&host=mini"), "/explore#pane=%252&host=mini");
  assert.equal(home.shellReturnPath("/explore", "#token=x"), "/explore");
  // Explore's whole selection survives (session and window detail, the
  // workspace filter), in the link's order; nothing else, never a token.
  assert.equal(
    home.shellReturnPath("/explore", "#session=api&window=%401&socket=s&host=mini&token=x&workspace=tend&wsocket=w&pane=%253&junk=1"),
    "/explore#session=api&window=%401&socket=s&host=mini&workspace=tend&wsocket=w&pane=%253",
  );
  // What the link adapter (legacy-link.mjs) maps an old link to is what a
  // 401 keeps for after sign-in, without the link's credential.
  for (const old of ["#window=%403&session=api&socket=s&token=x", "#session=api&socket=s&workspace=tend&wsocket=w", "#pane=%251&socket=s&workspace=tend"]) {
    const mapped = new URL(home.legacyLink("/", "", old).href, "http://x");
    assert.equal(mapped.pathname, "/explore");
    assert.equal(home.shellReturnPath("/explore", mapped.hash), `/explore${mapped.hash}`, old);
    assert.doesNotMatch(home.welcomeHref("/explore", mapped.hash), /token/);
  }
  assert.equal(home.welcomeHref("/explore", "#workspace=tend&token=x"), `/welcome?return_to=${encodeURIComponent("/explore#workspace=tend")}`);
  assert.equal(home.shellReturnPath("/explore/messages", "#message=r1&token=x"), "/explore/messages#message=r1");
  assert.equal(home.shellReturnPath("/settings", "#token=x"), "/settings");
  assert.equal(home.shellReturnPath("/home", "#item=a&token=x"), "/home#item=a");
});

test("public_read, anonymous: the shell read-only, Home's operator notice with the upgrade", async () => {
  stored.clear();
  principal = "publicRead";
  replaced.length = 0;
  await home.refresh();
  assert.equal(replaced.length, 0, "not sent to /welcome: this visitor may read");
  assert.equal(notice().hidden, false);
  assert.match(notice().text, /Home needs an operator/);
  assert.match(notice().text, /may look but not act/);
  assert.ok(find(notice(), (n) => n.tag === "form"), "Upgrade with token, in the page");
  assert.equal(find(notice(), (n) => n.tag === "a" && /Sign in/.test(n.text)), null, "no sign-in without a provider");
  assert.equal($("lanes").children.length, 0, "no board");
  assert.equal($("lanes").hidden, true);
  assert.equal($("start-work").hidden, true);
  assert.equal($("segments").hidden, true);
  assert.equal($("rail-explore").hidden, false, "Explore is offered");
  assert.equal($("rail-settings").hidden, false);
  assert.match($("hosts-list").text, /mba/, "the Fleet's hosts");
});

test("public_read, anonymous: Explore lists the panes and their state, and never asks for output", async () => {
  fetched.length = 0;
  rail[1].click();
  await settle();
  assert.equal(window.location.pathname, "/explore");
  const row = find(exploreView(), (n) => n.dataset?.id === "mba:s:%1");
  assert.ok(row, "the pane is listed");
  row.click();
  await home.refresh();
  assert.match(exploreView().text, /Pane output is for operators\./);
  assert.equal(find(exploreView(), (n) => n.tag === "textarea"), null, "no composer");
  assert.equal(find(exploreView(), (n) => n.tag === "button" && /Interrupt/.test(n.text)), null, "no abort");
  const asked = fetched.map((f) => f.path);
  assert.ok(asked.some((p) => p === "/api/fleet"), "the topology from /api/fleet");
  assert.equal(asked.filter((p) => p.includes("/output")).length, 0, "no pane output read or stream");
  assert.equal(asked.filter((p) => p === "/api/home" || p.startsWith("/api/decisions")).length, 0, "no operator reads while read-only");
  assert.equal(posts.length, 0);
});

test("a wrong token stays out of storage; the right one makes this browser an operator", async () => {
  rail[0].click();
  await settle();
  let form = find(notice(), (n) => n.tag === "form");
  let input = find(form, (n) => n.tag === "input");
  assert.equal(input.type, "password");
  input.value = "wrong";
  form.fire("submit");
  await settle();
  assert.equal(stored.has("tend.token"), false, "a token that does not upgrade is not kept");
  assert.match(notice().text, /does not make this browser an operator/);
  form = find(notice(), (n) => n.tag === "form");
  input = find(form, (n) => n.tag === "input");
  input.value = "good";
  form.fire("submit");
  await settle();
  await home.refresh();
  assert.equal(stored.get("tend.token"), "good");
  assert.equal(notice().hidden, true, "the notice goes");
  assert.match($("lanes").text, /Ship it\?/, "the operator's board");
  assert.equal(posts.filter((p) => p.path === "/auth/enroll").length, 0, "no account to enroll");
});

test("Settings forgets the token this browser keeps, as the workbench's lock edit did", async () => {
  assert.equal(stored.get("tend.token"), "good");
  rail[2].click();
  await settle();
  const forget = find($("view-settings"), (n) => n.tag === "button" && /Forget token/.test(n.text));
  assert.ok(forget, "offered while a token is kept");
  const before = reloads;
  forget.click();
  assert.equal(stored.has("tend.token"), false);
  assert.equal(reloads, before + 1, "starts over without it");
  reloads = before;
  stored.set("tend.token", "good");
  rail[0].click();
  await settle();
});

test("an operator coming down to a viewer keeps nothing private, and sees the notice and the upgrade", async () => {
  stored.clear();
  principal = "operator";
  await home.refresh();
  assert.match($("lanes").text, /Ship it\?/);
  principal = "viewer";
  await home.refresh();
  assert.equal($("lanes").children.length, 0, "the board went");
  assert.doesNotMatch($("lanes").text, /Ship it/);
  assert.equal($("review-list").children.length, 0);
  assert.equal($("start-work").hidden, true);
  assert.equal(replaced.length, 0, "a viewer is not sent to /welcome");
  assert.match(notice().text, /Signed in as viewer@x, view only/);
  assert.ok(find(notice(), (n) => n.tag === "button" && n.text.includes("Sign out")));
  const form = find(notice(), (n) => n.tag === "form");
  assert.match(form.text, /Upgrade with token/);
  assert.match(form.text, /Makes this account an operator/);
  find(form, (n) => n.tag === "input").value = " t0ken ";
  form.fire("submit");
  await settle();
  const enroll = posts.find((p) => p.path === "/auth/enroll");
  assert.ok(enroll, "POST /auth/enroll");
  assert.equal(enroll.init.headers["X-Tend-Operator"], "1");
  assert.equal(enroll.init.headers.Authorization, undefined, "the token goes in the body, once");
  assert.deepEqual(JSON.parse(enroll.init.body), { token: "t0ken" });
  assert.equal(reloads, 1, "the operator session takes over on reload");
  assert.equal(stored.has("tend.token"), false, "nothing stored for an account");
  enrollAnswer = [401, { error: "invalid token", restart: false }];
  find(notice(), (n) => n.tag === "input").value = "nope";
  find(notice(), (n) => n.tag === "form").fire("submit");
  await settle();
  assert.match(notice().text, /invalid token/);
  assert.equal(reloads, 1);
});

test("Settings shows who this is, sign-out and the upgrade", async () => {
  rail[2].click();
  await settle();
  const text = $("view-settings").text;
  assert.match(text, /viewer@x \(viewer\)/);
  assert.match(text, /Sign out/);
  assert.match(text, /Upgrade with token/);
  // A poll does not redraw the field someone types in.
  const input = find($("view-settings"), (n) => n.tag === "input");
  input.value = "half a tok";
  await home.refresh();
  assert.equal(find($("view-settings"), (n) => n.tag === "input"), input);
  assert.equal(input.value, "half a tok");
  rail[0].click();
  await settle();
});

test("a viewer on a dashboard that does not enroll accounts gets no upgrade it could not use", async () => {
  principal = "viewerNoEnroll";
  await home.refresh();
  assert.match(notice().text, /does not enroll accounts/);
  assert.equal(find(notice(), (n) => n.tag === "form"), null, "no form: /auth/enroll would refuse it");
  assert.ok(find(notice(), (n) => n.tag === "button" && n.text.includes("Sign out")));
  principal = "viewer";
  await home.refresh();
  assert.ok(find(notice(), (n) => n.tag === "form"), "enrollment back on: the notice redraws with the form");
});

test("auth off: read-only with no upgrade, since there is no token", async () => {
  principal = "noAuth";
  await home.refresh();
  assert.match(notice().text, /no token \(auth = "none"\)/);
  assert.equal(find(notice(), (n) => n.tag === "form"), null);
  assert.equal(replaced.length, 0);
});

test("a viewer whose session ends goes to /welcome from the page they were on", async () => {
  principal = "viewer";
  await home.refresh();
  rail[1].click();
  await settle();
  find(exploreView(), (n) => n.dataset?.id === "mba:s:%1").click();
  principal = "anonymous";
  await home.refresh();
  assert.equal(replaced.at(-1), `/welcome?return_to=${encodeURIComponent("/explore#pane=%251&socket=s")}`);
  assert.equal(exploreView().children.length, 0, "Explore went with it");
  assert.equal($("hosts-list").children.length, 0, "no host names left");
});
