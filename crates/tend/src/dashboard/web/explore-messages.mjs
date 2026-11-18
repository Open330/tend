// Explore → Messages: the collaboration reader in the new shell. The
// requests agents and the console sent each other on this host's daemon
// (/api/collaboration), as a message list, a directional graph and a
// sequence diagram, with the retained-history search, the range, room,
// Work, thread, kind and status filters, the updates, reply and artifacts
// of a message, and paging into older history.
//
// What the reader shows is in the address (`/explore/messages#…`,
// messages-model.mjs), with its host and scope: request ids repeat across
// daemons, so a link for another Fleet host is never looked up here.
// Bodies come only with operator credentials (`details_included`); every
// text an agent wrote is set with textContent. The page wires DOM helpers
// in (`deps`) so this module stays testable.

import {
  collaborationSequence,
  collaborationStatusClass,
  conversationText,
  graphLayout,
  needsHumanResponse,
  normalizeCollaborationPayload,
  participantLabel,
  projectCollaboration,
  requestThreadId,
  requestWorkId,
  roomKey,
  roomLabel,
  sequenceLayout,
} from "./collaboration-model.mjs";
import { collaborationSummary, formatDuration, graphFitScale, participantLocation, responseLatency } from "./collaboration-view.mjs";
import {
  KINDS,
  MESSAGES_PATH,
  RANGES,
  STATUSES,
  TABS,
  collaborationParams,
  hostCheck,
  localHostAlias,
  mergeCollaborationRequests,
  messagesFromHash,
  messagesHash,
  messagesLink,
  paneResolver,
  participantPaneLink,
  safeExternalHref,
  sameQuery,
  scopeReaderRequests,
} from "./messages-model.mjs";

const SVG = "http://www.w3.org/2000/svg";
/// The shell polls about this often; a refresh waits at least this long.
const REFRESH_MS = 4500;
const SEARCH_DEBOUNCE_MS = 300;

export function mountMessages(deps) {
  const { S, el, icon, button, read, access = () => null, modeSwitch, go = () => {}, fleetSettled = () => true } = deps;
  const view = {
    state: messagesFromHash(window.location.hash),
    fleet: null,
    data: null,
    error: "",
    periodError: "",
    edge: "",
    zoom: "fit",
    generation: 0,
    inflight: null,
    inflightKey: "",
    lastKey: "",
    lastFetch: 0,
    appended: false,
    written: null,
    closed: false,
    suspended: false,
    signatures: new Map(),
    note: "",
  };

  // ---- Static frame: sidebar controls and the reader ------------------
  const side = el("aside", "ex-side mx-side");
  const main = el("section", "ex-view mx-main");

  const head = el("header", "ex-head");
  const count = el("span", "ex-count", "");
  head.append(el("span", "ex-title", S.messages.toUpperCase()), count);
  if (modeSwitch) head.append(modeSwitch("messages"));

  const scopeChip = el("div", "mx-scope");

  const search = el("input", "ex-search");
  search.type = "search";
  search.placeholder = S.searchMessages;
  search.setAttribute("aria-label", S.searchMessages);
  let searchTimer = null;
  search.addEventListener("input", () => {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => change({ q: search.value.trim(), message: "", open: "" }), SEARCH_DEBOUNCE_MS);
  });
  const field = el("div", "ex-field");
  field.append(icon("search"), search);

  const labelled = (word, control) => {
    const label = el("label", "mx-filter");
    label.append(el("span", null, word), control);
    return label;
  };
  const select = (options, onChange) => {
    const node = el("select", "mx-select");
    for (const [value, word] of options) {
      const option = el("option", null, word);
      option.value = value;
      node.append(option);
    }
    node.addEventListener("change", () => onChange(node.value));
    return node;
  };
  const range = select(RANGES.map((value) => [value, S.rangeLabels[value] || value]), (value) =>
    change({ range: value, message: "", open: "" }));
  const kind = select([["", S.anyValue], ...KINDS.map((value) => [value, S.kindLabels[value] || value])], (value) =>
    change({ kind: value, message: "", open: "" }));
  const status = select([["", S.anyValue], ...STATUSES.map((value) => [value, S.requestStatusLabels[value] || value])], (value) =>
    change({ status: value, message: "", open: "" }));
  const room = el("select", "mx-select");
  room.addEventListener("change", () => {
    const chosen = view.projection?.rooms.find((candidate) => candidate.key === room.value);
    chooseRoom(chosen || null);
  });
  const datalist = (id) => {
    const list = el("datalist");
    list.id = id;
    return list;
  };
  const workOptions = datalist("mx-work-options");
  const threadOptions = datalist("mx-thread-options");
  const textFilter = (key, listId) => {
    const input = el("input", "mx-input");
    input.type = "search";
    input.autocomplete = "off";
    input.placeholder = S.anyValue;
    input.setAttribute("list", listId);
    let timer = null;
    input.addEventListener("input", () => {
      clearTimeout(timer);
      timer = setTimeout(() => change({ [key]: input.value.trim(), message: "", open: "" }), SEARCH_DEBOUNCE_MS);
    });
    return input;
  };
  const work = textFilter("work", "mx-work-options");
  const thread = textFilter("thread", "mx-thread-options");
  const since = el("input", "mx-input");
  since.type = "datetime-local";
  const until = el("input", "mx-input");
  until.type = "datetime-local";
  for (const input of [since, until]) {
    input.addEventListener("change", () => change({ since: since.value, until: until.value, message: "", open: "" }));
  }
  const period = el("div", "mx-period");
  const periodError = el("p", "mx-error");
  periodError.setAttribute("role", "status");
  period.append(labelled(S.periodFrom, since), labelled(S.periodTo, until), periodError);

  const filters = el("div", "mx-filters");
  filters.append(
    labelled(S.rangeFilter, range),
    labelled(S.roomFilter, room),
    labelled(S.workFilter, work),
    labelled(S.threadFilter, thread),
    labelled(S.kindFilter, kind),
    labelled(S.statusFilter, status),
    workOptions,
    threadOptions,
  );
  const refreshRow = el("div", "row");
  const refreshButton = button(S.refresh, () => load({ force: true }), "btn small", "turn");
  refreshRow.append(refreshButton);

  const controls = el("div", "ex-controls mx-controls");
  controls.append(field, filters, period, refreshRow);
  const roomsHead = el("div", "ex-section", S.sessionsRooms);
  const rooms = el("div", "ex-list mx-rooms");
  side.append(head, scopeChip, controls, roomsHead, rooms);

  // The reader: tabs, summary, the list/graph/sequence beside the detail.
  const bar = el("header", "mx-bar");
  const tabs = el("div", "seg mx-tabs");
  tabs.setAttribute("role", "tablist");
  const tabButtons = TABS.map((tab) => {
    const node = el("button", null, S[tab]);
    node.type = "button";
    node.setAttribute("role", "tab");
    node.dataset.tab = tab;
    node.addEventListener("click", () => change({ tab }, { fetch: false }));
    node.addEventListener("keydown", (event) => {
      if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
      event.preventDefault();
      const index = TABS.indexOf(tab);
      const next = event.key === "Home" ? 0 : event.key === "End" ? TABS.length - 1
        : (index + (event.key === "ArrowRight" ? 1 : TABS.length - 1)) % TABS.length;
      change({ tab: TABS[next] }, { fetch: false });
      tabButtons[next].focus?.();
    });
    return node;
  });
  tabs.append(...tabButtons);
  const meta = el("span", "mx-meta");
  const showAll = button(S.showAllMessages, () => clearSelection(), "btn ghost small");
  bar.append(tabs, meta, showAll);
  const summary = el("div", "mx-summary");
  summary.setAttribute("aria-label", S.sumMessages);
  const notice = el("div", "mx-notice");
  const history = el("div", "mx-history");
  const list = el("div", "mx-list");
  list.setAttribute("role", "tabpanel");
  const graphPane = el("div", "mx-graph-pane");
  const zoomRow = el("div", "row mx-zoom");
  const zoomLabel = el("span", "mx-zoom-label", "100%");
  const zoomBy = (factor) => {
    const current = Number(canvas.firstChild?.dataset?.scale) || 1;
    view.zoom = Math.max(.1, Math.min(3, current * factor));
    applyZoom();
  };
  const zoomOut = button("−", () => zoomBy(.8), "btn small");
  zoomOut.setAttribute("aria-label", S.zoomOut);
  const zoomIn = button("+", () => zoomBy(1.25), "btn small");
  zoomIn.setAttribute("aria-label", S.zoomIn);
  zoomRow.append(zoomOut, zoomLabel, zoomIn, button(S.zoomFit, () => {
    view.zoom = "fit";
    applyZoom();
  }, "btn small"));
  const canvas = el("div", "mx-canvas");
  graphPane.append(zoomRow, canvas);
  const sequencePane = el("div", "mx-sequence-pane");
  const sequenceMeta = el("p", "mx-meta");
  const sequence = el("div", "mx-sequence");
  sequencePane.append(sequenceMeta, sequence);
  history.append(list, graphPane, sequencePane);
  const detail = el("aside", "mx-detail");
  const body = el("div", "mx-body");
  body.append(history, detail);
  const footer = el("footer", "mx-footer");
  const pageMeta = el("span", "mx-meta");
  const loadMore = button(S.loadOlder, () => load({ append: true }), "btn small");
  const footStatus = el("span", "mx-status");
  footStatus.setAttribute("role", "status");
  footer.append(pageMeta, loadMore, footStatus);
  main.append(bar, summary, notice, body, footer);
  if (globalThis.ResizeObserver) new ResizeObserver(() => applyZoom()).observe(canvas);

  // ---- State --------------------------------------------------------------

  function writeHash() {
    const hash = messagesHash(view.state);
    window.history?.replaceState?.(null, "", `${MESSAGES_PATH}${hash}`);
    view.written = window.location.hash;
  }

  /// Change what the reader shows; a change to what it asks the server for
  /// fetches again.
  function change(patch, { fetch = true } = {}) {
    if (view.closed) return;
    const before = view.state;
    view.state = { ...before, ...patch };
    if ("room" in patch || "scope" in patch || "thread" in patch || "message" in patch) view.edge = "";
    writeHash();
    if (fetch && !sameQuery(before, view.state)) load({ force: true });
    render();
  }

  function chooseRoom(chosen) {
    change({ room: chosen?.room ? { host: chosen.room.host, window_id: chosen.room.window_id, ...(chosen.room.socket ? { socket: chosen.room.socket } : {}) } : null, message: "", open: "" });
  }

  function clearSelection() {
    view.edge = "";
    change({ room: null, message: "", thread: "", open: "" });
  }

  /// A thread or parent link inside a message: the whole retained history
  /// of it, whatever the filters or scope were (its panes may be gone).
  function openHistory({ thread = "", message = "" }) {
    change({
      thread,
      message,
      open: message,
      room: null,
      work: "",
      kind: "",
      status: "",
      q: "",
      range: "all",
      since: "",
      until: "",
      scope: null,
      tab: "messages",
    });
  }

  function hostState() {
    return hostCheck(view.state, view.fleet);
  }

  // ---- Loading ------------------------------------------------------------

  /// Forget any answer still on its way.
  function cancel() {
    view.generation += 1;
    view.inflight = null;
    refreshButton.disabled = false;
  }

  function load({ append = false, force = false } = {}) {
    if (view.closed || view.suspended) return null;
    if (hostState() !== "local") {
      cancel();
      view.data = null;
      render();
      return null;
    }
    const { params, error } = collaborationParams(view.state, { append, pagination: view.data?.pagination });
    view.periodError = error;
    if (error) {
      cancel();
      render();
      return null;
    }
    const key = params.toString();
    if (view.inflight && view.inflightKey === key) return view.inflight;
    if (!force && !append && key === view.lastKey && Date.now() - view.lastFetch < REFRESH_MS) return null;
    const generation = ++view.generation;
    // A refresh of the same query after older pages were loaded keeps them.
    const keep = !append && view.appended && key === view.lastKey ? view.data : null;
    view.inflightKey = key;
    refreshButton.disabled = true;
    view.inflight = (async () => {
      try {
        const payload = normalizeCollaborationPayload(await read(`/api/collaboration?${params}`));
        if (generation !== view.generation || view.closed) return;
        if (append) {
          view.data = { ...payload, requests: mergeCollaborationRequests(view.data?.requests, payload.requests) };
          view.appended = true;
        } else if (keep && keep.details_included === payload.details_included) {
          view.data = { ...payload, requests: mergeCollaborationRequests(keep.requests, payload.requests), pagination: keep.pagination };
        } else {
          view.data = payload;
          view.appended = false;
        }
        view.error = "";
        view.lastKey = append ? view.lastKey : key;
      } catch (failure) {
        if (generation !== view.generation || view.closed) return;
        // Refused: nothing private stays. Off: nothing to show. A blip
        // keeps what was loaded.
        if (failure.auth) {
          view.data = null;
          view.error = S.messagesOperatorOnly;
        } else if (failure.status === 503) {
          view.data = null;
          view.error = S.messagesDisabled;
        } else {
          view.error = failure.message || S.messagesFailed;
        }
      } finally {
        if (generation === view.generation) {
          view.inflight = null;
          view.lastFetch = Date.now();
          refreshButton.disabled = false;
          render();
        }
      }
    })();
    return view.inflight;
  }

  // ---- Drawing --------------------------------------------------------------

  /// Rebuild a node only when what it shows changed, so a poll does not
  /// take away a selection, a scroll position or keyboard focus.
  function draw(node, signature, build) {
    if (view.signatures.get(node) === signature) return;
    view.signatures.set(node, signature);
    const top = node.scrollTop;
    node.replaceChildren(...build());
    node.scrollTop = top;
  }

  function time(iso) {
    if (!iso) return "—";
    const date = new Date(iso);
    return Number.isFinite(date.getTime()) ? date.toLocaleString([], { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" }) : "—";
  }

  function fullTime(iso) {
    const date = new Date(iso || "");
    return Number.isFinite(date.getTime()) ? date.toLocaleString() : "—";
  }

  function statusWord(value) {
    return S.requestStatusLabels[value] || value || "—";
  }

  function scoped() {
    return scopeReaderRequests(view.data?.requests || [], view.state.scope, paneResolver(view.fleet));
  }

  function visible(requests) {
    return collaborationSequence(requests, { edgeKey: view.edge, room: view.state.room ? roomKey(view.state.room) : "" });
  }

  function render() {
    if (view.closed) return;
    const state = view.state;
    // Controls follow the state (without stealing a field someone types in).
    const active = document.activeElement;
    if (active !== search) search.value = state.q;
    if (active !== work) work.value = state.work;
    if (active !== thread) thread.value = state.thread;
    range.value = state.range;
    kind.value = state.kind;
    status.value = state.status;
    period.hidden = state.range !== "custom";
    if (active !== since) since.value = state.since;
    if (active !== until) until.value = state.until;
    periodError.textContent = view.periodError === "missing" ? S.periodMissing : view.periodError === "order" ? S.periodOrder : "";
    for (const node of tabButtons) {
      const on = node.dataset.tab === state.tab;
      node.setAttribute("aria-selected", String(on));
      node.tabIndex = on ? 0 : -1;
    }
    list.hidden = state.tab !== "messages";
    graphPane.hidden = state.tab !== "graph";
    sequencePane.hidden = state.tab !== "sequence";
    drawScope();

    const where = hostState();
    // A link for a named host waits for the Fleet to say which host this is.
    if (where === "unknown" && !fleetSettled()) {
      drawHostNotice("waiting");
      return;
    }
    if (where !== "local") {
      drawHostNotice(where);
      return;
    }
    notice.replaceChildren();
    notice.hidden = !view.error;
    if (view.error) notice.append(el("p", "mx-error", view.error));
    if (!view.data) {
      for (const node of [list, canvas, sequence, rooms, summary, detail]) {
        view.signatures.delete(node);
        node.replaceChildren();
      }
      if (!view.error) list.append(el("div", "ex-empty", S.loading));
      count.textContent = "";
      meta.textContent = "";
      pageMeta.textContent = "";
      loadMore.hidden = true;
      showAll.hidden = true;
      return;
    }
    const all = scoped();
    const projection = projectCollaboration(all);
    view.projection = projection;
    const shown = visible(all);
    count.textContent = String(all.length);
    meta.textContent = S.messagesMeta({ human: projection.humanCount, participants: projection.nodes.length, messages: all.length });
    const page = view.data.pagination;
    pageMeta.textContent = page.has_more ? S.pageMore({ loaded: view.data.requests.length, total: page.total }) : S.pageAll({ loaded: view.data.requests.length });
    loadMore.hidden = !page.has_more;
    showAll.hidden = !(view.edge || state.room || state.message || state.thread);
    drawRoomOptions(projection);
    drawRooms(projection);
    drawSummary(shown);
    if (state.tab === "messages") drawList(shown);
    if (state.tab === "graph") drawGraph(projection);
    if (state.tab === "sequence") drawSequence(shown);
    drawDetail(shown.find((request) => request.id === state.open) || null);
  }

  function drawHostNotice(where) {
    for (const node of [list, canvas, sequence, rooms, summary, detail]) {
      view.signatures.delete(node);
      node.replaceChildren();
    }
    count.textContent = "";
    meta.textContent = "";
    pageMeta.textContent = "";
    loadMore.hidden = true;
    showAll.hidden = true;
    notice.hidden = false;
    const host = view.state.host;
    if (where === "waiting") {
      notice.replaceChildren(el("div", "ex-empty", S.loading));
      return;
    }
    const box = el("div", "mx-host");
    box.append(icon("server"), el("p", null, where === "remote" ? S.messagesRemote({ host }) : S.messagesHostUnknown({ host })));
    if (where === "unknown") box.append(button(S.openHereAnyway, () => change({ host: "" }), "btn"));
    notice.replaceChildren(box);
  }

  function drawScope() {
    const scope = view.state.scope;
    scopeChip.hidden = !scope;
    if (!scope) {
      scopeChip.replaceChildren();
      return;
    }
    const words = [
      scope.session ? S.scopeSession({ session: scope.session }) : "",
      scope.pane ? S.scopePane({ pane: scope.pane }) : "",
      scope.workspace && !scope.session ? S.scopeWorkspace({ workspace: scope.workspace }) : "",
    ].filter(Boolean).join(" · ");
    const text = el("span", "mx-scope-text");
    text.append(el("span", null, `${S.scopedTo} `), el("b", null, words));
    if (scope.socket) text.title = scope.socket;
    scopeChip.replaceChildren(text, button(S.clearScope, () => change({ scope: null }), "btn ghost small"));
  }

  function drawRoomOptions(projection) {
    const selected = view.state.room ? roomKey(view.state.room) : "";
    const signature = JSON.stringify([selected, projection.rooms.map((r) => [r.key, r.label, r.count]), projection.works, projection.threads]);
    if (view.signatures.get(room) === signature) return;
    view.signatures.set(room, signature);
    const option = (value, word) => {
      const node = el("option", null, word);
      node.value = value;
      return node;
    };
    const options = [option("", S.allRooms), ...projection.rooms.map((r) => option(r.key, `${r.label} (${r.count})`))];
    if (selected && !projection.rooms.some((r) => r.key === selected)) options.push(option(selected, S.roomNoResults));
    room.replaceChildren(...options);
    room.value = selected;
    workOptions.replaceChildren(...projection.works.map((value) => option(value, null)));
    threadOptions.replaceChildren(...projection.threads.map((value) => option(value, null)));
  }

  function drawRooms(projection) {
    const selected = view.state.room ? roomKey(view.state.room) : "";
    const groups = new Map();
    for (const entry of projection.rooms) {
      const key = JSON.stringify([entry.socket, entry.session]);
      if (!groups.has(key)) groups.set(key, { session: entry.session, socket: entry.socket, rooms: [], count: 0 });
      const group = groups.get(key);
      group.rooms.push(entry);
      group.count += entry.count;
    }
    draw(rooms, JSON.stringify([selected, [...groups.values()].map((g) => [g.session, g.socket, g.count, g.rooms.map((r) => [r.key, r.label, r.count])])]), () => {
      const nodes = [];
      for (const group of groups.values()) {
        const header = el("button", "ex-session mx-session");
        header.type = "button";
        header.append(icon("layers"), el("span", "ex-name", group.session || S.outsideTmux), el("span", "ex-n", S.roomsCount({ messages: group.count, rooms: group.rooms.length })));
        if (group.session) {
          header.title = S.sessionMessages;
          header.addEventListener("click", () => change({ scope: { session: group.session, ...(group.socket ? { socket: group.socket } : {}) }, room: null, message: "", open: "" }));
        } else header.disabled = true;
        nodes.push(header);
        for (const entry of group.rooms) {
          const chip = el("button", `ex-row mx-room${entry.key === selected ? " selected" : ""}`);
          chip.type = "button";
          chip.setAttribute("aria-pressed", String(entry.key === selected));
          chip.append(icon("window"), el("span", "ex-name", entry.label), el("span", "ex-n", String(entry.count)));
          chip.addEventListener("click", () => chooseRoom(entry.key === selected ? null : entry));
          nodes.push(chip);
        }
      }
      if (!nodes.length) nodes.push(el("div", "ex-empty", S.noGraph));
      return nodes;
    });
  }

  function drawSummary(shown) {
    const numbers = collaborationSummary(shown);
    const partial = view.data.pagination.has_more;
    draw(summary, JSON.stringify([numbers, partial, Boolean(view.state.q)]), () => {
      const cells = [
        [S.sumMessages, numbers.total],
        [S.sumReplies, numbers.replies],
        [S.sumWaiting, numbers.pending],
        [S.sumAttention, numbers.attention],
        [S.sumMedian, numbers.medianLatency === null ? "—" : formatDuration(numbers.medianLatency / 1000)],
      ].map(([word, value]) => {
        const cell = el("div", "mx-stat");
        cell.append(el("strong", null, String(value)), el("span", null, word));
        return cell;
      });
      const note = [partial ? S.loadedPartial : S.loadedAll, view.state.q ? S.serverSearch : ""].filter(Boolean).join(" · ");
      return [...cells, el("small", null, note)];
    });
  }

  function preview(request) {
    if (!view.data.details_included) return S.previewLocked;
    return String(request.body || "").replace(/\s+/g, " ").trim().slice(0, 240) || S.emptyMessage;
  }

  function endpoint(participant) {
    return participant?.console ? S.consoleShort : participant?.pane || "—";
  }

  function drawList(shown) {
    const newest = [...shown].reverse();
    const details = view.data.details_included;
    const signature = JSON.stringify([view.state.open, details, newest.map((r) => [r.id, r.status, r.reply?.status, r.reply?.at, r.updates?.length, details ? r.body : ""])]);
    draw(list, signature, () => {
      if (!newest.length) return [el("div", "ex-empty", S.noMessagesMatch)];
      return newest.map((request) => {
        const selected = request.id === view.state.open;
        const row = el("button", `mx-row${selected ? " selected" : ""}`);
        row.type = "button";
        row.dataset.id = request.id;
        row.setAttribute("aria-pressed", String(selected));
        const heading = el("div", "mx-row-head");
        heading.append(el("b", null, `${participantLabel(request.from)} → ${participantLabel(request.to)}`), el("span", `mx-status-pill ${collaborationStatusClass(request.status)}`, statusWord(request.status)));
        const context = el("div", "mx-row-context");
        context.append(el("span", null, `${S.kindLabels[request.kind] || request.kind || ""} · ${roomLabel(request)}`), el("time", null, time(request.created_at)));
        const panes = el("div", "mx-row-context");
        panes.append(el("span", null, `${endpoint(request.from)} → ${endpoint(request.to)}`));
        const artifacts = (request.artifacts?.length || 0) + (request.reply?.artifacts?.length || 0);
        if (details && artifacts) panes.append(el("span", null, S.artifactsCount({ n: artifacts })));
        const duration = responseLatency(request);
        const foot = el("div", "mx-row-foot");
        foot.append(el("code", null, request.id), el("span", null, [
          request.reply ? S.replied : S.noReply,
          duration !== null ? formatDuration(duration / 1000) : "",
          request.updates?.length ? S.updatesCount({ n: request.updates.length }) : "",
        ].filter(Boolean).join(" · ")));
        row.append(heading, context, panes, el("p", "mx-preview", preview(request)), foot);
        row.addEventListener("click", () => change({ open: request.id }, { fetch: false }));
        return row;
      });
    });
  }

  function svg(tag, attrs = {}, text = null) {
    const node = document.createElementNS(SVG, tag);
    for (const [name, value] of Object.entries(attrs)) node.setAttribute(name, String(value));
    if (text != null) node.textContent = text;
    return node;
  }

  function marker(id, orient) {
    const node = svg("marker", { id, viewBox: "0 0 10 10", refX: 8, refY: 5, markerWidth: 7, markerHeight: 7, orient });
    node.append(svg("path", { d: "M 0 0 L 10 5 L 0 10 z" }));
    return node;
  }

  function activate(node, onActivate) {
    node.setAttribute("role", "button");
    node.setAttribute("tabindex", "0");
    node.addEventListener("click", onActivate);
    node.addEventListener("keydown", (event) => {
      if (event.key !== "Enter" && event.key !== " ") return;
      event.preventDefault();
      onActivate();
    });
  }

  function drawGraph(projection) {
    const signature = JSON.stringify([view.edge, projection.nodes.map((n) => [n.id, n.label]), projection.edges.map((e) => [e.key, e.count, e.replyCount, e.humanCount, e.statuses, e.replyStatuses])]);
    draw(canvas, signature, () => {
      if (!projection.nodes.length) return [el("div", "ex-empty", S.noGraph)];
      const layout = graphLayout(projection);
      const root = svg("svg", { class: "mx-graph", viewBox: `0 0 ${layout.width} ${layout.height}`, role: "img", "aria-label": S.graphAria });
      const defs = svg("defs");
      defs.append(marker("mx-request-arrow", "auto-start-reverse"), marker("mx-reply-arrow", "auto-start-reverse"));
      const nodeById = new Map(projection.nodes.map((node) => [node.id, node]));
      const edges = svg("g", { class: "mx-edges" });
      for (const placed of layout.edges) {
        const { edge } = placed;
        const selected = edge.key === view.edge ? " selected" : "";
        const group = svg("g", { class: "mx-edge-group", "aria-label": S.edgeAria({ from: nodeById.get(edge.from)?.label || edge.from, to: nodeById.get(edge.to)?.label || edge.to, count: edge.count, replies: edge.replyCount }) });
        activate(group, () => {
          view.edge = view.edge === edge.key ? "" : edge.key;
          change({ open: "" }, { fetch: false });
        });
        group.append(svg("path", { class: "mx-edge-hit", d: placed.path }));
        const line = svg("path", { class: `mx-edge ${collaborationStatusClass(placed.status)}${selected}`, d: placed.path, "marker-end": "url(#mx-request-arrow)" });
        line.append(svg("title", {}, `${edge.count} · ${edge.replyCount} · ${statusWord(placed.status)}`));
        group.append(line);
        if (placed.replyPath) {
          const back = svg("path", { class: `mx-edge-reply ${collaborationStatusClass(placed.replyStatus)}${selected}`, d: placed.replyPath, "marker-end": "url(#mx-reply-arrow)" });
          back.append(svg("title", {}, S.repliesReturn({ n: edge.replyCount })));
          group.append(back);
        }
        group.append(svg("text", { class: "mx-edge-label", x: placed.midpoint.x, y: placed.midpoint.y }, S.edgeLabel({ count: edge.count, replies: edge.replyCount, human: edge.humanCount })));
        edges.append(group);
      }
      const nodes = svg("g", { class: "mx-nodes" });
      for (const node of projection.nodes) {
        const at = layout.positions.get(node.id);
        const kindClass = String(node.participant.agent_kind || "unknown").replace(/[^a-z0-9_-]/gi, "");
        const group = svg("g", { class: `mx-node kind-${kindClass}`, transform: `translate(${at.x} ${at.y})` });
        group.append(
          svg("rect", { x: -50, y: -30, width: 100, height: 60, rx: 9 }),
          svg("text", { class: "mx-node-label", "text-anchor": "middle", y: -3 }, String(node.label).slice(0, 18)),
          svg("text", { class: "mx-node-sub", "text-anchor": "middle", y: 14 }, participantLocation(node.participant)),
          svg("title", {}, node.participant.agent_session_id || node.id),
        );
        nodes.append(group);
      }
      root.append(defs, edges, nodes);
      root.dataset.width = String(layout.width);
      root.dataset.height = String(layout.height);
      return [root];
    });
    applyZoom();
  }

  function applyZoom() {
    const root = canvas.firstChild;
    if (!root?.dataset?.width || graphPane.hidden) return;
    const width = Number(root.dataset.width);
    const height = Number(root.dataset.height);
    const scale = view.zoom === "fit" ? graphFitScale(width, height, canvas.clientWidth, canvas.clientHeight) : view.zoom;
    root.setAttribute("width", String(Math.round(width * scale)));
    root.setAttribute("height", String(Math.round(height * scale)));
    root.dataset.scale = String(scale);
    zoomLabel.textContent = `${Math.round(scale * 100)}%`;
  }

  function drawSequence(shown) {
    sequenceMeta.textContent = S.sequenceMeta({ n: shown.length });
    const signature = JSON.stringify([shown.map((r) => [r.id, r.status, r.reply?.status, r.reply?.at])]);
    draw(sequence, signature, () => {
      if (!shown.length) return [el("div", "ex-empty", S.noSequence)];
      const layout = sequenceLayout(shown);
      const nodes = [];
      if (layout.total > layout.visible.length) nodes.push(el("p", "mx-meta", S.sequenceCap({ shown: layout.visible.length, total: layout.total })));
      const header = svg("svg", { class: "mx-seq-head", viewBox: `0 0 ${layout.width} ${layout.headerHeight}`, width: layout.width, height: layout.headerHeight, "aria-hidden": "true" });
      for (const lane of layout.lanes) {
        header.append(
          svg("text", { class: "mx-lane-label", "text-anchor": "middle", x: lane.x, y: 18 }, String(lane.label).slice(0, 18)),
          svg("text", { class: "mx-lane-sub", "text-anchor": "middle", x: lane.x, y: 34 }, participantLocation(lane.participant)),
        );
      }
      const head = el("div", "mx-seq-sticky");
      head.append(header);
      nodes.push(head);
      const chart = svg("svg", { class: "mx-seq", viewBox: `0 0 ${layout.width} ${layout.height}`, width: layout.width, height: layout.height, role: "img", "aria-label": S.sequenceAria });
      const defs = svg("defs");
      defs.append(marker("mx-seq-request-arrow", "auto"), marker("mx-seq-reply-arrow", "auto"));
      chart.append(defs);
      for (const lane of layout.lanes) {
        const line = svg("g", { class: "mx-lane" });
        line.append(svg("line", { x1: lane.x, x2: lane.x, y1: 0, y2: layout.height - 8 }));
        chart.append(line);
      }
      for (const row of layout.rows) {
        const label = row.reply ? S.seqReply({ status: statusWord(row.status || "completed") }) : S.seqSent({ kind: S.kindLabels[row.kind] || row.kind });
        const at = time(row.at);
        const group = svg("g", { class: `mx-event ${row.reply ? "reply" : "request"} ${collaborationStatusClass(row.status)}`, "aria-label": `${label}, ${at}` });
        group.dataset.id = row.request.id;
        activate(group, () => change({ open: row.request.id }, { fetch: false }));
        group.append(
          svg("rect", { class: "mx-event-hit", x: 0, y: row.y - 25, width: layout.width, height: layout.rowHeight - 2 }),
          svg("path", { d: row.path, "marker-end": `url(#${row.reply ? "mx-seq-reply-arrow" : "mx-seq-request-arrow"})` }),
          svg("circle", { cx: row.sx, cy: row.y, r: 4 }),
          svg("text", { class: "mx-event-label", "text-anchor": "middle", x: row.labelX, y: row.y - 9 }, label),
          svg("text", { class: "mx-event-time", "text-anchor": "middle", x: row.labelX, y: row.y + 17 }, at),
          svg("title", {}, `${label} · ${row.request.id || ""}`),
        );
        chart.append(group);
      }
      nodes.push(chart);
      return nodes;
    });
  }

  function definition(rows, term, value) {
    rows.append(el("dt", null, term));
    const dd = el("dd");
    if (typeof value === "string" || typeof value === "number") dd.textContent = String(value);
    else dd.append(value);
    rows.append(dd);
  }

  function linkButton(text, onClick) {
    return button(text, onClick, "btn ghost small mx-link");
  }

  function who(participant) {
    if (participant?.console) return S.operatorConsole;
    return `${participant?.agent_kind || "agent"} · ${participant?.pane || "—"}`;
  }

  function participantCell(participant) {
    const box = el("span", "mx-who");
    box.append(el("span", null, who(participant)));
    const href = participantPaneLink(participant, view.fleet);
    if (href) {
      const open = el("a", "mx-link", S.openPane);
      open.href = href;
      open.addEventListener("click", (event) => {
        if (event.metaKey || event.ctrlKey || event.shiftKey) return;
        event.preventDefault();
        go(href);
      });
      box.append(open);
    }
    return box;
  }

  function block(word, text, extra = "") {
    const box = el("div", `mx-message${extra ? ` ${extra}` : ""}`);
    box.append(el("span", null, word), el("pre", null, text || ""));
    return box;
  }

  function drawDetail(request) {
    const details = Boolean(view.data?.details_included);
    const signature = request ? JSON.stringify([request, details, access()?.mode, access()?.login?.signed_in]) : "";
    draw(detail, signature, () => {
      if (!request) return [el("p", "ex-empty mx-detail-empty", S.selectMessage)];
      const nodes = [];
      const top = el("div", "mx-detail-head");
      top.append(
        el("span", "mx-kind", S.kindLabels[request.kind] || request.kind || ""),
        el("strong", null, `${participantLabel(request.from)} → ${participantLabel(request.to)}`),
        el("span", `mx-status-pill ${collaborationStatusClass(request.status)}`, statusWord(request.status)),
      );
      const close = button("", () => change({ open: "" }, { fetch: false }), "btn ghost icon-only", "xmark");
      close.setAttribute("aria-label", S.close);
      top.append(close);
      nodes.push(top);
      const actions = el("div", "row mx-actions");
      actions.append(button(S.copyLink, () => copy(new URL(messagesLink({
        message: request.id,
        host: view.state.host || localHostAlias(view.fleet) || "",
        scope: view.state.scope,
      }), window.location.origin).toString()), "btn small"));
      if (details) actions.append(button(S.copyConversation, () => copy(conversationText(request)), "btn small"));
      nodes.push(actions);
      const rows = el("dl", "mx-fields");
      const live = ["queued", "claimed"].includes(request.status);
      definition(rows, S.responseNeeded, needsHumanResponse(request)
        ? S.responseYou({ action: request.human_action || "information" })
        : live && request.expects_reply ? S.responseAgent : S.responseNone);
      if (request.interruption) {
        definition(rows, S.peerRecovery, `${request.interruption.reason || ""} · ${request.interruption.active ? S.recoveryActive : S.recoveryCleared}`);
        definition(rows, S.originalRequest, request.interruption.request_id || "—");
        definition(rows, S.resetAt, request.interruption.reset_at || "—");
        definition(rows, S.actionRequest, request.interruption.action_request_id || S.coordinatorNotified);
      }
      definition(rows, S.fieldId, request.id);
      definition(rows, S.fieldRoom, roomLabel(request));
      definition(rows, S.fieldFrom, participantCell(request.from));
      definition(rows, S.fieldTo, participantCell(request.to));
      const workId = requestWorkId(request);
      if (workId) definition(rows, S.workFilter, workId);
      const threadId = requestThreadId(request);
      if (threadId) definition(rows, S.threadFilter, linkButton(threadId, () => openHistory({ thread: threadId })));
      const parent = request.parent_request_id || "";
      if (parent) definition(rows, S.fieldParent, linkButton(parent, () => openHistory({ message: parent })));
      definition(rows, S.fieldSent, fullTime(request.created_at));
      const latency = responseLatency(request);
      definition(rows, S.fieldResponseTime, latency === null ? "—" : formatDuration(latency / 1000));
      definition(rows, S.fieldUpdates, String(request.updates?.length || 0));
      nodes.push(rows);
      if (!details) {
        const locked = el("div", "mx-locked");
        const mode = access()?.mode;
        locked.append(el("p", null, mode === "read_only" ? S.detailsReadOnly : S.detailsLocked));
        const login = access()?.login;
        if (login?.available && !login.signed_in) {
          const signIn = el("a", "btn primary small", S.signIn);
          signIn.href = `/auth/login?return_to=${encodeURIComponent(`${MESSAGES_PATH}${window.location.hash}`)}`;
          locked.append(signIn);
        }
        nodes.push(locked);
        return nodes;
      }
      nodes.push(block(S.requestBody, request.body));
      for (const update of request.updates || []) {
        nodes.push(block(S.updateBy({ who: participantLabel(update.author), at: fullTime(update.at) }), update.body));
      }
      if (request.reply) {
        nodes.push(block(S.replyBody({ status: statusWord(request.reply.status || request.status), at: request.reply.at ? fullTime(request.reply.at) : "" }), request.reply.body, "reply"));
      }
      const listOf = (word, items) => {
        const box = el("div", "mx-detail-list");
        box.append(el("span", null, word), ...items);
        return box;
      };
      if (request.paths?.length) nodes.push(listOf(S.pathsLabel, request.paths.map((path) => el("code", null, String(path)))));
      const artifacts = [...(request.artifacts || []), ...(request.reply?.artifacts || [])];
      if (artifacts.length) nodes.push(listOf(S.artifactsLabel, artifacts.map((artifact) => el("code", null, typeof artifact === "string" ? artifact : JSON.stringify(artifact)))));
      const links = (request.links || []).map((link) => (typeof link === "string" ? { label: link, url: link } : link || {}));
      if (links.length) {
        nodes.push(listOf(S.linksLabel, links.map((link) => {
          const href = safeExternalHref(link.url || link.href);
          if (!href) return el("code", null, String(link.label || link.url || link.href || ""));
          const anchor = el("a", null, String(link.label || href));
          anchor.href = href;
          anchor.target = "_blank";
          anchor.rel = "noopener noreferrer";
          return anchor;
        })));
      }
      return nodes;
    });
  }

  async function copy(text) {
    try {
      await navigator.clipboard.writeText(text);
      footStatus.textContent = S.copied;
    } catch {
      footStatus.textContent = S.clipboardUnavailable;
    }
  }

  render();

  return {
    nodes: [side, main],
    /// The shell's poll (every few seconds) and any change to the address.
    update(fleet) {
      if (view.closed) return;
      let force = view.suspended;
      view.suspended = false;
      const before = hostState();
      view.fleet = fleet;
      // A credential that may no longer read bodies: what it read goes now,
      // not at the next answer.
      const authorized = Boolean(access()?.write_authorized);
      if (view.authorized !== undefined && authorized !== view.authorized) {
        if (!authorized) {
          view.data = null;
          view.signatures.clear();
        }
        force = true;
      }
      view.authorized = authorized;
      if (window.location.hash !== view.written) {
        // A new link (pasted, or back/forward): it is what the reader shows.
        const next = messagesFromHash(window.location.hash);
        view.written = window.location.hash;
        const query = !sameQuery(view.state, next);
        view.state = next;
        view.edge = "";
        if (query) {
          load({ force: true });
          render();
          return;
        }
      }
      if (before !== hostState() || force) load({ force: true });
      else load();
      render();
    },
    suspend() {
      view.suspended = true;
      cancel();
    },
    /// Everything private gone: bodies, the open message, the fleet, and
    /// no later answer draws it back.
    destroy() {
      view.closed = true;
      cancel();
      view.data = null;
      view.fleet = null;
      view.projection = null;
      clearTimeout(searchTimer);
      view.signatures.clear();
      for (const node of [side, main, list, canvas, sequence, rooms, summary, detail, notice]) node.replaceChildren();
    },
  };
}
