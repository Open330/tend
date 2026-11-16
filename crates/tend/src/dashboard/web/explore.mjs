// Explore in the new shell: Tend for Mac's Explore sidebar (every host's
// panes, by host tree or status group, needing-attention first) and the
// pane it opens: live output and a composer, as the Mac's terminal tab
// shows a pane. Output and prompts are operator-only and local-only (a
// pane on another Fleet host is listed with its state, and opens on that
// host). The page wires DOM helpers in (`deps`) so this module stays
// testable.

import { exploreSections, paneFromHash, paneHash } from "./explore-model.mjs";
import { openPaneStream } from "./pane-stream.mjs";

const LINES = 400;
const POLL_MS = 1500;

export function mountExplore(root, deps) {
  const { S, el, icon, button, logo, hostBadge, when, read, write, authHeaders, canWrite } = deps;
  const view = { show: "all", query: "", grouping: "host", fleet: null, open: null, stream: null, poll: null, sending: null, drafts: new Map(), collapsed: new Set(), closed: false, syncComposer: null, hash: null };

  const sidebar = el("aside", "ex-side");
  const viewer = el("section", "ex-view");
  root.replaceChildren(sidebar, viewer);

  const head = el("header", "ex-head");
  const title = el("span", "ex-title", S.explore.toUpperCase());
  const count = el("span", "ex-count", "");
  head.append(title, count);
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
  pop.append(shows, grouping);
  menu.append(summary, pop);
  const controls = el("div", "ex-controls");
  const field = el("div", "ex-field");
  field.append(icon("search"), search, menu);
  controls.append(field);
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

  function drawList() {
    for (const option of shows.children) option.setAttribute("aria-checked", String(option.dataset.value === view.show));
    if (!view.fleet) {
      list.replaceChildren(el("div", "ex-empty", S.loading));
      return;
    }
    const model = exploreSections(view.fleet, { show: view.show, query: view.query, grouping: view.grouping });
    count.textContent = String(model.total);
    const nodes = [];
    if (view.grouping === "host") nodes.push(el("div", "ex-section", S.topology));
    if (model.attention.length && view.grouping === "host") {
      nodes.push(el("div", "ex-group attention", `${S.needsAttention} · ${model.attention.length}`));
      nodes.push(...model.attention.map(row));
    }
    for (const section of model.sections) {
      if (section.kind === "host") {
        const header = el("div", "ex-host");
        header.append(icon("chevron"), hostBadge(section.title, section.local));
        const facts = [section.state !== "online" ? section.state : "", section.version].filter(Boolean).join(" · ");
        header.append(el("span", "ex-facts", facts), el("span", "ex-n", String(section.rows.length)), el("span", `dot ${section.state === "online" ? "" : "warn"}`));
        header.title = [section.error, section.latency != null ? `${section.latency} ms` : ""].filter(Boolean).join(" · ");
        nodes.push(header);
        for (const session of section.sessions) {
          const sessionRow = el("div", "ex-session");
          sessionRow.append(icon("chevron"), icon("layers"), el("span", "ex-name", session.name || "—"), el("span", "ex-n", String(session.rows.length)));
          nodes.push(sessionRow);
          for (const window of session.windows) {
            const key = `${section.title}\u0000${window.key}`;
            const closed = view.collapsed.has(key);
            const windowRow = el("button", `ex-window${closed ? " closed" : ""}`);
            windowRow.type = "button";
            windowRow.setAttribute("aria-expanded", String(!closed));
            windowRow.append(icon("chevron"), icon("window"), el("span", "ex-name", window.name || window.index), el("span", "ex-n", `#${window.index} · ${window.rows.length}`));
            windowRow.addEventListener("click", () => {
              if (closed) view.collapsed.delete(key);
              else view.collapsed.add(key);
              drawList();
            });
            nodes.push(windowRow);
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

  function openPane(item, { push = true } = {}) {
    closeStream();
    view.open = item;
    if (push) history.replaceState(null, "", `${window.location.pathname}${paneHash(item)}`);
    drawList();
    drawViewer();
  }

  function drawViewer() {
    const item = view.open;
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
    area.disabled = view.sending === key;
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
      area.disabled = view.sending === key;
      send.disabled = !area.value.trim() || view.sending === key;
    };
    view.syncComposer = sync;
    async function submit() {
      const text = area.value;
      if (!text.trim() || view.sending === key || view.open?.id !== key) return;
      view.sending = key;
      // Frozen while sending, so nothing typed now is lost when it clears.
      area.disabled = true;
      sync();
      try {
        const body = { text, submit: true };
        if (item.socket) body.socket = item.socket;
        await write(`/api/panes/${encodeURIComponent(item.pane)}/prompt`, body);
        if (view.drafts.get(key) === text) view.drafts.delete(key);
        if (area.value === text) area.value = "";
        status.textContent = S.sent;
      } catch (error) {
        status.textContent = error.message;
      } finally {
        if (view.sending === key) view.sending = null;
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

  drawList();
  drawViewer();

  return {
    update(fleet) {
      if (view.closed) return;
      view.fleet = fleet;
      // Do not redraw the list under someone typing in the filter.
      if (document.activeElement !== search) drawList();
      // A pane link: on mount, and again whenever the link itself changes.
      const hash = window.location.hash;
      const wanted = paneFromHash(hash);
      const changed = hash !== view.hash;
      view.hash = hash;
      if (view.suspended && view.open) {
        view.suspended = false;
        drawViewer();
      }
      if ((changed || !view.open) && wanted && fleet && !(view.open && view.open.pane === wanted.pane && view.open.socket === wanted.socket)) {
        const rows = exploreSections(fleet, { show: "all" }).sections.flatMap((s) => s.rows);
        const found = rows.find((r) => r.pane === wanted.pane && (!wanted.socket || r.socket === wanted.socket) && (!wanted.host || r.host === wanted.host));
        if (found) openPane(found, { push: false });
      }
    },
    /// Off screen (another page of the shell): no live stream or poll for a
    /// view nobody sees; the next update reopens the pane.
    suspend() {
      closeStream();
      view.suspended = true;
    },
    /// Everything private gone: the stream, the output, the drafts, and
    /// no later update draws it back.
    destroy() {
      view.closed = true;
      closeStream();
      view.drafts.clear();
      view.open = null;
      view.fleet = null;
      root.replaceChildren();
    },
  };
}
