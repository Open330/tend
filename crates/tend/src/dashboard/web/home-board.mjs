// Home's board, as Tend for Mac lays it out (FleetHome.swift,
// FleetHomeView.swift): three lanes, Your turn / In progress / Finished,
// each grouped by project, built from the decision queue (/api/decisions),
// what was decided (/api/decisions/history) and every host's agents
// (/api/fleet). Pure: no DOM, no clock of its own, no language; the page
// turns keys into words. Everything here is data an agent wrote and is
// rendered with textContent only.
//
// Unit rule (phase 1): one agent is one unit with one stage, which is how
// the Mac draws an agent that no Work owns. Work grouping (several agents
// and stages in one unit, helper folding) waits for the daemon-side board
// (docs/proposals/WEB_HOME.md, phase 2), so it is written once for every
// client instead of a second time here.

import { checksOf, decidedCard, isSystemAsk, safeHref } from "./needs-model.mjs";


export const FINISHED_WINDOW_MS = 24 * 60 * 60 * 1000;
export const LANES = ["needsYou", "running", "finished"];
const RUNNING = new Set(["working", "starting"]);
const WAITING = new Set(["waiting_input", "waiting_choice"]);
/// tendd's own asks group last, under their own heading.
export const TEND_PROJECT = "\u0000tend";

/// A registry row with no pane: a headless run (a script or bridge driving
/// an agent CLI without a terminal) whose hooks still reach tendd. The TUIs
/// hide these by default (`[watch] hide_paneless`), and so does the board.
export function isPaneless(agent) {
  return !agent?.pane;
}

/// An agent needs the operator when it waits on input or a choice, or failed.
export function needsAttention(agent) {
  return WAITING.has(agent?.state) || agent?.state === "error";
}

function firstLine(text) {
  return String(text || "").split("\n").find((line) => line.trim())?.trim() || "";
}

export function repoOf(cwd) {
  const parts = String(cwd || "").split(/[\\/]/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : "";
}

function instant(iso) {
  const at = Date.parse(iso || "");
  return Number.isFinite(at) ? at : 0;
}

/// FNV-1a of the alias, modulo the eight identity gradients (TendTheme).
export function hostHue(alias) {
  let hash = 0x811c9dc5;
  for (const byte of new TextEncoder().encode(String(alias || ""))) {
    hash ^= byte;
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash % 8;
}

function hostsOf(fleet) {
  return Array.isArray(fleet?.hosts) ? fleet.hosts : [];
}

/// A host's live agents, each with `project_path`: its pane's current path
/// first, then its recorded cwd, as the Mac's projection takes the repo
/// (FleetBoard.work(for:)). `cwd` stays what the agent recorded, which is
/// what a question's asker is grouped by (FleetDecisions.attachAskers).
function agentsOf(host) {
  const agents = Array.isArray(host?.remote?.agents) ? host.remote.agents : [];
  const panes = Array.isArray(host?.remote?.panes) ? host.remote.panes : [];
  const pathOf = new Map(panes.map((pane) => [`${pane.socket || ""}\u0000${pane.pane_id}`, pane.current_path]));
  return agents
    .filter((agent) => !isPaneless(agent) && agent.state !== "stopped")
    .map((agent) => ({ ...agent, project_path: pathOf.get(`${agent.tmux_socket || ""}\u0000${agent.pane}`) || agent.cwd || null }));
}

/// Which product an agent is, for its mark: the logos Tend for Mac ships.
export function productOf(kind) {
  const k = String(kind || "").toLowerCase();
  if (k.startsWith("claude")) return "claude";
  if (k.startsWith("codex") || k.startsWith("openai")) return "openai";
  return null;
}

/// A prompt as a card title (FleetBoard.promptTitle): tend's delivery
/// notices and Claude's background-task blocks read as what they are.
export function promptTitle(prompt) {
  const text = String(prompt || "").trimStart();
  if (!text) return "";
  if (text.startsWith("<task-notification>")) {
    const summary = text.match(/<summary>([\s\S]*?)<\/summary>/);
    const line = summary ? firstLine(summary[1]) : "";
    return line ? `⟳ ${line}` : { key: "backgroundTask" };
  }
  if (!text.startsWith("[tend:") || !text.includes("]")) return firstLine(text);
  const rest = text.slice(text.indexOf("]") + 1).trim();
  const line = rest.split("\n")[0];
  const reply = line.match(/(?:Completed )?reply from (\S+)/i);
  if (reply) return { key: "replyReady", who: reply[1] };
  return firstLine(line.split(".")[0]);
}

/// The title the Mac gives an agent on its own: its AI title, else its
/// prompt as a title, else "<kind> on <host>".
function agentTitle(agent, host) {
  return firstLine(agent.ai_title) || promptTitle(agent.last_prompt) || { key: "kindOnHost", kind: kindName(agent.kind), host: host.alias };
}

/// What the agent is doing or did last (FleetBoard.statusLine).
function statusLine(agent) {
  const line = firstLine(agent.recap) || firstLine(agent.last_response);
  return line ? { text: line } : { key: "state", state: agent.state };
}

function kindName(kind) {
  const k = String(kind || "agent");
  return k === "claude_code" ? "claude" : k;
}

/// Index of agents by session id and by host+pane, so a question or a
/// waiting agent from the queue finds its repo and title.
function agentIndex(fleet) {
  const bySession = new Map();
  const byPane = new Map();
  for (const host of hostsOf(fleet)) {
    for (const agent of agentsOf(host)) {
      const entry = { agent, host };
      if (agent.agent_session_id) bySession.set(`${host.alias}\u0000${agent.agent_session_id}`, entry);
      if (agent.pane) byPane.set(`${host.alias}\u0000${agent.tmux_socket || ""}\u0000${agent.pane}`, entry);
      if (agent.pane) byPane.set(`${host.alias}\u0000\u0000${agent.pane}`, entry);
    }
  }
  return { bySession, byPane };
}

function findAgent(index, host, sessionId, pane, socket) {
  return (
    (sessionId && index.bySession.get(`${host}\u0000${sessionId}`)) ||
    (pane && index.byPane.get(`${host}\u0000${socket || ""}\u0000${pane}`)) ||
    (pane && index.byPane.get(`${host}\u0000\u0000${pane}`)) ||
    null
  );
}

/// An evidence item as a chip: icon, text, and a link only for http(s).
export function evidenceChip(item) {
  switch (item?.kind) {
    case "pull_request": {
      const match = String(item.url || "").match(/github\.com\/([^/]+\/[^/]+)\/pull\/(\d+)/);
      return { icon: "pr", text: match ? `${match[1]}#${match[2]}` : String(item.url || ""), href: safeHref(item.url) };
    }
    case "commit":
      return { icon: "commit", text: String(item.sha || "").slice(0, 7) + (item.repo ? ` ${repoOf(item.repo)}` : "") };
    case "diff":
      return { icon: "diff", text: String(item.range || "") };
    case "test":
      return {
        icon: item.passed === true ? "test-pass" : item.passed === false ? "test-fail" : "test",
        text: String(item.command || ""),
        tone: item.passed === true ? "pass" : item.passed === false ? "fail" : "",
      };
    case "path":
      return { icon: "doc", text: repoOf(item.path) || String(item.path || "") };
    default:
      return null;
  }
}

function decisionFields(decision, verification) {
  const checks = checksOf(verification);
  const evidence = Array.isArray(decision?.evidence) ? decision.evidence : [];
  return {
    checks,
    options: Array.isArray(decision?.options) ? decision.options.map(String) : [],
    recommendation: decision?.recommendation ? String(decision.recommendation) : null,
    evidence: evidence
      .filter((item) => !(checks && item?.kind === "test" && item.verified === true))
      .map(evidenceChip)
      .filter(Boolean),
  };
}

/// Why a question can no longer be answered (`subject_check_v1`):
/// `ended`, `replaced`, `pane_gone` or `agent_exited`, and since when.
/// Null for a live question, and for anything an older daemon sends.
export function unanswerableOf(item) {
  const u = item?.unanswerable;
  if (!u || typeof u !== "object" || !u.episode) return null;
  return { cause: String(u.cause || ""), episode: String(u.episode), since: u.since || null, pane: u.pane || null };
}

/// A question or a result to review: the Mac's decision card.
function decisionUnit(item, host, index) {
  const remote = !host.local;
  const control = host.control !== false;
  if (item.kind === "human_request") {
    const system = isSystemAsk(item);
    const found = findAgent(index, host.alias, item.from_session_id, item.from_pane, null);
    const needed = item.decision?.needed ? String(item.decision.needed) : "";
    return {
      type: "decision",
      decision: "question",
      lane: "needsYou",
      id: item.request_id,
      host: host.alias,
      remote,
      actionable: !remote || control,
      system,
      asker: item.from_alias ? `@${item.from_alias}` : item.from_pane || (system ? "tend" : null),
      // tendd names nobody: the page says "an agent" in its language.
      askerMissing: !item.from_alias && !item.from_pane && !system,
      action: item.action || null,
      title: needed || String(item.body || ""),
      detail: needed ? String(item.body || "") : "",
      since: item.created_at || null,
      project: system ? TEND_PROJECT : repoOf(found?.agent?.cwd),
      pane: remote ? null : item.from_pane || null,
      socket: remote ? null : found?.agent?.tmux_socket || null,
      agentKind: found?.agent?.kind || null,
      unanswerable: unanswerableOf(item),
      // Keep and Close all, here or on a Fleet host whose section says it
      // takes them (docs/proposals/FLEET_UNANSWERABLE.md).
      keepable: host.subject_check === true,
      closable: host.close_unanswerable === true,
      ...decisionFields(item.decision, null),
    };
  }
  if (item.kind === "review") {
    const target = item.target || {};
    const found = target.kind === "session" ? findAgent(index, host.alias, target.session_id, null, null) : null;
    const needed = item.decision?.needed ? String(item.decision.needed) : "";
    return {
      type: "decision",
      decision: "review",
      lane: "needsYou",
      id: item.id,
      host: host.alias,
      remote,
      actionable: !remote || (control && host.decides_reviews === true),
      system: false,
      asker: target.alias || (target.kind === "work" ? `${target.workspace}/${target.work}` : found ? kindName(found.agent.kind) : ""),
      title: needed || String(item.summary || ""),
      detail: needed ? String(item.summary || "") : "",
      since: item.requested_at || null,
      project: repoOf(found?.agent?.cwd) || (target.kind === "work" ? String(target.workspace || "") : ""),
      ...decisionFields(item.decision, item.verification),
    };
  }
  return null;
}

/// One agent as a one-stage unit.
function agentUnit(agent, host, lane, overrides = {}) {
  const stage = lane === "needsYou" ? "needsYou" : lane === "running" ? "running" : "done";
  return {
    type: "work",
    lane,
    id: `${host.alias}:${agent.agent_session_id || agent.pane || ""}`,
    host: host.alias,
    remote: !host.local,
    title: agentTitle(agent, host),
    headline: statusLine(agent),
    stages: [{ name: kindName(agent.kind), state: stage }],
    agentKind: agent.kind || null,
    product: productOf(agent.kind),
    project: repoOf(agent.project_path || agent.cwd),
    since: agent.state_entered_at || agent.last_activity_at || null,
    pane: agent.pane || null,
    socket: agent.tmux_socket || null,
    agents: [{ kind: agent.kind, name: kindName(agent.kind), product: productOf(agent.kind), state: agent.state, host: host.alias, pane: host.local ? agent.pane || null : null, socket: host.local ? agent.tmux_socket || null : null, session: agent.agent_session_id || null, since: agent.state_entered_at || null, owner: `${kindName(agent.kind)} · ${host.alias}`, remote: !host.local, status: agent.awaiting_reply ? { key: "status", status: "asked" } : null }],
    ...overrides,
  };
}

/// A waiting agent from the queue (it stopped on a question in its pane).
function waitingUnit(item, host, index) {
  const found = findAgent(index, host.alias, item.session_id, item.pane, item.tmux_socket);
  const agent = found?.agent || { kind: item.agent_kind, pane: item.pane, tmux_socket: item.tmux_socket, state: item.state };
  const message = firstLine(item.message);
  return agentUnit(agent, host, "needsYou", {
    id: `${host.alias}:${item.session_id || item.pane || ""}`,
    headline: message ? { text: String(item.message) } : { key: item.state === "waiting_choice" ? "genericChoice" : "genericAsk" },
    since: item.since || agent.state_entered_at || null,
    pane: host.local ? item.pane || agent.pane || null : null,
    socket: host.local ? item.tmux_socket || agent.tmux_socket || null : null,
    waiting: item.state || null,
    remote: !host.local,
  });
}

/// A decision already made, shown in Finished.
function decidedUnit(item, host, now) {
  const card = decidedCard(item, now);
  if (!card) return null;
  if (item.kind === "closed_by_tend") {
    // tend closed one of its own items, with why (`?closures=true`).
    return {
      type: "decided",
      lane: "finished",
      id: `${item.kind}:${card.id}`,
      host: host.alias,
      remote: host.local !== true,
      decision: "question",
      title: item.decision?.needed ? String(item.decision.needed) : String(card.title || ""),
      outcome: "closedByTend",
      reason: String(item.reason || ""),
      reasonText: String(item.reason_text || item.reason || ""),
      subjectGone: item.subject_gone ? String(item.subject_gone) : null,
      note: "",
      by: null,
      since: item.closed_at || null,
      project: "",
    };
  }
  const disposition = item.kind === "review" ? item.disposition || {} : null;
  const outcome = item.kind === "review" ? disposition.kind || "decided" : item.status === "completed" ? "answered" : item.status || "answered";
  return {
    type: "decided",
    lane: "finished",
    id: `${item.kind}:${card.id}`,
    host: host.alias,
    remote: host.local !== true,
    decision: item.kind === "review" ? "review" : "question",
    title: item.decision?.needed ? String(item.decision.needed) : String(card.title || ""),
    outcome,
    note: item.kind === "review" ? firstLine(disposition.note) : firstLine(item.answer),
    by: disposition?.by?.label || null,
    since: (item.kind === "review" ? disposition.at : item.answered_at) || null,
    project: "",
    // A person closed it from Can't be answered: why (the asker's cause).
    closure: item.kind === "human_answer" && item.closure ? { cause: String(item.closure.cause || ""), surface: item.closure.surface || null, by: item.closure.by || "person" } : null,
  };
}

/// Your turn's folded group, **Can't be answered**: questions whose asker
/// is gone (`subject_check_v1`), out of Your turn and its count. The count
/// is the hosts' exact `unanswerable_count` (their lists may be cut), never
/// less than what is listed; under a project filter only what is listed
/// can be counted. Close all offers exactly the listed pairs, one host at a
/// time, on each host that allows it (`close_unanswerable`); a host whose
/// folded questions cannot be kept or closed from here says why.
export function cantAnswerGroup(units, sections = [], filter = {}) {
  const listed = units.filter((u) => u.unanswerable);
  let count = listed.length;
  if (!filter.project) {
    const exact = sections
      .filter((s) => !filter.host || s.alias === filter.host)
      .reduce((sum, s) => sum + (Number(s?.queue?.unanswerable_count ?? s?.unanswerable_count) || 0), 0);
    count = Math.max(count, exact);
  }
  const hosts = [...new Set(listed.map((u) => u.host))];
  const closeAll = [];
  const notes = [];
  for (const host of hosts) {
    const theirs = listed.filter((u) => u.host === host);
    const remote = theirs.some((u) => u.remote);
    const closable = theirs.filter((u) => u.closable && u.actionable !== false);
    if (closable.length) closeAll.push({ host, remote, count: closable.length, payload: closeAllPayload(closable) });
    if (!remote) continue;
    const section = sections.find((s) => s.alias === host) || {};
    const reason = cantAnswerReason(section);
    if (reason) notes.push({ host, reason });
  }
  return { count, units: listed, remote: listed.some((u) => u.remote), closeAll, notes };
}

/// Why another host's folded questions cannot be kept, or closed, from
/// here: an observe-only host, a relay that needs an upgrade, or Close all
/// turned off on that host. `null` when both work.
export function cantAnswerReason(section) {
  if (section.control === false) return "observeOnly";
  if (section.subject_check !== true) return "upgrade";
  if (section.close_unanswerable !== true) return "closeOff";
  return null;
}

/// `POST /api/decisions/keep`: this question, in this episode, back in
/// Your turn.
export function keepPayload(unit) {
  const payload = { request_id: unit.id, episode: unit.unanswerable.episode };
  return unit.remote ? { ...payload, host: unit.host } : payload;
}

/// `POST /api/decisions/close-unanswerable`: exactly these pairs.
export function closeAllPayload(units) {
  const payload = { items: units.map((u) => ({ request_id: u.id, episode: u.unanswerable.episode })), by_label: "web" };
  const remote = units.find((u) => u.remote);
  return remote ? { ...payload, host: remote.host } : payload;
}

function ordered(hosts) {
  return [...hosts].sort(
    (a, b) => Number(Boolean(b.local)) - Number(Boolean(a.local)) || String(a.alias || "").localeCompare(String(b.alias || "")),
  );
}

/// Groups a lane's units by project, the way the Mac does (FleetBoard
/// .byProject): a project sits where its first unit is, so the lane's own
/// order (the longest wait, the latest change) still decides which comes
/// first; units without one form "No project"; tend's own asks come last.
export function groupByProject(units) {
  const groups = new Map();
  for (const unit of units) {
    const key = unit.project || "";
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(unit);
  }
  const all = [...groups.entries()].map(([project, list]) => ({ project, units: list }));
  return [...all.filter((g) => g.project !== TEND_PROJECT), ...all.filter((g) => g.project === TEND_PROJECT)];
}

/// The whole board. `filter` narrows to one host and/or one project.
export function buildBoard({ decisions = null, history = null, fleet = null, now = Date.now(), filter = {}, window = FINISHED_WINDOW_MS } = {}) {
  const index = agentIndex(fleet);
  const lanes = { needsYou: [], running: [], finished: [] };
  const notes = { needsYou: [], running: [], finished: [] };
  const queueHosts = Array.isArray(decisions?.hosts) ? decisions.hosts : [];

  for (const host of ordered(queueHosts)) {
    for (const item of host.queue?.items || []) {
      const unit = item?.kind === "agent_waiting" ? waitingUnit(item, host, index) : decisionUnit(item, host, index);
      if (unit) lanes.needsYou.push(unit);
    }
    if (host.queue?.truncated) notes.needsYou.push({ key: "more", host: host.alias });
    for (const source of host.unavailable || []) notes.needsYou.push({ key: "unavailable", host: host.alias, what: source });
    if (!host.local && host.control === false) notes.needsYou.push({ key: "observeOnly", host: host.alias });
  }

  // Agents the queue already lists as waiting are not also "in progress".
  const waitingIds = new Set(lanes.needsYou.filter((u) => u.type === "work").map((u) => u.id));
  for (const host of ordered(hostsOf(fleet))) {
    for (const agent of agentsOf(host)) {
      const id = `${host.alias}:${agent.agent_session_id || agent.pane || ""}`;
      if (waitingIds.has(id)) continue;
      if (RUNNING.has(agent.state) && !needsAttention(agent) && agent.awaiting_reply == null) {
        lanes.running.push(agentUnit(agent, host, "running"));
      } else if (agent.state === "idle" && agent.awaiting_reply == null && (agent.last_response || agent.last_prompt)) {
        const at = instant(agent.state_entered_at || agent.last_activity_at);
        if (at && now - at <= window && at - now <= 60_000) lanes.finished.push(agentUnit(agent, host, "finished"));
        else if (at && now - at > window) lanes.finished.push({ ...agentUnit(agent, host, "finished"), older: true });
      }
    }
    if (!host.local && host.state && host.state !== "online") {
      notes.running.push({ key: "hostState", host: host.alias, state: host.state, error: host.error || null });
    }
  }

  for (const host of ordered(Array.isArray(history?.hosts) ? history.hosts : [])) {
    for (const item of host.history?.items || []) {
      const unit = decidedUnit(item, host, now);
      if (!unit) continue;
      const at = instant(unit.since);
      if (at && now - at > window) unit.older = true;
      lanes.finished.push(unit);
    }
  }

  // Your turn: the longest wait first; a decision before a unit on a tie.
  lanes.needsYou.sort((a, b) => instant(a.since) - instant(b.since) || (a.type === "decision" ? -1 : 1) - (b.type === "decision" ? -1 : 1));
  lanes.running.sort((a, b) => instant(b.since) - instant(a.since));
  lanes.finished.sort((a, b) => instant(b.since) - instant(a.since));

  const projects = [...new Set(LANES.flatMap((lane) => lanes[lane].map((u) => u.project)).filter((p) => p && p !== TEND_PROJECT))].sort();
  const keep = (unit) => (!filter.host || unit.host === filter.host) && (!filter.project || unit.project === filter.project);
  const result = {};
  for (const lane of LANES) {
    const all = lanes[lane].filter(keep);
    const visible = all.filter((u) => !u.unanswerable);
    result[lane] = {
      count: visible.filter((u) => !u.older).length,
      groups: groupByProject(visible.filter((u) => !u.older)),
      older: visible.filter((u) => u.older),
      notes: notes[lane],
    };
    if (lane === "needsYou") result[lane].cantAnswer = cantAnswerGroup(all, queueHosts, filter);
  }
  return {
    lanes: result,
    projects,
    hosts: hostList(fleet, decisions),
    agentCount: hostsOf(fleet).reduce((sum, host) => sum + agentsOf(host).length, 0),
    empty: LANES.every((lane) => lanes[lane].length === 0),
  };
}

/// Every host the page knows, for the Hosts button and its popover.
export function hostList(fleet, decisions) {
  const seen = new Map();
  for (const host of hostsOf(fleet)) {
    seen.set(host.alias, { alias: host.alias, local: Boolean(host.local), state: host.local ? "online" : host.state || "unknown", error: host.error || null, hue: hostHue(host.alias) });
  }
  for (const host of Array.isArray(decisions?.hosts) ? decisions.hosts : []) {
    if (!seen.has(host.alias)) seen.set(host.alias, { alias: host.alias, local: Boolean(host.local), state: "online", error: null, hue: hostHue(host.alias) });
  }
  return ordered([...seen.values()]);
}

/// Which lane a decision ends up in once decided: none (it leaves the queue).
export function laneCounts(board) {
  return Object.fromEntries(LANES.map((lane) => [lane, board.lanes[lane].count]));
}

// ---- The daemon's board (GET /api/home) ---------------------------------
//
// tendd computes the board once for every client (crates/tend/src/
// home_board.rs, Tend for Mac's FleetBoard rules: Work units with stages,
// helpers folded, usage-cap coverage, reviewed marks). This turns it into
// the shape the page draws; `buildBoard` above stays the fallback for a
// daemon without the read or a proxy that does not forward it.

const STAGE = { needs_you: "needsYou", running: "running", done: "done", waiting: "waiting" };
const LANE = { needs_you: "needsYou", running: "running", finished: "finished" };

function serverAgent(ref) {
  return {
    kind: ref?.kind || null,
    name: ref?.name || null,
    product: productOf(ref?.kind),
    state: ref?.state || null,
    status: ref?.status || null,
    owner: ref?.owner || null,
    host: ref?.host || null,
    session: ref?.session_id || null,
    since: ref?.state_entered_at || null,
    needsAttention: ref?.needs_attention === true,
    pane: ref?.local ? ref?.pane || null : null,
    socket: ref?.local ? ref?.socket || null : null,
    remote: !ref?.local,
  };
}

function serverUnit(item) {
  // The daemon's focus, null included: a blocked stage nobody is on has
  // no agent to answer, and no other agent stands in for it.
  const focus = item.focus || null;
  const lane = LANE[item.lane] || "running";
  return {
    type: "work",
    lane,
    id: item.id,
    source: item.source || null,
    focus: focus ? serverAgent(focus) : null,
    host: focus?.host || item.host_aliases?.[0] || item.agents?.[0]?.host || "",
    hosts: item.host_aliases || [],
    remote: focus ? !focus.local : !(item.agents || []).some((a) => a.local),
    title: item.title,
    headline: item.headline,
    stages: (item.stages || []).map((stage) => ({ name: stage.name, owner: stage.owner, state: STAGE[stage.state] || "waiting", kind: stage.kind })),
    agentKind: focus?.kind || null,
    product: productOf(item.agent_kinds?.[0] || focus?.kind),
    products: [...new Set((item.agent_kinds || []).map(productOf).filter(Boolean))],
    project: item.repo || "",
    since: item.shown_time || item.since || null,
    pane: focus?.local ? focus.pane || null : null,
    socket: focus?.local ? focus.socket || null : null,
    agents: (item.agents || []).map(serverAgent),
    helpers: (item.helpers || []).map(serverAgent),
    handledLimit: item.handled_limit === true,
    orphanHelper: item.orphan_helper === true,
    probableHelper: item.probable_helper === true,
    emptySession: item.empty_session === true,
    recent: item.recent !== false,
    reviewed: item.reviewed === true,
    reviewKey: item.review_key || null,
    requestedByPerson: item.requested_by_person === true,
    // What Review shows: what was asked and what came back (FleetReviewDetail).
    review: item.review ? { asked: item.review.asked || null, result: item.review.result || null, resultCut: item.review.result_truncated === true } : null,
  };
}

function serverDecision(item, sections) {
  const section = sections.find((s) => s.alias === item.host) || {};
  const host = {
    alias: item.host,
    local: item.local === true,
    control: section.control !== false,
    decides_reviews: section.decides_reviews === true,
    subject_check: section.subject_check === true,
    close_unanswerable: section.close_unanswerable === true,
  };
  const unit = decisionUnit(item.entry || {}, host, agentIndex(null));
  if (!unit) return null;
  // The asker as the daemon found it: its pane on its own socket.
  const asker = item.asker && item.asker.local ? { pane: item.asker.pane || unit.pane, socket: item.asker.socket || null } : null;
  return {
    ...unit,
    project: item.system ? TEND_PROJECT : item.repo || "",
    actionable: item.actionable !== false,
    system: item.system === true,
    pane: asker ? asker.pane : unit.pane,
    socket: asker ? asker.socket : unit.socket,
  };
}

/// The daemon's board in the page's shape, with what was decided (the
/// history) after the finished units, and the folds the Mac applies.
export function fromServerBoard(server, { history = null, filter = {}, now = Date.now(), window = FINISHED_WINDOW_MS, showReviewed = false } = {}) {
  const sections = Array.isArray(server?.decision_hosts) ? server.decision_hosts : [];
  const lanes = { needsYou: [], running: [], finished: [] };
  for (const [key, lane] of Object.entries(LANE)) {
    for (const group of server?.lanes?.[key]?.groups || []) {
      for (const item of group.items || []) {
        const unit = item.type === "decision" ? serverDecision(item, sections) : serverUnit(item);
        if (unit) lanes[lane].push(unit);
      }
    }
  }
  const notes = { needsYou: [], running: [], finished: [] };
  for (const section of sections) {
    if (section.truncated) notes.needsYou.push({ key: "more", host: section.alias });
    for (const source of section.unavailable || []) notes.needsYou.push({ key: "unavailable", host: section.alias, what: source });
    if (!section.local && section.control === false) notes.needsYou.push({ key: "observeOnly", host: section.alias });
  }
  for (const host of server?.hosts || []) {
    if (!host.local && host.state && host.state !== "online") notes.running.push({ key: "hostState", host: host.alias, state: host.state, error: host.error || null });
  }
  for (const host of ordered(Array.isArray(history?.hosts) ? history.hosts : [])) {
    for (const item of host.history?.items || []) {
      const unit = decidedUnit(item, host, now);
      if (!unit) continue;
      const at = instant(unit.since);
      if (at && now - at > window) unit.older = true;
      lanes.finished.push(unit);
    }
  }
  const keep = (unit) =>
    (!filter.host || unit.host === filter.host || unit.hosts?.includes(filter.host)) && (!filter.project || unit.project === filter.project);
  const result = {};
  for (const lane of LANES) {
    const all = lanes[lane].filter(keep);
    const visible = all.filter((u) => !u.unanswerable);
    // The Mac's folds, outside Your turn: possible helpers, empty
    // sessions, then older than a day; reviewed units hide behind
    // "Show N reviewed".
    const folds = { helpers: [], empty: [], older: [], reviewed: [] };
    const shown = [];
    for (const unit of visible) {
      if (lane === "needsYou") shown.push(unit);
      else if (unit.reviewed && !showReviewed) folds.reviewed.push(unit);
      else if (unit.probableHelper) folds.helpers.push(unit);
      else if (unit.emptySession) folds.empty.push(unit);
      else if (unit.recent === false || unit.older) folds.older.push(unit);
      else shown.push(unit);
    }
    result[lane] = {
      count: visible.length - folds.reviewed.length,
      groups: groupByProject(shown),
      older: folds.older,
      folds,
      notes: notes[lane],
    };
    if (lane === "needsYou") result[lane].cantAnswer = cantAnswerGroup(all, sections, filter);
  }
  const reviewedCount = result.finished.folds.reviewed.length;
  return {
    server: true,
    lanes: result,
    projects: Array.isArray(server?.projects) ? server.projects : [],
    hosts: ordered((server?.hosts || []).map((h) => ({ alias: h.alias, local: Boolean(h.local), state: h.local ? "online" : h.state || "unknown", error: h.error || null, hue: hostHue(h.alias) }))),
    agentCount: Number(server?.agent_count) || 0,
    // Hosts registered in observe mode: nothing is sent to their agents.
    observeHosts: sections.filter((s) => !s.local && s.control === false).map((s) => s.alias),
    backlog: (Array.isArray(server?.backlog) ? server.backlog : []).filter((b) => !filter.project || b.project === filter.project),
    reviewedCount,
    empty: LANES.every((lane) => lanes[lane].length === 0) && !(server?.backlog || []).length,
  };
}

/// Units Review lists (FleetReview.reviewQueue): finished, asked for by a
/// person, not yet marked, changed since the scope's start; newest first.
export function reviewQueue(board, scope = "day", now = Date.now()) {
  const start =
    scope === "all" ? -Infinity :
    scope === "week" ? now - 7 * 86400_000 :
    scope === "today" ? new Date(new Date(now).setHours(0, 0, 0, 0)).getTime() :
    now - 86400_000;
  const all = [
    ...board.lanes.finished.groups.flatMap((g) => g.units),
    ...board.lanes.finished.older,
    ...Object.values(board.lanes.finished.folds || {}).flat(),
  ];
  const seen = new Set();
  return all
    .filter((u) => u.type === "work" && u.requestedByPerson && !u.reviewed && !seen.has(u.id) && seen.add(u.id))
    .filter((u) => (instant(u.since) || -Infinity) >= start)
    .sort((a, b) => instant(b.since) - instant(a.since));
}


/// What the person may type to a unit's agent, and to whom
/// (FleetBoard.reply(for:) and FleetReplyTarget): an answer to an agent
/// asking or waiting for input in Your turn, or the next instruction to an
/// agent done with its turn in Finished. A choice dialog, an error or a
/// working agent is handled in the pane.
export function replyTarget(unit) {
  // The daemon names a focus (null for nobody); a unit built here is one agent.
  const focus = "focus" in unit ? unit.focus : unit.agents?.length === 1 ? unit.agents[0] : null;
  if (!focus || !focus.pane || focus.remote) return null;
  const asked = focus.status?.status === "asked" || (focus.state === "idle" && unit.lane === "needsYou");
  let kind = null;
  if (unit.lane === "needsYou" && (asked || focus.state === "waiting_input")) kind = "answer";
  else if (unit.lane === "finished" && (focus.state === "idle" || focus.state === "done")) kind = "sendBack";
  if (!kind) return null;
  return { kind, session: focus.session || null, state: focus.state || null, since: focus.since || null, owner: focus.owner || focus.name || focus.kind || "", pane: focus.pane, socket: focus.socket || null };
}

/// A draft's target is still the agent at the same question.
export function sameTarget(a, b) {
  return Boolean(
    a && b && a.kind === b.kind && a.session === b.session && a.state === b.state && a.since === b.since && a.pane === b.pane && a.socket === b.socket,
  );
}

/// Review's queue with the results put off with Later at the end, in the
/// order they were put off (FleetReviewView.queue).
export function withLater(queue, later = []) {
  const deferred = new Set(later);
  return [...queue.filter((u) => !deferred.has(u.id)), ...later.map((id) => queue.find((u) => u.id === id)).filter(Boolean)];
}

/// Units by project, in the order each project first appears
/// (FleetBoard.byProject).
export function byProject(units) {
  const groups = new Map();
  for (const unit of units) {
    const key = unit.project || "";
    if (!groups.has(key)) groups.set(key, { project: unit.project || null, units: [] });
    groups.get(key).units.push(unit);
  }
  return [...groups.values()];
}

/// A few plain lines of Markdown (FleetQuickLookText.digest): marks
/// trimmed, rules dropped, joined by spaces and cut at a sentence end past
/// the middle, else at a space, with an ellipsis.
export function digest(text, limit = 320) {
  if (!text) return null;
  const lines = String(text).split(/\r?\n/).map((raw) => {
    let line = raw.trim();
    while (line && "#>*-`".includes(line[0])) line = line.slice(1).trim();
    return line;
  }).filter((line) => line && ![...line].every((c) => "-=*_`|".includes(c)));
  const joined = lines.join(" ");
  if (!joined) return null;
  const chars = [...joined];
  if (chars.length <= limit) return joined;
  const head = chars.slice(0, limit);
  let stop = -1;
  for (let i = head.length - 1; i >= 0; i--) if (".!?。".includes(head[i])) { stop = i; break; }
  if (stop > limit / 2) return head.slice(0, stop + 1).join("");
  const space = head.lastIndexOf(" ");
  if (space >= 0) return `${head.slice(0, space).join("")}…`;
  return `${head.join("")}…`;
}

/// What the daemon's board says about each agent at a usage cap, keyed by
/// host and session: whether it still needs the person, which is whether
/// a limit policy covers it (AgentStatus.needsAttention). Only caps: every
/// other status is read from the live agent, which may be newer than the
/// board.
export function boardAttention(server) {
  if (!server?.lanes) return null;
  const attention = new Map();
  for (const lane of Object.values(server.lanes)) {
    for (const group of lane?.groups || []) {
      for (const item of group.items || []) {
        for (const ref of [...(item.agents || []), ...(item.helpers || []), ...(item.focus ? [item.focus] : [])]) {
          if (!ref?.host || !ref.session_id || typeof ref.needs_attention !== "boolean") continue;
          if (ref.status?.status !== "limited") continue;
          attention.set(`${ref.host}\u0000${ref.session_id}`, { needs: ref.needs_attention, until: ref.status.until || null });
        }
      }
    }
  }
  return attention;
}
