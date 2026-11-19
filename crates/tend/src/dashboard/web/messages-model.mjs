// Explore → Messages, the collaboration reader, as data: what a link names
// (`/explore/messages#…`), the /api/collaboration query a reader state
// asks for, and the scope a pane or session in Explore gives it. Pure: no
// DOM, no fetch; explore-messages.mjs draws it. Old `#message=` links reach
// it through the shell's link adapter (legacy-link.mjs).

import { requestsTouchingPane, roomKey, scopeCollaborationRequests } from "./collaboration-model.mjs";
import { baseSessionName } from "./session-model.mjs";

export const MESSAGES_PATH = "/explore/messages";
export const PAGE_SIZE = 500;
export const RANGES = ["1h", "24h", "7d", "30d", "all", "custom"];
export const KINDS = ["review", "question", "task", "notice"];
export const STATUSES = ["queued", "claimed", "completed", "blocked", "declined", "failed", "expired", "cancelled"];
export const TABS = ["messages", "graph", "sequence"];
const DEFAULT_RANGE = "7d";

/// A reader with nothing chosen: the last week, every kind and status, the
/// Messages tab, this host, no scope.
export function readerState(overrides = {}) {
  return {
    tab: "messages",
    range: DEFAULT_RANGE,
    since: "",
    until: "",
    q: "",
    work: "",
    thread: "",
    kind: "",
    status: "",
    /// A linked message: the history narrows to it (the "show all" button
    /// widens it again).
    message: "",
    /// The message open in the reader, within whatever the filters show.
    open: "",
    /// A room filter: the RoomId the server matches.
    room: null,
    /// The Fleet host whose daemon holds these messages ("" = this one).
    host: "",
    /// `{ session, socket, workspace, pane }` or null for everything.
    scope: null,
    ...overrides,
    range: overrides.range || defaultRange(overrides),
  };
}

/// A link to one message or thread reaches all retained history unless it
/// says otherwise: its pane may be long gone.
function defaultRange(state) {
  return state.message || state.thread ? "all" : DEFAULT_RANGE;
}

function oneOf(value, allowed, fallback = "") {
  return allowed.includes(value) ? value : fallback;
}

function parseRoom(raw) {
  if (!raw) return null;
  try {
    const room = JSON.parse(raw);
    if (room && typeof room === "object" && !Array.isArray(room) && typeof room.window_id === "string") {
      const clean = { host: typeof room.host === "string" ? room.host : "tmux", window_id: room.window_id };
      if (typeof room.socket === "string" && room.socket) clean.socket = room.socket;
      return clean;
    }
  } catch {
    // Not a room: ignored.
  }
  return null;
}

/// `#message=…&thread=…&host=…&session=…&socket=…&workspace=…&pane=…` plus
/// the filters and the tab: what `/explore/messages` shows. Anything else
/// in the fragment is ignored.
export function messagesFromHash(hash) {
  const params = new URLSearchParams(String(hash || "").replace(/^#/, ""));
  const get = (key) => (params.get(key) || "").trim();
  const scope = { session: get("session"), socket: get("socket"), workspace: get("workspace"), pane: get("pane") };
  const state = readerState({
    tab: oneOf(get("tab"), TABS, "messages"),
    since: get("since"),
    until: get("until"),
    q: get("q"),
    work: get("work"),
    thread: get("thread"),
    kind: oneOf(get("kind"), KINDS),
    status: oneOf(get("status"), STATUSES),
    message: get("message"),
    open: get("open"),
    room: parseRoom(params.get("room")),
    host: get("host"),
    scope: scope.session || scope.workspace || scope.pane ? clean(scope) : null,
  });
  state.range = oneOf(get("range"), RANGES, defaultRange(state));
  if (state.range !== "custom") {
    state.since = "";
    state.until = "";
  }
  if (!state.open && state.message) state.open = state.message;
  return state;
}

function clean(scope) {
  const out = {};
  for (const key of ["session", "socket", "workspace", "pane"]) if (scope[key]) out[key] = scope[key];
  return out;
}

/// The fragment for a reader state, defaults left out.
export function messagesHash(state) {
  const s = readerState(state);
  const params = new URLSearchParams();
  const put = (key, value) => {
    if (value) params.set(key, value);
  };
  put("message", s.message);
  put("thread", s.thread);
  put("host", s.host);
  for (const key of ["session", "socket", "workspace", "pane"]) put(key, s.scope?.[key]);
  if (s.tab !== "messages") put("tab", s.tab);
  if (s.range !== defaultRange(s)) put("range", s.range);
  if (s.range === "custom") {
    put("since", s.since);
    put("until", s.until);
  }
  put("q", s.q);
  put("work", s.work);
  put("kind", s.kind);
  put("status", s.status);
  if (s.room) put("room", JSON.stringify(s.room));
  if (s.open && s.open !== s.message) put("open", s.open);
  const text = params.toString();
  return text ? `#${text}` : "";
}

export function messagesLink(state = {}) {
  return `${MESSAGES_PATH}${messagesHash(state)}`;
}

/// "" when the custom period can be asked for, else why not: "missing"
/// (a bound is empty or unreadable) or "order" (start after end).
export function customRangeError(state) {
  if (state.range !== "custom") return "";
  const start = Date.parse(state.since || "");
  const end = Date.parse(state.until || "");
  if (!Number.isFinite(start) || !Number.isFinite(end)) return "missing";
  return start > end ? "order" : "";
}

/// The /api/collaboration query for a reader state (`append` continues from
/// the last page's cursor). `{ params: null, error }` when the custom
/// period is not yet a period.
export function collaborationParams(state, { append = false, pagination = null } = {}) {
  const params = new URLSearchParams({ limit: String(PAGE_SIZE) });
  const error = customRangeError(state);
  if (error) return { params: null, error };
  if (state.range === "custom") {
    params.set("since", new Date(state.since).toISOString());
    params.set("until", new Date(state.until).toISOString());
  } else if (state.range) params.set("since", state.range);
  if (state.q) params.set("q", state.q);
  if (state.message) params.set("id", state.message);
  if (state.work) {
    // A Work id is qualified by its workspace (requestWorkId): `ws/ENG-1`.
    const slash = state.work.indexOf("/");
    if (slash > 0) {
      params.set("workspace", state.work.slice(0, slash));
      params.set("work", state.work.slice(slash + 1));
    } else {
      params.set("work", state.work);
    }
  }
  if (state.thread) params.set("thread", state.thread);
  if (state.kind) params.set("kind", state.kind);
  if (state.status) params.set("status", state.status);
  if (state.room) params.set("room", JSON.stringify(state.room));
  const scope = state.scope;
  if (scope?.session) params.set("session", baseSessionName(scope.session));
  if (scope?.socket) params.set("socket", scope.socket);
  if (scope?.workspace && !scope.session) params.set("workspace", scope.workspace);
  if (append && pagination) {
    if (pagination.next_cursor) params.set("cursor", pagination.next_cursor);
    else if (pagination.next_offset != null) params.set("offset", String(pagination.next_offset));
  }
  return { params, error: "" };
}

/// Pages merged by request id, oldest first.
export function mergeCollaborationRequests(current, incoming) {
  const byId = new Map();
  for (const request of [...(current || []), ...(incoming || [])]) byId.set(request.id, request);
  return [...byId.values()].sort((left, right) =>
    String(left.created_at || "").localeCompare(String(right.created_at || ""))
  );
}

/// Only http(s) links an agent attached are made clickable.
export function safeExternalHref(value, base = globalThis.location?.origin) {
  if (!value) return "";
  try {
    const url = new URL(value, base);
    return url.protocol === "http:" || url.protocol === "https:" ? url.href : "";
  } catch {
    return "";
  }
}

// ---- Hosts and scope -------------------------------------------------------

export function localHostAlias(fleet) {
  const hosts = Array.isArray(fleet?.hosts) ? fleet.hosts : [];
  return hosts.find((host) => host.local)?.alias || null;
}

/// Whether the messages a link names are this daemon's: "local", "remote"
/// (another Fleet host's: request ids repeat across daemons, so they are
/// never looked up here), or "unknown" (the link names a host and this
/// page has no Fleet to say which host it is).
export function hostCheck(state, fleet) {
  if (!state.host) return "local";
  const alias = localHostAlias(fleet);
  if (!alias) return "unknown";
  return alias === state.host ? "local" : "remote";
}

function shortSocket(socket) {
  return String(socket || "default").split("/").pop() || "default";
}

/// The live pane scan as collaboration-model.mjs participantSession wants
/// it: this host's pane id (and socket) → its session. Ambiguous ids (two
/// tmux servers, no socket recorded) resolve to nothing.
export function paneResolver(fleet) {
  const host = (Array.isArray(fleet?.hosts) ? fleet.hosts : []).find((h) => h.local);
  const panes = Array.isArray(host?.remote?.panes) ? host.remote.panes : [];
  return (paneId, socket) => {
    let candidates = panes.filter((pane) => pane.pane_id === paneId);
    if (socket) candidates = candidates.filter((pane) => shortSocket(pane.socket) === shortSocket(socket));
    if (candidates.length !== 1) return null;
    return { session: baseSessionName(candidates[0].session), socket: shortSocket(candidates[0].socket) };
  };
}

/// The loaded requests inside a reader scope. The server already narrowed
/// by session and socket; this keeps an older page out of a new scope and
/// narrows a pane scope to that pane's own messages. A pane that has
/// closed still matches by the session its requests recorded.
export function scopeReaderRequests(requests, scope, resolve = () => null) {
  if (!scope) return requests || [];
  const scoped = scope.session || scope.workspace
    ? scopeCollaborationRequests(requests, { session: scope.session || "", socket: scope.socket || "", workspace: scope.workspace || "" }, resolve)
    : requests || [];
  return scope.pane ? requestsTouchingPane(scoped, scope.pane, scope.socket || null) : scoped;
}

/// The reader scoped to one Explore pane row (explore-model.mjs paneRows),
/// its host kept so a link from it never opens another daemon's messages.
export function paneMessagesState(row) {
  return readerState({
    host: row.local ? "" : row.host,
    scope: clean({ session: row.session, socket: row.socket || "", pane: row.pane }),
  });
}

/// The reader scoped to a tmux session on a host (explore-model.mjs
/// sections: host → session).
export function sessionMessagesState({ host, local, session, socket }) {
  return readerState({ host: local ? "" : host, scope: clean({ session, socket: socket || "" }) });
}

/// The Explore pane link for a participant whose pane is live on this host.
export function participantPaneLink(participant, fleet) {
  if (!participant || participant.console || !participant.pane) return null;
  const host = (Array.isArray(fleet?.hosts) ? fleet.hosts : []).find((h) => h.local);
  const socket = participant.socket || participant.room?.socket || null;
  const pane = (host?.remote?.panes || []).find((p) =>
    p.pane_id === participant.pane && (!socket || shortSocket(p.socket) === shortSocket(socket))
  );
  if (!pane) return null;
  const params = new URLSearchParams({ pane: pane.pane_id });
  if (pane.socket) params.set("socket", pane.socket);
  return `/explore#${params}`;
}

/// Where sign-in brings a messages link back to, with only the reader's own
/// keys (never a token).
export function messagesReturnPath(hash) {
  return messagesLink(messagesFromHash(hash));
}

/// Two reader states that ask the server the same thing (the tab and the
/// open message are only how it is shown).
export function sameQuery(left, right) {
  const strip = (state) => messagesHash({ ...state, tab: "messages", open: "" });
  return strip(left) === strip(right);
}

export function roomKeyOf(state) {
  return state.room ? roomKey(state.room) : "";
}
