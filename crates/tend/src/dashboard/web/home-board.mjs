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
import { isPaneless, needsAttention } from "./fleet-view.mjs";

export const FINISHED_WINDOW_MS = 24 * 60 * 60 * 1000;
export const LANES = ["needsYou", "running", "finished"];
const RUNNING = new Set(["working", "starting"]);
/// tendd's own asks group last, under their own heading.
export const TEND_PROJECT = "\u0000tend";

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

/// A host's live agents. An agent without a recorded cwd takes its pane's
/// current path, as the Mac's projection does, so it still has a project.
function agentsOf(host) {
  const agents = Array.isArray(host?.remote?.agents) ? host.remote.agents : [];
  const panes = Array.isArray(host?.remote?.panes) ? host.remote.panes : [];
  const pathOf = new Map(panes.map((pane) => [`${pane.socket || ""}\u0000${pane.pane_id}`, pane.current_path]));
  return agents
    .filter((agent) => !isPaneless(agent) && agent.state !== "stopped")
    .map((agent) => (agent.cwd ? agent : { ...agent, cwd: pathOf.get(`${agent.tmux_socket || ""}\u0000${agent.pane}`) || null }));
}

/// Which product an agent is, for its mark: the logos Tend for Mac ships.
export function productOf(kind) {
  const k = String(kind || "").toLowerCase();
  if (k.startsWith("claude")) return "claude";
  if (k.startsWith("codex") || k.startsWith("openai")) return "openai";
  return null;
}

function agentTitle(agent) {
  return (
    firstLine(agent.ai_title) ||
    firstLine(agent.recap) ||
    firstLine(agent.last_prompt) ||
    firstLine(agent.last_notification) ||
    ""
  );
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
      asker: item.from_alias ? `@${item.from_alias}` : item.from_pane || (system ? "tend" : "an agent"),
      action: item.action || null,
      title: needed || String(item.body || ""),
      detail: needed ? String(item.body || "") : "",
      since: item.created_at || null,
      project: system ? TEND_PROJECT : repoOf(found?.agent?.cwd),
      pane: remote ? null : item.from_pane || null,
      socket: remote ? null : found?.agent?.tmux_socket || null,
      agentKind: found?.agent?.kind || null,
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
  const said = firstLine(agent.last_response);
  const headline =
    lane === "running"
      ? { key: "state", state: agent.state }
      : lane === "finished"
        ? { text: said || firstLine(agent.last_prompt) }
        : { text: "" };
  return {
    type: "work",
    lane,
    id: `${host.alias}:${agent.agent_session_id || agent.pane || ""}`,
    host: host.alias,
    remote: !host.local,
    title: agentTitle(agent) || kindName(agent.kind),
    headline,
    stages: [{ name: kindName(agent.kind), state: stage }],
    agentKind: agent.kind || null,
    product: productOf(agent.kind),
    project: repoOf(agent.cwd),
    since: agent.state_entered_at || agent.last_activity_at || null,
    pane: agent.pane || null,
    socket: agent.tmux_socket || null,
    agents: [{ kind: agent.kind, name: kindName(agent.kind), product: productOf(agent.kind), state: agent.state, host: host.alias, pane: agent.pane || null, socket: agent.tmux_socket || null }],
    limited: agent.state === "limited" || agent.usage_limit != null,
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
  };
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
    const visible = lanes[lane].filter(keep);
    result[lane] = {
      count: visible.filter((u) => !u.older).length,
      groups: groupByProject(visible.filter((u) => !u.older)),
      older: visible.filter((u) => u.older),
      notes: notes[lane],
    };
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
