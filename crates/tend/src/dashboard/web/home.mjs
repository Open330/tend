// Home: Tend for Mac's Home in the browser (and in the Windows app's
// WebView2). Lanes, cards, inspector and words follow FleetHomeView.swift;
// the board itself is home-board.mjs. Reads with the operator's credential:
// the dashboard token (kept in localStorage, or added by a proxy such as the
// Windows app, in which case the page never sees it) or the OIDC operator
// session cookie. Everything an agent wrote is set with textContent only.

import { buildBoard, fromServerBoard, hostHue, LANES, productOf, replyTarget, reviewQueue, sameTarget, TEND_PROJECT } from "./home-board.mjs";
import { languageOf, relativeTime, stringsFor, textOf } from "./home-strings.mjs";
import { linkedItemOf } from "./needs-model.mjs";

const TOKEN_KEY = "tend.token";
/// The card layout, a per-browser preference like the Mac's
/// `tend.home.cardLayout`: "emphasis" (the default) or "compact".
const LAYOUT_KEY = "tend.home.cardLayout";
export const REFRESH_MS = 5000;
const LANG = languageOf(globalThis.navigator?.languages || [globalThis.navigator?.language || "en"]);
const S = stringsFor(LANG);
const $ = (id) => document.getElementById(id);
const tx = (value) => textOf(S, value);
/// Each read gives up after this long, so one slow host cannot hold the
/// board (the next poll starts only when this one ended).
const READ_TIMEOUT_MS = 8000;

const state = {
  data: { decisions: null, history: null, fleet: null, access: null, server: null },
  errors: { decisions: null, fleet: null, history: null },
  connection: "connecting",
  generation: 0,
  lost: false,
  selected: null,
  lane: "needsYou",
  layout: "emphasis",
  filter: { host: "", project: "" },
  showOlder: false,
  /// Per lane, which of the Mac's folds are open (helpers, empty,
  /// reviewed); `showOlder` is the older fold.
  folds: { needsYou: {}, running: {}, finished: {}, reviewed: false },
  reviewScope: "day",
  /// The one open composer, as on the Mac: {key, mode, option}.
  composer: null,
  drafts: new Map(),
  /// Inspector replies by unit: { text, target } (see replyTarget).
  replyDrafts: new Map(),
  busy: new Set(),
  results: new Map(),
  revealed: null,
  timer: null,
  board: null,
};

// ---- Credentials -------------------------------------------------------

/// `#token=…` from a link (never sent to the server) moves into storage and
/// out of the address bar, the same bootstrap the main page does.
export function bootstrapToken() {
  const params = new URLSearchParams(String(window.location.hash || "").replace(/^#/, ""));
  const token = params.get("token");
  if (!token) return;
  try {
    localStorage.setItem(TOKEN_KEY, token);
  } catch {
    // Storage blocked: the session cookie or a proxy may still authorize.
  }
  params.delete("token");
  const rest = params.toString();
  history.replaceState(null, "", `${window.location.pathname}${window.location.search}${rest ? `#${rest}` : ""}`);
}

function authHeaders() {
  let token = null;
  try {
    token = localStorage.getItem(TOKEN_KEY);
  } catch {
    // ignore
  }
  return token ? { Authorization: `Bearer ${token}` } : {};
}

class ApiError extends Error {
  constructor(message, status, auth = false) {
    super(message);
    this.status = status;
    this.auth = auth;
  }
}

async function read(path) {
  let resp;
  try {
    resp = await fetch(path, { credentials: "same-origin", headers: authHeaders(), signal: AbortSignal.timeout?.(READ_TIMEOUT_MS) });
  } catch (error) {
    throw new ApiError(error.message || "network", 0);
  }
  if (resp.status === 401) throw new ApiError("sign in to see what needs you", 401, true);
  if (resp.status === 403) throw new ApiError("signed in without operator access", 403, true);
  if (!resp.ok) throw new ApiError(`${path} → ${resp.status}`, resp.status);
  return resp.json();
}

/// A state change: the credential plus the header the server requires
/// before a session cookie may change anything.
async function write(path, body) {
  const resp = await fetch(path, {
    method: "POST",
    credentials: "same-origin",
    headers: { "Content-Type": "application/json", "X-Tend-Operator": "1", ...authHeaders() },
    body: JSON.stringify(body),
  });
  const payload = await resp.json().catch(() => null);
  // The auth gate answers with a bare status; a handler's own refusal
  // ("not addressed to you") carries a JSON error and keeps the page.
  if (resp.status === 401 || (resp.status === 403 && !payload?.error)) throw new ApiError("signed in without operator access", resp.status, true);
  if (!resp.ok) throw new ApiError(payload?.error || `${resp.status}`, resp.status);
  return payload;
}

// ---- DOM helpers -------------------------------------------------------

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

function icon(name, extra = "") {
  const node = el("span", `icon i-${name}${extra ? ` ${extra}` : ""}`);
  node.setAttribute("aria-hidden", "true");
  return node;
}

function button(label, onClick, className = "btn", iconName = null) {
  const node = el("button", className);
  node.type = "button";
  if (iconName) node.append(icon(iconName));
  node.append(el("span", null, label));
  node.addEventListener("click", (event) => {
    event.stopPropagation();
    onClick(event);
  });
  return node;
}

function logo(product) {
  if (product === "claude") {
    const img = el("img", "logo");
    img.src = "/static/icons/logo-claude.svg";
    img.alt = "";
    return img;
  }
  if (product === "openai") return el("span", "logo openai");
  return null;
}

function hostBadge(alias, local) {
  const box = el("span", "hostbadge");
  const disc = el("span", `disc hue-${hostHue(alias)}`);
  disc.append(icon(local ? "laptop" : "server"));
  box.append(disc, el("span", null, alias));
  return box;
}

function when(iso) {
  return relativeTime(iso, Date.now(), LANG);
}

/// The workbench's pane drawer deep link (pane-drawer.mjs parsePaneHash):
/// pane ids repeat across tmux servers, so the socket picks the right one.
export function paneLink(unit) {
  const params = new URLSearchParams({ pane: unit.pane });
  if (unit.socket) params.set("socket", unit.socket);
  return `/#${params}`;
}

function localHost(alias) {
  return Boolean(state.board?.hosts.find((h) => h.alias === alias)?.local) || !state.board?.hosts.length;
}

function canWrite() {
  const access = state.data.access;
  return Boolean(access && access.write_authorized !== false && access.mode !== "read_only");
}

// ---- Cards -------------------------------------------------------------

function cardKey(unit) {
  return JSON.stringify([unit.host, unit.type, unit.decision || "", unit.id]);
}

function resultLine(key) {
  const result = state.results.get(key);
  if (!result) return null;
  const line = el("p", `result ${result.ok ? "ok" : "error"}`);
  line.setAttribute("role", "status");
  line.append(icon(result.ok ? "check" : "exclaim"), el("span", null, result.text));
  return line;
}

async function decide(unit, path, body, done) {
  const key = cardKey(unit);
  state.busy.add(key);
  state.results.delete(key);
  render();
  try {
    await write(path, unit.remote ? { ...body, host: unit.host } : body);
    state.drafts.delete(key);
    if (state.composer?.key === key) state.composer = null;
    state.results.set(key, { ok: true, text: done });
    setTimeout(refresh, 400);
  } catch (error) {
    if (error.auth) return lostAccess(error);
    state.results.set(key, { ok: false, text: error.message });
  } finally {
    state.busy.delete(key);
    render();
  }
}

function openComposer(unit, mode, prefill = null, option = null) {
  const key = cardKey(unit);
  state.composer = { key, mode, option };
  if (prefill != null) state.drafts.set(key, prefill);
  render();
  const area = document.querySelector?.(`textarea[data-key="${CSS.escape(key)}"]`);
  area?.focus?.();
}

function composer(unit) {
  const key = cardKey(unit);
  const { mode, option } = state.composer;
  const box = el("div", "composer");
  const area = el("textarea", "field");
  area.rows = 2;
  area.dataset.key = key;
  area.maxLength = unit.decision === "question" ? 16384 : 1024;
  area.placeholder = unit.decision === "question" ? S.yourAnswer : S.noteForAsker;
  area.value = state.drafts.get(key) || "";
  const row = el("div", "row end");
  const cancel = button(S.cancel, () => {
    state.composer = null;
    render();
  }, "btn ghost");
  const questionPath = `/api/decisions/requests/${encodeURIComponent(unit.id)}`;
  const reviewPath = `/api/decisions/reviews/${encodeURIComponent(unit.id)}`;
  let primary;
  if (unit.decision === "question" && mode === "decline") {
    primary = button(S.decline, () => decide(unit, questionPath, { answer: area.value, declined: true }, S.declined), "btn primary");
    row.append(cancel, primary);
  } else if (unit.decision === "question") {
    primary = button(S.send, () => decide(unit, questionPath, { answer: area.value }, S.answered), "btn primary");
    row.append(cancel, primary);
  } else {
    const accept = button(S.accept, () => {
      const body = { disposition: "accepted" };
      if (area.value.trim()) body.note = area.value;
      if (option) body.option = option;
      decide(unit, reviewPath, body, S.accepted);
    }, "btn");
    primary = button(S.returnVerb, () => decide(unit, reviewPath, { disposition: "returned", note: area.value }, S.returned), "btn primary");
    row.append(cancel, accept, primary);
  }
  const sync = () => {
    primary.disabled = !area.value.trim();
  };
  area.addEventListener("input", () => {
    if (area.value) state.drafts.set(key, area.value);
    else state.drafts.delete(key);
    sync();
  });
  area.addEventListener("keydown", (event) => {
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && !primary.disabled) {
      event.preventDefault();
      primary.click();
    } else if (event.key === "Escape") {
      state.composer = null;
      render();
    }
  });
  sync();
  box.append(area, row);
  return box;
}

function overflowMenu(items) {
  const menu = el("details", "menu overflow-menu more");
  const summary = el("summary", "btn icon-only");
  summary.setAttribute("aria-label", S.more);
  summary.append(icon("ellipsis"));
  const pop = el("div", "popover");
  for (const [label, action, cls] of items) {
    pop.append(button(label, () => {
      menu.open = false;
      action();
    }, `btn ghost small ${cls || ""}`));
  }
  menu.append(summary, pop);
  menu.addEventListener("click", (event) => event.stopPropagation());
  return menu;
}

const LANE_STATUS = {
  needsYou: ["statusYourTurn", "hand"],
  running: ["statusWorking", "working"],
  finished: ["statusFinished", "done"],
};

/// The state, who, where and when on one subdued line (the emphasis
/// layout's meta line).
function metaLine(lane, parts) {
  const row = el("div", "meta");
  if (lane) {
    const [word, glyph] = LANE_STATUS[lane];
    const status = el("span", `state ${lane}`);
    status.append(icon(glyph), el("span", null, S[word]));
    row.append(status);
  }
  row.append(...parts.filter(Boolean));
  return row;
}

function checksBox(checks) {
  const box = el("div", `checks ${checks.passed ? "pass" : "fail"}`);
  const top = el("div", "head");
  top.append(icon(checks.passed ? "seal-pass" : "seal-fail"), el("span", null, checks.headline));
  box.append(top);
  for (const line of checks.lines) {
    const row = el("div", `line ${line.cls}`);
    row.append(icon(line.cls === "pass" ? "check" : "xmark"), el("span", null, line.text));
    box.append(row);
    if (line.tail) box.append(el("pre", null, line.tail));
  }
  return box;
}

/// The asker's account: the summary, the recommendation, the evidence.
function account(unit) {
  const nodes = [];
  if (unit.detail) nodes.push(el("div", "d-detail", unit.detail));
  if (unit.recommendation) {
    const rec = el("div", "d-rec");
    rec.append(icon("bulb"), el("span", null, unit.recommendation));
    nodes.push(rec);
  }
  if (unit.evidence.length) {
    const chips = el("div", "chips");
    for (const chip of unit.evidence) {
      const node = chip.href ? el("a", "chip-e") : el("span", `chip-e ${chip.tone || ""}`);
      if (chip.href) {
        node.href = chip.href;
        node.target = "_blank";
        node.rel = "noopener noreferrer";
        node.title = chip.href;
      }
      node.append(icon(chip.icon), el("span", null, chip.text));
      chips.append(node);
    }
    nodes.push(chips);
  }
  return nodes;
}

/// Options, then Answer… / Other… or Accept / Return…, or the composer.
function answering(unit) {
  const key = cardKey(unit);
  const nodes = [];
  if (unit.actionable === false) {
    nodes.push(el("div", "flag", unit.decision === "review" ? S.decideThere : S.observeOnly));
    return nodes;
  }
  if (!canDecide()) return nodes;
  if (unit.options.length) {
    const list = el("div", "options");
    unit.options.forEach((option, index) => {
      const row = el("button", "option");
      row.type = "button";
      row.title = S.optionHelp;
      row.append(el("span", "n", String(index + 1)), el("span", "t", option));
      row.addEventListener("click", () => openComposer(unit, unit.decision === "question" ? "answer" : "return", option, option));
      list.append(row);
    });
    nodes.push(list);
  }
  if (state.composer?.key === key) {
    nodes.push(composer(unit));
  } else if (unit.decision === "question") {
    const row = el("div", "row");
    row.append(unit.options.length
      ? button(S.other, () => openComposer(unit, "answer"), "btn ghost")
      : button(S.answer, () => openComposer(unit, "answer"), "btn"));
    nodes.push(row);
  } else {
    const row = el("div", "row");
    row.append(
      button(S.accept, () => decide(unit, `/api/decisions/reviews/${encodeURIComponent(unit.id)}`, { disposition: "accepted" }, S.accepted), "btn"),
      button(S.returnEllipsis, () => openComposer(unit, "return"), "btn ghost"),
    );
    nodes.push(row);
  }
  return nodes;
}

function decisionMenu(unit) {
  if (unit.actionable === false || !canDecide()) return null;
  return unit.decision === "question"
    ? overflowMenu([[`${S.decline}…`, () => openComposer(unit, "decline"), "danger"]])
    : overflowMenu([[S.withdraw, () => decide(unit, `/api/decisions/reviews/${encodeURIComponent(unit.id)}`, { disposition: "withdrawn" }, S.withdrawn), "danger"]]);
}

/// A question or a result to review. `emphasis`: the status-emphasis card
/// (FleetDecisionCard.emphasisBody); otherwise the plain card the details
/// panel shows.
function decisionCard(unit, emphasis = state.layout === "emphasis") {
  const key = cardKey(unit);
  const busy = state.busy.has(key);
  const card = el("article", `card decision${emphasis ? " emphasis needsYou" : ""}${busy ? " busy" : ""}`);
  card.dataset.id = unit.id;
  card.dataset.host = unit.host;
  const question = unit.decision === "question";
  if (emphasis) {
    const head = el("div", "e-kind");
    head.append(icon(question ? "question-fill" : "checklist"), el("span", "kind", question ? S.question : S.review));
    if (busy) head.append(el("span", "when", S.sending));
    const menu = decisionMenu(unit);
    if (menu) head.append(menu);
    card.append(head, el("div", "e-title", tx(unit.title)));
    const inset = el("div", "inset");
    if (unit.checks) inset.append(checksBox(unit.checks));
    inset.append(...account(unit), ...answering(unit));
    card.append(inset);
    const result = resultLine(key);
    if (result) card.append(result);
    card.append(metaLine(null, [
      unit.asker ? el("span", "who", unit.asker) : null,
      el("span", null, unit.host),
      unit.project && unit.project !== TEND_PROJECT ? el("span", "repo", `· ${unit.project}`) : null,
      el("span", "when", when(unit.since)),
    ]));
    return card;
  }
  const head = el("div", "d-head");
  head.append(icon(question ? "question" : "checklist", "kind-icon"), el("span", "kind", question ? S.question : S.review));
  if (unit.asker) head.append(el("span", "who", unit.asker));
  if (unit.remote) head.append(el("span", "who", unit.host));
  head.append(el("span", "when", busy ? S.sending : when(unit.since)));
  const menu = decisionMenu(unit);
  if (menu) head.append(menu);
  card.append(head);
  if (unit.checks) card.append(checksBox(unit.checks));
  card.append(el("div", "d-title", tx(unit.title)), ...account(unit), ...answering(unit));
  const result = resultLine(key);
  if (result) card.append(result);
  return card;
}

function canDecide() {
  // A paired device or a proxy may not read /api/access; the queue read
  // itself already proved the credential may decide.
  return state.data.access ? canWrite() || state.data.access.login?.role === "operator" : true;
}

function stageBar(unit) {
  const bar = el("div", "stagebar");
  for (const stage of unit.stages) bar.append(el("span", stage.state));
  return bar;
}

function headlineText(unit) {
  if (typeof unit.headline === "string") return unit.headline;
  if (unit.headline?.key === "state") return S.stateLabels[unit.headline.state] || unit.headline.state || "";
  if (unit.headline?.key) return tx(unit.headline);
  if (unit.headline?.text) return unit.headline.text;
  return "";
}

function workCard(unit) {
  const card = el("button", `card work emphasis ${unit.lane}${state.selected === unit.id ? " selected" : ""}`);
  card.type = "button";
  card.dataset.id = unit.id;
  card.setAttribute("aria-expanded", String(state.selected === unit.id));
  card.addEventListener("click", () => select(state.selected === unit.id ? null : unit.id));
  const titleRow = el("div", "title-row");
  if (unit.orphanHelper) titleRow.append(el("span", "pill", S.helperPill));
  titleRow.append(el("div", "e-title", tx(unit.title)));
  card.append(titleRow);
  const single = unit.stages.length === 1;
  if (!(unit.lane === "finished" && single)) {
    card.append(stageBar(unit));
    const line = el("div", "stage-line");
    line.append(el("span", "name", unit.stages.find((s) => s.state !== "done")?.name || unit.stages[0].name));
    if (!single) line.append(el("span", "n", `${unit.stages.filter((s) => s.state === "done").length}/${unit.stages.length}`));
    card.append(line);
  }
  const inset = el("div", "inset");
  const text = headlineText(unit);
  if (text) inset.append(el("div", "headline", text));
  if (unit.handledLimit) {
    const flag = el("div", "flag");
    flag.append(icon("hourglass"), el("span", null, S.limitHandled));
    inset.append(flag);
  }
  if (inset.children.length) card.append(inset);
  const marks = (unit.products?.length ? unit.products : [unit.product]).slice(0, 3).map(logo).filter(Boolean);
  card.append(metaLine(unit.lane, [
    ...marks,
    hostBadge(unit.host, !unit.remote),
    unit.project && unit.project !== TEND_PROJECT ? el("span", "repo", `· ${unit.project}`) : null,
    el("span", "when", when(unit.since)),
  ]));
  return card;
}

/// One row of the compact list: a status dot, the title, the host and how
/// long ago (FleetCompactRow). Selecting it opens its details.
function compactRow(unit) {
  const lane = unit.type === "decision" ? "needsYou" : unit.lane;
  const row = el("button", `row-c ${lane}${state.selected === unit.id ? " selected" : ""}`);
  row.type = "button";
  row.dataset.id = unit.id;
  row.setAttribute("aria-expanded", String(state.selected === unit.id));
  row.addEventListener("click", () => select(state.selected === unit.id ? null : unit.id));
  row.append(el("span", "dot-c"));
  if (unit.type === "decision") row.append(icon("question", "kind-icon"));
  row.append(el("span", "title", tx(unit.title)), el("span", "host", unit.host), el("span", "when", when(unit.since)));
  return row;
}

/// What was decided, in Finished: the decision, the outcome in the inset,
/// who decided it on the meta line.
function decidedCard(unit) {
  const card = el("article", `card decided emphasis finished ${unit.outcome}`);
  card.dataset.id = unit.id;
  card.append(el("div", "e-title", tx(unit.title)));
  const inset = el("div", "inset");
  const outcome = el("div", "outcome");
  const word = S[unit.outcome] || unit.outcome;
  outcome.append(icon(unit.outcome === "accepted" || unit.outcome === "answered" ? "check" : "turn"), el("span", null, unit.note ? `${word}: ${unit.note}` : word));
  inset.append(outcome);
  card.append(inset);
  card.append(metaLine("finished", [
    icon(unit.decision === "question" ? "question" : "checklist"),
    unit.by ? el("span", "who", S.by(unit.by)) : null,
    hostBadge(unit.host, !unit.remote),
    el("span", "when", when(unit.since)),
  ]));
  return card;
}

function unitCard(unit) {
  if (state.layout === "compact") return compactRow(unit);
  if (unit.type === "decision") return decisionCard(unit);
  if (unit.type === "decided") return decidedCard(unit);
  return workCard(unit);
}

// ---- Lanes -------------------------------------------------------------

const LANE_WORDS = {
  needsYou: ["yourTurn", "yourTurnSub", "yourTurnEmpty"],
  running: ["inProgress", "inProgressSub", "inProgressEmpty"],
  finished: ["finished", "finishedSub", "finishedEmpty"],
};

function noteText(note) {
  if (note.key === "observeOnly") return `${note.host}: ${S.observeOnly}`;
  if (note.key === "unavailable") return `${note.host}: ${S.unavailable(note.what)}`;
  if (note.key === "hostState") return `${note.host}: ${note.state}${note.error ? ` · ${note.error}` : ""}`;
  if (note.key === "more") return `${note.host}: …`;
  return String(note.text || "");
}

function renderLane(lane, data) {
  const [title, sub, empty] = LANE_WORDS[lane];
  const section = el("section", `lane ${lane}${data.count ? " has-items" : ""} ${state.layout}`);
  section.setAttribute("aria-label", S[title]);
  const head = el("header", "lane-head");
  head.append(el("h2", null, S[title]), el("span", "count", String(data.count)), el("span", "sub", S[sub]));
  const body = el("div", "lane-body");
  const error = state.errors[lane === "needsYou" ? "decisions" : lane === "running" ? "fleet" : "history"];
  if (error) {
    const line = el("div", "lane-note");
    line.append(icon("exclaim"), el("span", null, error));
    body.append(line);
  }
  for (const note of data.notes) {
    const line = el("div", "lane-note quiet");
    line.append(el("span", null, noteText(note)));
    body.append(line);
  }
  const several = data.groups.length > 1;
  for (const group of data.groups) {
    if (several) {
      const header = el("div", "project");
      const system = group.project === TEND_PROJECT;
      header.append(icon(system ? "server" : "folder"), el("span", null, system ? S.fromTend : group.project || S.noProject), el("span", "n", String(group.units.length)));
      body.append(header);
    }
    for (const unit of group.units) body.append(unitCard(unit));
  }
  if (!data.count && !error) body.append(el("div", "lane-empty", S[empty]));
  const fold = (list, open, show, hide, toggle) => {
    if (!list?.length) return;
    body.append(button(open ? hide : show(list.length), () => {
      toggle();
      render();
    }, "btn ghost small fold"));
    if (open) for (const unit of list) body.append(unitCard(unit));
  };
  // Each lane opens its own folds (FleetLaneColumn's own toggles).
  const open = state.folds[lane];
  fold(data.folds?.helpers, open.helpers, S.foldHelpers, S.hideHelpers, () => (open.helpers = !open.helpers));
  fold(data.folds?.empty, open.empty, S.foldEmpty, S.hideEmpty, () => (open.empty = !open.empty));
  fold(data.older, open.older, S.olderCount, S.hideOlder, () => (open.older = !open.older));
  if (lane === "finished" && data.folds?.reviewed?.length) {
    body.append(button(S.showReviewed(data.folds.reviewed.length), () => {
      state.folds.reviewed = true;
      render();
    }, "btn ghost small fold"));
  } else if (lane === "finished" && state.folds.reviewed && state.board?.server) {
    body.append(button(S.hideReviewed, () => {
      state.folds.reviewed = false;
      render();
    }, "btn ghost small fold"));
  }
  section.append(head, body);
  return section;
}

// ---- Backlog (FleetBacklog.swift) -----------------------------------------

function renderBacklog(records) {
  const section = el("section", `lane backlog ${state.layout}`);
  section.setAttribute("aria-label", S.backlog);
  const head = el("header", "lane-head");
  head.append(el("h2", null, S.backlog), el("span", "count", String(records.length)), el("span", "sub", S.backlogSub));
  const body = el("div", "lane-body");
  for (const record of records) {
    const card = el("article", "card backlog-card");
    const key = `backlog:${record.workspace}/${record.work}`;
    const top = el("div", "title-row");
    top.append(el("div", "e-title", record.title || record.work));
    if (record.priority != null) top.append(el("span", "pill", `P${record.priority}`));
    card.append(top);
    if (record.line) card.append(el("div", "headline", record.line));
    card.append(el("div", "meta", [record.workspace, record.pipeline, record.external].filter(Boolean).join(" · ")));
    if (canWrite() && state.data.access?.capabilities?.work_start) {
      const row = el("div", "row");
      const start = button(S.start, async () => {
        state.busy.add(key);
        render();
        try {
          const body = { work: record.work };
          if (record.workspace) body.workspace = record.workspace;
          if (record.external) body.external = record.external;
          await write("/api/work-control/up", body);
          state.results.delete(key);
          setTimeout(refresh, 600);
        } catch (error) {
          state.results.set(key, { ok: false, text: error.message });
        } finally {
          state.busy.delete(key);
          render();
        }
      }, "btn small", "play");
      start.disabled = state.busy.has(key);
      row.append(start);
      card.append(row);
    }
    const line = resultLine(key);
    if (line) card.append(line);
    body.append(card);
  }
  section.append(head, body);
  return section;
}

// ---- Review (FleetReviewView.swift, the list) ---------------------------

const SCOPES = [["today", "scopeToday"], ["day", "scopeDay"], ["week", "scopeWeek"], ["all", "scopeAll"]];

/// Review counts every finished result, whatever Home's host and project
/// filters show (FleetHomeView passes the unfiltered units).
function reviewBoard() {
  if (!state.data.server) return null;
  return fromServerBoard(state.data.server, { history: null, filter: {}, now: Date.now(), showReviewed: false });
}

function renderReviewButton(board) {
  const buttonNode = $("review-button");
  if (!buttonNode) return;
  const all = reviewBoard();
  const queue = all ? reviewQueue(all, state.reviewScope) : [];
  buttonNode.hidden = !board.server;
  const badge = $("review-count");
  badge.hidden = !queue.length;
  badge.textContent = queue.length > 99 ? "99+" : String(queue.length);
  if ($("review-dialog")?.open) renderReviewSheet();
}

function renderReviewSheet() {
  const list = $("review-list");
  const all = reviewBoard();
  const queue = all ? reviewQueue(all, state.reviewScope) : [];
  $("review-total").textContent = String(queue.length);
  for (const option of document.querySelectorAll("#review-scope button")) {
    option.setAttribute("aria-checked", String(option.dataset.scope === state.reviewScope));
  }
  if (!queue.length) {
    const empty = el("div", "review-empty");
    empty.append(icon("test-pass", "big"), el("p", null, S.reviewEmpty));
    list.replaceChildren(empty);
    return;
  }
  list.replaceChildren(...queue.map((unit) => {
    const key = `reviewed:${unit.id}`;
    const row = el("article", "card review-row emphasis finished");
    row.append(el("div", "e-title", tx(unit.title)));
    const text = headlineText(unit);
    if (text) {
      const inset = el("div", "inset");
      inset.append(el("div", "headline", text));
      row.append(inset);
    }
    const actions = el("div", "row");
    const marks = (unit.products?.length ? unit.products : [unit.product]).slice(0, 3).map(logo).filter(Boolean);
    actions.append(metaLine("finished", [...marks, hostBadge(unit.host, !unit.remote), unit.project ? el("span", "repo", `· ${unit.project}`) : null, el("span", "when", when(unit.since))]));
    if (canWrite()) {
      const good = button(S.looksGood, () => markReviewed(unit, key), "btn small primary", "check");
      good.disabled = state.busy.has(key);
      actions.append(good);
    }
    row.append(actions);
    const line = resultLine(key);
    if (line) row.append(line);
    return row;
  }));
}

function wireReview() {
  const dialog = $("review-dialog");
  $("review-button")?.addEventListener("click", () => {
    renderReviewSheet();
    dialog.showModal?.();
  });
  $("review-done")?.addEventListener("click", () => dialog.close?.());
  for (const option of document.querySelectorAll("#review-scope button")) {
    option.addEventListener("click", () => {
      state.reviewScope = option.dataset.scope;
      try {
        localStorage.setItem("tend.home.reviewScope", state.reviewScope);
      } catch {
        // A preference only.
      }
      render();
      renderReviewSheet();
    });
  }
}

// ---- Inspector ---------------------------------------------------------

function findUnit(id) {
  if (!id || !state.board) return null;
  for (const lane of LANES) {
    const data = state.board.lanes[lane];
    for (const group of data.groups) for (const unit of group.units) if (unit.id === id) return unit;
    for (const unit of data.older) if (unit.id === id) return unit;
    for (const list of Object.values(data.folds || {})) for (const unit of list) if (unit.id === id) return unit;
  }
  return null;
}

/// Send a draft to the agent it was written for, and only to that one.
async function sendToPane(target, text, key) {
  state.busy.add(key);
  state.results.delete(key);
  render();
  try {
    const body = { text, submit: true };
    if (target.socket) body.socket = target.socket;
    await write(`/api/panes/${encodeURIComponent(target.pane)}/prompt`, body);
    state.replyDrafts.delete(key);
    state.results.set(key, { ok: true, text: S.sent });
    setTimeout(refresh, 600);
  } catch (error) {
    if (error.auth && error.status === 401) return lostAccess(error);
    state.results.set(key, { ok: false, text: error.message });
  } finally {
    state.busy.delete(key);
    render();
  }
}

function renderInspector() {
  const box = $("inspector");
  const unit = findUnit(state.selected);
  if (!unit) {
    state.selected = null;
    box.hidden = true;
    box.replaceChildren();
    return;
  }
  box.hidden = false;
  if (unit.type === "decision") return renderDecisionDetail(box, unit);
  if (unit.type === "decided") return renderDecidedDetail(box, unit);
  const head = el("div", "ins-head");
  const titles = el("div", "titles");
  titles.append(el("h2", null, tx(unit.title)), el("div", "sub", [unit.project, unit.host].filter(Boolean).join(" · ")));
  const close = button("", () => select(null), "btn icon-only", "xmark");
  close.setAttribute("aria-label", S.close);
  head.append(titles, close);

  const body = el("div", "ins-body");
  const now = el("section", `nowbox ${unit.lane}`);
  now.append(el("div", "label", unit.lane === "needsYou" ? S.needsDecision : unit.lane === "running" ? S.now : S.lastResult));
  const text = headlineText(unit);
  if (text) now.append(el("div", "text", text));
  const key = `reply:${unit.id}`;
  const target = canWrite() ? replyTarget(unit) : null;
  const replyable = Boolean(target);
  const paneLinks = Boolean(state.data.access);
  // A draft keeps the agent and the question it was written for
  // (FleetReplyDraft): text for one question never goes to another agent,
  // a replacement session or the same agent's next question unasked.
  const draft = state.replyDrafts.get(key);
  if (draft?.text && !sameTarget(draft.target, target)) {
    const stale = el("div", "stale");
    stale.append(icon("exclaim"), el("span", null, S.staleDraft(draft.target?.owner || "")));
    const row = el("div", "row");
    if (target) {
      row.append(button(S.useFor(target.owner), () => {
        state.replyDrafts.set(key, { text: draft.text, target });
        render();
      }, "btn small"));
    }
    row.append(button(S.discard, () => {
      state.replyDrafts.delete(key);
      render();
    }, "btn ghost small danger"));
    stale.append(row);
    now.append(stale);
  } else if (replyable) {
    const area = el("textarea", "field");
    area.rows = 2;
    area.dataset.key = key;
    area.maxLength = 16384;
    area.placeholder = target.kind === "answer" ? S.yourAnswer : S.nextInstruction;
    area.value = draft?.text || "";
    const send = button(target.kind === "answer" ? S.answerVerb : S.sendBack, () => sendToPane(target, area.value, key), "btn primary");
    const sync = () => (send.disabled = !area.value.trim() || state.busy.has(key));
    area.addEventListener("input", () => {
      if (area.value) state.replyDrafts.set(key, { text: area.value, target });
      else state.replyDrafts.delete(key);
      sync();
    });
    area.addEventListener("keydown", (event) => {
      if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && !send.disabled) {
        event.preventDefault();
        send.click();
      }
    });
    sync();
    const composerBox = el("div", "composer reply");
    const row = el("div", "row end");
    row.append(send);
    composerBox.append(area, row);
    now.append(composerBox);
  }
  const result = resultLine(key);
  if (result) now.append(result);
  if (unit.pane && !unit.remote && paneLinks) {
    const row = el("div", "row");
    const open = el("a", replyable ? "btn" : "btn primary", unit.lane === "needsYou" && !replyable ? S.answerInPane : S.openPane);
    open.href = paneLink(unit);
    row.append(open);
    now.append(row);
  }
  // Looks Good: the review mark the Mac records, for a finished unit the
  // daemon's board knows.
  if (state.board?.server && unit.lane === "finished" && !unit.reviewed && canWrite()) {
    const row = el("div", "row end");
    const key = `reviewed:${unit.id}`;
    row.append(button(S.looksGood, () => markReviewed(unit, key), "btn ghost", "check"));
    const line = resultLine(key);
    now.append(row);
    if (line) now.append(line);
  }
  body.append(now);

  const stages = el("section");
  stages.append(el("div", "section-label", S.stages));
  const timeline = el("div", "timeline");
  for (const stage of unit.stages) {
    const row = el("div", `tl-row c-${stage.state}`);
    row.append(el("span", `ring${stage.state === "done" ? " filled" : ""}`));
    const mid = el("div");
    mid.append(el("div", "name", stage.name));
    const owner = el("div", "owner");
    const mark = logo(productOf(stage.kind) || unit.product);
    if (mark) owner.append(mark);
    owner.append(el("span", null, stage.owner || `${stage.name} · ${unit.host}`));
    mid.append(owner);
    row.append(mid, el("span", "st", stage.state === "needsYou" ? S.stageYourTurn : stage.state === "running" ? S.working : stage.state === "done" ? S.done : S.waiting));
    timeline.append(row);
  }
  stages.append(timeline);
  body.append(stages);

  for (const [label, list, help] of [[S.agents, unit.agents, null], [S.helpers, unit.helpers, S.helpersHelp]]) {
    if (!list?.length) continue;
    const agents = el("section");
    agents.append(el("div", "section-label", label));
    if (help) agents.append(el("p", "help", help));
    for (const agent of list) {
      const row = el("div", "agent-row");
      const tone = agent.state === "working" || agent.state === "starting" ? "connecting" : agent.state === "error" ? "bad" : agent.state?.startsWith("waiting") ? "warn" : "off";
      row.append(el("span", `dot ${tone}`));
      const mark = logo(agent.product);
      if (mark) row.append(mark);
      row.append(el("span", "owner", agent.owner || agent.name || agent.kind || "agent"), el("span", "st", agent.status ? tx(agent.status) : S.stateLabels[agent.state] || agent.state || ""));
      if (agent.pane && !agent.remote && paneLinks) {
        const open = el("a", "btn small", S.open);
        open.href = paneLink(agent);
        row.append(open);
      }
      agents.append(row);
    }
    body.append(agents);
  }
  box.replaceChildren(head, body);
}

/// A decision's details beside the board in the compact layout
/// (FleetDecisionDetail): the whole card, and the asker to open.
function renderDecisionDetail(box, unit) {
  const head = el("div", "ins-head");
  const titles = el("div", "titles");
  const kind = el("div", "e-kind");
  kind.append(icon(unit.decision === "question" ? "question" : "checklist"), el("span", "kind", unit.decision === "question" ? S.question : S.review));
  titles.append(kind, el("div", "sub", [unit.asker, unit.host, unit.project !== TEND_PROJECT ? unit.project : ""].filter(Boolean).join(" · ")));
  head.append(titles);
  if (unit.pane && !unit.remote && state.data.access) {
    const open = el("a", "btn", S.openAgent);
    open.href = paneLink(unit);
    head.append(open);
  }
  const close = button("", () => select(null), "btn icon-only", "xmark");
  close.setAttribute("aria-label", S.close);
  head.append(close);
  const body = el("div", "ins-body divided");
  body.append(decisionCard(unit, false));
  box.replaceChildren(head, body);
}

function renderDecidedDetail(box, unit) {
  const head = el("div", "ins-head");
  const titles = el("div", "titles");
  titles.append(el("h2", null, tx(unit.title)), el("div", "sub", unit.host));
  const close = button("", () => select(null), "btn icon-only", "xmark");
  close.setAttribute("aria-label", S.close);
  head.append(titles, close);
  const body = el("div", "ins-body");
  const now = el("section", "nowbox finished");
  const word = S[unit.outcome] || unit.outcome;
  now.append(el("div", "label", S.lastResult), el("div", "text", unit.note ? `${word}: ${unit.note}` : word));
  if (unit.by) now.append(el("div", "sub", S.by(unit.by)));
  body.append(now);
  box.replaceChildren(head, body);
}

async function markReviewed(unit, key) {
  state.busy.add(key);
  render();
  try {
    // The key of the result on screen: tendd refuses (409) a unit that
    // changed since, so a mark never accepts a newer result unseen.
    await write("/api/home/reviewed", { id: unit.id, review_key: unit.reviewKey });
    state.results.delete(key);
    if (state.selected === unit.id) state.selected = null;
    // Hidden at once; the next read confirms the mark.
    if (state.data.server) markLocally(unit.id, unit.reviewKey);
    setTimeout(refresh, 400);
  } catch (error) {
    if (error.auth && error.status === 401) return lostAccess(error);
    state.results.set(key, { ok: false, text: error.message });
    if (error.status === 409) setTimeout(refresh, 0);
  } finally {
    state.busy.delete(key);
    render();
  }
}

/// Mark a unit reviewed in the board on hand, until the next read.
function markLocally(id, key) {
  for (const group of state.data.server?.lanes?.finished?.groups || []) {
    for (const item of group.items || []) if (item.id === id && item.review_key === key) item.reviewed = true;
  }
}

function setLayout(layout) {
  state.layout = layout === "compact" ? "compact" : "emphasis";
  try {
    localStorage.setItem(LAYOUT_KEY, state.layout);
  } catch {
    // A preference only.
  }
  // A decision opens beside the board only from the compact list.
  if (state.layout === "emphasis" && findUnit(state.selected)?.type !== "work") state.selected = null;
  render();
}

function select(id) {
  state.selected = id;
  render();
}

// ---- Header, status, mark ----------------------------------------------

function renderHeader(board) {
  const hosts = board.hosts;
  const online = hosts.filter((h) => h.state === "online").length;
  $("hosts-label").textContent = S.hostsCount(online, hosts.length);
  $("hosts-dot").className = `dot${online === hosts.length ? "" : " warn"}`;
  const list = $("hosts-list");
  list.replaceChildren(
    ...(hosts.length
      ? hosts.map((host) => {
          const li = el("li");
          li.append(el("span", `dot${host.state === "online" ? "" : host.state === "auth_failed" ? " bad" : host.state === "connecting" ? " connecting" : host.state === "degraded" || host.state === "version_skew" ? " warn" : " off"}`));
          li.append(el("span", "name", host.alias), el("span", "state", host.state));
          if (host.error) li.title = host.error;
          return li;
        })
      : [el("li", "state", S.noHosts)]),
  );
  fillSelect($("filter-host"), S.allHosts, hosts.map((h) => h.alias), state.filter.host);
  fillSelect($("filter-project"), S.allProjects, board.projects, state.filter.project);
  const active = [state.filter.project, state.filter.host].filter(Boolean);
  $("filter-label").textContent = active.length ? active.join(" · ") : S.filter;
  $("filter-button").classList.toggle("active", active.length > 0);
  $("filter-clear").hidden = !active.length;
  // Shown to whoever may write, like the Mac's; where this host turned
  // dashboard starts off, the sheet says so instead of starting.
  $("start-work").hidden = !canWrite();
}

function fillSelect(select, all, values, current) {
  const options = [["", all], ...values.map((v) => [v, v])];
  const signature = JSON.stringify(options);
  if (select.dataset.signature !== signature) {
    select.replaceChildren(...options.map(([value, label]) => {
      const option = el("option", null, label);
      option.value = value;
      return option;
    }));
    select.dataset.signature = signature;
  }
  select.value = current;
}

function renderStatus(board) {
  const bar = document.querySelector(".statusbar");
  bar.classList.toggle("connecting", state.connection === "connecting");
  bar.classList.toggle("failed", state.connection === "failed");
  $("conn-label").textContent = state.connection === "connected" ? S.connected : state.connection === "connecting" ? S.connecting : S.disconnected;
  $("stat-hosts").textContent = S.hostsStatus(board?.hosts.length || 0);
  $("stat-agents").textContent = S.agentsStatus(board?.agentCount || 0);
  const counts = board ? board.lanes : null;
  const badge = $("rail-badge");
  const waiting = counts?.needsYou.count || 0;
  badge.hidden = !waiting;
  badge.textContent = waiting > 99 ? "99+" : String(waiting);
  for (const lane of LANES) $(`seg-${lane}`).textContent = counts ? String(counts[lane].count) : "";
  // The four-room mark, rule A: a room lights when anything is in it.
  // The agents on the board (the daemon's or the Fleet snapshot's).
  const boardAgents = board
    ? LANES.flatMap((lane) => [...board.lanes[lane].groups.flatMap((g) => g.units), ...Object.values(board.lanes[lane].folds || {}).flat()])
        .flatMap((unit) => [...(unit.agents || []), ...(unit.helpers || [])])
    : [];
  const agents = [...(state.data.fleet?.hosts || []).flatMap((h) => h.remote?.agents || []), ...boardAgents];
  const lit = {
    waiting: waiting > 0,
    error: agents.some((a) => a.state === "error"),
    working: (counts?.running.count || 0) > 0,
    done: (counts?.finished.count || 0) > 0,
  };
  const disconnected = state.connection !== "connected";
  for (const [room, on] of Object.entries(lit)) {
    document.querySelector(`#mark .room-${room}`)?.classList.toggle("lit", on && !disconnected);
  }
  document.title = waiting ? `(${waiting}) ${S.home} · tend` : `${S.home} · tend`;
}

// ---- Render ------------------------------------------------------------

/// Typing in a composer: a re-render now would move the cursor, so the
/// page waits until the field loses focus.
export function composing() {
  const active = document.activeElement;
  // An open card menu would close under a re-render too.
  return Boolean((active?.closest?.(".composer") && active.value) || document.querySelector?.(".lane details[open]"));
}

function render() {
  if (state.lost) return;
  const board = currentBoard();
  state.board = board;
  renderHeader(board);
  renderStatus(board);
  const lanes = $("lanes");
  // "No work yet" only when every read answered and nothing is noted: an
  // empty board beside a failed Fleet read is not an empty fleet.
  const quiet = !Object.values(state.errors).some(Boolean) && LANES.every((lane) => !board.lanes[lane].notes.length);
  const nothing = board.empty && quiet && Boolean(state.data.decisions || state.data.server) && !board.reviewedCount;
  $("empty-board").hidden = !nothing;
  lanes.hidden = nothing;
  const columns = LANES.map((lane) => renderLane(lane, board.lanes[lane]));
  if (board.backlog?.length) columns.unshift(renderBacklog(board.backlog));
  lanes.replaceChildren(...columns);
  lanes.classList.toggle("with-backlog", Boolean(board.backlog?.length));
  // The phone reaches the backlog through its own segment.
  $("seg-backlog-tab").hidden = !board.backlog?.length;
  $("seg-backlog").textContent = board.backlog?.length ? String(board.backlog.length) : "";
  if (state.lane === "backlog" && !board.backlog?.length) state.lane = "needsYou";
  renderReviewButton(board);
  $("board").dataset.lane = state.lane;
  for (const option of document.querySelectorAll("#layout-toggle button")) {
    option.setAttribute("aria-checked", String(option.dataset.layout === state.layout));
  }
  for (const tab of document.querySelectorAll("#segments button")) tab.setAttribute("aria-selected", String(tab.dataset.lane === state.lane));
  // Explore and pane links lead to the workbench, which only the full
  // dashboard serves: hidden where /api/access is not answered (a paired
  // device, the Windows app's scoped proxy).
  $("rail-explore").hidden = !state.data.access;
  renderInspector();
  revealLinkedItem();
}

/// Brings the card a link names (`#item=<id>`) into view once per link:
/// on the phone's Your turn segment, and opened beside the board in the
/// compact list, whose rows answer nothing in place.
function revealLinkedItem() {
  const id = linkedItemOf(window.location.hash);
  if (!id || state.revealed === id || !state.board) return;
  const unit = findUnit(id);
  if (!unit || unit.lane !== "needsYou") return;
  state.revealed = id;
  state.lane = "needsYou";
  $("board").dataset.lane = "needsYou";
  for (const tab of document.querySelectorAll("#segments button")) tab.setAttribute("aria-selected", String(tab.dataset.lane === "needsYou"));
  if (state.layout === "compact") {
    state.selected = id;
    render();
  }
  const node = [...document.querySelectorAll(".lane.needsYou [data-id]")].find((n) => n.dataset.id === id);
  node?.classList.add("linked");
  node?.scrollIntoView?.({ block: "center" });
}

/// Where sign-in comes back to: Home, and the linked card.
export function homeReturnPath(hash) {
  const item = linkedItemOf(hash);
  return item ? `/home#item=${encodeURIComponent(item)}` : "/home";
}

/// Signed out, expired or downgraded: nothing private stays on the page,
/// drafts, host names, filters and an open sheet included, and no older
/// read may bring it back.
function lostAccess(error) {
  state.generation += 1;
  state.lost = true;
  state.drafts.clear();
  state.replyDrafts.clear();
  state.results.clear();
  $("review-dialog").close?.();
  $("review-list").replaceChildren();
  $("review-total").textContent = "";
  $("review-button").hidden = true;
  state.composer = null;
  state.selected = null;
  state.filter = { host: "", project: "" };
  state.data = { decisions: null, history: null, fleet: null, access: null };
  state.board = null;
  $("lanes").replaceChildren();
  $("inspector").replaceChildren();
  $("inspector").hidden = true;
  $("empty-board").hidden = true;
  $("hosts-list").replaceChildren();
  $("hosts-label").textContent = S.hosts;
  for (const id of ["filter-host", "filter-project"]) {
    $(id).replaceChildren();
    delete $(id).dataset.signature;
  }
  $("filter-label").textContent = S.filter;
  for (const menu of document.querySelectorAll("details.menu[open]")) menu.open = false;
  $("start-work").hidden = true;
  $("start-work-id").value = "";
  $("start-body").value = "";
  $("start-dialog").close?.();
  $("rail-explore").hidden = true;
  $("status").textContent = error.message;
  const login = $("login");
  login.href = `/auth/login?return_to=${encodeURIComponent(homeReturnPath(window.location.hash))}`;
  login.hidden = false;
  state.connection = "connected";
  renderStatus(null);
}

/// One poll at a time: a slow host makes the next one wait rather than
/// overtake it (each read also gives up after READ_TIMEOUT_MS).
let inFlight = null;
let again = false;

export function refresh() {
  if (inFlight) {
    again = true;
    return inFlight;
  }
  inFlight = poll().finally(() => {
    inFlight = null;
    if (again) {
      again = false;
      refresh();
    }
  });
  return inFlight;
}

async function poll() {
  const generation = ++state.generation;
  const current = () => generation === state.generation;
  // The daemon's board (GET /api/home) when it has one; otherwise, or
  // where a proxy does not forward it (the Windows app) or a credential
  // may only decide (a paired device), the queue and the Fleet snapshot,
  // laid out here.
  const [home, history, access] = await Promise.allSettled([
    read("/api/home"),
    read("/api/decisions/history"),
    read("/api/access"),
  ]);
  if (!current()) return;
  let decisions;
  let fleet;
  if (home.status === "fulfilled") {
    decisions = { status: "fulfilled", value: null };
    fleet = { status: "fulfilled", value: null };
  } else {
    [decisions, fleet] = await Promise.allSettled([read("/api/decisions"), read("/api/fleet")]);
    if (!current()) return;
  }
  // The queue is what Home is for: losing it (401/403) is losing access.
  // The other reads may be refused to a credential that may only decide (a
  // paired device gets 401 there, the Windows app's proxy 403) while the
  // queue still reads: that leaves those parts empty, not the page. A
  // session that ended fails the queue read too.
  if (home.status === "rejected" && decisions.status === "rejected" && decisions.reason.auth) return lostAccess(decisions.reason);
  state.lost = false;
  $("login").hidden = true;
  const reached = [home, decisions, fleet, history].some((r) => r.status === "fulfilled" || r.reason?.status > 0);
  state.connection = reached ? "connected" : "failed";
  let narrowed = false;
  const take = (name, result) => {
    if (result.status === "fulfilled") {
      state.data[name] = result.value;
      state.errors[name] = null;
    } else if (result.reason.auth || result.reason.status === 404) {
      narrowed ||= state.data[name] != null && result.reason.auth;
      state.data[name] = null;
      state.errors[name] = null;
    } else {
      state.errors[name] = result.reason.message;
    }
  };
  take("server", home);
  // Whenever the board read failed, what the page shows comes from the
  // reads it fell back to, never a board left over from before.
  if (home.status === "rejected") state.data.server = null;
  take("decisions", decisions);
  take("fleet", fleet);
  take("history", history);
  const accessNow = access.status === "fulfilled" ? access.value : null;
  narrowed ||= Boolean(state.data.access && (!accessNow || (state.data.access.write_authorized !== false && accessNow.write_authorized === false)));
  state.data.access = accessNow;
  $("status").textContent = state.connection === "failed" ? S.disconnected : "";
  // While someone types, the board waits, unless what this credential may
  // see or do just narrowed: then what it no longer may goes at once.
  if (composing() && !narrowed) {
    renderStatus(currentBoard());
    return;
  }
  if (narrowed && !canWrite()) {
    state.composer = null;
    $("start-dialog").close?.();
  }
  render();
}

/// The board to draw: the daemon's when it served one, else built here.
function currentBoard() {
  const now = Date.now();
  if (state.data.server) {
    return fromServerBoard(state.data.server, { history: state.data.history, filter: state.filter, now, showReviewed: state.folds.reviewed });
  }
  return buildBoard({ ...state.data, now, filter: state.filter });
}

// ---- Start Work ----------------------------------------------------------

function wireStartWork() {
  const dialog = $("start-dialog");
  $("start-work").addEventListener("click", () => {
    const allowed = Boolean(state.data.access?.capabilities?.work_start);
    $("start-error").textContent = allowed ? "" : S.startNotAllowed;
    $("start-submit").disabled = !allowed;
    dialog.showModal?.();
  });
  $("start-cancel").addEventListener("click", () => dialog.close?.());
  $("start-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const work = $("start-work-id").value.trim();
    const body = $("start-body").value;
    if (!work) return;
    $("start-submit").disabled = true;
    try {
      await write("/api/work-control/up", body.trim() ? { work, body } : { work });
      dialog.close?.();
      $("start-work-id").value = "";
      $("start-body").value = "";
      setTimeout(refresh, 600);
    } catch (error) {
      if (error.auth && error.status === 401) return lostAccess(error);
      $("start-error").textContent = error.status === 501 ? S.startNotAllowed : error.message;
    } finally {
      $("start-submit").disabled = false;
    }
  });
}

// ---- Boot ----------------------------------------------------------------

function translate() {
  document.documentElement.lang = LANG;
  for (const node of document.querySelectorAll("[data-t]")) {
    const value = S[node.dataset.t];
    if (typeof value === "string") node.textContent = value;
  }
  for (const node of document.querySelectorAll("[data-t-label]")) node.setAttribute("aria-label", S[node.dataset.tLabel]);
  for (const node of document.querySelectorAll("[data-t-title]")) node.title = S[node.dataset.tTitle];
}

export function start() {
  bootstrapToken();
  translate();
  wireStartWork();
  wireReview();
  try {
    const scope = localStorage.getItem("tend.home.reviewScope");
    if (SCOPES.some(([value]) => value === scope)) state.reviewScope = scope;
  } catch {
    // ignore
  }
  $("filter-host").addEventListener("change", (event) => {
    state.filter.host = event.target.value;
    render();
  });
  $("filter-project").addEventListener("change", (event) => {
    state.filter.project = event.target.value;
    render();
  });
  $("filter-clear").addEventListener("click", () => {
    state.filter = { host: "", project: "" };
    $("filter-menu").open = false;
    render();
  });
  try {
    state.layout = localStorage.getItem(LAYOUT_KEY) === "compact" ? "compact" : "emphasis";
  } catch {
    // ignore
  }
  for (const option of document.querySelectorAll("#layout-toggle button")) {
    option.addEventListener("click", () => setLayout(option.dataset.layout));
  }
  for (const tab of document.querySelectorAll("#segments button")) {
    tab.addEventListener("click", () => {
      state.lane = tab.dataset.lane;
      render();
    });
  }
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && state.selected && !composing()) select(null);
  });
  window.addEventListener?.("hashchange", () => {
    state.revealed = null;
    revealLinkedItem();
  });
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) refresh();
  });
  // Clicking outside an open menu closes it, as a popover does.
  document.addEventListener("click", (event) => {
    for (const menu of document.querySelectorAll("details.menu[open]")) {
      if (!menu.contains(event.target)) menu.open = false;
    }
  });
  state.timer = setInterval(() => {
    if (!document.hidden) refresh();
  }, REFRESH_MS);
  refresh();
}

if (!globalThis.__TEND_HOME_NO_START__) start();
