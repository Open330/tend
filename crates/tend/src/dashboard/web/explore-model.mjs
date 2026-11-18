// Explore, as Tend for Mac's Explore sidebar lists it (SidebarViews.swift):
// every host's panes, agent panes and shell panes, by host tree or by
// status group, with a filter for all / needing attention / active, and
// the panes that need you first, in topology order or by recency (the
// Mac's `ExploreSort`). Pure: no DOM; the page draws it.

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
        windowIndex: String(pane.window_index ?? ""),
        windowName: pane.window_name || "",
        paneIndex: String(pane.pane_index ?? ""),
        agentAlias: pane.agent_alias || "",
        windowId: pane.window_id || "",
        command: pane.current_command || "",
        repo: repoOf(pane.current_path || agent?.cwd || ""),
        path: pane.current_path || "",
        title: agent ? firstLine(agent.ai_title) || firstLine(agent.last_prompt) || kindName(agent.kind) : pane.title || pane.current_command || pane.pane_id,
        agent: agent
          ? {
              kind: agent.kind,
              product: productOf(agent.kind),
              state: agent.state,
              session: agent.agent_session_id,
              aiTitle: agent.ai_title || "",
              promptAt: agent.last_prompt_at || null,
              activityAt: agent.last_activity_at || null,
            }
          : null,
        status,
        group: groupOf(status),
        attention: groupOf(status) === "attention",
        since: agent?.state_entered_at || agent?.last_activity_at || null,
      });
    }
  }
  return rows;
}

/// The orders Explore offers, as the Mac's `ExploreSort` (its raw values,
/// which are also what `tend.explore.sort` stores). Topology is the default.
export const EXPLORE_SORTS = ["topology", "recent", "myPrompt", "agentActivity"];
export const DEFAULT_EXPLORE_SORT = "topology";

/// A stored or chosen order, or the default for anything else.
export function exploreSortOf(value) {
  return EXPLORE_SORTS.includes(value) ? value : DEFAULT_EXPLORE_SORT;
}

// `Date.distantPast`: what a pane without the time sorts as.
const PAST = -Infinity;
const ISO = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:?\d{2})$/i;

/// An ISO 8601 time with its zone (fractional seconds or not), or the
/// distant past, as the Mac's `parsedDate`.
function parsedDate(value) {
  if (typeof value !== "string" || !ISO.test(value)) return PAST;
  const time = Date.parse(value);
  return Number.isFinite(time) ? time : PAST;
}

/// The time a pane sorts by (`activityDate`): none for a shell pane or in
/// topology order; for "Latest activity" the later of your prompt and the
/// agent's update.
export function activityTime(row, sort) {
  const agent = row?.agent;
  if (!agent) return PAST;
  switch (sort) {
    case "recent":
      return Math.max(parsedDate(agent.promptAt), parsedDate(agent.activityAt));
    case "myPrompt":
      return parsedDate(agent.promptAt);
    case "agentActivity":
      return parsedDate(agent.activityAt);
    default:
      return PAST;
  }
}

function latestTime(rows, sort) {
  let latest = PAST;
  for (const row of rows) latest = Math.max(latest, activityTime(row, sort));
  return latest;
}

// `localizedStandardCompare`: Finder order, numbers by value, case aside.
const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
const standard = (a, b) => collator.compare(String(a ?? ""), String(b ?? ""));
// `Int(text) ?? Int.max`.
const indexOf = (text) => (/^[+-]?\d+$/.test(String(text ?? "")) ? Number(text) : Number.MAX_SAFE_INTEGER);

/// The newer first, when the order is by time and the times differ; 0
/// otherwise, for the topology tie-breaks to decide.
function byTime(a, b, sort) {
  if (sort === DEFAULT_EXPLORE_SORT || a === b) return 0;
  return a > b ? -1 : 1;
}

/// Panes (`sortedPanes`): by time, then this host first, host alias,
/// session name, window index, pane index, pane id.
export function comparePanes(a, b, sort = DEFAULT_EXPLORE_SORT) {
  return (
    byTime(activityTime(a, sort), activityTime(b, sort), sort) ||
    (a.host !== b.host ? Number(Boolean(b.local)) - Number(Boolean(a.local)) || standard(a.host, b.host) : 0) ||
    (a.session !== b.session ? standard(a.session, b.session) : 0) ||
    indexOf(a.windowIndex) - indexOf(b.windowIndex) ||
    indexOf(a.paneIndex) - indexOf(b.paneIndex) ||
    standard(a.pane, b.pane)
  );
}

/// Windows: by their newest pane, then window index, then name.
function compareWindows(a, b, sort) {
  return byTime(latestTime(a.rows, sort), latestTime(b.rows, sort), sort) || indexOf(a.index) - indexOf(b.index) || standard(a.name, b.name);
}

/// Sessions: by their newest pane, then name.
function compareSessions(a, b, sort) {
  return byTime(latestTime(a.rows, sort), latestTime(b.rows, sort), sort) || standard(a.name, b.name);
}

/// Hosts: by their newest pane, then this host first, then alias.
function compareHosts(a, b, sort) {
  return (
    byTime(latestTime(a.rows, sort), latestTime(b.rows, sort), sort) ||
    Number(Boolean(b.local)) - Number(Boolean(a.local)) ||
    standard(a.title, b.title)
  );
}

/// What the filter field matches, as the Mac's Explore filter: the pane's
/// host alias, session, window name, pane id, command, path, agent alias,
/// or its agent's title or session id, ignoring case. A session or window
/// shows while one of its panes matches.
export function paneMatches(row, query) {
  if (!query) return true;
  const needle = String(query).toLocaleLowerCase();
  return [row.host, row.session, row.windowName, row.pane, row.command, row.path, row.agentAlias, row.agent?.aiTitle, row.agent?.session].some(
    (field) => field != null && String(field).toLocaleLowerCase().includes(needle),
  );
}

// The Mac's `statusPriority`, for the needing-attention panes on top.
const PRIORITY = { error: 0, needs_input: 1, asked: 2, limited: 3, working: 4, idle: 5 };
const priorityOf = (row) => (row.agent ? (PRIORITY[row.status] ?? 7) : 8);

/// How many needing-attention panes the Mac shows over the host tree.
export const PINNED_ATTENTION_LIMIT = 5;

/// The sidebar: an attention section first (host tree only), then either
/// hosts → sessions → windows → panes, or the status groups, each in the
/// chosen order. `empty` says why nothing is listed: "none" when no host
/// has a pane, "noMatch" when the filter hides them all.
export function exploreSections(fleet, { show = "all", query = "", grouping = "host", sort = DEFAULT_EXPLORE_SORT } = {}) {
  sort = exploreSortOf(sort);
  const every = paneRows(fleet);
  const rows = every
    .filter((row) => paneMatches(row, query))
    // Active is the working panes only (SidebarViews `.active`).
    .filter((row) => show === "all" || (show === "attention" ? row.attention : row.group === "active"))
    .sort((a, b) => comparePanes(a, b, sort));
  const filtering = Boolean(query) || show !== "all";
  const empty = every.length === 0 ? "none" : rows.length === 0 ? "noMatch" : null;
  const base = { total: rows.length, all: every.length, filtering, empty, sort };
  const hosts = Array.isArray(fleet?.hosts) ? fleet.hosts : [];
  if (grouping === "status") {
    return {
      ...base,
      attention: [],
      sections: STATUS_GROUPS.map((group) => ({ key: group, kind: "status", title: group, rows: rows.filter((row) => row.group === group) })).filter((s) => s.rows.length),
    };
  }
  const sections = hosts.map((host) => {
    const mine = rows.filter((row) => row.host === host.alias);
    // host → session → window → pane, the Mac's execution topology.
    const sessions = [];
    // Identities include the tmux server: two servers can each have a
    // session "tend" with a window @1 (WorkModels keys them the same way).
    for (const row of mine) {
      const sessionKey = `${row.socket || ""}\u0000${row.session}`;
      let session = sessions.find((s) => s.key === sessionKey);
      if (!session) sessions.push((session = { key: sessionKey, name: row.session, socket: row.socket, rows: [], windows: [] }));
      session.rows.push(row);
      const windowKey = `${sessionKey}\u0000${row.windowId || row.window}`;
      let window = session.windows.find((w) => w.key === windowKey);
      if (!window) session.windows.push((window = { key: windowKey, id: row.windowId || row.window, index: row.windowIndex, name: row.windowName, rows: [] }));
      window.rows.push(row);
    }
    for (const session of sessions) session.windows.sort((a, b) => compareWindows(a, b, sort));
    sessions.sort((a, b) => compareSessions(a, b, sort));
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
  });
  sections.sort((a, b) => compareHosts(a, b, sort));
  return {
    ...base,
    // Only while no status filter is on, as on the Mac; errors first, then
    // the waiting, each in the chosen order.
    attention: show === "all" ? rows.filter((row) => row.attention).sort((a, b) => priorityOf(a) - priorityOf(b)) : [],
    // A host with no pane stays listed (its state is worth seeing) until a
    // filter is on.
    sections: sections.filter((section) => section.rows.length || !filtering),
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
