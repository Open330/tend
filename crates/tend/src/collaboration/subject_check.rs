//! Questions nobody can answer: a human item whose **subject** is
//! established gone is folded out of the person's turn, and closed only by
//! a person (`docs/proposals/STALE_QUESTIONS.md`).
//!
//! Each open human item has one subject per original request it covers:
//!
//! | Item | Subject |
//! | --- | --- |
//! | An agent's question | its sender (`from`) |
//! | A recovery item | the original request's recipient (`parent.to`) |
//! | A reply-wake item | the original request's sender (`parent.from`) |
//!
//! An item folds only when the subject of every request it covers is gone.
//! A subject on another Fleet host, the operator console, or an address
//! still waiting for its session is never judged here.
//!
//! **A live session anywhere on this host wins**: a bound, not-Stopped row
//! for the session id (`recovery::exact_agent`, the session's own row after
//! a resume or `tend restore`) makes the subject alive. Otherwise only
//! these count, each from something tendd recorded as it happened or a
//! fresh scan, never a Stopped row read back later:
//!
//! - **E1, ended**: the provider's `SessionEnded` hook for the session
//!   (a fact written at ingestion, `subject_evidence.rs`).
//! - **E2, replaced**: a real provider `Started` hook for another session
//!   in its pane, on the same server incarnation (a fact written before
//!   the predecessor's row changed).
//! - **E3, pane gone**: the pane is absent from a complete tmux or rmux
//!   scan of a socket that answered, unless the server incarnation is known
//!   to have changed since the session was bound there.
//! - **E4, agent exited**: exactly what `stop_rows_without_agent_process`
//!   requires for the pane (a complete host, a recognized agent kind, the
//!   inactivity floor, no agent process in the workload, a shell in the
//!   foreground), on the incarnation the session was bound on.
//!
//! Everything else, `binding_unavailable`, a rate limit, a failed or
//! partial scan, is **unknown**.
//!
//! Every reconcile pass runs [`step`] on the item's [`SubjectCheck`]: alive
//! clears it, gone extends the episode, unknown pauses it (after
//! [`UNKNOWN_GAP`] only the grace clock restarts, never the episode or
//! [`SubjectCheck::episode_since`]: a daemon restart or a sleeping host is
//! such a gap, and only the subject seen alive ends the episode). An item
//! is unanswerable
//! when the episode is gone, confirmed in the current pass, gone for
//! [`SUBJECT_GONE_GRACE`], the item is that old too, and the person has not
//! kept this episode.
use super::{
    CollaborationError, CollaborationReply, CollaborationRequest, CollaborationStore, Participant,
    RequestStatus,
};
use crate::state::Agent;
use crate::subject_evidence::{FactKind, PaneScan, SubjectBinding, SubjectFact, UnnamedAgent};
use crate::{AgentKind, AgentState};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use time::OffsetDateTime;

/// `hello` capability: queue entries carry `unanswerable`, the queue
/// `unanswerable_count`, and `decision_keep` is served.
pub const SUBJECT_CHECK_CAPABILITY: &str = "subject_check_v1";

/// `hello` capability: the daemon takes `fleet_decision_keep` and
/// `fleet_decision_close_unanswerable`, Keep and Close all sent over Fleet,
/// and records the person's actor and the controller they came through. A
/// daemon without it refuses those kinds as unknown, so an updated relay
/// over an older daemon fails closed instead of closing without them.
pub const DECISION_VIA_FLEET_CAPABILITY: &str = "decision_via_fleet_v1";

/// `hello` capability: the daemon takes `decision_keep_on_host` and
/// `decision_close_on_host`, a person's Keep or Close of another Fleet
/// host's question sent from this host (`tend review keep|close --host`).
pub const DECISION_ON_HOST_CAPABILITY: &str = "decision_on_host_v1";

/// `hello` capability, only while `[review] close_unanswerable` is on:
/// `decision_close_unanswerable` closes what it is given.
pub const CLOSE_UNANSWERABLE_CAPABILITY: &str = "close_unanswerable_v1";

/// The refusal while closing is turned off.
pub const CLOSE_UNANSWERABLE_OFF: &str =
    "closing unanswerable questions is turned off; set [review] close_unanswerable = true";

/// How long a subject must have been established gone before its item
/// leaves the person's turn. Only long enough to absorb the ordinary ways a
/// live session is briefly unseen: an account swap or `tend reload`
/// relaunching it (a minute or two, and its resume process makes it alive
/// again at once), a daemon restart (the episode is durable, and
/// [`UNKNOWN_GAP`] covers the scans it misses), a reboot that `tend
/// restore` brings back. A fold is undone the moment the subject is seen
/// alive, and the item stays one click away under "Can't be answered"
/// with Keep, so waiting longer only keeps a question nobody can answer in
/// the person's turn (`docs/proposals/STALE_QUESTIONS.md`, "Grace").
pub const SUBJECT_GONE_GRACE: time::Duration = time::Duration::minutes(15);
/// How long the item itself must have been open: the same, so a question
/// whose asker exits right after asking is not folded before the person
/// could have seen it.
pub const ITEM_MIN_AGE: time::Duration = SUBJECT_GONE_GRACE;
/// An unknown stretch longer than this since the last confirmed gone
/// restarts the grace clock (not the episode).
pub const UNKNOWN_GAP: time::Duration = time::Duration::minutes(10);
/// A gone confirmation is written to the row at most this often; the
/// current pass keeps the exact time in memory.
const CONFIRM_PERSIST_EVERY: time::Duration = time::Duration::minutes(5);
/// A pass's confirmation counts for this long. Passes run at least every
/// 30 seconds while tendd is up.
pub const CONFIRMATION_FRESH: time::Duration = time::Duration::minutes(5);
/// A scan older than this shows nothing.
/// How long one unnamed agent (or one pane walk that did not finish) keeps
/// the present-state judgement from a subject it could be. A restart or
/// swap that tend drives names its session (`--resume <id>`), and an
/// agent a person starts unnamed gets a named row at its first prompt,
/// which brings its question back at once. One left unprompted this long
/// is not where the conversation goes on, and the fold it then allows is
/// still reachable under "Can't be answered". Close all never uses it.
pub const UNNAMED_AGENT_BOUND: time::Duration = time::Duration::hours(1);

const SCAN_FRESH: time::Duration = time::Duration::minutes(5);
/// The inactivity floor `stop_rows_without_agent_process` uses.
const AGENTLESS_PANE_IDLE: time::Duration = time::Duration::hours(1);

/// The body a dismissed question gets, exactly as Tend for Mac's Dismiss
/// sends it: the asking agent reads it.
pub const DISMISSED_BODY: &str = "The person dismissed this question from Tend Home without answering it. Do not wait for an answer to it; ask again only if it still matters.";

/// Surfaces a person closes from.
pub const SURFACES: [&str; 4] = ["cli", "web", "mac", "ios"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoneCause {
    /// E1: the session's own end hook.
    Ended,
    /// E2: another session started in its pane.
    Replaced,
    /// E4: its pane runs a shell and no agent.
    AgentExited,
    /// E3: its pane is gone.
    PaneGone,
}

impl GoneCause {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ended => "ended",
            Self::Replaced => "replaced by another agent",
            Self::AgentExited => "agent exited",
            Self::PaneGone => "pane gone",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubjectState {
    Gone,
    Unknown,
}

/// The evidence episode kept on an open item (`subject_check`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubjectCheck {
    pub episode: String,
    pub state: SubjectState,
    /// When the current grace clock started; `None` after an unknown gap,
    /// until the next confirmed gone.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub gone_since: Option<OffsetDateTime>,
    /// When the episode began: its first confirmed gone. An unknown gap
    /// (a daemon restart, a host asleep) restarts the grace clock above but
    /// never this; only the subject seen alive ends the episode. What the
    /// person reads as "gone since". `None` on a row written before the
    /// field, until the next pass fills it in ([`legacy_since`]).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub episode_since: Option<OffsetDateTime>,
    /// The last pass that confirmed gone, written at most every few
    /// minutes.
    #[serde(with = "time::serde::rfc3339")]
    pub confirmed_at: OffsetDateTime,
    #[serde(default)]
    pub causes: Vec<GoneCause>,
    /// The episode the person kept the item through, if any.
    #[serde(default)]
    pub kept_episode: Option<String>,
    /// The subject's pane, for the person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
}

/// A queue entry's `unanswerable` object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unanswerable {
    pub cause: GoneCause,
    pub episode: String,
    /// When the episode began ([`SubjectCheck::episode_since`]).
    #[serde(with = "time::serde::rfc3339")]
    pub since: OffsetDateTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClosureReason {
    /// The person closed it because its subject is gone.
    SubjectGone,
}

/// Why a person's decline closed a question, kept on the request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionClosure {
    pub reason: ClosureReason,
    pub cause: GoneCause,
    pub episode: String,
    /// Always `person`: nothing closes these without one.
    pub by: String,
    /// Where: `cli`, `web`, `mac` or `ios`.
    pub surface: String,
    /// The signed-in operator or paired device, when the surface knows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
}

/// One `(request id, episode)` pair Close all sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseItem {
    pub request_id: String,
    pub episode: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedClose {
    pub request_id: String,
    pub reason: String,
}

/// What `decision_close_unanswerable` did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseOutcome {
    pub closed: Vec<String>,
    pub skipped: Vec<SkippedClose>,
}

/// What `decision_keep` did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeepOutcome {
    pub request_id: String,
    pub episode: String,
}

/// What one subject check pass saw (for logs and tests).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubjectPassReport {
    /// Open items whose subject is gone this pass.
    pub gone: usize,
    /// Of those, unanswerable now.
    pub unanswerable: usize,
    /// Rows written.
    pub written: usize,
}

/// The inputs a pass reads from the agent registry.
#[derive(Debug, Clone, Copy)]
pub struct SubjectInputs<'a> {
    pub agents: &'a [Agent],
    pub scan: Option<&'a PaneScan>,
    pub bindings: &'a HashMap<String, SubjectBinding>,
}

/// The current pass: when it ran and which items it confirmed gone, in
/// which episode. Lost on restart, so nothing is unanswerable until the
/// first pass after start.
#[derive(Debug, Default)]
struct Pass {
    at: Option<OffsetDateTime>,
    confirmed: HashMap<String, (String, OffsetDateTime)>,
}

/// The collaboration store's subject-check state.
#[derive(Debug, Default)]
pub(super) struct SubjectRuntime {
    facts: std::sync::Mutex<Vec<SubjectFact>>,
    pass: std::sync::Mutex<Pass>,
    close_enabled: std::sync::atomic::AtomicBool,
}

impl SubjectRuntime {
    pub(super) fn with_facts(facts: Vec<SubjectFact>) -> Self {
        Self {
            facts: std::sync::Mutex::new(facts),
            ..Self::default()
        }
    }

    fn facts(&self) -> Vec<SubjectFact> {
        self.facts.lock().map(|f| f.clone()).unwrap_or_default()
    }

    fn confirmed(&self, id: &str) -> Option<(String, OffsetDateTime)> {
        self.pass
            .lock()
            .ok()
            .and_then(|pass| pass.confirmed.get(id).cloned())
    }

    fn snapshot(&self) -> HashMap<String, (String, OffsetDateTime)> {
        self.pass
            .lock()
            .map(|pass| pass.confirmed.clone())
            .unwrap_or_default()
    }
}

/// What a verdict is for. Folding only moves a question to "Can't be
/// answered", and it unfolds when its asker is seen again, so an unnamed
/// agent stops holding it after [`UNNAMED_AGENT_BOUND`]. Close all
/// declines the question for good: there an unnamed agent that could be
/// the asker holds it for as long as it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Purpose {
    Fold,
    Close,
}

/// A pass's verdict on one subject or item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Verdict {
    Alive,
    Gone {
        causes: Vec<GoneCause>,
        pane: Option<String>,
    },
    Unknown,
}

/// The state machine: alive clears the record; gone extends the episode
/// (the first gone opens one); unknown pauses it, and after
/// [`UNKNOWN_GAP`] without a confirmation restarts only the grace clock.
/// `last_confirmed` is the newest confirmation known, the row's or this
/// process's. `legacy` is when a row written before
/// [`SubjectCheck::episode_since`] began its episode ([`legacy_since`]).
pub(super) fn step(
    old: Option<&SubjectCheck>,
    verdict: &Verdict,
    now: OffsetDateTime,
    last_confirmed: Option<OffsetDateTime>,
    legacy: Option<OffsetDateTime>,
) -> Option<SubjectCheck> {
    let began = |old: &SubjectCheck| {
        old.episode_since
            .or(legacy)
            .or(old.gone_since)
            .or(Some(old.confirmed_at))
    };
    match verdict {
        Verdict::Alive => None,
        Verdict::Unknown => old.map(|old| {
            let mut next = old.clone();
            next.episode_since = began(old);
            next.state = SubjectState::Unknown;
            if last_confirmed.is_none_or(|at| now - at > UNKNOWN_GAP) {
                next.gone_since = None;
            }
            next
        }),
        Verdict::Gone { causes, pane } => Some(match old {
            Some(old) => {
                let gap = last_confirmed.is_none_or(|at| now - at > UNKNOWN_GAP);
                SubjectCheck {
                    episode: old.episode.clone(),
                    state: SubjectState::Gone,
                    gone_since: if gap || old.gone_since.is_none() {
                        Some(now)
                    } else {
                        old.gone_since
                    },
                    episode_since: began(old),
                    confirmed_at: now,
                    causes: causes.clone(),
                    kept_episode: old.kept_episode.clone(),
                    pane: pane.clone().or_else(|| old.pane.clone()),
                }
            }
            None => SubjectCheck {
                episode: format!("ep_{}", uuid::Uuid::new_v4().simple()),
                state: SubjectState::Gone,
                gone_since: Some(now),
                episode_since: Some(now),
                confirmed_at: now,
                causes: causes.clone(),
                kept_episode: None,
                pane: pane.clone(),
            },
        }),
    }
}

/// Whether `request` is unanswerable now, given the episode the newest
/// pass confirmed for it and when. A confirmation older than
/// [`CONFIRMATION_FRESH`] confirms nothing: the queue, its count and every
/// action see the item as live until a pass confirms it again.
pub(super) fn unanswerable(
    request: &CollaborationRequest,
    confirmed: Option<(&str, OffsetDateTime)>,
    now: OffsetDateTime,
) -> Option<Unanswerable> {
    let check = request.subject_check.as_ref()?;
    let since = check.gone_since?;
    (request.needs_human_response()
        && check.state == SubjectState::Gone
        && confirmed.is_some_and(|(episode, at)| {
            episode == check.episode && (now - at).abs() <= CONFIRMATION_FRESH
        })
        && now - since >= SUBJECT_GONE_GRACE
        && now - request.created_at >= ITEM_MIN_AGE
        && check.kept_episode.as_deref() != Some(check.episode.as_str()))
    .then(|| Unanswerable {
        cause: check.causes.first().copied().unwrap_or(GoneCause::PaneGone),
        episode: check.episode.clone(),
        since: check.episode_since.unwrap_or(since),
        pane: check.pane.clone(),
    })
}

/// The subjects of an open item: one per original request it covers, or
/// `None` when it has none this host may judge (a console sender, a
/// subject on another host, an address still pending). A covered request
/// that cannot be found is a subject nobody can show gone: `Some(None)`.
fn subjects<'a>(
    request: &'a CollaborationRequest,
    requests: &'a HashMap<String, CollaborationRequest>,
) -> Option<Vec<Option<&'a Participant>>> {
    use super::system_asks::{system_ask_kind, SystemAskKind};
    let Some(kind) = system_ask_kind(request) else {
        // An agent's own question. One that came over Fleet has its asker
        // on another host.
        return (!request.from.console
            && request.source_node_id.is_none()
            && request.delegation_parent.is_none())
        .then(|| vec![Some(&request.from)]);
    };
    let mut parents: Vec<&str> = request
        .parent_request_id
        .iter()
        .map(String::as_str)
        .collect();
    if let Some(state) = &request.system_ask {
        parents.extend(state.covers.iter().map(String::as_str));
    }
    if kind == SystemAskKind::Recovery {
        parents.extend(
            requests
                .values()
                .filter(|parent| {
                    parent.interruption.as_ref().is_some_and(|info| {
                        info.request_id == parent.id
                            && info.action_request_id.as_deref() == Some(request.id.as_str())
                    })
                })
                .map(|parent| parent.id.as_str()),
        );
    }
    parents.sort_unstable();
    parents.dedup();
    let mut out = Vec::new();
    for id in &parents {
        let Some(parent) = requests.get(*id) else {
            out.push(None);
            continue;
        };
        let subject = match kind {
            SystemAskKind::Recovery => &parent.to,
            SystemAskKind::ReplyWake => {
                if parent.source_node_id.is_some() || parent.delegation_parent.is_some() {
                    return None;
                }
                &parent.from
            }
        };
        out.push(Some(subject));
    }
    (!out.is_empty()).then_some(out)
}

/// The kinds `stop_rows_without_agent_process` recognizes by process.
fn recognized(kind: AgentKind) -> bool {
    matches!(
        kind,
        AgentKind::ClaudeCode | AgentKind::Codex | AgentKind::Antigravity | AgentKind::Copilot
    )
}

/// One subject's verdict this pass.
fn judge(
    subject: &Participant,
    stored: &BTreeMap<String, SubjectBinding>,
    inputs: &SubjectInputs<'_>,
    facts: &[SubjectFact],
    now: OffsetDateTime,
    purpose: Purpose,
) -> Verdict {
    let session = subject.agent_session_id.as_str();
    if subject.console
        || session.is_empty()
        || super::is_pending_session(session)
        || session.starts_with(crate::state::SYNTHETIC_SESSION_PREFIX)
        || session.starts_with("tend:")
        || session.starts_with("muxa:")
    {
        return Verdict::Unknown;
    }
    let row = super::recovery::exact_agent(subject, inputs.agents);
    if row.is_some_and(|row| row.state != AgentState::Stopped)
        || inputs
            .agents
            .iter()
            .any(|agent| agent.session_id == session && agent.state != AgentState::Stopped)
    {
        return Verdict::Alive;
    }
    // A process explicitly started on the session (`codex resume <id>`,
    // `claude --resume <id>`) is the session back, before its first prompt
    // gives it a row.
    if inputs.scan.is_some_and(|scan| {
        scan.hosts
            .iter()
            .flat_map(|host| &host.panes)
            .flat_map(|pane| &pane.resumed)
            .chain(&scan.outside_resumed)
            .any(|id| id == session)
    }) {
        return Verdict::Alive;
    }
    let mut causes = Vec::new();
    // A fact counts only if it is newer than the last activity the row
    // shows, so an end before a resume says nothing about the resumed run.
    let floor = row.map(|row| row.last_activity_at);
    for fact in facts.iter().filter(|fact| {
        fact.session_id == session && floor.is_none_or(|floor| fact.observed_at >= floor)
    }) {
        match fact.kind {
            FactKind::Ended => causes.push(GoneCause::Ended),
            FactKind::Replaced if fact.same_incarnation => causes.push(GoneCause::Replaced),
            FactKind::Replaced => {}
        }
    }
    let pane = row
        .and_then(|row| row.pane.clone())
        .or_else(|| (!subject.pane.is_empty()).then(|| subject.pane.clone()));
    let socket = row
        .and_then(|row| row.tmux_socket.clone())
        .or_else(|| subject.socket.clone());
    if let (Some(pane), Some(socket), Some(scan)) =
        (pane.as_deref(), socket.as_deref(), inputs.scan)
    {
        let binding = merged_binding(stored.get(session), inputs.bindings.get(session));
        let seen = Seen {
            session,
            kind: subject.agent_kind,
            purpose,
            row,
            binding: binding.as_ref(),
            agents: inputs.agents,
        };
        if let Some(cause) = scan_evidence(pane, socket, &seen, scan, now) {
            causes.push(cause);
        }
    }
    causes.sort_unstable();
    causes.dedup();
    if causes.is_empty() {
        Verdict::Unknown
    } else {
        Verdict::Gone { causes, pane }
    }
}

/// For tend's own permanent closes, on top of [`Purpose::Close`]: the
/// present state accounts for every agent running here, so nothing unnamed
/// (a session picked from a list, not yet prompted) could be a subject,
/// even when its old pane is conclusively gone. A person's Close all does
/// not need this; tend, closing on nobody's say, does.
pub(super) fn nothing_could_be_subject(
    request: &CollaborationRequest,
    requests: &HashMap<String, CollaborationRequest>,
    inputs: &SubjectInputs<'_>,
    now: OffsetDateTime,
) -> bool {
    let (Some(subjects), Some(scan)) = (subjects(request, requests), inputs.scan) else {
        return false;
    };
    now - scan.at <= SCAN_FRESH
        && subjects.into_iter().all(|subject| {
            subject.is_some_and(|subject| {
                present_state(scan, inputs.agents, subject.agent_kind, now, Purpose::Close)
            })
        })
}

/// What a scan is judged against for one subject.
struct Seen<'a> {
    session: &'a str,
    kind: AgentKind,
    purpose: Purpose,
    /// The subject's own row, never live here (`judge` returned already).
    row: Option<&'a Agent>,
    binding: Option<&'a SubjectBinding>,
    agents: &'a [Agent],
}

/// E2, E3 or E4 from the newest scan, if it shows one.
///
/// Two ways a scan can show the subject's pane no longer holds it:
///
/// - **On the bound incarnation**: the session was seen bound to this pane
///   on the server incarnation the socket answers with now.
/// - **From present state**: [`present_state`] holds, so every agent
///   running on this host is accounted for by another session, and `judge`
///   has already established that the subject has no live row and no
///   process was started on it. The pane then needs no history: it is
///   absent, or a different, named session's live row holds it, or it runs
///   a shell and no agent.
fn scan_evidence(
    pane: &str,
    socket: &str,
    seen: &Seen<'_>,
    scan: &PaneScan,
    now: OffsetDateTime,
) -> Option<GoneCause> {
    use crate::backend::HostKind;
    if now - scan.at > SCAN_FRESH {
        return None;
    }
    let kind = crate::backend::pane_id_host_kind(pane)?;
    if !matches!(kind, HostKind::Tmux | HostKind::Rmux) {
        return None;
    }
    let endpoint = crate::backend::pane_endpoint_identity(Some(pane), socket);
    let host = scan.complete_host(kind)?;
    let server = host
        .servers
        .iter()
        .find(|server| server.endpoint == endpoint)?;
    let bound = seen
        .binding
        .filter(|binding| binding.pane == pane && binding.endpoint == endpoint)
        .and_then(|binding| binding.incarnation);
    let on_bound = bound.is_some() && bound == server.incarnation;
    let present = present_state(scan, seen.agents, seen.kind, now, seen.purpose);
    let entry = host
        .panes
        .iter()
        .find(|p| p.pane_id == pane && p.endpoint.as_deref() == Some(endpoint.as_str()))
        .filter(|_| server.running);
    let Some(entry) = entry else {
        // E3: absent from a socket that answered.
        // - "No server running": no server holds any pane there.
        // - rmux reports no incarnation, and its pane ids are namespaced to
        //   the endpoint tendd scans; an rmux restart ends every pane it
        //   had, so absence from a running endpoint is that pane's end.
        // - tmux: on the bound incarnation, or from present state. Without
        //   either (a scan that did not walk the processes) the session may
        //   have been restored into a pane tendd has not tied to it yet.
        let gone = !server.running
            || match kind {
                HostKind::Rmux => true,
                _ => on_bound || present,
            };
        return gone.then_some(GoneCause::PaneGone);
    };
    let unique = host.panes.iter().filter(|p| p.pane_id == pane).count() == 1;
    let holders: Vec<&Agent> = seen
        .agents
        .iter()
        .filter(|agent| {
            agent.state != AgentState::Stopped
                && agent.pane.as_deref() == Some(pane)
                && agent.tmux_socket.as_deref().is_some_and(|other| {
                    crate::backend::pane_endpoint_identity(Some(pane), other) == endpoint
                })
        })
        .collect();
    // E2 from present state: another session tendd can name runs there.
    if present
        && unique
        && !holders.is_empty()
        && holders
            .iter()
            .all(|agent| agent.session_id != seen.session && named(agent))
    {
        return Some(GoneCause::Replaced);
    }
    // E4: exactly the agentless-pane conjunction, on the bound incarnation
    // or from present state with nothing else holding the pane.
    let quiet = match seen.row {
        Some(row) => {
            row.pid.is_none()
                && row.surface.is_none()
                && recognized(row.kind)
                && now - row.last_activity_at >= AGENTLESS_PANE_IDLE
        }
        None => present,
    };
    ((on_bound || (present && holders.is_empty()))
        && unique
        && quiet
        && entry.agent_process == Some(false)
        && crate::shell::is_shell_program(&entry.foreground))
    .then_some(GoneCause::AgentExited)
}

/// A row with a session id tendd learned from the provider, not one it
/// minted for a pane it could not tie to a session.
fn named(agent: &Agent) -> bool {
    !agent
        .session_id
        .starts_with(crate::state::SYNTHETIC_SESSION_PREFIX)
}

/// Whether `scan` can show a subject of `kind` gone without history: the
/// whole process table was read (`processes_read`), and nothing running
/// that no session id names could be this subject.
///
/// Each unnamed agent (`claude --continue`, a session picked from a list, a
/// fresh start), in a pane or outside every pane, is first matched with a
/// named, not-Stopped row of its kind in the same place, other than the
/// sessions a pane already names; the subject has no such row (`judge`
/// returned already). What is left blocks the subject only while it
/// [could be](UnnamedAgent::could_be) one of its kind, and, for a fold, for
/// at most [`UNNAMED_AGENT_BOUND`] from when a scan first showed it
/// ([`Purpose`]). A pane whose
/// walk did not finish leaves an entry of no kind, which could be any.
fn present_state(
    scan: &PaneScan,
    agents: &[Agent],
    kind: AgentKind,
    now: OffsetDateTime,
    purpose: Purpose,
) -> bool {
    if !scan.processes_read {
        return false;
    }
    let live = |agent: &&Agent| agent.state != AgentState::Stopped && named(agent);
    let at = |agent: &Agent, pane: &str, endpoint: Option<&str>| {
        agent.pane.as_deref() == Some(pane)
            && match (agent.tmux_socket.as_deref(), endpoint) {
                (Some(socket), Some(endpoint)) => {
                    crate::backend::pane_endpoint_identity(Some(pane), socket) == endpoint
                }
                (None, None) => true,
                _ => false,
            }
    };
    let panes: Vec<&crate::subject_evidence::ScannedPane> =
        scan.hosts.iter().flat_map(|host| &host.panes).collect();
    let in_panes = panes.iter().all(|pane| {
        let rows = agents
            .iter()
            .filter(live)
            .filter(|agent| {
                !pane.resumed.contains(&agent.session_id)
                    && at(agent, &pane.pane_id, pane.endpoint.as_deref())
            })
            .map(|agent| agent.kind)
            .collect();
        !could_be_subject(&pane.unnamed, rows, kind, now, purpose)
    });
    // Outside every pane, as in one: a session a process names by its
    // resume id accounts for that process, not for another one.
    let rows = agents
        .iter()
        .filter(live)
        .filter(|agent| {
            !scan.outside_resumed.contains(&agent.session_id)
                && !panes
                    .iter()
                    .any(|pane| at(agent, &pane.pane_id, pane.endpoint.as_deref()))
        })
        .map(|agent| agent.kind)
        .collect();
    in_panes && !could_be_subject(&scan.outside, rows, kind, now, purpose)
}

/// Whether anything in `unnamed` that `rows` (the kinds of the named rows
/// in the same place) leave unaccounted could be a subject of `kind` now.
/// A row accounts for the oldest unnamed agent of its kind, so the newest
/// are the ones left: the ones that block longest.
fn could_be_subject(
    unnamed: &[UnnamedAgent],
    mut rows: Vec<AgentKind>,
    kind: AgentKind,
    now: OffsetDateTime,
    purpose: Purpose,
) -> bool {
    let mut unnamed: Vec<&UnnamedAgent> = unnamed.iter().collect();
    unnamed.sort_by_key(|agent| agent.since);
    unnamed.into_iter().any(|agent| {
        if let Some(own) = agent.kind {
            if let Some(index) = rows.iter().position(|row| *row == own) {
                rows.swap_remove(index);
                return false;
            }
        }
        agent.could_be(kind)
            && (purpose == Purpose::Close || now - agent.since < UNNAMED_AGENT_BOUND)
    })
}

/// The binding to judge a subject by: the one stored on the item, or the
/// one tendd holds in memory. On the same pane and endpoint the stored
/// incarnation wins, as in `stamp_bindings`: the first incarnation the
/// session was seen on, so a pane id a restarted server reuses never
/// re-stamps it. Another pane or endpoint in memory means the session was
/// seen live elsewhere since.
fn merged_binding(
    stored: Option<&SubjectBinding>,
    memory: Option<&SubjectBinding>,
) -> Option<SubjectBinding> {
    match (stored, memory) {
        (Some(stored), Some(memory))
            if stored.pane == memory.pane && stored.endpoint == memory.endpoint =>
        {
            Some(SubjectBinding {
                incarnation: stored.incarnation.or(memory.incarnation),
                ..stored.clone()
            })
        }
        (_, Some(memory)) => Some(memory.clone()),
        (stored, None) => stored.cloned(),
    }
}

/// The bindings an open item should store: one per subject tendd has seen
/// bound, from memory merged over what the item already stores. A daemon
/// restart then keeps where each subject was bound
/// (`docs/proposals/STALE_QUESTIONS.md`, "Bindings survive a daemon
/// restart").
fn stored_bindings(
    request: &CollaborationRequest,
    requests: &HashMap<String, CollaborationRequest>,
    inputs: &SubjectInputs<'_>,
) -> BTreeMap<String, SubjectBinding> {
    let mut out = BTreeMap::new();
    let Some(subjects) = subjects(request, requests) else {
        return out;
    };
    for subject in subjects.into_iter().flatten() {
        let session = subject.agent_session_id.as_str();
        if subject.console || session.is_empty() {
            continue;
        }
        if let Some(binding) = merged_binding(
            request.subject_bindings.get(session),
            inputs.bindings.get(session),
        ) {
            out.insert(session.to_owned(), binding);
        }
    }
    out
}

/// One item's verdict: alive if any subject is, gone if every subject is.
pub(super) fn evaluate(
    request: &CollaborationRequest,
    requests: &HashMap<String, CollaborationRequest>,
    inputs: &SubjectInputs<'_>,
    facts: &[SubjectFact],
    now: OffsetDateTime,
    purpose: Purpose,
) -> Option<Verdict> {
    let subjects = subjects(request, requests)?;
    let mut causes = Vec::new();
    let mut pane = None;
    let mut all_gone = true;
    for subject in subjects {
        let Some(subject) = subject else {
            all_gone = false;
            continue;
        };
        match judge(
            subject,
            &request.subject_bindings,
            inputs,
            facts,
            now,
            purpose,
        ) {
            Verdict::Alive => return Some(Verdict::Alive),
            Verdict::Gone {
                causes: these,
                pane: here,
            } => {
                causes.extend(these);
                pane = pane.or(here);
            }
            Verdict::Unknown => all_gone = false,
        }
    }
    causes.sort_unstable();
    causes.dedup();
    Some(if all_gone {
        Verdict::Gone { causes, pane }
    } else {
        Verdict::Unknown
    })
}

/// When the episode of a row written before
/// [`SubjectCheck::episode_since`] began, from confirmed evidence only: the
/// row's own clocks, or, when every subject has one, the end hook tendd
/// recorded for it, whichever is oldest; never before the item itself,
/// whose asker was there to ask. Those rows' grace clock may have been
/// restarted by every daemon restart since, which made an asker gone for
/// weeks read as gone since the last upgrade. A last activity is not
/// evidence: an idle agent stays quiet for days and is still there.
fn legacy_since(
    request: &CollaborationRequest,
    old: &SubjectCheck,
    requests: &HashMap<String, CollaborationRequest>,
    agents: &[Agent],
    facts: &[SubjectFact],
) -> OffsetDateTime {
    let ended = subjects(request, requests).and_then(|subjects| {
        let mut latest: Option<OffsetDateTime> = None;
        for subject in subjects {
            let subject = subject?;
            let session = subject.agent_session_id.as_str();
            // As in `judge`: an end before the row's last activity belongs
            // to an earlier run, not to this episode.
            let floor = super::recovery::exact_agent(subject, agents)
                .map(|row| row.last_activity_at)
                .into_iter()
                .chain(
                    agents
                        .iter()
                        .filter(|agent| agent.session_id == session)
                        .map(|agent| agent.last_activity_at),
                )
                .max();
            let end = facts
                .iter()
                .filter(|fact| {
                    fact.session_id == session
                        && fact.kind == FactKind::Ended
                        && floor.is_none_or(|floor| fact.observed_at >= floor)
                })
                .map(|fact| fact.observed_at)
                .max()?;
            latest = Some(latest.map_or(end, |at| at.max(end)));
        }
        latest
    });
    [ended, old.gone_since, Some(old.confirmed_at)]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(old.confirmed_at)
        .max(request.created_at)
}

/// Load the durable facts, dropping expired ones.
pub(super) fn load_facts(
    connection: &rusqlite::Connection,
) -> Result<Vec<SubjectFact>, CollaborationError> {
    let cutoff = OffsetDateTime::now_utc() - crate::subject_evidence::FACT_RETENTION;
    connection.execute(
        "DELETE FROM collaboration_subject_facts WHERE observed_at_ns < ?1",
        [super::timestamp_nanos(cutoff)?],
    )?;
    let mut statement = connection
        .prepare("SELECT payload FROM collaboration_subject_facts ORDER BY observed_at_ns")?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    let mut facts = Vec::new();
    for row in rows {
        // A fact this build cannot read is skipped, not fatal.
        if let Ok(fact) = serde_json::from_str::<SubjectFact>(&row?) {
            facts.push(fact);
        }
    }
    Ok(facts)
}

impl CollaborationStore {
    /// Turn `decision_close_unanswerable` on or off (`[review]
    /// close_unanswerable`). Off by default.
    pub fn set_close_unanswerable(&self, enabled: bool) {
        self.subject
            .close_enabled
            .store(enabled, std::sync::atomic::Ordering::SeqCst);
    }

    #[must_use]
    pub fn close_unanswerable_enabled(&self) -> bool {
        self.subject
            .close_enabled
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Keep the facts recorded at ingestion, durably where the mailbox is,
    /// for [`crate::subject_evidence::FACT_RETENTION`].
    pub fn record_subject_facts(&self, facts: Vec<SubjectFact>) -> Result<(), CollaborationError> {
        if facts.is_empty() {
            return Ok(());
        }
        self.ensure_enabled()?;
        let cutoff = OffsetDateTime::now_utc() - crate::subject_evidence::FACT_RETENTION;
        if let Some(database) = &self.database {
            database.with_connection(|connection| {
                let transaction = connection.transaction()?;
                for fact in &facts {
                    transaction.execute(
                        "INSERT OR REPLACE INTO collaboration_subject_facts (id, observed_at_ns, payload) VALUES (?1, ?2, ?3)",
                        rusqlite::params![
                            fact.id,
                            super::timestamp_nanos(fact.observed_at)?,
                            serde_json::to_string(fact)?
                        ],
                    )?;
                }
                transaction.execute(
                    "DELETE FROM collaboration_subject_facts WHERE observed_at_ns < ?1",
                    [super::timestamp_nanos(cutoff)?],
                )?;
                transaction.commit()?;
                Ok(())
            })?;
        }
        if let Ok(mut held) = self.subject.facts.lock() {
            held.extend(facts);
            held.retain(|fact| fact.observed_at >= cutoff);
        }
        Ok(())
    }

    /// Take the facts the registry recorded and keep them durably. On a
    /// failed write they go back to the registry, ahead of newer ones, for
    /// the next pass: a fact is dropped only once it is written.
    pub fn keep_facts_from(
        &self,
        observations: &crate::subject_evidence::SubjectObservations,
    ) -> Result<(), CollaborationError> {
        let facts = observations.take_facts();
        if facts.is_empty() {
            return Ok(());
        }
        self.record_subject_facts(facts.clone()).inspect_err(|_| {
            observations.requeue(facts);
        })
    }

    /// The facts kept now (tests and diagnostics).
    #[must_use]
    pub fn subject_facts(&self) -> Vec<SubjectFact> {
        self.subject.facts()
    }

    /// Whether `request` is unanswerable now, by the current pass.
    #[must_use]
    pub fn unanswerable(
        &self,
        request: &CollaborationRequest,
        now: OffsetDateTime,
    ) -> Option<Unanswerable> {
        let confirmed = self.subject.confirmed(&request.id);
        unanswerable(
            request,
            confirmed.as_ref().map(|(ep, at)| (ep.as_str(), *at)),
            now,
        )
    }

    /// tend's own asks that may close because their subject is gone: folded
    /// by the newest pass, and established gone again now from the held
    /// registry with Close all's stricter evidence ([`Purpose::Close`]).
    pub(super) fn gone_system_asks(
        &self,
        requests: &HashMap<String, CollaborationRequest>,
        hold: &crate::state::RegistryHold<'_>,
        agents: &[Agent],
        now: OffsetDateTime,
    ) -> HashMap<String, super::system_asks::Gone> {
        let observations = hold.observations();
        let scan = observations.scan();
        let bindings = observations.bindings();
        let mut facts = self.subject.facts();
        facts.extend(observations.peek_facts());
        let inputs = SubjectInputs {
            agents,
            scan: scan.as_deref(),
            bindings: &bindings,
        };
        requests
            .values()
            .filter(|request| super::system_asks::system_ask_kind(request).is_some())
            .filter_map(|request| {
                let folded = self.unanswerable(request, now)?;
                (matches!(
                    evaluate(request, requests, &inputs, &facts, now, Purpose::Close),
                    Some(Verdict::Gone { .. })
                ) && nothing_could_be_subject(request, requests, &inputs, now))
                .then(|| {
                    (
                        request.id.clone(),
                        super::system_asks::Gone {
                            cause: folded.cause,
                            since: folded.since,
                        },
                    )
                })
            })
            .collect()
    }

    fn unanswerable_ids(
        requests: &HashMap<String, CollaborationRequest>,
        confirmed: &HashMap<String, (String, OffsetDateTime)>,
        now: OffsetDateTime,
    ) -> Vec<String> {
        let mut ids: Vec<String> = requests
            .values()
            .filter(|request| {
                unanswerable(
                    request,
                    confirmed
                        .get(&request.id)
                        .map(|(ep, at)| (ep.as_str(), *at)),
                    now,
                )
                .is_some()
            })
            .map(|request| request.id.clone())
            .collect();
        ids.sort();
        ids
    }

    /// One subject check pass over every open human item, under the
    /// collaboration transaction lock. Writes an item's record when it
    /// changes, and its confirmation at most every few minutes; publishes a
    /// change only when what is unanswerable changed.
    pub async fn check_subjects(
        &self,
        inputs: &SubjectInputs<'_>,
    ) -> Result<SubjectPassReport, CollaborationError> {
        self.check_subjects_at(inputs, OffsetDateTime::now_utc())
            .await
    }

    /// [`Self::check_subjects`] at a given time (tests).
    pub async fn check_subjects_at(
        &self,
        inputs: &SubjectInputs<'_>,
        now: OffsetDateTime,
    ) -> Result<SubjectPassReport, CollaborationError> {
        self.ensure_enabled()?;
        let _transaction = self.transaction_lock.lock().await;
        let facts = self.subject.facts();
        let snapshot = self.requests.read().await;
        let previous = self.subject.snapshot();
        let before = Self::unanswerable_ids(&snapshot, &previous, now);
        let mut confirmed = HashMap::new();
        let mut changed = Vec::new();
        let mut report = SubjectPassReport::default();
        for request in snapshot.values().filter(|r| r.needs_human_response()) {
            let Some(verdict) = evaluate(request, &snapshot, inputs, &facts, now, Purpose::Fold)
            else {
                continue;
            };
            let old = request.subject_check.as_ref();
            let last_confirmed = [
                old.map(|old| old.confirmed_at),
                previous
                    .get(&request.id)
                    .filter(|(ep, _)| old.is_some_and(|old| &old.episode == ep))
                    .map(|(_, at)| *at),
            ]
            .into_iter()
            .flatten()
            .max();
            let legacy = old
                .filter(|old| old.episode_since.is_none())
                .map(|old| legacy_since(request, old, &snapshot, inputs.agents, &facts));
            let next = step(old, &verdict, now, last_confirmed, legacy);
            if let Some(next) = next.as_ref().filter(|n| n.state == SubjectState::Gone) {
                report.gone += 1;
                confirmed.insert(request.id.clone(), (next.episode.clone(), now));
            }
            let check_changed = match (old, next.as_ref()) {
                (None, None) => false,
                (Some(old), Some(next)) => {
                    let mut same = next.clone();
                    same.confirmed_at = old.confirmed_at;
                    same != *old || next.confirmed_at - old.confirmed_at >= CONFIRM_PERSIST_EVERY
                }
                _ => true,
            };
            let bindings = stored_bindings(request, &snapshot, inputs);
            if check_changed || bindings != request.subject_bindings {
                let mut row = request.clone();
                row.subject_check = next;
                row.subject_bindings = bindings;
                changed.push(row);
            }
        }
        let after = {
            let mut overlay = HashMap::new();
            for row in &changed {
                overlay.insert(row.id.clone(), row.clone());
            }
            let mut ids: Vec<String> = snapshot
                .values()
                .map(|r| overlay.get(&r.id).unwrap_or(r))
                .filter(|r| {
                    unanswerable(
                        r,
                        confirmed.get(&r.id).map(|(ep, at)| (ep.as_str(), *at)),
                        now,
                    )
                    .is_some()
                })
                .map(|r| r.id.clone())
                .collect();
            ids.sort();
            ids
        };
        report.unanswerable = after.len();
        report.written = changed.len();
        drop(snapshot);
        if !changed.is_empty() {
            self.commit_request_rows(&changed).await?;
        }
        if let Ok(mut pass) = self.subject.pass.lock() {
            pass.at = Some(now);
            pass.confirmed = confirmed;
        }
        if before != after {
            for id in after.iter().filter(|id| !before.contains(id)) {
                tracing::info!(
                    request_id = id,
                    "a question can no longer be answered: its subject is gone"
                );
            }
            self.publish_change();
        }
        Ok(report)
    }

    /// `decision_keep`: list the item in the person's turn again until its
    /// subject is seen alive and then gone in a new episode.
    pub async fn keep_unanswerable(
        &self,
        request_id: &str,
        episode: &str,
    ) -> Result<KeepOutcome, CollaborationError> {
        self.ensure_enabled()?;
        let _transaction = self.transaction_lock.lock().await;
        let mut row = self
            .requests
            .read()
            .await
            .get(request_id)
            .cloned()
            .ok_or_else(|| CollaborationError::NotFound(request_id.to_owned()))?;
        if !row.needs_human_response() {
            return Err(CollaborationError::AlreadyTerminal(request_id.to_owned()));
        }
        let Some(check) = row.subject_check.as_mut().filter(|c| c.episode == episode) else {
            return Err(CollaborationError::SubjectCheck(format!(
                "request {request_id} is not in episode {episode}; read the queue again"
            )));
        };
        if check.kept_episode.as_deref() != Some(episode) {
            check.kept_episode = Some(episode.to_owned());
            self.commit_request_rows(std::slice::from_ref(&row)).await?;
            self.publish_change();
            tracing::info!(
                request_id,
                episode,
                "the person kept an unanswerable question"
            );
        }
        Ok(KeepOutcome {
            request_id: request_id.to_owned(),
            episode: episode.to_owned(),
        })
    }

    /// `decision_close_unanswerable`: decline exactly the listed pairs that
    /// are, inside this one transaction, still open, still unanswerable, in
    /// the same episode, not kept, and with no subject back. Each gets the
    /// body Dismiss sends and a [`QuestionClosure`]; the rest are reported
    /// skipped. Refused while `[review] close_unanswerable` is off.
    ///
    /// Inside the transaction the subject evidence is read again from the
    /// agent registry (`registry`): its rows, newest scan, bindings and the
    /// facts it has not handed over yet. A pair whose subjects are not all
    /// established gone by that evidence now is skipped.
    #[allow(clippy::too_many_lines)] // every recheck of one pair sits in one transaction
    pub async fn close_unanswerable(
        &self,
        items: &[CloseItem],
        registry: &crate::state::Store,
        surface: &str,
        actor: Option<String>,
    ) -> Result<CloseOutcome, CollaborationError> {
        self.ensure_enabled()?;
        if !self.close_unanswerable_enabled() {
            return Err(CollaborationError::SubjectCheck(
                CLOSE_UNANSWERABLE_OFF.into(),
            ));
        }
        if !SURFACES.contains(&surface) {
            return Err(CollaborationError::SubjectCheck(format!(
                "unknown surface {surface:?}; use one of {}",
                SURFACES.join(", ")
            )));
        }
        let _transaction = self.transaction_lock.lock().await;
        let snapshot = self.requests.read().await;
        // Lock order: the mailbox's, then the registry's. The registry is
        // held still from this read through the commit below, so a resume,
        // a hook or a scan lands either before this evaluation or after the
        // commit, never in between.
        let hold = registry.hold().await;
        let agents = hold.agents();
        let observations = hold.observations();
        let scan = observations.scan();
        let bindings = observations.bindings();
        let mut facts = self.subject.facts();
        facts.extend(observations.peek_facts());
        let inputs = SubjectInputs {
            agents: &agents,
            scan: scan.as_deref(),
            bindings: &bindings,
        };
        let now = OffsetDateTime::now_utc();
        let mut outcome = CloseOutcome::default();
        let mut rows = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for item in items {
            let skip = |reason: &str| SkippedClose {
                request_id: item.request_id.clone(),
                reason: reason.to_owned(),
            };
            if !seen.insert(item.request_id.clone()) {
                outcome.skipped.push(skip("listed twice"));
                continue;
            }
            let Some(request) = snapshot.get(&item.request_id) else {
                outcome.skipped.push(skip("no such request"));
                continue;
            };
            if !request.needs_human_response() {
                outcome.skipped.push(skip("no longer open"));
                continue;
            }
            let Some(check) = request.subject_check.as_ref() else {
                outcome.skipped.push(skip("its subject is not gone"));
                continue;
            };
            if check.episode != item.episode {
                outcome.skipped.push(skip("a new episode since the list"));
                continue;
            }
            if check.kept_episode.as_deref() == Some(item.episode.as_str()) {
                outcome.skipped.push(skip("kept"));
                continue;
            }
            let Some(unanswerable) = self.unanswerable(request, now) else {
                outcome.skipped.push(skip("not unanswerable now"));
                continue;
            };
            match evaluate(request, &snapshot, &inputs, &facts, now, Purpose::Close) {
                Some(Verdict::Gone { .. }) => {}
                Some(Verdict::Alive) => {
                    outcome.skipped.push(skip("its subject is back"));
                    continue;
                }
                _ => {
                    outcome
                        .skipped
                        .push(skip("its subject is no longer established gone"));
                    continue;
                }
            }
            let mut row = request.clone();
            row.status = RequestStatus::Declined;
            row.reply = Some(CollaborationReply {
                status: RequestStatus::Declined,
                body: DISMISSED_BODY.to_owned(),
                report: None,
                artifacts: Vec::new(),
                at: now,
            });
            row.closure = Some(QuestionClosure {
                reason: ClosureReason::SubjectGone,
                cause: unanswerable.cause,
                episode: item.episode.clone(),
                by: "person".into(),
                surface: surface.to_owned(),
                actor: actor.clone(),
                at: now,
            });
            outcome.closed.push(row.id.clone());
            rows.push(row);
        }
        drop(snapshot);
        if !rows.is_empty() {
            self.commit_request_rows(&rows).await?;
            drop(hold);
            self.publish_change();
            for row in &rows {
                tracing::info!(
                    request_id = row.id,
                    surface,
                    "the person closed a question whose subject is gone",
                );
            }
        }
        Ok(outcome)
    }

    /// Open human requests in queue order (agents' questions, then tend's
    /// own items, then the unanswerable ones), at most `limit`, with what is
    /// unanswerable among them and the exact count of unanswerable ones.
    pub async fn open_human_requests_folded(
        &self,
        limit: usize,
    ) -> Result<FoldedPage, CollaborationError> {
        self.ensure_enabled()?;
        let now = OffsetDateTime::now_utc();
        let requests = self.requests.read().await;
        let confirmed = self.subject.snapshot();
        let mut open: Vec<(&CollaborationRequest, Option<Unanswerable>)> = requests
            .values()
            .filter(|request| request.needs_human_response())
            .map(|request| {
                let folded = unanswerable(
                    request,
                    confirmed
                        .get(&request.id)
                        .map(|(ep, at)| (ep.as_str(), *at)),
                    now,
                );
                (request, folded)
            })
            .collect();
        let unanswerable_count = open.iter().filter(|(_, f)| f.is_some()).count();
        open.sort_by(|(a, fa), (b, fb)| {
            let system =
                |r: &CollaborationRequest| super::system_asks::system_ask_kind(r).is_some();
            fa.is_some()
                .cmp(&fb.is_some())
                .then(system(a).cmp(&system(b)))
                .then(a.created_at.cmp(&b.created_at))
                .then(a.id.cmp(&b.id))
        });
        let more = open.len() > limit;
        let mut folded = HashMap::new();
        let mut page = Vec::new();
        for (request, unanswerable) in open.into_iter().take(limit) {
            if let Some(unanswerable) = unanswerable {
                folded.insert(request.id.clone(), unanswerable);
            }
            page.push(request.clone());
        }
        Ok(FoldedPage {
            requests: page,
            more,
            folded,
            unanswerable_count,
        })
    }
}

#[cfg(test)]
impl CollaborationStore {
    /// Make `id` an unanswerable question with `cause`, as a day of passes
    /// would (route tests outside this module). Returns its episode.
    pub(crate) async fn fold_for_test(&self, id: &str, cause: GoneCause) -> String {
        let now = OffsetDateTime::now_utc();
        let mut row = self.requests.read().await.get(id).cloned().unwrap();
        row.created_at -= time::Duration::days(2);
        let episode = format!("ep_{id}");
        row.subject_check = Some(SubjectCheck {
            episode: episode.clone(),
            state: SubjectState::Gone,
            gone_since: Some(now - time::Duration::hours(25)),
            episode_since: Some(now - time::Duration::hours(25)),
            confirmed_at: now,
            causes: vec![cause],
            kept_episode: None,
            pane: Some(row.from.pane.clone()),
        });
        self.commit_request_rows(&[row]).await.unwrap();
        self.subject
            .pass
            .lock()
            .unwrap()
            .confirmed
            .insert(id.to_owned(), (episode.clone(), now));
        episode
    }
}

/// A page of open human requests with their folds.
#[derive(Debug, Clone, Default)]
pub struct FoldedPage {
    pub requests: Vec<CollaborationRequest>,
    pub more: bool,
    pub folded: HashMap<String, Unanswerable>,
    pub unanswerable_count: usize,
}

#[cfg(test)]
#[path = "subject_check_tests.rs"]
mod tests;
