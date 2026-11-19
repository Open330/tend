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
  // Blocked waits on the person like a question (AgentStatus: needsInput).
  if (ATTENTION.has(state)) return state === "waiting_input" || state === "waiting_choice" || state === "blocked" ? "needs_input" : "error";
  if (ACTIVE.has(state)) return "working";
  if (state === "idle") return "idle";
  return state || "other";
}

/// The Mac's sidebar status groups, in their order.
export const STATUS_GROUPS = ["attention", "limited", "active", "idle", "shell"];

/// The sidebar's status group (SidebarViews `statusBucket`): an agent that
/// asked sits with the idle ones; it is pinned through `attention`.
function groupOf(status) {
  if (status === "needs_input" || status === "error") return "attention";
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

/// Whether an agent needs the person (AgentStatus.needsAttention): an
/// error, a question, input it waits for, or a usage cap no limit policy
/// covers. Whether a policy covers a cap is the daemon's to say
/// (`attention`, its board's word on each capped agent); it counts only
/// for the same cap (same reset time) the live agent shows, and without it
/// a cap counts, as on a Mac whose daemon sent no rules.
function needsAttention(status, attention, key, agent) {
  if (status !== "limited") return status === "error" || status === "needs_input" || status === "asked";
  const word = attention?.get(key);
  if (!word) return true;
  const live = agent?.rate_limited_until || null;
  return (word.until || null) !== live ? true : word.needs;
}

/// One row per pane on every host, with the agent in it if any.
export function paneRows(fleet, attention = null) {
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
        sessionId: pane.session_id || "",
        // The Work identity tmux records on a managed pane (an old
        // `#workspace=` link names a session or such a workspace).
        workspaceId: pane.tend?.workspace_id || "",
        workId: pane.tend?.work_id || "",
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
              lastResponse: agent.last_response || "",
              recap: agent.recap || "",
              lastNotification: agent.last_notification || "",
              lastPrompt: agent.last_prompt || "",
              subagents: Array.isArray(agent.subagents) ? agent.subagents.length : 0,
              processes: Number(agent.workload?.process_count) || 0,
            }
          : null,
        status,
        group: groupOf(status),
        attention: agent ? needsAttention(status, attention, `${host.alias}\u0000${agent.agent_session_id || ""}`, agent) : false,
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
///
/// `workspace` ({ workspace, socket }, the old workbench's `#workspace=` and
/// `&wsocket=`) narrows it to that workspace's session; `empty` is then
/// "noWorkspace" when no live session matches, so a stale link never
/// silently shows everything.
export function exploreSections(fleet, { show = "all", query = "", grouping = "host", sort = DEFAULT_EXPLORE_SORT, attention = null, workspace = null } = {}) {
  sort = exploreSortOf(sort);
  const all = paneRows(fleet, attention);
  const every = workspace?.workspace ? all.filter((row) => inWorkspace(row, workspace)) : all;
  const rows = every
    .filter((row) => paneMatches(row, query))
    // Active is the working panes only (SidebarViews `.active`).
    .filter((row) => show === "all" || (show === "attention" ? row.attention : row.group === "active"))
    .sort((a, b) => comparePanes(a, b, sort));
  const filtering = Boolean(query) || show !== "all" || Boolean(workspace?.workspace);
  const empty = all.length === 0 ? "none" : every.length === 0 ? "noWorkspace" : rows.length === 0 ? "noMatch" : null;
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

// ---- Session and window detail -------------------------------------------
//
// Selecting a session or a window opens its detail, as the Mac's
// FleetSessionDetailView and FleetWindowDetailView (DetailViews.swift): its
// name, host, window and pane counts, and its panes as cards. An identity
// carries the host and the tmux server, so the same names on two servers
// never collide.

/// The tmux server's short name: old workbench links wrote the
/// socket file's name, the Fleet reports its path.
function shortSocket(socket) {
  return String(socket || "default").split("/").pop() || "default";
}

/// Whether a pane's socket is the one a link names (any, when it names
/// none). A full path must match exactly: two servers can share a file
/// name in different directories. Only a bare name (the short form
/// the old workbench's `socket` and `wsocket` keys wrote) matches
/// by file name.
export function sameSocket(have, want) {
  if (!want) return true;
  if (have === want) return true;
  if (String(want).includes("/")) return false;
  return shortSocket(have) === want;
}

/// Whether a pane belongs to the workspace a `#workspace=` link names: its
/// tmux session by name, or the managed workspace recorded on the pane.
export function inWorkspace(row, filter) {
  if (!filter?.workspace) return true;
  return (row.session === filter.workspace || row.workspaceId === filter.workspace) && sameSocket(row.socket, filter.socket);
}

function hashParams(hash) {
  return new URLSearchParams(String(hash || "").replace(/^#/, ""));
}

/// What Explore's address selects: a pane, a window or a session (a pane
/// wins over a window, a window over a session, as the old workbench read
/// them), or null.
///   `#pane=%1&socket=…&host=…`
///   `#window=@3&session=…&socket=…&host=…` (or `window=<session>:<index>`)
///   `#session=…&socket=…&host=…`
export function selectionFromHash(hash) {
  const params = hashParams(hash);
  const socket = params.get("socket") || null;
  const host = params.get("host") || null;
  const pane = params.get("pane");
  if (pane) return { kind: "pane", pane, socket, host };
  const window = params.get("window");
  const session = params.get("session") || null;
  if (window) return { kind: "window", window, session, socket, host };
  if (session) return { kind: "session", session, socket, host };
  return null;
}

/// `#workspace=…&wsocket=…`: the workspace Explore is narrowed to. Its
/// socket is its own (`wsocket`), apart from the selection's `socket`.
export function workspaceFromHash(hash) {
  const params = hashParams(hash);
  const workspace = params.get("workspace");
  return workspace ? { workspace, socket: params.get("wsocket") || null } : null;
}

/// Explore's address for a selection, keeping the workspace filter.
export function exploreHash(selection, workspace = null) {
  const params = new URLSearchParams();
  if (selection?.kind === "pane") params.set("pane", selection.pane);
  else if (selection?.kind === "window") {
    params.set("window", selection.window);
    if (selection.session) params.set("session", selection.session);
  } else if (selection?.kind === "session") params.set("session", selection.session);
  if (selection?.kind && selection.socket) params.set("socket", selection.socket);
  if (selection?.kind && selection.host) params.set("host", selection.host);
  if (workspace?.workspace) {
    params.set("workspace", workspace.workspace);
    if (workspace.socket) params.set("wsocket", workspace.socket);
  }
  const text = params.toString();
  return text ? `#${text}` : "";
}

/// The selection a pane row, a session or a window of a host section
/// names. This host's are written without a host, as paneHash writes them.
export function paneSelection(row) {
  return { kind: "pane", pane: row.pane, socket: row.socket || null, host: row.local ? null : row.host };
}

export function sessionSelection(section, session) {
  return { kind: "session", session: session.name, socket: session.socket || null, host: section.local ? null : section.title };
}

export function windowSelection(section, session, window) {
  // A tmux window id (`@3`) when the Fleet has one, else `session:index`
  // as the old workbench wrote it.
  const id = String(window.id || "").startsWith("@") ? window.id : `${session.name}:${window.index}`;
  return { kind: "window", window: id, session: session.name, socket: session.socket || null, host: section.local ? null : section.title };
}

/// The tree's key for a session or a window of a host section (explore.mjs
/// marks the open one, and folds by it).
export function treeKey(section, node) {
  return `${section.title}\u0000${node.key}`;
}

function present(text) {
  const value = String(text || "").trim();
  return value || null;
}

/// A pane as a detail card (WindowAgentReportCard): its agent and status,
/// and the latest thing it answered, recapped, noted or was asked.
export function paneCard(row) {
  const agent = row.agent;
  const latest = agent ? present(agent.lastResponse) || present(agent.recap) || present(agent.lastNotification) || present(agent.lastPrompt) : null;
  return {
    id: row.id,
    row,
    title: row.title || row.pane,
    pane: row.pane,
    status: row.status,
    group: row.group,
    state: agent?.state || null,
    product: agent?.product || null,
    command: row.command,
    latest,
    activityAt: agent?.activityAt || agent?.promptAt || null,
    attention: row.attention,
  };
}

/// The Mac's `panePriority`: what needs you, then the working, other
/// agents, then shells; the latest activity first within each.
function comparePriority(a, b) {
  const rank = (row) => (row.attention ? 0 : row.status === "working" ? 1 : row.agent ? 2 : 3);
  const time = (row) => String(row.agent?.activityAt || "");
  return rank(a) - rank(b) || (time(a) === time(b) ? 0 : time(a) > time(b) ? -1 : 1);
}

function cardsOf(rows) {
  return [...rows].sort(comparePriority).map(paneCard);
}

/// `workspace / work` when every pane that has one runs the same Work (the
/// Mac's `workIdentity`), else null.
function workLabel(rows) {
  const ids = new Set(rows.filter((row) => row.workspaceId && row.workId).map((row) => `${row.workspaceId} / ${row.workId}`));
  return ids.size === 1 ? [...ids][0] : null;
}

function hostOf(section) {
  return { host: section.title, local: section.local, hue: section.hue };
}

export function sessionDetail(section, session) {
  return {
    kind: "session",
    key: treeKey(section, session),
    ...hostOf(section),
    name: session.name,
    sessionId: session.rows.find((row) => row.sessionId)?.sessionId || "",
    socket: session.socket || null,
    windowCount: session.windows.length,
    paneCount: session.rows.length,
    selection: sessionSelection(section, session),
    windows: session.windows.map((window) => ({
      key: treeKey(section, window),
      id: window.id,
      index: window.index,
      name: window.name,
      paneCount: window.rows.length,
      work: workLabel(window.rows),
      selection: windowSelection(section, session, window),
      cards: cardsOf(window.rows),
    })),
  };
}

export function windowDetail(section, session, window) {
  const rows = window.rows;
  const agents = rows.filter((row) => row.agent);
  return {
    kind: "window",
    key: treeKey(section, window),
    sessionKey: treeKey(section, session),
    ...hostOf(section),
    name: window.name,
    id: window.id,
    index: window.index,
    session: session.name,
    sessionId: rows.find((row) => row.sessionId)?.sessionId || "",
    socket: session.socket || null,
    paneCount: rows.length,
    work: workLabel(rows),
    selection: windowSelection(section, session, window),
    sessionSelection: sessionSelection(section, session),
    // The Mac's window metrics.
    metrics: {
      agents: agents.length,
      working: agents.filter((row) => row.status === "working").length,
      attention: rows.filter((row) => row.attention).length,
      subagents: agents.reduce((sum, row) => sum + (row.agent.subagents || 0), 0),
      processes: agents.reduce((sum, row) => sum + (row.agent.processes || 0), 0),
    },
    cards: cardsOf(rows),
  };
}

/// The detail a session or window selection opens in the current Fleet: a
/// window, or its session when the window is gone (as the workbench fell back,
/// marked `fallback`), or `{ kind: "gone" }` when neither is live. Null for
/// a pane or nothing.
export function exploreDetail(fleet, selection, { sort = DEFAULT_EXPLORE_SORT, attention = null } = {}) {
  if (!selection || (selection.kind !== "session" && selection.kind !== "window")) return null;
  const { sections } = exploreSections(fleet, { show: "all", sort, attention });
  // A link without a host names this host's (paneHash, sessionSelection).
  const section = sections.find((s) => s.kind === "host" && (selection.host ? s.title === selection.host : s.local));
  const gone = { kind: "gone", selection };
  if (!section) return gone;
  const sessions = section.sessions.filter((session) => sameSocket(session.socket, selection.socket));
  let name = selection.session;
  if (selection.kind === "window") {
    for (const session of sessions) {
      if (name && session.name !== name) continue;
      const window = session.windows.find((w) => w.id === selection.window || `${session.name}:${w.index}` === selection.window);
      if (window) return windowDetail(section, session, window);
    }
    // `session:index` names its session too.
    if (!name && !String(selection.window).startsWith("@")) {
      const cut = String(selection.window).lastIndexOf(":");
      if (cut > 0) name = String(selection.window).slice(0, cut);
    }
  }
  const session = name ? sessions.find((s) => s.name === name) : null;
  if (session) return { ...sessionDetail(section, session), fallback: selection.kind === "window" };
  return gone;
}
