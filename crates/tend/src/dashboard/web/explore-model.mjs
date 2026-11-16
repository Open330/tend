// Explore, as Tend for Mac's Explore sidebar lists it (SidebarViews.swift):
// every host's panes, agent panes and shell panes, by host tree or by
// status group, with a filter for all / needing attention / active, and
// the panes that need you first. Pure: no DOM; the page draws it.

import { hostHue, repoOf, productOf } from "./home-board.mjs";

const ATTENTION = new Set(["waiting_input", "waiting_choice", "error", "blocked", "failed"]);
const ACTIVE = new Set(["working", "starting", "running"]);

function statusOf(agent) {
  if (!agent) return "shell";
  const state = agent.state || "";
  if (agent.rate_limit_scope && ["idle", "error", "failed"].includes(state)) {
    const until = Date.parse(agent.rate_limited_until || "");
    if (!Number.isFinite(until) || until > Date.now()) return "limited";
  }
  if (state === "idle" && agent.awaiting_reply) return "asked";
  if (ATTENTION.has(state)) return state === "waiting_input" || state === "waiting_choice" ? "needs_input" : "error";
  if (ACTIVE.has(state)) return "working";
  if (state === "idle") return "idle";
  return state || "other";
}

/// The Mac's sidebar status groups, in their order.
export const STATUS_GROUPS = ["attention", "limited", "active", "idle", "shell"];

function groupOf(status) {
  if (status === "needs_input" || status === "error" || status === "asked") return "attention";
  if (status === "limited") return "limited";
  if (status === "working") return "active";
  if (status === "shell") return "shell";
  return "idle";
}

function kindName(kind) {
  const k = String(kind || "agent");
  return k === "claude_code" ? "claude" : k.replaceAll("_", " ");
}

function firstLine(text) {
  return String(text || "").split("\n").find((line) => line.trim())?.trim() || "";
}

/// One row per pane on every host, with the agent in it if any.
export function paneRows(fleet) {
  const rows = [];
  for (const host of Array.isArray(fleet?.hosts) ? fleet.hosts : []) {
    const remote = host.remote || {};
    const agents = Array.isArray(remote.agents) ? remote.agents.filter((a) => a.pane && a.state !== "stopped") : [];
    const byPane = new Map();
    for (const agent of agents) byPane.set(`${agent.tmux_socket || ""}\u0000${agent.pane}`, agent);
    for (const pane of Array.isArray(remote.panes) ? remote.panes : []) {
      const agent = byPane.get(`${pane.socket || ""}\u0000${pane.pane_id}`) || agents.find((a) => a.pane === pane.pane_id && !a.tmux_socket) || null;
      const status = statusOf(agent);
      rows.push({
        id: `${host.alias}:${pane.socket || ""}:${pane.pane_id}`,
        host: host.alias,
        local: Boolean(host.local),
        hostState: host.local ? "online" : host.state || "unknown",
        hue: hostHue(host.alias),
        pane: pane.pane_id,
        socket: pane.socket || null,
        session: pane.session || "",
        window: `${pane.window_index ?? ""}:${pane.window_name || ""}`,
        windowId: pane.window_id || "",
        command: pane.current_command || "",
        repo: repoOf(pane.current_path || agent?.cwd || ""),
        path: pane.current_path || "",
        title: agent ? firstLine(agent.ai_title) || firstLine(agent.last_prompt) || kindName(agent.kind) : pane.title || pane.current_command || pane.pane_id,
        agent: agent ? { kind: agent.kind, product: productOf(agent.kind), state: agent.state, session: agent.agent_session_id } : null,
        status,
        group: groupOf(status),
        attention: groupOf(status) === "attention",
        since: agent?.state_entered_at || agent?.last_activity_at || null,
      });
    }
  }
  return rows;
}

function matches(row, query) {
  if (!query) return true;
  const needle = query.toLowerCase();
  return [row.host, row.session, row.window, row.title, row.repo, row.command, row.agent?.kind].some((field) => String(field || "").toLowerCase().includes(needle));
}

/// The sidebar: an attention section first (host tree only), then either
/// hosts → sessions → panes, or the status groups.
export function exploreSections(fleet, { show = "all", query = "", grouping = "host" } = {}) {
  const rows = paneRows(fleet)
    .filter((row) => matches(row, query))
    .filter((row) => show === "all" || (show === "attention" ? row.attention : row.group === "active" || row.attention));
  const hosts = Array.isArray(fleet?.hosts) ? fleet.hosts : [];
  if (grouping === "status") {
    return {
      total: rows.length,
      attention: [],
      sections: STATUS_GROUPS.map((group) => ({ key: group, kind: "status", title: group, rows: rows.filter((row) => row.group === group) })).filter((s) => s.rows.length),
    };
  }
  const ordered = [...hosts].sort((a, b) => Number(Boolean(b.local)) - Number(Boolean(a.local)) || String(a.alias).localeCompare(String(b.alias)));
  return {
    total: rows.length,
    attention: rows.filter((row) => row.attention),
    sections: ordered.map((host) => {
      const mine = rows.filter((row) => row.host === host.alias);
      // host → session → window → pane, the Mac's execution topology.
      const sessions = [];
      for (const row of mine) {
        let session = sessions.find((s) => s.name === row.session);
        if (!session) sessions.push((session = { name: row.session, rows: [], windows: [] }));
        session.rows.push(row);
        let window = session.windows.find((w) => w.id === (row.windowId || row.window));
        if (!window) {
          const [index, ...name] = String(row.window).split(":");
          session.windows.push((window = { id: row.windowId || row.window, index, name: name.join(":"), rows: [] }));
        }
        window.rows.push(row);
      }
      return {
        key: host.alias,
        kind: "host",
        title: host.alias,
        local: Boolean(host.local),
        state: host.local ? "online" : host.state || "unknown",
        version: host.tend_version || "",
        latency: Number.isFinite(host.latency_ms) ? host.latency_ms : null,
        error: host.error || null,
        hue: hostHue(host.alias),
        sessions,
        rows: mine,
      };
    }).filter((section) => section.rows.length || !query),
  };
}

/// `#pane=<id>&socket=<socket>&host=<alias>`: the pane Explore shows.
export function paneFromHash(hash) {
  const params = new URLSearchParams(String(hash || "").replace(/^#/, ""));
  const pane = params.get("pane");
  return pane ? { pane, socket: params.get("socket") || null, host: params.get("host") || null } : null;
}

export function paneHash(row) {
  const params = new URLSearchParams({ pane: row.pane });
  if (row.socket) params.set("socket", row.socket);
  if (row.host && !row.local) params.set("host", row.host);
  return `#${params}`;
}
