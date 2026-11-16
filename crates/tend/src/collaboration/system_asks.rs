//! Human items tendd opens itself ("system asks"): peer-recovery questions
//! (`recovery.rs`) and reply-wake notices for a sender that is gone
//! (`sender_follow.rs`).
//!
//! Left alone they pile up: one per original request even when they are all
//! about the same vanished session, and each stays open for as long as its
//! original request does, which for an abandoned attempt is forever. This
//! module keeps them to what still needs the person:
//!
//! - **One item per subject.** Asks of one kind about the same session, for
//!   the same room, share one item. Its answer reaches every original
//!   request it covers, exactly as a one-to-one answer did.
//! - **Closed when they can no longer matter**, never when they still might:
//!   the original request is gone, or it no longer holds the condition the
//!   ask was about (it finished, was cancelled or retired, recovered, or the
//!   reply was delivered or read); or, after [`SYSTEM_ASK_STALE_AFTER`],
//!   nobody is left to act on an answer (see [`abandoned`]). A recovery ask
//!   whose original request came from the operator console, or from a Fleet
//!   initiator, is never closed for age: a person is that request's
//!   sender, so the decision is theirs.
//! - **Closing is recorded, not deleted.** The row stays in the mailbox,
//!   `Cancelled`, with [`SystemAskClosure`] saying why, when, by whom, and
//!   (for a merged duplicate) which item now covers it.
//!
//! The daemon applies these rules only to items it opened under them
//! (`system_ask` set). Items from before, the backlog, are left to
//! `tend review cleanup`, which shows what it would close before closing it.
use super::{CollaborationError, CollaborationRequest, CollaborationStore, RequestStatus};
use crate::state::Agent;
use crate::AgentState;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use time::OffsetDateTime;

/// `hello` capability: the daemon serves `system_ask_cleanup`.
pub const SYSTEM_ASK_CLEANUP_CAPABILITY: &str = "system_ask_cleanup_v1";

/// How long an ask must have waited before "nobody is left to act on it"
/// can close it. A week covers a weekend away and a provider's weekly limit
/// reset, the longest ordinary reason an agent session is absent and then
/// comes back; the rule also needs the sessions involved to be absent now
/// (see [`abandoned`]).
pub const SYSTEM_ASK_STALE_AFTER: time::Duration = time::Duration::days(7);

/// Request ids listed in one item's text; the rest are counted.
const LISTED_COVERS: usize = 10;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SystemAskKind {
    /// Peer recovery: an original request's recipient is interrupted.
    Recovery,
    /// A reply arrived for a sender that is gone.
    ReplyWake,
}

/// Lifecycle tendd keeps on a system ask it manages.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SystemAskState {
    /// The session the ask is about: the recipient for recovery, the gone
    /// sender for a reply wake.
    pub subject_session: String,
    /// Original requests this item answers for besides `parent_request_id`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub covers: Vec<String>,
    /// Set when tendd closed the item without an answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<SystemAskClosure>,
}

/// The audit record of a system ask closed without an answer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SystemAskClosure {
    pub reason: SystemAskCloseReason,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    pub by: SystemAskCloser,
    /// For [`SystemAskCloseReason::Collapsed`]: the item that covers it now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub into: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SystemAskCloseReason {
    /// No original request it was about exists any more.
    ParentMissing,
    /// No original request still holds the condition it was about.
    ParentClosed,
    /// Older than [`SYSTEM_ASK_STALE_AFTER`] and nobody is left to act.
    Stale,
    /// Another open item about the same session covers it now.
    Collapsed,
}

impl SystemAskCloseReason {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ParentMissing => "original request gone",
            Self::ParentClosed => "original request no longer waiting",
            Self::Stale => "stale: nobody left to act on it",
            Self::Collapsed => "merged",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SystemAskCloser {
    /// The daemon's own reconcile, for items it opened under these rules.
    Daemon,
    /// `tend review cleanup --apply`.
    Cleanup,
}

/// One ask a cleanup closed, or would close.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SettledAsk {
    pub request_id: String,
    pub kind: SystemAskKind,
    pub reason: SystemAskCloseReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub into: Option<String>,
    pub subject_session: String,
    /// The original requests it was about.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parents: Vec<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// What `system_ask_cleanup` did, or with `dry_run` would do.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SystemAskCleanup {
    pub dry_run: bool,
    /// Oldest first.
    pub closed: Vec<SettledAsk>,
    /// System asks still waiting on the person afterwards.
    pub open_after: usize,
    /// Human items from agents, untouched, for comparison.
    pub agent_asks_open: usize,
}

const RECOVERY_SESSIONS: [(&str, &str); 2] = [
    ("tend:peer-recovery", "tend-recovery"),
    ("muxa:peer-recovery", "muxa-recovery"),
];
const REPLY_WAKE_SESSIONS: [(&str, &str); 2] = [
    ("tend:reply-wake", "tend-reply-wake"),
    ("muxa:reply-wake", "muxa-reply-wake"),
];

/// Whether `request` is an item tendd opened itself, and which kind. Every
/// mark of the daemon's own construction must be present: its service
/// session and pane, the task kind, no IPC provenance (every request an IPC
/// caller creates records one), a parent, and a human action. An agent's
/// own question to the person never matches, so nothing here can close it.
#[must_use]
pub fn system_ask_kind(request: &CollaborationRequest) -> Option<SystemAskKind> {
    if !request.to.console
        || request.from.console
        || request.from.agent_kind != crate::AgentKind::Task
        || request.provenance.is_some()
        || request.parent_request_id.is_none()
        || request.human_action.is_none()
    {
        return None;
    }
    let service = (
        request.from.agent_session_id.as_str(),
        request.from.pane.as_str(),
    );
    if RECOVERY_SESSIONS.contains(&service) {
        Some(SystemAskKind::Recovery)
    } else if REPLY_WAKE_SESSIONS.contains(&service) {
        Some(SystemAskKind::ReplyWake)
    } else {
        None
    }
}

/// Whether `session` has a live agent row.
fn session_live(session: &str, agents: &[Agent]) -> bool {
    agents
        .iter()
        .any(|agent| agent.session_id == session && agent.state != AgentState::Stopped)
}

/// Nobody is left to act on an answer about `parent`.
///
/// Recovery: the answer is a decision for the original request's sender,
/// read off that request. It is abandoned only when that sender is an
/// agent session (not the operator console, not a Fleet initiator acting
/// for a person) and the recipient it was about is gone, and either the
/// sender has no live row either, or the recipient vanished
/// (`binding_unavailable`: no agent row at all). A vanished recipient
/// leaves nothing for a person to unblock: no prompt to answer, no account
/// to fix. What is left is the live sender's own request, which its
/// obligations keep listing as interrupted and which it (or `tend msg
/// retire`) can resend or retire; a person's answer would only be a note
/// on it. A recipient that comes back ends the episode by itself. A reply
/// wake: the reply's sender session is still gone.
pub(super) fn abandoned(
    kind: SystemAskKind,
    parent: &CollaborationRequest,
    agents: &[Agent],
) -> bool {
    match kind {
        SystemAskKind::Recovery => {
            let vanished = parent
                .interruption
                .as_ref()
                .is_some_and(|info| info.active && info.reason == "binding_unavailable");
            !parent.from.console
                && parent.initiator.is_none()
                && !parent.from.agent_session_id.starts_with("tend:")
                && (vanished || !session_live(&parent.from.agent_session_id, agents))
                && super::recovery::exact_agent(&parent.to, agents).is_none()
                && !session_live(&parent.to.agent_session_id, agents)
        }
        SystemAskKind::ReplyWake => {
            !parent.from.console && !session_live(&parent.from.agent_session_id, agents)
        }
    }
}

/// Whether `parent` still holds what `ask_id` (of `kind`) is about.
pub(super) fn still_waiting(
    kind: SystemAskKind,
    parent: &CollaborationRequest,
    ask_id: &str,
) -> bool {
    match kind {
        SystemAskKind::Recovery => {
            !parent.status.is_terminal()
                && parent.interruption.as_ref().is_some_and(|info| {
                    info.active && info.action_request_id.as_deref() == Some(ask_id)
                })
        }
        SystemAskKind::ReplyWake => {
            parent.reply_wake_owed()
                && parent
                    .sender_follow
                    .as_ref()
                    .and_then(|follow| follow.gone_action_request_id.as_deref())
                    == Some(ask_id)
        }
    }
}

/// When `parent`'s own condition began: its interruption episode, or the
/// reply that found its sender gone. A request that joined an older item
/// gets the full grace from here, not from the item's age.
fn waiting_since(kind: SystemAskKind, parent: &CollaborationRequest) -> Option<OffsetDateTime> {
    match kind {
        SystemAskKind::Recovery => parent.interruption.as_ref().map(|info| info.observed_at),
        SystemAskKind::ReplyWake => parent.reply.as_ref().map(|reply| reply.at),
    }
}

/// The session an unmarked (backlog) ask is about.
fn legacy_subject(kind: SystemAskKind, parent: &CollaborationRequest) -> String {
    match kind {
        SystemAskKind::Recovery => parent.to.agent_session_id.clone(),
        SystemAskKind::ReplyWake => parent.from.agent_session_id.clone(),
    }
}

/// The text after an item's own: the other requests it covers.
pub(super) fn covers_note(kind: SystemAskKind, covers: &[String]) -> String {
    if covers.is_empty() {
        return String::new();
    }
    let what = match kind {
        SystemAskKind::Recovery => "to the same recipient",
        SystemAskKind::ReplyWake => "whose replies went to the same gone sender",
    };
    let mut listed = covers
        .iter()
        .take(LISTED_COVERS)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if covers.len() > LISTED_COVERS {
        let _ = write!(listed, " and {} more", covers.len() - LISTED_COVERS);
    }
    format!(
        "\nThis item also covers {} other request{} {what}: {listed}. Your answer applies to all of them.",
        covers.len(),
        if covers.len() == 1 { "" } else { "s" },
    )
}

/// Requests as one transaction sees them: the store's rows with this
/// transaction's changes over them.
pub(super) struct View<'a> {
    pub base: &'a HashMap<String, CollaborationRequest>,
    pub overlay: HashMap<String, CollaborationRequest>,
}

impl View<'_> {
    pub fn get(&self, id: &str) -> Option<&CollaborationRequest> {
        self.overlay.get(id).or_else(|| self.base.get(id))
    }

    fn all(&self) -> impl Iterator<Item = &CollaborationRequest> {
        self.base
            .values()
            .filter(|request| !self.overlay.contains_key(&request.id))
            .chain(self.overlay.values())
    }

    fn put(&mut self, request: CollaborationRequest) {
        self.overlay.insert(request.id.clone(), request);
    }
}

/// Which asks a settle pass may touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Scope {
    /// Items opened under these rules (`system_ask` set): the daemon's own.
    Managed,
    /// Every system ask, the backlog included: `tend review cleanup`.
    All,
}

struct OpenAsk {
    id: String,
    kind: SystemAskKind,
    subject: String,
    room: super::RoomId,
    created_at: OffsetDateTime,
    /// Original requests still holding what it is about.
    waiting: Vec<String>,
    /// Every original request it was linked to.
    linked: Vec<String>,
}

/// Apply the rules to `view`: close what can no longer matter, merge
/// duplicates, and keep each managed item's text and cover list current.
/// Changed rows land in `view.overlay`; the closures are returned, oldest
/// first. With [`Scope::All`], every ask left open becomes managed.
pub(super) fn settle(
    view: &mut View<'_>,
    agents: &[Agent],
    now: OffsetDateTime,
    scope: Scope,
    by: SystemAskCloser,
) -> Vec<SettledAsk> {
    let open = open_asks(view, scope);
    let mut settled = Vec::new();
    let mut survivors = Vec::new();
    for ask in open {
        let reason = if !ask.linked.iter().any(|id| view.get(id).is_some()) {
            Some(SystemAskCloseReason::ParentMissing)
        } else if ask.waiting.is_empty() {
            Some(SystemAskCloseReason::ParentClosed)
        } else if now - ask.created_at >= SYSTEM_ASK_STALE_AFTER
            && ask.waiting.iter().all(|id| {
                view.get(id).is_some_and(|parent| {
                    abandoned(ask.kind, parent, agents)
                        && waiting_since(ask.kind, parent)
                            .is_none_or(|since| now - since >= SYSTEM_ASK_STALE_AFTER)
                })
            })
        {
            Some(SystemAskCloseReason::Stale)
        } else {
            None
        };
        match reason {
            Some(reason) => {
                close(view, &ask, reason, None, now, by);
                settled.push(report(&ask, reason, None));
            }
            None => survivors.push(ask),
        }
    }
    collapse(view, survivors, now, by, &mut settled);
    settled.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then(a.request_id.cmp(&b.request_id))
    });
    settled
}

/// The open system asks `scope` covers, oldest first, with the original
/// requests each is linked to and those still waiting.
fn open_asks(view: &View<'_>, scope: Scope) -> Vec<OpenAsk> {
    // Recovery parents by the item they point at.
    let mut pointing: HashMap<String, Vec<String>> = HashMap::new();
    for request in view.all() {
        if let Some(action) = request
            .interruption
            .as_ref()
            .filter(|info| info.request_id == request.id)
            .and_then(|info| info.action_request_id.clone())
        {
            pointing.entry(action).or_default().push(request.id.clone());
        }
    }
    let mut open: Vec<OpenAsk> = view
        .all()
        .filter(|request| request.needs_human_response())
        .filter(|request| scope == Scope::All || request.system_ask.is_some())
        .filter_map(|ask| {
            let kind = system_ask_kind(ask)?;
            let parent_id = ask.parent_request_id.clone()?;
            let mut linked = vec![parent_id];
            if let Some(state) = &ask.system_ask {
                linked.extend(state.covers.iter().cloned());
            }
            if kind == SystemAskKind::Recovery {
                linked.extend(pointing.get(&ask.id).into_iter().flatten().cloned());
            }
            linked.sort();
            linked.dedup();
            let waiting = linked
                .iter()
                .filter(|id| {
                    view.get(id)
                        .is_some_and(|p| still_waiting(kind, p, &ask.id))
                })
                .cloned()
                .collect();
            let subject = match &ask.system_ask {
                Some(state) => state.subject_session.clone(),
                None => ask
                    .parent_request_id
                    .as_deref()
                    .and_then(|id| view.get(id))
                    .map(|parent| legacy_subject(kind, parent))
                    .unwrap_or_default(),
            };
            Some(OpenAsk {
                id: ask.id.clone(),
                kind,
                subject,
                room: ask.to.room.clone(),
                created_at: ask.created_at,
                waiting,
                linked,
            })
        })
        .collect();
    open.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    open
}

/// One item per (kind, room, subject): the newest stays, the rest merge
/// into it and their original requests point at it.
fn collapse(
    view: &mut View<'_>,
    survivors: Vec<OpenAsk>,
    now: OffsetDateTime,
    by: SystemAskCloser,
    settled: &mut Vec<SettledAsk>,
) {
    // One item per (kind, room, subject): the newest stays, the rest merge
    // into it and their original requests point at it.
    let mut groups: BTreeMap<(SystemAskKind, String, String), Vec<OpenAsk>> = BTreeMap::new();
    for ask in survivors {
        let room = serde_json::to_string(&ask.room).unwrap_or_default();
        groups
            .entry((ask.kind, room, ask.subject.clone()))
            .or_default()
            .push(ask);
    }
    for (_, mut group) in groups {
        // Oldest first, so the survivor is the last.
        let survivor = group.pop().expect("groups are never empty");
        let mut covered: Vec<String> = survivor.waiting.clone();
        for ask in &group {
            for parent_id in &ask.waiting {
                if let Some(mut parent) = view.get(parent_id).cloned() {
                    point_at(&mut parent, ask.kind, &survivor.id);
                    view.put(parent);
                }
                covered.push(parent_id.clone());
            }
            close(
                view,
                ask,
                SystemAskCloseReason::Collapsed,
                Some(survivor.id.clone()),
                now,
                by,
            );
            settled.push(report(
                ask,
                SystemAskCloseReason::Collapsed,
                Some(survivor.id.clone()),
            ));
        }
        covered.sort();
        covered.dedup();
        refresh(view, &survivor, covered);
    }
}

fn point_at(parent: &mut CollaborationRequest, kind: SystemAskKind, ask_id: &str) {
    match kind {
        SystemAskKind::Recovery => {
            if let Some(info) = parent.interruption.as_mut() {
                info.action_request_id = Some(ask_id.to_owned());
            }
        }
        SystemAskKind::ReplyWake => {
            if let Some(follow) = parent.sender_follow.as_mut() {
                follow.gone_action_request_id = Some(ask_id.to_owned());
            }
        }
    }
}

fn report(ask: &OpenAsk, reason: SystemAskCloseReason, into: Option<String>) -> SettledAsk {
    SettledAsk {
        request_id: ask.id.clone(),
        kind: ask.kind,
        reason,
        into,
        subject_session: ask.subject.clone(),
        parents: ask.linked.clone(),
        created_at: ask.created_at,
    }
}

fn close(
    view: &mut View<'_>,
    ask: &OpenAsk,
    reason: SystemAskCloseReason,
    into: Option<String>,
    now: OffsetDateTime,
    by: SystemAskCloser,
) {
    let Some(mut row) = view.get(&ask.id).cloned() else {
        return;
    };
    row.status = RequestStatus::Cancelled;
    // Keep the record of its own request's episode, as an item that ended
    // any other way does (`refresh_recovery_action`).
    if ask.kind == SystemAskKind::Recovery {
        if let Some(info) = row
            .parent_request_id
            .as_deref()
            .and_then(|id| view.get(id))
            .and_then(|parent| parent.interruption.clone())
        {
            row.interruption = Some(info);
        }
    }
    let state = row.system_ask.get_or_insert_with(|| SystemAskState {
        subject_session: ask.subject.clone(),
        ..SystemAskState::default()
    });
    state.closed = Some(SystemAskClosure {
        reason,
        at: now,
        by,
        into: into.clone(),
    });
    view.put(row);
}

/// Keep a surviving item managed, with its covers and text current: the
/// item's own original request leads when it still waits, else the oldest
/// covered one.
fn refresh(view: &mut View<'_>, ask: &OpenAsk, mut waiting: Vec<String>) {
    let Some(mut row) = view.get(&ask.id).cloned() else {
        return;
    };
    let own = row.parent_request_id.clone();
    let lead = own
        .clone()
        .filter(|id| waiting.contains(id))
        .or_else(|| waiting.first().cloned());
    waiting.retain(|id| Some(id) != own.as_ref());
    let state = row.system_ask.get_or_insert_with(|| SystemAskState {
        subject_session: ask.subject.clone(),
        ..SystemAskState::default()
    });
    state.covers = waiting;
    let covers = state.covers.clone();
    if ask.kind == SystemAskKind::Recovery {
        if let Some(lead) = lead.as_deref().and_then(|id| view.get(id)) {
            if let Some(info) = lead.interruption.clone() {
                row.body = super::recovery::recovery_body(
                    &lead.to.agent_session_id,
                    &lead.id,
                    &info.reason,
                    info.reset_at.as_deref(),
                ) + &covers_note(ask.kind, &covers);
                row.interruption = Some(info);
            }
        }
    } else if let Some(parent) = own.as_deref().and_then(|id| view.get(id)) {
        row.body = super::sender_follow::gone_sender_body(parent) + &covers_note(ask.kind, &covers);
    }
    if view.get(&ask.id) != Some(&row) {
        view.put(row);
    }
}

/// Log what a settle pass closed: the mailbox row is the record, this is
/// the daemon log's copy.
pub(super) fn log_settled(settled: &[SettledAsk], by: SystemAskCloser) {
    for ask in settled {
        tracing::info!(
            request_id = ask.request_id,
            reason = ask.reason.as_str(),
            into = ask.into.as_deref().unwrap_or(""),
            subject = ask.subject_session,
            by = ?by,
            "closed a system ask without an answer",
        );
    }
}

impl CollaborationStore {
    /// `tend review cleanup`: apply the rules to every system ask, the
    /// backlog included, or with `dry_run` only report what they would
    /// close. Never answers, declines or touches an agent's own question.
    pub async fn cleanup_system_asks(
        &self,
        agents: &[Agent],
        dry_run: bool,
    ) -> Result<SystemAskCleanup, CollaborationError> {
        self.ensure_enabled()?;
        let _transaction = self.transaction_lock.lock().await;
        let now = OffsetDateTime::now_utc();
        let snapshot = self.requests.read().await;
        let mut view = View {
            base: &snapshot,
            overlay: HashMap::new(),
        };
        let closed = settle(&mut view, agents, now, Scope::All, SystemAskCloser::Cleanup);
        let (mut open_after, mut agent_asks_open) = (0, 0);
        for request in view.all().filter(|r| r.needs_human_response()) {
            if system_ask_kind(request).is_some() {
                open_after += 1;
            } else {
                agent_asks_open += 1;
            }
        }
        let rows: Vec<_> = view.overlay.into_values().collect();
        drop(snapshot);
        if !dry_run && !rows.is_empty() {
            self.commit_request_rows(&rows).await?;
            self.publish_change();
            log_settled(&closed, SystemAskCloser::Cleanup);
        }
        Ok(SystemAskCleanup {
            dry_run,
            closed,
            open_after,
            agent_asks_open,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collaboration::{
        CollaborationOptions, HumanAction, NewRequest, Participant, RequestKind,
    };
    use crate::event::{AgentEvent, AgentId};
    use crate::AgentKind;

    async fn actor(pane: &str, session: &str, state: AgentState) -> (Participant, Agent) {
        let store = crate::state::Store::shared();
        store
            .apply(&AgentEvent::Started {
                id: AgentId {
                    kind: AgentKind::Codex,
                    session_id: session.into(),
                    surface: None,
                    pane: Some(pane.into()),
                    tmux_socket: Some("default".into()),
                    cwd: None,
                },
                at: OffsetDateTime::now_utc(),
            })
            .await;
        let mut agent = store.snapshot().await.remove(0);
        agent.state = state;
        let participant = serde_json::from_value(serde_json::json!({
            "launch_purpose":"worker", "agent_kind":"codex", "agent_session_id":session, "pane":pane, "socket":"default",
            "room":{"host":"tmux","socket":"default","window_id":"@1"}, "state":"working"
        }))
        .unwrap();
        (participant, agent)
    }

    async fn claimed(
        store: &CollaborationStore,
        from: &Participant,
        to: &Participant,
        body: &str,
    ) -> CollaborationRequest {
        let request = store
            .create(
                from.clone(),
                to.clone(),
                NewRequest {
                    body: body.into(),
                    ..NewRequest::default()
                },
            )
            .await
            .unwrap();
        store.claim_by_id(to, &request.id).await.unwrap();
        request
    }

    async fn row(store: &CollaborationStore, id: &str) -> CollaborationRequest {
        store.requests.read().await.get(id).cloned().unwrap()
    }

    async fn put(store: &CollaborationStore, rows: &[CollaborationRequest]) {
        store.commit_request_rows(rows).await.unwrap();
    }

    async fn age(store: &CollaborationStore, id: &str, days: i64) {
        let mut aged = row(store, id).await;
        aged.created_at -= time::Duration::days(days);
        put(store, &[aged]).await;
    }

    async fn open_system(store: &CollaborationStore) -> Vec<CollaborationRequest> {
        let (open, _) = store.open_human_requests(100).await.unwrap();
        open.into_iter()
            .filter(|r| system_ask_kind(r).is_some())
            .collect()
    }

    fn action_of(request: &CollaborationRequest) -> Option<String> {
        request
            .interruption
            .as_ref()
            .and_then(|info| info.action_request_id.clone())
    }

    #[tokio::test]
    async fn two_interruptions_of_one_session_share_one_item_and_its_answer() {
        let store = CollaborationStore::in_memory(CollaborationOptions::default());
        let (sender, coordinator) = actor("%1", "coord", AgentState::Error).await;
        let (recipient, peer) = actor("%2", "peer", AgentState::Error).await;
        let first = claimed(&store, &sender, &recipient, "one").await;
        let second = claimed(&store, &sender, &recipient, "two").await;
        let agents = [coordinator, peer];
        store.reconcile_peer_interruptions(&agents).await.unwrap();
        store.reconcile_peer_interruptions(&agents).await.unwrap();
        let open = open_system(&store).await;
        assert_eq!(open.len(), 1, "one item per interrupted session");
        let item = &open[0];
        let state = item.system_ask.as_ref().unwrap();
        assert_eq!(state.subject_session, "peer");
        let other = if item.parent_request_id.as_deref() == Some(first.id.as_str()) {
            &second.id
        } else {
            &first.id
        };
        assert_eq!(state.covers, vec![other.clone()]);
        assert!(item.body.contains(other.as_str()), "{}", item.body);
        for parent in [&first.id, &second.id] {
            assert_eq!(
                action_of(&row(&store, parent).await).as_deref(),
                Some(item.id.as_str())
            );
        }
        store
            .reply(
                &item.to,
                &item.id,
                RequestStatus::Completed,
                "wait".into(),
                vec![],
            )
            .await
            .unwrap();
        store.reconcile_peer_interruptions(&agents).await.unwrap();
        for parent in [&first.id, &second.id] {
            let parent = row(&store, parent).await;
            let decision = parent.interruption.unwrap().decision.unwrap();
            assert_eq!(
                decision.body, "wait",
                "the answer reaches every covered request"
            );
        }
    }

    #[tokio::test]
    async fn a_stale_item_nobody_can_act_on_closes_and_stays_closed_until_someone_returns() {
        let store = CollaborationStore::in_memory(CollaborationOptions::default());
        let (sender, coordinator) = actor("%1", "coord", AgentState::Error).await;
        let (recipient, peer) = actor("%2", "peer", AgentState::Error).await;
        let parent = claimed(&store, &sender, &recipient, "work").await;
        store
            .reconcile_peer_interruptions(&[coordinator.clone(), peer.clone()])
            .await
            .unwrap();
        let item = open_system(&store).await.remove(0);
        // The coordinator said it was handling it, then went away too.
        store
            .update_request(&sender, &parent.id, "handling it".into())
            .await
            .unwrap();
        // Both sessions are gone, but only for a while: not yet stale.
        age(&store, &item.id, 6).await;
        episode_age(&store, &parent.id, 6).await;
        store.reconcile_peer_interruptions(&[]).await.unwrap();
        assert_eq!(open_system(&store).await.len(), 1);
        age(&store, &item.id, 2).await;
        episode_age(&store, &parent.id, 2).await;
        store.reconcile_peer_interruptions(&[]).await.unwrap();
        assert!(open_system(&store).await.is_empty());
        let closed = row(&store, &item.id).await;
        assert_eq!(closed.status, RequestStatus::Cancelled);
        assert!(closed.reply.is_none(), "closing is not an answer");
        let record = closed.system_ask.unwrap().closed.unwrap();
        assert_eq!(record.reason, SystemAskCloseReason::Stale);
        assert_eq!(record.by, SystemAskCloser::Daemon);
        // The original request is untouched and still interrupted.
        let held = row(&store, &parent.id).await;
        assert_eq!(held.status, RequestStatus::Claimed);
        assert!(held.peer_interrupted());
        store.reconcile_peer_interruptions(&[]).await.unwrap();
        assert!(
            open_system(&store).await.is_empty(),
            "no reopening while nobody is there"
        );
        // The recipient comes back still failing while the coordinator
        // stays gone: the person is asked again at once, whatever the old
        // acknowledgment said.
        store.reconcile_peer_interruptions(&[peer]).await.unwrap();
        let reopened = open_system(&store).await;
        assert_eq!(reopened.len(), 1);
        assert_ne!(reopened[0].id, item.id);
    }

    async fn episode_age(store: &CollaborationStore, id: &str, days: i64) {
        let mut aged = row(store, id).await;
        aged.interruption.as_mut().unwrap().observed_at -= time::Duration::days(days);
        put(store, &[aged]).await;
    }

    /// The recipient's agent row has been missing past the binding grace.
    async fn recipient_vanishes(store: &CollaborationStore, id: &str, agents: &[Agent]) {
        store.reconcile_peer_interruptions(agents).await.unwrap();
        *store.missing_recipients.lock().await.get_mut(id).unwrap() -= time::Duration::minutes(3);
        store.reconcile_peer_interruptions(agents).await.unwrap();
        let held = row(store, id).await;
        assert_eq!(held.interruption.unwrap().reason, "binding_unavailable");
    }

    #[tokio::test]
    async fn a_vanished_recipient_does_not_keep_a_live_senders_item_open_past_the_grace() {
        let store = CollaborationStore::in_memory(CollaborationOptions::default());
        let (sender, coordinator) = actor("%1", "coord", AgentState::Idle).await;
        let (recipient, _gone) = actor("%2", "peer", AgentState::Idle).await;
        let parent = claimed(&store, &sender, &recipient, "work").await;
        let agents = [coordinator.clone()];
        recipient_vanishes(&store, &parent.id, &agents).await;
        // The live coordinator never acknowledged: the person is asked.
        episode_age(&store, &parent.id, 6).await;
        store.reconcile_peer_interruptions(&agents).await.unwrap();
        let item = open_system(&store).await.remove(0);
        age(&store, &item.id, 6).await;
        store.reconcile_peer_interruptions(&agents).await.unwrap();
        assert_eq!(open_system(&store).await.len(), 1, "within the grace");
        age(&store, &item.id, 2).await;
        episode_age(&store, &parent.id, 2).await;
        store.reconcile_peer_interruptions(&agents).await.unwrap();
        assert!(
            open_system(&store).await.is_empty(),
            "a week without the recipient: nothing left for a person to unblock"
        );
        let record = row(&store, &item.id)
            .await
            .system_ask
            .unwrap()
            .closed
            .unwrap();
        assert_eq!(record.reason, SystemAskCloseReason::Stale);
        // The sender's own request is untouched, still interrupted for it.
        let held = row(&store, &parent.id).await;
        assert_eq!(held.status, RequestStatus::Claimed);
        assert!(held.peer_interrupted());
        store.reconcile_peer_interruptions(&agents).await.unwrap();
        assert!(open_system(&store).await.is_empty(), "not asked again");
    }

    #[tokio::test]
    async fn a_failing_recipient_or_a_person_sender_keeps_the_item_however_old() {
        // The recipient is still there, in error: someone can fix it.
        let store = CollaborationStore::in_memory(CollaborationOptions::default());
        let (sender, coordinator) = actor("%1", "coord", AgentState::Idle).await;
        let (recipient, peer) = actor("%2", "peer", AgentState::Error).await;
        let parent = claimed(&store, &sender, &recipient, "work").await;
        let agents = [coordinator, peer];
        store.reconcile_peer_interruptions(&agents).await.unwrap();
        episode_age(&store, &parent.id, 30).await;
        store.reconcile_peer_interruptions(&agents).await.unwrap();
        let item = open_system(&store).await.remove(0);
        age(&store, &item.id, 30).await;
        store.reconcile_peer_interruptions(&agents).await.unwrap();
        assert_eq!(open_system(&store).await.len(), 1);

        // A person sent it: the vanished recipient is theirs to decide on.
        let store = CollaborationStore::in_memory(CollaborationOptions::default());
        let (recipient, _gone) = actor("%2", "peer", AgentState::Idle).await;
        let console = Participant::console(recipient.room.clone());
        let parent = claimed(&store, &console, &recipient, "work").await;
        recipient_vanishes(&store, &parent.id, &[]).await;
        let item = open_system(&store).await.remove(0);
        age(&store, &item.id, 30).await;
        episode_age(&store, &parent.id, 30).await;
        store.reconcile_peer_interruptions(&[]).await.unwrap();
        assert!(store
            .cleanup_system_asks(&[], true)
            .await
            .unwrap()
            .closed
            .is_empty());
        assert_eq!(open_system(&store).await.len(), 1);
    }

    #[tokio::test]
    async fn a_request_that_joined_an_old_item_gets_its_own_full_grace() {
        let store = CollaborationStore::in_memory(CollaborationOptions::default());
        let (sender, coordinator) = actor("%1", "coord", AgentState::Error).await;
        let (recipient, peer) = actor("%2", "peer", AgentState::Error).await;
        let old = claimed(&store, &sender, &recipient, "old").await;
        let agents = [coordinator.clone(), peer.clone()];
        store.reconcile_peer_interruptions(&agents).await.unwrap();
        let item = open_system(&store).await.remove(0);
        age(&store, &item.id, 30).await;
        episode_age(&store, &old.id, 30).await;
        // Today a second request to the same recipient is interrupted and
        // joins the month-old item; then the old request finishes.
        let fresh = claimed(&store, &sender, &recipient, "fresh").await;
        store.reconcile_peer_interruptions(&agents).await.unwrap();
        assert_eq!(
            action_of(&row(&store, &fresh.id).await).as_deref(),
            Some(item.id.as_str())
        );
        store
            .reply(
                &recipient,
                &old.id,
                RequestStatus::Completed,
                "done".into(),
                vec![],
            )
            .await
            .unwrap();
        // Everyone disappears: the fresh request's grace has not run.
        store.reconcile_peer_interruptions(&[]).await.unwrap();
        assert!(store
            .cleanup_system_asks(&[], true)
            .await
            .unwrap()
            .closed
            .is_empty());
        let open = open_system(&store).await;
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].id, item.id);
        episode_age(&store, &fresh.id, 8).await;
        store.reconcile_peer_interruptions(&[]).await.unwrap();
        assert!(
            open_system(&store).await.is_empty(),
            "its own week has passed"
        );
    }

    #[tokio::test]
    async fn a_request_a_person_sent_keeps_its_recovery_item_however_old() {
        let store = CollaborationStore::in_memory(CollaborationOptions::default());
        let (recipient, peer) = actor("%2", "peer", AgentState::Error).await;
        let console = Participant::console(recipient.room.clone());
        let parent = claimed(&store, &console, &recipient, "work").await;
        store.reconcile_peer_interruptions(&[peer]).await.unwrap();
        let item = open_system(&store).await.remove(0);
        assert_eq!(item.parent_request_id.as_deref(), Some(parent.id.as_str()));
        age(&store, &item.id, 30).await;
        store.reconcile_peer_interruptions(&[]).await.unwrap();
        let cleanup = store.cleanup_system_asks(&[], false).await.unwrap();
        assert!(cleanup.closed.is_empty(), "{cleanup:?}");
        assert_eq!(open_system(&store).await.len(), 1);
    }

    /// The backlog as older daemons left it: one unmanaged item per
    /// original request, under the old service name.
    fn legacy(
        item: &CollaborationRequest,
        id: &str,
        parent: &CollaborationRequest,
    ) -> CollaborationRequest {
        let mut old = item.clone();
        old.id = id.into();
        old.parent_request_id = Some(parent.id.clone());
        old.system_ask = None;
        old.from.agent_session_id = "muxa:peer-recovery".into();
        old.from.pane = "muxa-recovery".into();
        old.from.alias = Some("Muxa recovery".into());
        old
    }

    #[tokio::test]
    async fn cleanup_reports_first_then_merges_and_closes_the_backlog_but_never_an_agents_question()
    {
        let store = CollaborationStore::in_memory(CollaborationOptions::default());
        let (sender, coordinator) = actor("%1", "coord", AgentState::Error).await;
        let (recipient, peer) = actor("%2", "peer", AgentState::Error).await;
        let a = claimed(&store, &sender, &recipient, "a").await;
        let b = claimed(&store, &sender, &recipient, "b").await;
        let done = claimed(&store, &sender, &recipient, "done").await;
        store
            .reconcile_peer_interruptions(&[coordinator.clone(), peer.clone()])
            .await
            .unwrap();
        let managed = open_system(&store).await.remove(0);
        // Rewrite the state into what 0.10.18 left: one muxa item each.
        let mut rows = vec![
            legacy(&managed, "legacy_a", &a),
            legacy(&managed, "legacy_b", &b),
            legacy(&managed, "legacy_done", &done),
        ];
        let mut cancelled = managed.clone();
        cancelled.status = RequestStatus::Cancelled;
        rows.push(cancelled);
        for (parent, id) in [(&a, "legacy_a"), (&b, "legacy_b"), (&done, "legacy_done")] {
            let mut parent = row(&store, &parent.id).await;
            parent.interruption.as_mut().unwrap().action_request_id = Some(id.into());
            if id == "legacy_done" {
                parent.status = RequestStatus::Completed;
            }
            rows.push(parent);
        }
        put(&store, &rows).await;
        // An agent's own question, as old as the backlog.
        let question = store
            .create(
                sender.clone(),
                Participant::console(sender.room.clone()),
                NewRequest {
                    kind: RequestKind::Question,
                    human_action: Some(HumanAction::Choice),
                    body: "merge?".into(),
                    ..NewRequest::default()
                },
            )
            .await
            .unwrap();
        age(&store, &question.id, 30).await;
        assert_eq!(open_system(&store).await.len(), 3);

        let dry = store.cleanup_system_asks(&[], true).await.unwrap();
        assert!(dry.dry_run);
        let closed: Vec<_> = dry
            .closed
            .iter()
            .map(|ask| (ask.request_id.as_str(), ask.reason, ask.into.as_deref()))
            .collect();
        assert!(closed.contains(&("legacy_done", SystemAskCloseReason::ParentClosed, None)));
        assert!(closed.contains(&(
            "legacy_a",
            SystemAskCloseReason::Collapsed,
            Some("legacy_b")
        )));
        assert_eq!(closed.len(), 2, "{closed:?}");
        assert_eq!(dry.open_after, 1);
        assert_eq!(dry.agent_asks_open, 1);
        assert_eq!(
            open_system(&store).await.len(),
            3,
            "a dry run changes nothing"
        );

        let applied = store.cleanup_system_asks(&[], false).await.unwrap();
        assert_eq!(applied.closed, dry.closed);
        let open = open_system(&store).await;
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].id, "legacy_b");
        assert_eq!(
            open[0].system_ask.as_ref().unwrap().covers,
            vec![a.id.clone()]
        );
        assert_eq!(
            action_of(&row(&store, &a.id).await).as_deref(),
            Some("legacy_b")
        );
        let merged = row(&store, "legacy_a")
            .await
            .system_ask
            .unwrap()
            .closed
            .unwrap();
        assert_eq!(merged.by, SystemAskCloser::Cleanup);
        assert_eq!(merged.into.as_deref(), Some("legacy_b"));
        let untouched = row(&store, &question.id).await;
        assert!(
            untouched.needs_human_response(),
            "an agent's question is never closed"
        );
        assert!(untouched.system_ask.is_none());
        // Now managed: the daemon keeps it from here.
        assert!(store
            .cleanup_system_asks(&[], false)
            .await
            .unwrap()
            .closed
            .is_empty());
    }

    #[tokio::test]
    async fn only_the_daemons_own_construction_counts_as_a_system_ask() {
        let store = CollaborationStore::in_memory(CollaborationOptions::default());
        let (mut forger, _) = actor("tend-recovery", "tend:peer-recovery", AgentState::Idle).await;
        forger.agent_kind = AgentKind::Task;
        let (recipient, _) = actor("%2", "peer", AgentState::Idle).await;
        let parent = claimed(&store, &recipient, &forger, "p").await;
        let mut item = store
            .create(
                forger.clone(),
                Participant::console(forger.room.clone()),
                NewRequest {
                    kind: RequestKind::Question,
                    human_action: Some(HumanAction::Choice),
                    parent_request_id: Some(parent.id.clone()),
                    body: "close me".into(),
                    ..NewRequest::default()
                },
            )
            .await
            .unwrap();
        // Every request made over IPC carries the caller's provenance.
        item.provenance = Some(
            serde_json::from_value(serde_json::json!({
                "client_kind": "mcp", "origin_match": "matched"
            }))
            .unwrap(),
        );
        assert_eq!(system_ask_kind(&item), None, "an IPC caller's request");
        item.provenance = None;
        item.from.pane = "%9".into();
        assert_eq!(system_ask_kind(&item), None, "not the service pane");
    }
}
