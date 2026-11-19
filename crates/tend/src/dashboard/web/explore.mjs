// Explore in the new shell: Tend for Mac's Explore sidebar (every host's
// panes, by host tree or status group, needing-attention first) and the
// pane it opens: live output and a composer, as the Mac's terminal tab
// shows a pane. Output and prompts are operator-only and local-only (a
// pane on another Fleet host is listed with its state, and opens on that
// host). Its other view, Messages (`/explore/messages`), is the
// collaboration reader (explore-messages.mjs): a switch in the header
// moves between the two, and a pane or session opens Messages scoped to
// it. The page wires DOM helpers in (`deps`) so this module stays
// testable.

import {
  EXPLORE_SORTS,
  PINNED_ATTENTION_LIMIT,
  exploreDetail,
  exploreHash,
  exploreSections,
  exploreSortOf,
  paneSelection,
  sameSocket,
  selectionFromHash,
  sessionSelection,
  treeKey,
  windowSelection,
  workspaceFromHash,
} from "./explore-model.mjs";
import { mountMessages } from "./explore-messages.mjs";
import { MESSAGES_PATH, messagesLink, paneMessagesState, sessionMessagesState } from "./messages-model.mjs";
import { openPaneStream } from "./pane-stream.mjs";

const LINES = 400;
const POLL_MS = 1500;
/// The chosen order, per browser, under the Mac's `@AppStorage` key.
const SORT_KEY = "tend.explore.sort";

function storedSort() {
  try {
    return exploreSortOf(globalThis.localStorage?.getItem(SORT_KEY));
  } catch {
    return exploreSortOf(null);
  }
}

function storeSort(sort) {
  try {
    globalThis.localStorage?.setItem(SORT_KEY, sort);
  } catch {
    // A preference only.
  }
}

export function mountExplore(root, deps) {
  const { S, el, icon, button, logo, hostBadge, when, read, write, authHeaders, canWrite } = deps;
  const view = { attention: null, show: "all", query: "", grouping: "host", sort: storedSort(), fleet: null, open: null, stream: null, poll: null, sending: new Set(), drafts: new Map(), collapsed: new Set(), closed: false, syncComposer: null, hash: null, mode: null, messages: null, detail: null, detailKey: null, facts: null, workspace: null };

  const sidebar = el("aside", "ex-side");
  const viewer = el("section", "ex-view");

  /// Panes | Messages, in both views' headers.
  function modeSwitch(current) {
    const box = el("div", "seg ex-mode");
    box.setAttribute("role", "radiogroup");
    box.setAttribute("aria-label", S.exploreMode);
    for (const mode of ["panes", "messages"]) {
      const option = el("button", null, S[mode]);
      option.type = "button";
      option.setAttribute("role", "radio");
      option.dataset.mode = mode;
      option.setAttribute("aria-checked", String(mode === current));
      option.addEventListener("click", () => {
        if (mode !== view.mode) go(mode === "messages" ? MESSAGES_PATH : "/explore");
      });
      box.append(option);
    }
    return box;
  }

  /// Move within Explore (a pane link, a scoped Messages link): a new
  /// history entry, then the view it names.
  function go(href) {
    window.history?.pushState?.(null, "", href);
    sync();
  }

  function modeOf() {
    return window.location.pathname === MESSAGES_PATH ? "messages" : "panes";
  }

  /// Show the view the address names.
  function sync() {
    if (view.closed) return;
    const mode = modeOf();
    if (mode !== view.mode) {
      view.mode = mode;
      if (mode === "messages") {
        closeStream();
        view.suspended = Boolean(view.open);
        view.messages ||= mountMessages({ ...deps, modeSwitch, go });
        root.replaceChildren(...view.messages.nodes);
      } else {
        view.messages?.suspend();
        root.replaceChildren(sidebar, viewer);
        // An open pane comes back (stream and all) in showLinkedPane.
        if (!view.open) drawViewer();
      }
    }
    if (mode === "messages") view.messages.update(view.fleet);
    else showLinkedPane();
  }

  const head = el("header", "ex-head");
  const title = el("span", "ex-title", S.explore.toUpperCase());
  const count = el("span", "ex-count", "");
  head.append(title, count, modeSwitch("panes"));
  const search = el("input", "ex-search");
  search.type = "search";
  search.placeholder = S.filterExplore;
  search.addEventListener("input", () => {
    view.query = search.value;
    drawList();
  });
  // What to show and how to group it, in one menu beside the field (the
  // Mac's filter button in its search field).
  const menu = el("details", "menu ex-menu");
  const summary = el("summary", "btn icon-only");
  summary.setAttribute("aria-label", S.filter);
  summary.append(icon("filter"));
  const pop = el("div", "popover");
  const shows = el("div", "seg ex-shows");
  shows.setAttribute("role", "radiogroup");
  for (const [value, word] of [["all", "showAll"], ["attention", "showAttention"], ["active", "showActive"]]) {
    const option = el("button", null, S[word]);
    option.type = "button";
    option.setAttribute("role", "radio");
    option.dataset.value = value;
    option.addEventListener("click", () => {
      view.show = value;
      drawList();
    });
    shows.append(option);
  }
  const grouping = el("select", "ex-grouping");
  for (const [value, word] of [["host", "groupHost"], ["status", "groupStatus"]]) {
    const option = el("option", null, S[word]);
    option.value = value;
    grouping.append(option);
  }
  grouping.addEventListener("change", () => {
    view.grouping = grouping.value;
    drawList();
  });
  // The order, as the Mac's Order picker in the same menu.
  const sort = el("select", "ex-sort");
  for (const value of EXPLORE_SORTS) {
    const option = el("option", null, S.exploreSorts[value]);
    option.value = value;
    sort.append(option);
  }
  sort.value = view.sort;
  sort.addEventListener("change", () => {
    view.sort = exploreSortOf(sort.value);
    storeSort(view.sort);
    drawList();
  });
  const groupLabel = el("label", null, S.exploreGroup);
  groupLabel.append(grouping);
  const sortLabel = el("label", null, S.exploreOrder);
  sortLabel.append(sort);
  pop.append(shows, groupLabel, sortLabel);
  menu.append(summary, pop);
  const controls = el("div", "ex-controls");
  const field = el("div", "ex-field");
  field.append(icon("search"), search, menu);
  // The workspace a `#workspace=` link narrowed Explore to, with a way out.
  const scope = el("div", "ex-scope");
  scope.hidden = true;
  controls.append(field, scope);
  const list = el("div", "ex-list");
  list.setAttribute("role", "tree");
  sidebar.append(head, controls, list);

  function row(item) {
    const node = el("button", `ex-row ${item.group}${view.open?.id === item.id ? " selected" : ""}`);
    node.type = "button";
    node.setAttribute("role", "treeitem");
    node.dataset.id = item.id;
    node.append(el("span", `ex-dot ${item.group}`));
    const mark = item.agent ? logo(item.agent.product) : null;
    if (mark) node.append(mark);
    else node.append(icon(item.agent ? "people" : "terminal"));
    node.append(el("span", "ex-name", item.title || item.pane));
    node.classList.add("pane");
    node.append(el("span", "ex-sub", item.agent ? S.stateLabels[item.agent.state] || item.agent.state : item.command));
    node.addEventListener("click", () => openPane(item));
    return node;
  }

  /// A session or window row: the chevron folds it, the rest opens its
  /// detail (the Mac's sidebar selects a session or window the same way).
  function treeRow(kind, key, closed, glyph, name, n, open) {
    const node = el("div", `${kind}${closed ? " closed" : ""}${view.detailKey === key ? " selected" : ""}`);
    node.setAttribute("role", "treeitem");
    node.setAttribute("aria-expanded", String(!closed));
    const fold = el("button", "ex-fold");
    fold.type = "button";
    fold.setAttribute("aria-label", S.toggleFold);
    fold.append(icon("chevron"));
    fold.addEventListener("click", (event) => {
      event?.stopPropagation?.();
      if (closed) view.collapsed.delete(key);
      else view.collapsed.add(key);
      drawList();
    });
    const pick = el("button", "ex-pick");
    pick.type = "button";
    pick.append(icon(glyph), el("span", "ex-name", name), el("span", "ex-n", n));
    pick.addEventListener("click", open);
    node.append(fold, pick);
    return node;
  }

  function drawScope() {
    const ws = view.workspace;
    scope.hidden = !ws;
    if (!ws) {
      scope.replaceChildren();
      return;
    }
    const text = el("span", "ex-scope-text", S.workspaceFilter(ws));
    const clear = button(S.clearScope, () => {
      view.workspace = null;
      writeHash(view.open ? paneSelection(view.open) : view.detail);
      drawList();
    }, "btn ghost small");
    scope.replaceChildren(icon("filter"), text, clear);
  }

  function drawList() {
    for (const option of shows.children) option.setAttribute("aria-checked", String(option.dataset.value === view.show));
    drawScope();
    if (!view.fleet) {
      list.replaceChildren(el("div", "ex-empty", S.loading));
      return;
    }
    grouping.value = view.grouping;
    summary.title = S.exploreMenuHelp(grouping.value === "status" ? S.groupStatus : S.groupHost, S.exploreSorts[view.sort]);
    const model = exploreSections(view.fleet, { show: view.show, query: view.query, grouping: view.grouping, sort: view.sort, attention: view.attention, workspace: view.workspace });
    // "shown/all" while a filter is on, as the Mac's count.
    count.textContent = model.filtering ? `${model.total}/${model.all}` : String(model.all);
    if (model.empty) {
      const empty = el("div", "ex-empty");
      if (model.empty === "noWorkspace") {
        empty.textContent = S.noWorkspaceMatch(view.workspace);
      } else if (model.empty === "none") {
        empty.append(el("div", "ex-empty-title", S.noPanesDetected), el("div", "ex-empty-detail", S.noPanesDetectedDetail));
      } else {
        empty.textContent = S.noMatchingPanes;
      }
      list.replaceChildren(el("div", "ex-section", S.topology), empty);
      return;
    }
    const nodes = [];
    if (model.attention.length && view.grouping === "host") {
      nodes.push(el("div", "ex-group attention", `${S.needsAttention} · ${model.attention.length}`));
      nodes.push(...model.attention.slice(0, PINNED_ATTENTION_LIMIT).map(row));
      if (model.attention.length > PINNED_ATTENTION_LIMIT) {
        nodes.push(
          button(S.showAllCount(model.attention.length), () => {
            view.grouping = "status";
            drawList();
          }, "ex-more"),
        );
      }
    }
    if (view.grouping === "host") nodes.push(el("div", "ex-section", S.topology));
    for (const section of model.sections) {
      if (section.kind === "host") {
        const header = el("div", "ex-host");
        header.append(icon("chevron"), hostBadge(section.title, section.local));
        const facts = [section.state !== "online" ? section.state : "", section.version].filter(Boolean).join(" · ");
        header.append(el("span", "ex-facts", facts), el("span", "ex-n", String(section.rows.length)), el("span", `dot ${section.state === "online" ? "" : "warn"}`));
        header.title = [section.error, section.latency != null ? `${section.latency} ms` : ""].filter(Boolean).join(" · ");
        nodes.push(header);
        for (const session of section.sessions) {
          const sessionKey = treeKey(section, session);
          // A filter opens everything it matched in (the Mac's
          // `forceExpanded`).
          const sessionClosed = !model.filtering && view.collapsed.has(sessionKey);
          const sessionRow = treeRow("ex-session", sessionKey, sessionClosed, "layers", session.name || "—", String(session.rows.length), () => openDetail(sessionSelection(section, session)));
          // This host's sessions open Messages scoped to them (another
          // host's messages are on that host's daemon).
          if (section.local && session.name) {
            const messages = button("", () => go(messagesLink(sessionMessagesState({ host: section.title, local: true, session: session.name, socket: session.socket }))), "btn ghost icon-only ex-session-messages", "people");
            messages.setAttribute("aria-label", S.sessionMessages);
            messages.title = S.sessionMessages;
            sessionRow.append(messages);
          }
          nodes.push(sessionRow);
          if (sessionClosed) continue;
          for (const window of session.windows) {
            const key = treeKey(section, window);
            const closed = !model.filtering && view.collapsed.has(key);
            nodes.push(treeRow("ex-window", key, closed, "window", window.name || window.index, `#${window.index} · ${window.rows.length}`, () => openDetail(windowSelection(section, session, window))));
            if (!closed) nodes.push(...window.rows.map(row));
          }
        }
        if (!section.rows.length) nodes.push(el("div", "ex-empty", section.state === "online" ? S.noPanes : section.state));
      } else {
        nodes.push(el("div", `ex-group ${section.key}`, `${S.statusGroups[section.key] || section.key} · ${section.rows.length}`));
        nodes.push(...section.rows.map(row));
      }
    }
    if (!nodes.length) nodes.push(el("div", "ex-empty", S.noPanes));
    list.replaceChildren(...nodes);
  }

  function closeStream() {
    view.stream?.close?.();
    view.stream = null;
    clearInterval(view.poll);
    view.poll = null;
  }

  /// The address names what is open, and keeps the workspace filter.
  function writeHash(selection) {
    const hash = exploreHash(selection, view.workspace);
    view.hash = hash;
    history.replaceState(null, "", `${window.location.pathname}${hash}`);
  }

  function openPane(item, { push = true } = {}) {
    closeStream();
    view.open = item;
    view.detail = null;
    view.detailKey = null;
    if (push) writeHash(paneSelection(item));
    drawList();
    drawViewer();
  }

  /// A session's or a window's detail in the main area.
  function openDetail(selection, { push = true } = {}) {
    closeStream();
    view.open = null;
    view.detail = selection;
    view.facts = null;
    if (push) writeHash(selection);
    drawViewer();
    drawList();
  }

  function fact(list, label, value) {
    if (!value) return;
    list.append(el("dt", null, label), el("dd", null, value));
  }

  function card(item) {
    const box = el("article", `ex-card ${item.group}`);
    const head = el("button", "ex-card-head");
    head.type = "button";
    head.append(el("span", `ex-dot ${item.group}`));
    const mark = item.product ? logo(item.product) : null;
    head.append(mark || icon(item.state ? "people" : "terminal"));
    head.append(el("span", "ex-card-title", item.title));
    if (item.state) head.append(el("span", `ex-state ${item.group}`, S.stateLabels[item.state] || item.state));
    head.append(el("span", "ex-card-pane", item.pane), icon("chevron", "ex-card-go"));
    head.addEventListener("click", () => openPane(item.row));
    box.append(head);
    const text = item.latest || (item.state ? S.noSummary : item.command);
    if (text) box.append(el("p", `ex-card-text${item.latest ? "" : " none"}`, text));
    if (item.activityAt) box.append(el("div", "ex-card-meta", `${S.exploreSorts.recent} · ${when(item.activityAt)}`));
    return box;
  }

  function cardGrid(cards) {
    const grid = el("div", "ex-cards");
    grid.append(...cards.map(card));
    if (!cards.length) grid.append(el("div", "ex-empty", S.noPanes));
    return grid;
  }

  function drawDetail() {
    if (!view.fleet) {
      viewer.replaceChildren(el("div", "ex-placeholder", S.loading));
      return;
    }
    // A redraw would close the facts someone is reading.
    if (view.facts?.open) return;
    const detail = exploreDetail(view.fleet, view.detail, { sort: view.sort, attention: view.attention });
    const before = viewer.querySelector?.(".ex-detail");
    const scroll = before ? before.scrollTop : 0;
    if (!detail || detail.kind === "gone") {
      view.detailKey = null;
      const empty = el("div", "ex-placeholder");
      empty.append(icon("explore", "big"), el("p", null, S.detailGone));
      viewer.replaceChildren(empty);
      return;
    }
    view.detailKey = detail.key;
    const windowView = detail.kind === "window";
    const top = el("header", "ex-pane-head ex-detail-head");
    const badge = el("span", "ex-detail-icon");
    badge.append(icon(windowView ? "window" : "layers"));
    top.append(badge);
    const titles = el("div", "titles");
    titles.append(el("h2", null, detail.name || (windowView ? detail.id : detail.sessionId) || "—"));
    const meta = el("div", "sub");
    meta.append(el("span", null, windowView ? S.windowMeta({ host: detail.host, panes: detail.paneCount }) : S.sessionMeta({ host: detail.host, windows: detail.windowCount, panes: detail.paneCount })));
    if (windowView && detail.work) meta.append(el("span", "ex-work", detail.work));
    titles.append(meta);
    top.append(titles);
    if (detail.local && (windowView ? detail.session : detail.name)) {
      top.append(button(S.paneMessages, () => go(messagesLink(sessionMessagesState({ host: detail.host, local: true, session: windowView ? detail.session : detail.name, socket: detail.socket }))), "btn small", "people"));
    }
    // Details: the runtime identity, as the Mac's popover.
    const facts = el("details", "menu ex-facts-menu");
    const factsButton = el("summary", "btn small");
    factsButton.append(el("span", null, windowView ? S.runtimeDetails : S.details));
    const pop = el("div", "popover");
    const list = el("dl", "ex-facts-list");
    fact(list, S.factHost, detail.host);
    fact(list, S.factSocket, detail.socket);
    fact(list, S.factSession, windowView ? detail.session : detail.sessionId || detail.name);
    if (windowView) {
      fact(list, S.factWindow, detail.id);
      fact(list, S.factIndex, detail.index);
    }
    pop.append(list);
    facts.append(factsButton, pop);
    view.facts = facts;
    top.append(facts);

    const body = el("div", "ex-detail");
    if (detail.fallback) body.append(el("p", "flag warn", S.windowFellBack));
    if (windowView) {
      const metrics = el("div", "ex-metrics");
      for (const [label, value] of [
        [S.metricAgents, detail.metrics.agents],
        [S.metricWorking, detail.metrics.working],
        [S.metricAttention, detail.metrics.attention],
        [S.metricSubagents, detail.metrics.subagents],
        [S.metricProcesses, detail.metrics.processes],
      ]) {
        const metric = el("div", "ex-metric");
        metric.append(el("span", "ex-metric-value", String(value)), el("span", "ex-metric-label", label));
        metrics.append(metric);
      }
      body.append(metrics);
      const heading = el("div", "ex-detail-heading");
      heading.append(el("h3", null, S.agentReports), el("p", null, S.agentReportsHelp));
      body.append(heading, cardGrid(detail.cards));
    } else {
      const heading = el("div", "ex-detail-heading");
      heading.append(el("h3", null, S.windowsHeading), el("p", null, S.windowsHelp));
      body.append(heading);
      for (const window of detail.windows) {
        const section = el("section", "ex-detail-window");
        const head = el("button", "ex-window-head");
        head.type = "button";
        const name = el("span", "ex-window-name");
        name.append(el("span", "ex-name", window.name || window.id));
        if (window.work) name.append(el("span", "ex-work", window.work));
        head.append(icon("window"), name, el("span", "ex-n", S.windowCardMeta({ index: window.index, panes: window.paneCount })), icon("chevron", "ex-card-go"));
        head.addEventListener("click", () => openDetail(window.selection));
        section.append(head, cardGrid(window.cards));
        body.append(section);
      }
    }
    viewer.replaceChildren(top, body);
    body.scrollTop = scroll;
  }

  function drawViewer() {
    const item = view.open;
    if (!item && view.detail) return drawDetail();
    if (!item) {
      const empty = el("div", "ex-placeholder");
      empty.append(icon("explore", "big"), el("p", null, S.selectPane));
      viewer.replaceChildren(empty);
      return;
    }
    const top = el("header", "ex-pane-head");
    const titles = el("div", "titles");
    titles.append(el("h2", null, item.title || item.pane));
    const meta = el("div", "sub");
    meta.append(hostBadge(item.host, item.local), el("span", null, [item.session, item.window, item.repo].filter(Boolean).join(" · ")));
    titles.append(meta);
    top.append(titles);
    if (item.agent) top.append(el("span", `ex-state ${item.group}`, S.stateLabels[item.agent.state] || item.agent.state));
    if (item.local) top.append(button(S.paneMessages, () => go(messagesLink(paneMessagesState(item))), "btn small", "people"));
    const output = el("pre", "ex-output");
    output.setAttribute("aria-live", "off");
    const status = el("p", "ex-status");
    const nodes = [top, output, status];
    if (!item.local) {
      status.textContent = S.remotePane;
    } else {
      startOutput(item, output, status);
      if (canWrite()) nodes.push(composer(item, status));
    }
    viewer.replaceChildren(...nodes);
  }

  function startOutput(item, output, status) {
    const params = new URLSearchParams({ lines: String(LINES) });
    if (item.socket) params.set("socket", item.socket);
    const pinned = () => output.scrollHeight - output.scrollTop - output.clientHeight < 40;
    const show = (text) => {
      const stick = pinned();
      output.textContent = text || "";
      if (stick) output.scrollTop = output.scrollHeight;
    };
    const poll = () => {
      clearInterval(view.poll);
      const tick = async () => {
        try {
          const data = await read(`/api/panes/${encodeURIComponent(item.pane)}/output?${params}`);
          show(data.text);
          status.textContent = "";
        } catch (error) {
          status.textContent = error.status === 401 || error.status === 403 ? S.outputOperatorOnly : error.message;
          if (error.auth) {
            clearInterval(view.poll);
            output.textContent = "";
          }
        }
      };
      tick();
      view.poll = setInterval(tick, POLL_MS);
    };
    view.stream = openPaneStream({
      url: `/api/panes/${encodeURIComponent(item.pane)}/output/stream?${params}`,
      headers: authHeaders,
      onOutput: (payload) => {
        if (!view.closed && view.open?.id === item.id) show(payload?.text);
      },
      onGone: () => (status.textContent = S.paneGone),
      onState: (state) => {
        if (state === "live") status.textContent = "";
        else if (state === "denied") {
          // Refused now: what it showed before goes too.
          output.textContent = "";
          status.textContent = S.outputOperatorOnly;
        }
        else if (state === "unsupported") poll();
      },
    });
  }

  function composer(item, status) {
    const box = el("form", "ex-composer composer");
    box.addEventListener("submit", (event) => event.preventDefault());
    const area = el("textarea", "field");
    area.rows = 2;
    area.placeholder = S.promptPlaceholder;
    // A draft belongs to its pane: text for one pane never goes to another.
    const key = item.id;
    area.value = view.drafts.get(key) || "";
    area.disabled = view.sending.has(key);
    area.maxLength = 16384;
    const send = button(S.send, () => submit(), "btn primary");
    const interrupt = button(S.interrupt, async () => {
      try {
        const body = item.socket ? { socket: item.socket } : {};
        await write(`/api/panes/${encodeURIComponent(item.pane)}/abort`, body);
      } catch (error) {
        status.textContent = error.message;
      }
    }, "btn ghost danger");
    const sync = () => {
      area.disabled = view.sending.has(key);
      send.disabled = !area.value.trim() || view.sending.has(key);
    };
    view.syncComposer = sync;
    // The composer on screen now, which a send that started on an earlier
    // copy of this pane's view completes into.
    view.current = { key, area, status };
    async function submit() {
      const text = area.value;
      if (!text.trim() || view.sending.has(key) || view.open?.id !== key) return;
      // One send per pane at a time; other panes stay usable.
      view.sending.add(key);
      // Frozen while sending, so nothing typed now is lost when it clears.
      area.disabled = true;
      sync();
      try {
        const body = { text, submit: true };
        if (item.socket) body.socket = item.socket;
        await write(`/api/panes/${encodeURIComponent(item.pane)}/prompt`, body);
        if (view.drafts.get(key) === text) view.drafts.delete(key);
        // Clear the field showing this pane now (the view may have been
        // redrawn while the send was pending), only if it still holds what
        // was sent.
        const shown = view.current?.key === key ? view.current : { area, status };
        if (shown.area.value === text) shown.area.value = "";
        shown.status.textContent = S.sent;
      } catch (error) {
        (view.current?.key === key ? view.current.status : status).textContent = error.message;
      } finally {
        view.sending.delete(key);
        // Whichever pane is open now gets its composer back.
        view.syncComposer?.();
      }
    }
    area.addEventListener("input", () => {
      if (area.value) view.drafts.set(key, area.value);
      else view.drafts.delete(key);
      sync();
    });
    area.addEventListener("keydown", (event) => {
      // Enter sends, Shift+Enter is a new line (the pane drawer's keys).
      if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
        event.preventDefault();
        submit();
      }
    });
    sync();
    const row = el("div", "row end");
    row.append(interrupt, send);
    box.append(area, row);
    return box;
  }

  /// A pane link: on mount, and again whenever the link itself changes.
  function showLinkedPane() {
    const fleet = view.fleet;
    const hash = window.location.hash;
    const selection = selectionFromHash(hash);
    const changed = hash !== view.hash;
    view.hash = hash;
    // The workspace a link names narrows the tree until it is cleared.
    if (changed) view.workspace = workspaceFromHash(hash);
    // Do not redraw the list under someone typing in the filter.
    if (document.activeElement !== search) drawList();
    if (view.suspended && view.open) {
      view.suspended = false;
      drawViewer();
    }
    if (selection?.kind === "pane") {
      if ((changed || !view.open) && fleet) {
        const rows = exploreSections(fleet, { show: "all" }).sections.flatMap((s) => s.rows);
        // A link without a host names this host's pane (paneHash).
        const found = rows.find(
          (r) => r.pane === selection.pane && sameSocket(r.socket, selection.socket) && (selection.host ? r.host === selection.host : r.local),
        );
        if (found && found.id !== view.open?.id) openPane(found, { push: false });
      }
    } else if (selection && changed) {
      openDetail(selection, { push: false });
    } else if (view.detail && !view.open) {
      // A session or window detail follows the Fleet as it changes.
      drawDetail();
      if (document.activeElement !== search) drawList();
    }
  }

  sync();

  return {
    /// `attention`: the daemon board's needs-attention per agent, when it
    /// sent one (boardAttention).
    update(fleet, attention = null) {
      if (view.closed) return;
      view.fleet = fleet;
      view.attention = attention;
      sync();
    },
    /// Off screen (another page of the shell): no live stream or poll for a
    /// view nobody sees; the next update reopens the pane.
    suspend() {
      closeStream();
      view.suspended = true;
      view.messages?.suspend();
    },
    /// Everything private gone: the stream, the output, the drafts, and
    /// no later update draws it back.
    destroy() {
      view.closed = true;
      closeStream();
      view.messages?.destroy();
      view.messages = null;
      view.drafts.clear();
      view.open = null;
      view.detail = null;
      view.fleet = null;
      root.replaceChildren();
    },
  };
}
