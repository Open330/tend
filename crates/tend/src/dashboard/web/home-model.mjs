// Pure view model for Home's other two lanes. "Your turn" is the decision
// queue (needs-model.mjs); "In progress" and the agents half of
// "Finished" come from the Fleet snapshot (/api/fleet), which carries every
// host's agents, this one's included. No DOM here, so node's test runner
// covers it; the page renders cards with textContent only.
//
// The lanes and their words follow the Mac app's Home (FleetLane in
// apps/tend-macos/Sources/FleetHome.swift). Its unit rules (Work grouping,
// helper folding, pipeline stages) are not repeated here: an agent is one
// row.

import { ago } from "./needs-model.mjs";
import { isPaneless, needsAttention } from "./fleet-view.mjs";

/// How long an agent that went idle still counts as recently finished.
export const FINISHED_WINDOW_MS = 24 * 60 * 60 * 1000;

const RUNNING = new Set(["working", "starting"]);

function firstLine(text) {
  return String(text || "").split("\n").find((line) => line.trim())?.trim() || "";
}

function since(agent) {
  return agent.state_entered_at || agent.last_activity_at || null;
}

function where(agent) {
  const session = agent.tmux_session || "";
  const pane = String(agent.pane || "").replace(/^[a-z]+:/, "");
  return [session, pane].filter(Boolean).join(" · ");
}

function repo(cwd) {
  const parts = String(cwd || "").split(/[\\/]/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : "";
}

/// One agent as a card: what it is on, where it runs, since when.
export function agentCard(agent, host, now = Date.now()) {
  const label = String(agent.state || "").replaceAll("_", " ");
  const title =
    firstLine(agent.ai_title) ||
    firstLine(agent.recap) ||
    firstLine(agent.last_prompt) ||
    firstLine(agent.last_notification) ||
    (RUNNING.has(agent.state) ? "working" : "no task recorded");
  const when = ago(since(agent), now);
  const meta = [agent.kind, repo(agent.cwd), where(agent), when].filter(Boolean).join(" · ");
  const card = {
    id: `${host.alias}:${agent.agent_session_id || agent.pane || ""}`,
    kind: "agent",
    label,
    title,
    meta,
    needed: null,
    options: [],
    recommendation: null,
    evidence: [],
    checks: null,
    host: host.alias,
    remote: !host.local,
    actionable: false,
    // Only this host's panes open in this dashboard.
    pane: host.local ? agent.pane || null : null,
    socket: host.local ? agent.tmux_socket || null : null,
  };
  if (!RUNNING.has(agent.state)) {
    const said = firstLine(agent.last_response);
    if (said) card.outcome = said;
  }
  return card;
}

function hostsOf(fleet) {
  return Array.isArray(fleet?.hosts) ? fleet.hosts : [];
}

function liveAgents(host) {
  const agents = Array.isArray(host?.remote?.agents) ? host.remote.agents : [];
  return agents.filter((agent) => !isPaneless(agent) && agent.state !== "stopped");
}

function instant(agent) {
  const at = Date.parse(since(agent) || "");
  return Number.isFinite(at) ? at : 0;
}

function newestFirst(a, b) {
  return instant(b) - instant(a);
}

/// Waiting on the person even while idle: it asked a question in its pane
/// (the daemon's `awaiting_reply`). Such an agent is in Your turn.
function awaitingReply(agent) {
  return agent?.awaiting_reply != null;
}

function notesOf(host) {
  if (host.state === "online" || host.local) return [];
  return [host.error ? `${host.state || "offline"}: ${host.error}` : `${host.state || "offline"}`];
}

/// Local first, then the other hosts by name, like the decision queue.
function ordered(fleet) {
  return [...hostsOf(fleet)].sort(
    (a, b) => Number(Boolean(b.local)) - Number(Boolean(a.local)) || String(a.alias || "").localeCompare(String(b.alias || "")),
  );
}

/// "In progress": agents working or starting, per host, newest first.
/// An agent waiting on the person is in "Your turn", never here.
export function progressSections(fleet, now = Date.now()) {
  return ordered(fleet).map((host) => ({
    alias: host.alias,
    local: Boolean(host.local),
    covers: [],
    notes: notesOf(host),
    cards: liveAgents(host)
      .filter((agent) => RUNNING.has(agent.state) && !needsAttention(agent) && !awaitingReply(agent))
      .sort(newestFirst)
      .map((agent) => agentCard(agent, host, now)),
  }));
}

/// The agents half of "Finished": idle agents that did something (a
/// prompt or a response) within `window`, per host, newest first.
export function finishedAgentSections(fleet, now = Date.now(), window = FINISHED_WINDOW_MS) {
  return ordered(fleet).map((host) => ({
    alias: host.alias,
    local: Boolean(host.local),
    covers: [],
    notes: [],
    cards: liveAgents(host)
      .filter((agent) => agent.state === "idle" && !awaitingReply(agent) && (agent.last_response || agent.last_prompt))
      .filter((agent) => {
        const at = Date.parse(since(agent) || "");
        // A clock a little ahead is tolerated; the future is not "finished".
        return Number.isFinite(at) && now - at <= window && at - now <= 60_000;
      })
      .sort(newestFirst)
      .map((agent) => agentCard(agent, host, now)),
  }));
}

/// "Finished": what was decided (history sections), then the agents that
/// finished, merged per host so each host has one heading.
export function finishedSections(history, agents) {
  const byAlias = new Map();
  for (const section of [...history, ...agents]) {
    const known = byAlias.get(section.alias);
    if (known) {
      known.cards.push(...section.cards);
      known.notes.push(...section.notes.filter((note) => !known.notes.includes(note)));
    } else {
      byAlias.set(section.alias, { ...section, notes: [...section.notes], cards: [...section.cards] });
    }
  }
  return [...byAlias.values()];
}

/// How many cards the sections hold.
export function cardCount(sections) {
  return sections.reduce((sum, section) => sum + section.cards.length, 0);
}
