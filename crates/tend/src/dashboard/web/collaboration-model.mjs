import { baseSessionName } from "./session-model.mjs";
// Pure collaboration projections used by the dashboard graph and sequence
// views. Keeping this module DOM-free makes the identity and aggregation rules
// independently testable without adding a frontend build step.

const SEP = "\u001f";

// Recipient and live request state determine attention, never sender/prose.
export function needsHumanResponse(request) {
  return request?.to?.console === true && request.expects_reply === true
    && request.kind !== "notice" && ["queued", "claimed"].includes(request.status);
}

export function normalizeCollaborationPayload(payload) {
  if (Array.isArray(payload)) {
    return {
      requests: payload,
      pagination: {
        total: payload.length,
        limit: payload.length,
        offset: 0,
        has_more: false,
        next_offset: null,
        next_cursor: null,
      },
      generated_at: null,
      details_included: true,
    };
  }
  const requests = Array.isArray(payload?.requests) ? payload.requests : [];
  const page = payload?.pagination || {};
  return {
    requests,
    pagination: {
      total: finiteNumber(page.total, requests.length),
      limit: finiteNumber(page.limit, requests.length),
      offset: finiteNumber(page.offset, 0),
      has_more: Boolean(page.has_more),
      next_offset: page.next_offset != null && Number.isFinite(Number(page.next_offset)) ? Number(page.next_offset) : null,
      next_cursor: typeof page.next_cursor === "string" ? page.next_cursor : null,
    },
    generated_at: payload?.generated_at || null,
    details_included: payload?.details_included !== false,
  };
}

function finiteNumber(value, fallback) {
  const number = Number(value);
  return Number.isFinite(number) && number >= 0 ? number : fallback;
}

export function roomKey(room) {
  if (!room) return "unknown-room";
  return [room.host || "tmux", room.socket || "default", room.window_id || "unknown"].join(SEP);
}

export function roomLabel(request) {
  const participant = requestAnchor(request);
  const room = participant.room || {};
  const session = baseSessionName(participant.tmux_session_name) || participant.tmux_session_id || room.host || "host";
  const window = participant.window_name || room.window_id || "room";
  const socket = room.socket ? ` · ${shortSocket(room.socket)}` : "";
  return `${session}:${window}${socket}`;
}

function shortSocket(value) {
  const parts = String(value).split("/");
  return parts[parts.length - 1] || "default";
}

export function participantIdentity(participant) {
  if (!participant) return "unknown";
  if (participant.console) return "console";
  const host = participant.room?.host || "tmux";
  const socket = participant.socket || participant.room?.socket || "default";
  const session = participant.agent_session_id || `pane:${participant.socket || "default"}:${participant.pane || "?"}`;
  return `${host}${SEP}${socket}${SEP}${session}`;
}

export function participantLabel(participant) {
  if (!participant) return "unknown";
  if (participant.console) return "console";
  return participant.alias || participant.roles?.[0] || participant.agent_kind || participant.pane || "agent";
}

export function participantSubtitle(participant) {
  if (!participant || participant.console) return "operator";
  const roles = (participant.roles || []).filter((role) => role !== participant.alias);
  if (roles.length) return roles.join(", ");
  return `${participant.agent_kind || "agent"} · ${participant.pane || "pane"}`;
}

export function requestRoomKey(request) {
  return roomKey(requestAnchor(request)?.room);
}

function requestAnchor(request) {
  if (request?.from?.console && request?.to) return request.to;
  return request?.from || request?.to || {};
}

export function requestWorkId(request) {
  const work = request?.work_id || request?.work?.work_id || request?.work?.id || "";
  const workspace = request?.workspace_id || request?.work?.workspace_id || "";
  return workspace && work ? `${workspace}/${work}` : work;
}

export function requestThreadId(request) {
  return request?.thread_id || "";
}

export function edgeIdentity(fromId, toId) {
  return `${fromId}${SEP}→${SEP}${toId}`;
}

export function projectCollaboration(requests) {
  const nodes = new Map();
  const edges = new Map();
  const rooms = new Map();
  const works = new Set();
  const threads = new Set();

  for (const request of requests || []) {
    if (!request?.from || !request?.to) continue;
    const from = addNode(nodes, request.from, request.created_at || "");
    const to = addNode(nodes, request.to, request.created_at || "");
    const key = edgeIdentity(from.id, to.id);
    if (!edges.has(key)) {
      edges.set(key, {
        key,
        from: from.id,
        to: to.id,
        count: 0,
        replyCount: 0,
        humanCount: 0,
        replyStatuses: {},
        kinds: {},
        statuses: {},
        rooms: new Set(),
        latestAt: "",
      });
    }
    const edge = edges.get(key);
    edge.count += 1;
    if (needsHumanResponse(request)) edge.humanCount += 1;
    increment(edge.kinds, request.kind || "unknown");
    increment(edge.statuses, request.status || "unknown");
    if (request.reply) {
      edge.replyCount += 1;
      increment(edge.replyStatuses, request.reply.status || request.status || "unknown");
    }
    const room = requestRoomKey(request);
    edge.rooms.add(room);
    if ((request.created_at || "") > edge.latestAt) edge.latestAt = request.created_at || "";
    if (!rooms.has(room)) {
      rooms.set(room, {
        key: room,
        room: requestAnchor(request)?.room || null,
        label: roomLabel(request),
        session: baseSessionName(requestAnchor(request)?.tmux_session_name),
        socket: requestAnchor(request)?.socket || requestAnchor(request)?.room?.socket || "",
        count: 0,
        latestAt: request.created_at || "",
      });
    }
    const roomEntry = rooms.get(room);
    roomEntry.count += 1;
    if ((request.created_at || "") > roomEntry.latestAt) {
      roomEntry.room = requestAnchor(request)?.room || null;
      roomEntry.label = roomLabel(request);
      roomEntry.latestAt = request.created_at || "";
    }
    const work = requestWorkId(request);
    const thread = requestThreadId(request);
    if (work) works.add(work);
    if (thread) threads.add(thread);
  }

  return {
    nodes: [...nodes.values()].sort((left, right) => left.label.localeCompare(right.label)),
    humanCount: (requests || []).filter(needsHumanResponse).length,
    edges: [...edges.values()].sort((left, right) => right.count - left.count || left.key.localeCompare(right.key)),
    rooms: [...rooms.values()].sort((left, right) => right.count - left.count || left.label.localeCompare(right.label)),
    works: [...works].sort(),
    threads: [...threads].sort(),
  };
}

function addNode(nodes, participant, at) {
  const id = participantIdentity(participant);
  if (!nodes.has(id)) {
    nodes.set(id, {
      id,
      participant,
      label: participantLabel(participant),
      subtitle: participantSubtitle(participant),
      latestAt: at,
    });
  } else {
    // Requests can arrive newest-first from the API or oldest-first after
    // paging is merged, so choose labels by timestamp rather than input order.
    const current = nodes.get(id);
    if ((!current.participant.alias && participant.alias) || at > current.latestAt) {
      current.participant = participant;
      current.label = participantLabel(participant);
      current.subtitle = participantSubtitle(participant);
      current.latestAt = at;
    }
  }
  return nodes.get(id);
}

function increment(counts, key) {
  counts[key] = (counts[key] || 0) + 1;
}

export function collaborationSequence(requests, { edgeKey = "", room = "" } = {}) {
  let selected = [...(requests || [])];
  if (room) selected = selected.filter((request) => requestRoomKey(request) === room);
  if (edgeKey) {
    selected = selected.filter((request) => edgeIdentity(
      participantIdentity(request.from),
      participantIdentity(request.to),
    ) === edgeKey);
  }
  return selected.sort((left, right) => {
    const byTime = String(left.created_at || "").localeCompare(String(right.created_at || ""));
    return byTime || String(left.id || "").localeCompare(String(right.id || ""));
  });
}

export function sequenceParticipants(requests) {
  const participants = new Map();
  for (const request of requests || []) {
    for (const participant of [request?.from, request?.to]) {
      if (!participant) continue;
      const id = participantIdentity(participant);
      if (!participants.has(id)) participants.set(id, participant);
    }
  }
  return [...participants.entries()].map(([id, participant]) => ({
    id,
    participant,
    label: participantLabel(participant),
    subtitle: participantSubtitle(participant),
  }));
}

/// The tmux session a participant's pane lives in, `{ session, socket }`, or
/// null. `resolvePane(paneId, socket)` is the live pane scan's answer; a
/// pane the scan no longer knows
/// falls back to the session name the request recorded. The operator console
/// and agents without a pane have no session.
export function participantSession(participant, resolvePane = () => null) {
  if (!participant || participant.console || !participant.pane) return null;
  const socket = participant.socket || participant.room?.socket || null;
  const live = resolvePane(participant.pane, socket);
  if (live) return live;
  const name = participant.tmux_session_name;
  return name ? { session: baseSessionName(name), socket: socket ? shortSocket(socket) : null } : null;
}

/// Requests inside a scope (`{ session, socket, workspace }`; `null` =
/// everything). For a session scope a
/// request is kept when its sender or recipient pane belongs to that session,
/// so a participant without a pane (the console, an agent outside tmux)
/// appears only through a message exchanged with an in-scope participant. A
/// managed workspace with no live session keeps the requests filed under its
/// Work.
export function scopeCollaborationRequests(requests, scope, resolvePane = () => null) {
  const all = requests || [];
  if (!scope) return all;
  if (!scope.session) {
    if (!scope.workspace) return [];
    return all.filter((request) =>
      (request?.workspace_id || request?.work?.workspace_id || "") === scope.workspace);
  }
  const inScope = (participant) => {
    const hit = participantSession(participant, resolvePane);
    if (!hit || baseSessionName(hit.session) !== baseSessionName(scope.session)) return false;
    return !scope.socket || !hit.socket || hit.socket === shortSocket(scope.socket);
  };
  return all.filter((request) => inScope(request?.from) || inScope(request?.to));
}

export function dominantCount(counts) {
  return Object.entries(counts || {}).sort((left, right) => right[1] - left[1] || left[0].localeCompare(right[0]))[0]?.[0] || "unknown";
}

/// A narrower scope than a session: the requests one pane sent or received.
/// `socket` (a path or its short name) tells apart two tmux servers whose
/// panes share an id; a request recorded without a socket still matches.
export function requestsTouchingPane(requests, pane, socket = null) {
  if (!pane) return requests || [];
  const short = socket ? shortSocket(socket) : null;
  const touches = (participant) => {
    if (!participant || participant.console || participant.pane !== pane) return false;
    const own = participant.socket || participant.room?.socket || null;
    return !short || !own || shortSocket(own) === short;
  };
  return (requests || []).filter((request) => touches(request?.from) || touches(request?.to));
}

/// What "copy conversation" puts on the clipboard: the request, its
/// updates and the reply, in that order.
export function conversationText(request) {
  return [request?.body, ...(request?.updates || []).map((update) => update.body), request?.reply?.body].filter(Boolean).join("\n\n");
}

/// The status colour family a request or reply is drawn in.
export function collaborationStatusClass(status) {
  if (["failed", "blocked", "declined", "cancelled", "expired"].includes(status)) return "danger";
  if (["queued", "claimed"].includes(status)) return "pending";
  return status === "completed" ? "success" : "neutral";
}

// ---- Graph and sequence geometry -----------------------------------------
// The layout Explore -> Messages draws: positions, paths and labels, no
// elements.

// Nodes sit on an ellipse whose perimeter leaves roughly one node box of
// room per participant; the canvas is sized around it so every node is on
// screen and the SVG can be scaled to the pane.
const GRAPH_NODE_SPACING = 125;
const GRAPH_MARGIN_X = 90;
const GRAPH_MARGIN_Y = 60;

export function graphRadii(count) {
  const radiusX = Math.max(150, (count * GRAPH_NODE_SPACING) / (2 * Math.PI) * 1.15);
  return { radiusX, radiusY: Math.max(90, radiusX * .62) };
}

export function graphNodePositions(nodes, width, height) {
  if (nodes.length === 1) return new Map([[nodes[0].id, { x: width / 2, y: height / 2 }]]);
  const { radiusX, radiusY } = graphRadii(nodes.length);
  const positions = new Map();
  nodes.forEach((node, index) => {
    const angle = -Math.PI / 2 + (index * Math.PI * 2 / nodes.length);
    positions.set(node.id, {
      x: width / 2 + Math.cos(angle) * radiusX,
      y: height / 2 + Math.sin(angle) * radiusY,
    });
  });
  return positions;
}

export function graphPath(from, to, bend = 0) {
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const distance = Math.max(1, Math.hypot(dx, dy));
  if (distance < 2) {
    return `M ${from.x - 22} ${from.y - 5} C ${from.x - 78} ${from.y - 76}, ${from.x + 78} ${from.y - 76}, ${from.x + 22} ${from.y - 5}`;
  }
  const start = { x: from.x + dx / distance * 48, y: from.y + dy / distance * 30 };
  const end = { x: to.x - dx / distance * 52, y: to.y - dy / distance * 32 };
  const mx = (start.x + end.x) / 2 - dy / distance * bend;
  const my = (start.y + end.y) / 2 + dx / distance * bend;
  return `M ${start.x.toFixed(1)} ${start.y.toFixed(1)} Q ${mx.toFixed(1)} ${my.toFixed(1)} ${end.x.toFixed(1)} ${end.y.toFixed(1)}`;
}

/// The directional graph of a projection (projectCollaboration): canvas
/// size, node positions, and per edge its request path, its reply path
/// (back toward the sender, when any reply came) and its label point.
export function graphLayout(projection) {
  const { radiusX, radiusY } = graphRadii(projection.nodes.length);
  const width = Math.max(560, Math.ceil(radiusX * 2 + GRAPH_MARGIN_X * 2));
  const height = Math.max(320, Math.ceil(radiusY * 2 + GRAPH_MARGIN_Y * 2));
  const positions = graphNodePositions(projection.nodes, width, height);
  const edges = projection.edges.map((edge, index) => {
    const from = positions.get(edge.from);
    const to = positions.get(edge.to);
    const reverse = projection.edges.find((candidate) => candidate.from === edge.to && candidate.to === edge.from);
    const bend = reverse ? (edge.from.localeCompare(edge.to) < 0 ? 34 : -34) : 0;
    return {
      edge,
      index,
      path: graphPath(from, to, bend),
      replyPath: edge.replyCount ? graphPath(to, from, bend ? -bend - 10 : 16) : null,
      midpoint: { x: (from.x + to.x) / 2, y: (from.y + to.y) / 2 + bend * .45 },
      status: dominantCount(edge.statuses),
      replyStatus: dominantCount(edge.replyStatuses),
    };
  });
  return { width, height, positions, edges };
}

/// One row per request and one per reply, chronological (a reply after its
/// request at the same instant), for the sequence diagram.
export function sequenceEvents(requests) {
  const events = [];
  for (const request of requests || []) {
    events.push({
      request,
      at: request.created_at,
      from: participantIdentity(request.from),
      to: participantIdentity(request.to),
      reply: false,
      status: "request",
      kind: request.kind || "request",
    });
    if (request.reply) {
      events.push({
        request,
        at: request.reply.at || request.created_at,
        from: participantIdentity(request.to),
        to: participantIdentity(request.from),
        reply: true,
        status: request.reply.status || request.status,
        kind: "reply",
      });
    }
  }
  return events.sort((left, right) =>
    String(left.at || "").localeCompare(String(right.at || "")) ||
    String(left.request.id || "").localeCompare(String(right.request.id || "")) ||
    Number(left.reply) - Number(right.reply)
  );
}

/// The sequence diagram of (at most the latest `cap` of) `requests`: its
/// lifelines, rows and canvas.
export function sequenceLayout(requests, { cap = 200 } = {}) {
  const all = requests || [];
  const visible = all.slice(-cap);
  const participants = sequenceParticipants(visible);
  const participantIndex = new Map(participants.map((participant, index) => [participant.id, index]));
  const events = sequenceEvents(visible);
  const laneWidth = 170;
  const left = 65;
  const width = Math.max(420, left * 2 + Math.max(1, participants.length - 1) * laneWidth);
  // Lane names live in their own strip that sticks to the top of the
  // scroll box, so long histories keep saying who each lifeline belongs to.
  const headerHeight = 44;
  const rowHeight = 58;
  const height = 8 + events.length * rowHeight + 20;
  const laneX = (id) => left + (participantIndex.get(id) || 0) * laneWidth;
  const lanes = participants.map((participant, index) => ({ ...participant, x: left + index * laneWidth }));
  const rows = events.map((event, index) => {
    const y = 8 + index * rowHeight + 25;
    const sx = laneX(event.from);
    const tx = laneX(event.to);
    return {
      ...event,
      y,
      sx,
      tx,
      labelX: sx === tx ? sx + 44 : (sx + tx) / 2,
      path: sx === tx
        ? `M ${sx} ${y} C ${sx + 80} ${y - 28}, ${sx + 80} ${y + 28}, ${sx + 4} ${y + 8}`
        : `M ${sx} ${y} L ${tx} ${y}`,
    };
  });
  return { total: all.length, visible, lanes, rows, width, height, headerHeight, rowHeight };
}
