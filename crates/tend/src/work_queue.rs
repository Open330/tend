//! The Work queue: prompts waiting to be sent to an agent once it is idle,
//! or to start a new Work once the queue's launch throttle allows.
//!
//! tendd owns the queue (`work-queue.json` under `$XDG_DATA_HOME/tend/`).
//! The contract is durable intent with at most one automatic attempt per
//! item:
//!
//! - an item is claimed (`Dispatching`, with a fresh attempt id) and that
//!   claim is on disk before anything is typed or launched;
//! - the outcome is recorded after the effect;
//! - an attempt whose outcome tendd cannot know (it stopped mid-send, or
//!   could not record the outcome before stopping) becomes `Uncertain` and
//!   is never repeated on its own. A person resolves it: mark it sent, or
//!   run it again knowing it may arrive twice.
//!
//! Terminal input and `work up` have no receiver-side deduplication, so
//! this is not exactly-once consumption; it is never-twice-on-its-own,
//! with every ambiguous case shown.
//!
//! [`plan`] holds the pure decisions (which item may go, or why it waits),
//! [`WorkQueueStore`] the durable state. The executor lives with the IPC
//! server (`ipc::work_queue`), so it uses the same delivery and launch
//! paths as `send_prompt` and `work_up`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::sync::{watch, Mutex};

use crate::work::WorkIdentity;
use crate::work_control::WorkUpRequest;

pub mod plan;

#[cfg(test)]
mod tests;

/// Daemons that serve the `work_queue_*` requests.
pub const WORK_QUEUE_CAPABILITY: &str = "work_queue_v1";
/// Filename under `$XDG_DATA_HOME/tend/`.
pub const WORK_QUEUE_FILENAME: &str = "work-queue.json";
const FILE_VERSION: u8 = 1;

/// Marker of a `work up --new-only` refusal made before anything ran: the
/// Work already existed. The queue reads it as a clean failure; every other
/// launch failure may have left part of a Work behind.
pub const WORK_EXISTS_BEFORE_START: &str = "work_exists_before_start";

/// Longest prompt one item may carry.
pub const MAX_PROMPT_BYTES: usize = 64 * 1024;
const MAX_TITLE_BYTES: usize = 160;
const MAX_LABEL_BYTES: usize = 160;
/// Items not yet finished (waiting, dispatching, uncertain).
pub const MAX_LIVE_ITEMS: usize = 200;
/// Finished items kept for their status until cleared.
pub const MAX_FINISHED_ITEMS: usize = 100;
/// Idempotency keys remembered, independent of clearing items.
const MAX_REQUEST_KEYS: usize = 1000;
const REQUEST_KEY_DAYS: i64 = 7;
const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
/// How long a target agent may be missing (a swap, a restart of the same
/// session) before its item fails.
pub const MISSING_GRACE: time::Duration = time::Duration::minutes(10);
/// Consecutive "the pane did not take the text" before an item fails.
pub const MAX_SEND_FAILURES: u32 = 6;
/// Fences kept at most: one per agent session the queue typed to and has
/// not yet seen take a turn. At the cap new agent sends wait; nothing is
/// evicted.
pub const MAX_FENCES: usize = 2000;

/// Default `$XDG_DATA_HOME/tend/work-queue.json`.
#[must_use]
pub fn default_work_queue_file() -> Option<PathBuf> {
    crate::paths::data_dir().map(|dir| {
        dir.join(crate::paths::CONFIG_DIRNAME)
            .join(WORK_QUEUE_FILENAME)
    })
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

/// Who asked: the person, an agent (by its session), or a caller tendd
/// could not place. `Unknown` may only read.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueueActor {
    Operator,
    Agent { session_id: String },
    Unknown,
}

impl QueueActor {
    /// Whether this caller may change `item`: the person any, an agent its
    /// own, an unplaced caller none.
    #[must_use]
    pub fn may_manage_item(&self, item: &QueueItem) -> bool {
        self.may_manage(item)
    }

    fn may_manage(&self, item: &QueueItem) -> bool {
        match self {
            Self::Operator => true,
            Self::Agent { .. } => &item.created_by == self,
            Self::Unknown => false,
        }
    }
}

/// What a queued item does when it goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueueTarget {
    /// Start a new Work with `work up`: `request.work` is fixed when the
    /// item is added, `request.body` is the item's prompt at dispatch, and
    /// the host is always this one.
    NewWork { request: WorkUpRequest },
    /// Send the prompt to this agent session as its next turn, once it is
    /// idle. The pane follows the session.
    Agent {
        session_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// Send the prompt to one agent of this Work once every agent in it is
    /// idle: the one named by `alias`, else its only agent.
    Work {
        identity: WorkIdentity,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        alias: Option<String>,
    },
}

impl QueueTarget {
    /// The Work identity a `NewWork` item reserves.
    #[must_use]
    pub fn new_work_identity(&self) -> Option<WorkIdentity> {
        match self {
            Self::NewWork { request } => Some(WorkIdentity::new(
                request
                    .workspace
                    .clone()
                    .filter(|w| !w.trim().is_empty())
                    .unwrap_or_default(),
                request.work.clone(),
            )),
            _ => None,
        }
    }
}

/// A row's own clock readings when the queue typed into it: the queue
/// compares later readings of the same row for equality, never with its
/// own clock, to tell that a turn happened since.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdleStamp {
    #[serde(with = "time::serde::rfc3339")]
    pub state_entered_at: OffsetDateTime,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub last_prompt_at: Option<OffsetDateTime>,
}

impl IdleStamp {
    #[must_use]
    pub fn of(agent: &crate::Agent) -> Self {
        Self {
            state_entered_at: agent.state_entered_at,
            last_prompt_at: agent.last_prompt_at,
        }
    }
}

/// The exact agent an attempt went to, kept so recovery and reservations
/// never re-resolve to a different one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedTarget {
    pub session_id: String,
    pub pane: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket: Option<String>,
    pub idle_stamp: IdleStamp,
}

/// What is known about a send.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    /// The text reached the pane.
    pub typed: bool,
    /// Enter was delivered after it.
    pub submitted: bool,
    /// The provider's hook reported this prompt (`None`: no evidence
    /// either way).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed: Option<bool>,
    /// A person said it was sent (resolving an uncertain attempt).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub manual: bool,
}

/// Why an item failed. Typed so clients can word it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailReason {
    /// The target agent session went away and did not come back.
    TargetGone,
    /// An agent's item targets a session that is not a worker (a protected
    /// primary).
    TargetNotWorker,
    /// The pane would not take the text, repeatedly.
    PaneRefusedInput,
    /// tendd refused the launch before starting anything.
    LaunchRefused,
    /// The Work id is already used by a record, run, pane or other item;
    /// the launch refused before starting any agent. Final: queue it again
    /// under a new Work id.
    IdentityInUse,
    /// The target cannot be used from this host.
    NotLocal,
}

impl FailReason {
    /// Whether "run now" may try the item again: nothing was typed or
    /// launched, and trying again could change the outcome.
    #[must_use]
    pub fn rearmable(self) -> bool {
        !matches!(self, Self::IdentityInUse)
    }
}

/// What a fence is holding back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FenceKind {
    /// An attempt is under way.
    InFlight,
    /// The prompt was typed and submitted; released once the same session
    /// shows a turn (its row's stamp changes).
    Delivered,
    /// The outcome is unknown (or the text was typed without a proven
    /// Enter); released only by a person.
    Unresolved,
}

/// Keeps the queue from typing into one agent session again. Stored apart
/// from the items, so clearing or trimming finished items never drops one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fence {
    pub session_id: String,
    pub pane: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket: Option<String>,
    pub item_id: String,
    pub attempt: String,
    pub kind: FenceKind,
    /// The row's stamp when the queue typed (or planned to type).
    pub idle_stamp: IdleStamp,
    #[serde(with = "time::serde::rfc3339")]
    pub since: OffsetDateTime,
    /// Read from tendd's clock just before the text was typed (delivered
    /// fences). Only a provider prompt reported at or after it releases
    /// the fence; one reported late for an earlier prompt does not.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub typed_at: Option<OffsetDateTime>,
}

impl Fence {
    /// Whether it still holds, given the agent rows seen now. Only a
    /// delivered fence ends on its own, and only on provider evidence of a
    /// prompt submitted since the queue typed: a live row of the same
    /// session whose `last_prompt_at` (written only by the provider's
    /// prompt-submitted hook or transcript, never by timing, screen or
    /// inventory repair) is at or after `typed_at` and after the stamp
    /// read before typing. A prompt reported late for an earlier turn, a
    /// state change, or a missing row proves nothing.
    #[must_use]
    pub fn holds(&self, agents: &[crate::Agent]) -> bool {
        match self.kind {
            FenceKind::InFlight | FenceKind::Unresolved => true,
            FenceKind::Delivered => {
                let after = self.typed_at.unwrap_or(self.since);
                !agents.iter().any(|agent| {
                    plan::live(agent)
                        && agent.session_id == self.session_id
                        && agent.last_prompt_at.is_some_and(|at| {
                            at >= after
                                && self.idle_stamp.last_prompt_at.is_none_or(|then| at > then)
                        })
                })
            }
        }
    }
}

/// A queued Work's lease change the queue still has to write to the Run
/// store. Kept in the queue file with the outcome that caused it, apart
/// from the items, so clearing or trimming an item never loses it; the
/// executor applies it and then drops it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseIntent {
    pub identity: WorkIdentity,
    pub item_id: String,
    /// The attempt whose lease this settles; a lease another attempt has
    /// taken over since is left alone. `None`: whichever attempt of this
    /// item holds it (the item is gone or cancelled, so no newer one can
    /// come).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder: Option<String>,
    pub state: crate::pipeline_run::QueueLeaseState,
}

/// What an outcome does to the attempt's fence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FenceChange {
    Keep,
    /// Nothing reached the agent.
    Remove,
    /// Typed and submitted, with the stamp and clock read just before
    /// typing.
    Delivered(IdleStamp, OffsetDateTime),
    Unresolved,
}

/// Why a waiting item has not gone yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitReason {
    /// The agent is working on a turn.
    AgentBusy,
    /// The agent is waiting on the person: a permission prompt, a choice,
    /// a question at the end of its turn, or an open attention request.
    AgentNeedsYou,
    /// The agent is stopped by a usage limit.
    AgentRateLimited,
    /// The agent's row is in error.
    AgentError,
    /// The target agent is not running right now.
    AgentMissing,
    /// The session shows on more than one pane.
    AgentAmbiguous,
    /// The Work has several agents and none was named.
    WorkAmbiguous,
    /// A worker drain paused the pane's input.
    Draining,
    /// An earlier queued prompt to this agent has not been picked up yet,
    /// or its outcome is uncertain.
    Reserved,
    /// An earlier item goes to the same target first.
    Behind,
    /// The queue's launch throttle is full.
    NoSlot,
    /// Another queued Work is starting.
    Launching,
    /// A send or launch was refused for now; it is tried again later.
    RetryLater,
    /// tendd could not save the queue; nothing goes until it can.
    StoreUnsaved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum QueueState {
    Waiting,
    /// Claimed; the effect may be under way. On disk before the effect.
    Dispatching {
        attempt: String,
        #[serde(with = "time::serde::rfc3339")]
        started_at: OffsetDateTime,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resolved: Option<ResolvedTarget>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        operation_id: Option<String>,
    },
    Dispatched {
        attempt: String,
        #[serde(with = "time::serde::rfc3339")]
        at: OffsetDateTime,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resolved: Option<ResolvedTarget>,
        #[serde(default)]
        evidence: Evidence,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        work: Option<WorkIdentity>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    /// The attempt may or may not have happened. Never repeated on its own.
    Uncertain {
        attempt: String,
        #[serde(with = "time::serde::rfc3339")]
        at: OffsetDateTime,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resolved: Option<ResolvedTarget>,
        reason: String,
    },
    Failed {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attempt: Option<String>,
        #[serde(with = "time::serde::rfc3339")]
        at: OffsetDateTime,
        reason: FailReason,
        detail: String,
    },
    Cancelled {
        #[serde(with = "time::serde::rfc3339")]
        at: OffsetDateTime,
    },
}

impl QueueState {
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Dispatching { .. } => "dispatching",
            Self::Dispatched { .. } => "dispatched",
            Self::Uncertain { .. } => "uncertain",
            Self::Failed { .. } => "failed",
            Self::Cancelled { .. } => "cancelled",
        }
    }

    /// Done with, as far as the queue is concerned: kept only for status.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        matches!(
            self,
            Self::Dispatched { .. } | Self::Failed { .. } | Self::Cancelled { .. }
        )
    }

    #[must_use]
    pub fn resolved(&self) -> Option<&ResolvedTarget> {
        match self {
            Self::Dispatching { resolved, .. }
            | Self::Dispatched { resolved, .. }
            | Self::Uncertain { resolved, .. } => resolved.as_ref(),
            _ => None,
        }
    }

    fn attempt(&self) -> Option<&str> {
        match self {
            Self::Dispatching { attempt, .. }
            | Self::Dispatched { attempt, .. }
            | Self::Uncertain { attempt, .. } => Some(attempt),
            Self::Failed { attempt, .. } => attempt.as_deref(),
            _ => None,
        }
    }
}

/// Transient refusals: when to look again, and how many sends in a row
/// the pane refused.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryState {
    #[serde(default, skip_serializing_if = "is_zero")]
    pub send_failures: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub refusals: u32,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub retry_after: Option<OffsetDateTime>,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde's skip_serializing_if signature
fn is_zero(value: &u32) -> bool {
    *value == 0
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueItem {
    pub id: String,
    /// Bumped by every change; edits, moves and run-now name the revision
    /// they were made against.
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub prompt: String,
    pub target: QueueTarget,
    pub created_by: QueueActor,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    pub state: QueueState,
    /// When the target agent was first seen missing; cleared when it is
    /// back.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub missing_since: Option<OffsetDateTime>,
    #[serde(default)]
    pub retry: RetryState,
}

impl QueueItem {
    fn touch(&mut self, now: OffsetDateTime) {
        self.revision += 1;
        self.updated_at = now;
    }
}

/// One item as `work_queue_list` returns it: the item, its place, and why
/// it is waiting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueItemView {
    #[serde(flatten)]
    pub item: QueueItem,
    /// 0-based place among the items not finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait: Option<WaitReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueSnapshot {
    pub items: Vec<QueueItemView>,
    /// Bumped by every change to the queue.
    pub revision: u64,
    /// When the dispatcher last looked at the waiting items.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub evaluated_at: Option<OffsetDateTime>,
    /// Set when the file could not be read; the queue is then read-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broken: Option<String>,
    /// Set while an outcome could not be saved; nothing new goes.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unsaved: bool,
    /// Agent sessions the queue will not type into until a turn shows or a
    /// person settles the attempt.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fences: Vec<Fence>,
}

/// What a client asks to add. The target is already resolved to a session
/// or a Work identity by the IPC layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueAdd {
    /// The caller's own key (a UUID), so a retried add after a lost reply
    /// returns the same item instead of adding another.
    pub request_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub prompt: String,
    pub target: QueueTarget,
    /// The fingerprint of the request as the client sent it
    /// ([`request_fingerprint`]), so a retry matches before its target is
    /// resolved again (a new Work id is minted on every resolution). `None`
    /// fingerprints this resolved add.
    #[serde(skip)]
    pub request_fingerprint: Option<String>,
}

/// A partial edit of a waiting item. `None` keeps a field; an empty title
/// clears it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueEdit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<QueueTarget>,
}

/// `[work_queue]` in `config.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorkQueueConfig {
    /// The queue starts a new Work only while fewer agents than this are
    /// busy on this host (starting, working or waiting on you), counting a
    /// queued launch under way. A throttle for the queue alone: it does not
    /// limit Work you start yourself.
    pub max_busy_agents: usize,
}

impl Default for WorkQueueConfig {
    fn default() -> Self {
        Self { max_busy_agents: 3 }
    }
}

/// A target as a client names it; tendd resolves it to a [`QueueTarget`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueueTargetSpec {
    /// Start a new Work with `work up` when the throttle allows.
    NewWork {
        /// Work id; made from the prompt when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        work: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace: Option<String>,
        /// Absolute directory to start in (required: tendd's own working
        /// directory is never a project).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<PathBuf>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pipeline: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        external: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        skill: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context: Option<String>,
    },
    /// An agent, by session id or by the pane it runs in.
    Agent {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        socket: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// One agent of a Work (by alias, else its only one).
    Work {
        workspace: String,
        work: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        alias: Option<String>,
    },
}

/// `work_queue_add`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueAddRequest {
    pub request_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub prompt: String,
    pub target: QueueTargetSpec,
}

/// `work_queue_edit`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueEditRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<QueueTargetSpec>,
}

/// Why a request was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QueueError {
    #[error("{0}")]
    Invalid(String),
    #[error("no queued item {0}")]
    NotFound(String),
    #[error("item {id} changed (revision {current}); reload and try again")]
    Conflict { id: String, current: u64 },
    #[error("{0}")]
    Forbidden(String),
    #[error("item {id} is {state}; {action}")]
    WrongState {
        id: String,
        state: &'static str,
        action: &'static str,
    },
    #[error("request key {0} was used for a different item")]
    RequestKeyConflict(String),
    #[error("the queue is full ({MAX_LIVE_ITEMS} items not finished)")]
    Full,
    #[error("work-queue.json could not be read ({0}); repair or remove it and restart tendd")]
    Broken(String),
    #[error("tendd could not save the queue: {0}")]
    Persist(String),
    /// Refused until the caller acknowledges the risk of a duplicate.
    #[error("{0}")]
    NeedsAck(String),
}

fn clean_text(value: &str, max: usize, label: &str) -> Result<String, QueueError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(QueueError::Invalid(format!("{label} is empty")));
    }
    if trimmed.len() > max {
        return Err(QueueError::Invalid(format!(
            "{label} is too long (max {max} bytes)"
        )));
    }
    if trimmed
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(QueueError::Invalid(format!(
            "{label} cannot contain control characters"
        )));
    }
    Ok(trimmed.to_string())
}

fn clean_title(title: Option<&str>) -> Result<Option<String>, QueueError> {
    match title.map(str::trim) {
        None | Some("") => Ok(None),
        Some(title) => {
            if title.contains('\n') {
                return Err(QueueError::Invalid("title must be one line".into()));
            }
            clean_text(title, MAX_TITLE_BYTES, "title").map(Some)
        }
    }
}

/// Check a target's own shape. Host locality and identity reuse are
/// checked by the caller, which can see the daemon's state.
pub fn validate_target(target: &QueueTarget) -> Result<(), QueueError> {
    match target {
        QueueTarget::NewWork { request } => {
            if request.dry_run {
                return Err(QueueError::Invalid(
                    "a queued Work cannot be a dry run".into(),
                ));
            }
            if crate::work_control::remote_host_alias(request.host.as_deref()).is_some() {
                return Err(QueueError::Invalid(
                    "queued Work runs on this host only".into(),
                ));
            }
            if request.body.is_some() {
                return Err(QueueError::Invalid(
                    "a queued Work takes its body from the item's prompt".into(),
                ));
            }
            crate::work::validate_identity(&WorkIdentity::new(
                request.workspace.clone().unwrap_or_else(|| "queue".into()),
                request.work.clone(),
            ))
            .map_err(QueueError::Invalid)?;
            request
                .validate()
                .map_err(|error| QueueError::Invalid(error.to_string()))
        }
        QueueTarget::Agent { session_id, label } => {
            clean_text(session_id, 256, "agent session")?;
            if session_id.starts_with(crate::state::SYNTHETIC_SESSION_PREFIX) {
                return Err(QueueError::Invalid(
                    "an untracked pane cannot be a queue target".into(),
                ));
            }
            if let Some(label) = label {
                clean_text(label, MAX_LABEL_BYTES, "label")?;
            }
            Ok(())
        }
        QueueTarget::Work { identity, alias } => {
            crate::work::validate_identity(identity).map_err(QueueError::Invalid)?;
            if let Some(alias) = alias {
                clean_text(alias, MAX_LABEL_BYTES, "alias")?;
            }
            Ok(())
        }
    }
}

/// A queued Work's id, in the queue's reserved namespace
/// ([`crate::pipeline_run::is_queue_work_id`]): `Q-`, a short slug of
/// `hint` (a name the caller gave, else the prompt), the date, and 12
/// random hex digits. Only the queue mints these, so nothing else can
/// start, join or claim the Work while the queue owns its launch.
#[must_use]
pub fn make_work_id(hint: &str, now: OffsetDateTime) -> String {
    let mut slug = String::new();
    for word in hint
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(4)
    {
        if !slug.is_empty() {
            slug.push('-');
        }
        slug.push_str(&word.to_ascii_uppercase());
        if slug.len() > 32 {
            slug.truncate(32);
            break;
        }
    }
    let slug = slug.trim_end_matches('-');
    let slug = if slug.is_empty() { "QUEUED" } else { slug };
    let suffix = uuid::Uuid::new_v4()
        .simple()
        .to_string()
        .to_ascii_uppercase();
    format!(
        "{}{slug}-{:04}{:02}{:02}-{}",
        crate::pipeline_run::QUEUE_WORK_PREFIX,
        now.year(),
        u8::from(now.month()),
        now.day(),
        &suffix[..crate::pipeline_run::QUEUE_WORK_SUFFIX_LEN]
    )
}

/// The workspace a queued Work starts in, fixed when it is queued: the
/// one named, else the directory's name, normalised as `work up` does
/// (`tend_work::tmux_work::workspace_id_for_cwd`).
pub fn queued_workspace(given: Option<&str>, cwd: &Path) -> Result<String, String> {
    let raw = given
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map_or_else(
            || {
                cwd.file_name()
                    .and_then(|name| name.to_str())
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or("workspace")
                    .to_string()
            },
            str::to_string,
        );
    let workspace = raw.trim().to_ascii_lowercase();
    crate::work::validate_identity(&WorkIdentity::new(workspace.clone(), "X"))?;
    Ok(workspace)
}

/// The refusals a new-only `work up` makes before it starts any agent:
/// the Work existed before the launch looked, or its registration found
/// one. `Some(detail)` for those; `None` for every other failure, which
/// may have started part of the Work.
///
/// The message is the operation's, `tend work up failed: ` and the last
/// line the CLI printed, which may carry an anyhow chain index (`1: `);
/// only the first token after those is compared.
#[must_use]
pub fn launch_refusal(message: &str) -> Option<&'static str> {
    let line = message.trim();
    let line = line
        .strip_prefix(crate::work_control::WORK_UP_FAILED_PREFIX)
        .unwrap_or(line)
        .trim_start();
    let line = match line.split_once(": ") {
        Some((index, rest)) if !index.is_empty() && index.bytes().all(|b| b.is_ascii_digit()) => {
            rest
        }
        _ => line,
    };
    let code = line.split(':').next().unwrap_or_default().trim();
    if code == WORK_EXISTS_BEFORE_START {
        Some("the Work already existed; nothing was started. Queue it again under a new Work id")
    } else if code == crate::pipeline_run::WORK_EXISTS_PREFIX {
        Some("the Work was started elsewhere first; no agent was started for this item (a worktree may have been prepared). Queue it again under a new Work id")
    } else {
        None
    }
}

fn new_id(prefix: &str) -> String {
    format!("{prefix}{}", uuid::Uuid::new_v4().simple())
}

fn fingerprint(add: &QueueAdd, creator: &QueueActor) -> String {
    if let Some(print) = &add.request_fingerprint {
        return print.clone();
    }
    hash_payload(&serde_json::json!({
        "creator": creator,
        "title": add.title,
        "prompt": add.prompt,
        "target": add.target,
    }))
}

/// What identifies one client add for idempotency: the caller and the
/// request exactly as sent, before tendd resolves its target.
#[must_use]
pub fn request_fingerprint(request: &QueueAddRequest, creator: &QueueActor) -> String {
    hash_payload(&serde_json::json!({
        "creator": creator,
        "title": request.title,
        "prompt": request.prompt,
        "target": request.target,
    }))
}

fn hash_payload(payload: &serde_json::Value) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(payload.to_string().as_bytes());
    hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            use std::fmt::Write as _;
            let _ = write!(out, "{byte:02x}");
            out
        })
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RequestKeyRecord {
    key: String,
    item_id: String,
    fingerprint: String,
    #[serde(with = "time::serde::rfc3339")]
    at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct QueueFile {
    version: u8,
    #[serde(default)]
    items: Vec<QueueItem>,
    #[serde(default)]
    request_keys: Vec<RequestKeyRecord>,
    #[serde(default)]
    fences: Vec<Fence>,
    #[serde(default)]
    lease_intents: Vec<LeaseIntent>,
}

#[derive(Debug, Default)]
struct Inner {
    items: Vec<QueueItem>,
    request_keys: Vec<RequestKeyRecord>,
    fences: Vec<Fence>,
    lease_intents: Vec<LeaseIntent>,
    /// Why the file could not be read; every mutation is refused.
    broken: Option<String>,
    /// An outcome is in memory but not on disk.
    dirty: bool,
    revision: u64,
    waits: HashMap<String, WaitReason>,
    evaluated_at: Option<OffsetDateTime>,
}

impl Inner {
    fn file(&self) -> QueueFile {
        QueueFile {
            version: FILE_VERSION,
            items: self.items.clone(),
            request_keys: self.request_keys.clone(),
            fences: self.fences.clone(),
            lease_intents: self.lease_intents.clone(),
        }
    }

    /// Note the lease change `item`'s state calls for (a queued Work only):
    /// released once dispatched, suspended for any other settled state.
    fn intend_lease(&mut self, item: &QueueItem, holder: &str) {
        use crate::pipeline_run::QueueLeaseState;
        let state = match item.state {
            QueueState::Dispatching { .. } => return,
            QueueState::Dispatched { .. } => QueueLeaseState::Released,
            _ => QueueLeaseState::Suspended,
        };
        self.push_lease_intent(item, Some(holder.to_string()), state);
    }

    /// The person let go of a queued Work (cancelled or removed its item):
    /// whatever of it exists is an ordinary Work from now on.
    fn release_lease_of(&mut self, item: &QueueItem) {
        self.push_lease_intent(item, None, crate::pipeline_run::QueueLeaseState::Released);
    }

    fn push_lease_intent(
        &mut self,
        item: &QueueItem,
        holder: Option<String>,
        state: crate::pipeline_run::QueueLeaseState,
    ) {
        let Some(identity) = item.target.new_work_identity() else {
            return;
        };
        self.lease_intents
            .retain(|intent| intent.item_id != item.id);
        self.lease_intents.push(LeaseIntent {
            identity,
            item_id: item.id.clone(),
            holder,
            state,
        });
    }

    fn fence_of(&self, item_id: &str) -> Option<usize> {
        self.fences
            .iter()
            .position(|fence| fence.item_id == item_id)
    }

    fn index(&self, id: &str) -> Result<usize, QueueError> {
        self.items
            .iter()
            .position(|item| item.id == id)
            .ok_or_else(|| QueueError::NotFound(id.to_string()))
    }

    fn check_writable(&self) -> Result<(), QueueError> {
        match &self.broken {
            Some(reason) => Err(QueueError::Broken(reason.clone())),
            None => Ok(()),
        }
    }

    fn prune(&mut self, now: OffsetDateTime) {
        let finished = self
            .items
            .iter()
            .filter(|item| item.state.is_finished())
            .count();
        let mut drop = finished.saturating_sub(MAX_FINISHED_ITEMS);
        if drop > 0 {
            // Oldest finished first.
            let mut finished_ids: Vec<_> = self
                .items
                .iter()
                .filter(|item| item.state.is_finished())
                .map(|item| (item.updated_at, item.id.clone()))
                .collect();
            finished_ids.sort();
            let remove: HashSet<_> = finished_ids
                .into_iter()
                .take(drop)
                .map(|(_, id)| id)
                .collect();
            self.items.retain(|item| !remove.contains(&item.id));
            drop = 0;
        }
        debug_assert_eq!(drop, 0);
        let cutoff = now - time::Duration::days(REQUEST_KEY_DAYS);
        self.request_keys.retain(|record| record.at >= cutoff);
        let excess = self.request_keys.len().saturating_sub(MAX_REQUEST_KEYS);
        self.request_keys.drain(..excess);
    }
}

/// What the store does to the disk. Tests swap in one that fails.
pub trait QueuePersist: Send + Sync + std::fmt::Debug {
    fn write(&self, bytes: &[u8]) -> Result<(), String>;
}

#[derive(Debug)]
struct FilePersist(PathBuf);

impl QueuePersist for FilePersist {
    fn write(&self, bytes: &[u8]) -> Result<(), String> {
        write_atomic(&self.0, bytes)
    }
}

#[derive(Debug)]
struct NoPersist;

impl QueuePersist for NoPersist {
    fn write(&self, _bytes: &[u8]) -> Result<(), String> {
        Ok(())
    }
}

/// Write-then-rename in the target's directory, owner-only, with the file
/// and the directory synced so a crash leaves the old or the new file.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("creating {}: {error}", parent.display()))?;
    let tmp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("work-queue.json"),
        std::process::id()
    ));
    let result = (|| -> std::io::Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)?;
        #[cfg(unix)]
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
        Ok(())
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("writing {}: {error}", path.display()));
    }
    Ok(())
}

/// The durable queue, shared by the IPC handlers and the dispatcher.
#[derive(Debug)]
pub struct WorkQueueStore {
    persist: Box<dyn QueuePersist>,
    inner: Mutex<Inner>,
    changes: watch::Sender<u64>,
}

/// What the dispatcher needs to know about a claim it made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub item: QueueItem,
    pub attempt: String,
}

impl WorkQueueStore {
    /// A queue that lives only in memory (tests, `Server::new`).
    #[must_use]
    pub fn in_memory() -> Arc<Self> {
        Self::with_persist(Box::new(NoPersist), Inner::default())
    }

    /// A queue on top of `persist`, starting from `items` (tests).
    #[must_use]
    pub fn with_items(persist: Box<dyn QueuePersist>, items: Vec<QueueItem>) -> Arc<Self> {
        Self::with_persist(
            persist,
            Inner {
                items,
                ..Inner::default()
            },
        )
    }

    fn with_persist(persist: Box<dyn QueuePersist>, inner: Inner) -> Arc<Self> {
        let (changes, _) = watch::channel(0);
        Arc::new(Self {
            persist,
            inner: Mutex::new(inner),
            changes,
        })
    }

    /// Read the queue back. A missing file is an empty queue; an unreadable
    /// one leaves the queue read-only and the file untouched.
    #[must_use]
    pub fn load(path: Option<PathBuf>) -> Arc<Self> {
        let Some(path) = path else {
            return Self::in_memory();
        };
        let inner = match std::fs::read(&path) {
            Ok(bytes) if bytes.len() > MAX_FILE_BYTES => Inner {
                broken: Some(format!("{} is larger than 8 MiB", path.display())),
                ..Inner::default()
            },
            Ok(bytes) => match serde_json::from_slice::<QueueFile>(&bytes) {
                Ok(file) if file.version == FILE_VERSION => Inner {
                    items: file.items,
                    request_keys: file.request_keys,
                    fences: file.fences,
                    lease_intents: file.lease_intents,
                    ..Inner::default()
                },
                Ok(file) => Inner {
                    broken: Some(format!("unknown version {}", file.version)),
                    ..Inner::default()
                },
                Err(error) => Inner {
                    broken: Some(error.to_string()),
                    ..Inner::default()
                },
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Inner::default(),
            Err(error) => Inner {
                broken: Some(error.to_string()),
                ..Inner::default()
            },
        };
        if let Some(reason) = &inner.broken {
            tracing::error!(path = %path.display(), %reason, "work queue is read-only");
        }
        Self::with_persist(Box::new(FilePersist(path)), inner)
    }

    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    fn notify(&self, inner: &mut Inner) {
        inner.revision += 1;
        let revision = inner.revision;
        self.changes.send_replace(revision);
    }

    /// Persist `next` and make it current, or leave everything as it was.
    fn commit(&self, inner: &mut Inner, next: Inner) -> Result<(), QueueError> {
        self.write(&next).map_err(QueueError::Persist)?;
        let revision = inner.revision;
        let waits = std::mem::take(&mut inner.waits);
        let evaluated_at = inner.evaluated_at;
        *inner = next;
        inner.revision = revision;
        inner.waits = waits;
        inner.evaluated_at = evaluated_at;
        inner.dirty = false;
        self.notify(inner);
        Ok(())
    }

    fn write(&self, inner: &Inner) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&inner.file()).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err("the queue file would exceed 8 MiB".into());
        }
        self.persist.write(&bytes)
    }

    fn cloned(inner: &Inner) -> Inner {
        Inner {
            items: inner.items.clone(),
            request_keys: inner.request_keys.clone(),
            fences: inner.fences.clone(),
            lease_intents: inner.lease_intents.clone(),
            broken: inner.broken.clone(),
            dirty: inner.dirty,
            revision: inner.revision,
            waits: HashMap::new(),
            evaluated_at: None,
        }
    }

    /// Everything, in queue order, with each waiting item's reason.
    pub async fn snapshot(&self, include_finished: bool) -> QueueSnapshot {
        let inner = self.inner.lock().await;
        let mut position = 0;
        let items = inner
            .items
            .iter()
            .filter(|item| include_finished || !item.state.is_finished())
            .map(|item| {
                let place = (!item.state.is_finished()).then(|| {
                    position += 1;
                    position - 1
                });
                QueueItemView {
                    item: item.clone(),
                    position: place,
                    wait: matches!(item.state, QueueState::Waiting)
                        .then(|| inner.waits.get(&item.id).copied())
                        .flatten(),
                }
            })
            .collect();
        QueueSnapshot {
            items,
            revision: inner.revision,
            evaluated_at: inner.evaluated_at,
            broken: inner.broken.clone(),
            unsaved: inner.dirty,
            fences: inner.fences.clone(),
        }
    }

    /// The fences, for planning.
    pub async fn fences(&self) -> Vec<Fence> {
        self.inner.lock().await.fences.clone()
    }

    /// Lease changes not yet written to the Run store.
    pub async fn lease_intents(&self) -> Vec<LeaseIntent> {
        self.inner.lock().await.lease_intents.clone()
    }

    /// Forget `intent` once the Run store has it (a newer one for the same
    /// item stays).
    pub async fn lease_applied(&self, intent: &LeaseIntent) -> Result<(), QueueError> {
        let mut inner = self.inner.lock().await;
        if !inner.lease_intents.contains(intent) {
            return Ok(());
        }
        let mut next = Self::cloned(&inner);
        next.lease_intents.retain(|kept| kept != intent);
        self.commit(&mut inner, next)
    }

    /// Drop delivered fences whose session has shown a turn since. Unknown
    /// and in-flight ones stay. Returns how many were released.
    pub async fn release_delivered(&self, agents: &[crate::Agent]) -> Result<usize, QueueError> {
        let mut inner = self.inner.lock().await;
        if inner.broken.is_some() || inner.dirty {
            return Ok(0);
        }
        let before = inner.fences.len();
        let mut next = Self::cloned(&inner);
        next.fences.retain(|fence| fence.holds(agents));
        let released = before - next.fences.len();
        if released > 0 {
            self.commit(&mut inner, next)?;
        }
        Ok(released)
    }

    /// The raw items, for the dispatcher's planning.
    pub async fn items(&self) -> Vec<QueueItem> {
        self.inner.lock().await.items.clone()
    }

    pub async fn get(&self, id: &str) -> Option<QueueItem> {
        let inner = self.inner.lock().await;
        inner.items.iter().find(|item| item.id == id).cloned()
    }

    /// Whether the store refuses new claims: unreadable, or an outcome not
    /// yet on disk.
    pub async fn frozen(&self) -> bool {
        let inner = self.inner.lock().await;
        inner.broken.is_some() || inner.dirty
    }

    /// Whether a live item already reserves `identity` (another
    /// `NewWork` with the same Work), other than `except`.
    pub async fn identity_queued(&self, identity: &WorkIdentity, except: Option<&str>) -> bool {
        let inner = self.inner.lock().await;
        inner.items.iter().any(|item| {
            Some(item.id.as_str()) != except
                && !matches!(
                    item.state,
                    QueueState::Cancelled { .. } | QueueState::Failed { .. }
                )
                && item.target.new_work_identity().as_ref() == Some(identity)
        })
    }

    /// The item an earlier add with `key` made, when this is a retry of it
    /// (same caller and payload, by `fingerprint`); a conflict when the key
    /// was used for something else; `None` for a new key.
    pub async fn existing_for_key(
        &self,
        key: &str,
        fingerprint: &str,
    ) -> Option<Result<QueueItem, QueueError>> {
        let key = key.trim();
        let inner = self.inner.lock().await;
        let record = inner.request_keys.iter().find(|r| r.key == key)?;
        if record.fingerprint != fingerprint {
            return Some(Err(QueueError::RequestKeyConflict(key.to_string())));
        }
        Some(
            inner
                .items
                .iter()
                .find(|item| item.id == record.item_id)
                .cloned()
                .ok_or_else(|| {
                    QueueError::Invalid(format!(
                        "request key {key} was already used for an item that has since been cleared"
                    ))
                }),
        )
    }

    pub async fn add(
        &self,
        actor: &QueueActor,
        add: QueueAdd,
        now: OffsetDateTime,
    ) -> Result<QueueItem, QueueError> {
        if matches!(actor, QueueActor::Unknown) {
            return Err(QueueError::Forbidden(
                "tendd could not tell who is asking; only listing is allowed".into(),
            ));
        }
        let key = clean_text(&add.request_key, 128, "request key")?;
        let prompt = clean_text(&add.prompt, MAX_PROMPT_BYTES, "prompt")?;
        let title = clean_title(add.title.as_deref())?;
        validate_target(&add.target)?;
        let print = fingerprint(&add, actor);
        let mut inner = self.inner.lock().await;
        inner.check_writable()?;
        if let Some(record) = inner.request_keys.iter().find(|r| r.key == key) {
            if record.fingerprint != print {
                return Err(QueueError::RequestKeyConflict(key));
            }
            return inner
                .items
                .iter()
                .find(|item| item.id == record.item_id)
                .cloned()
                .ok_or_else(|| {
                    QueueError::Invalid(format!(
                        "request key {key} was already used for an item that has since been cleared"
                    ))
                });
        }
        let live = inner
            .items
            .iter()
            .filter(|item| !item.state.is_finished())
            .count();
        if live >= MAX_LIVE_ITEMS {
            return Err(QueueError::Full);
        }
        if let Some(identity) = add.target.new_work_identity() {
            if inner.items.iter().any(|item| {
                !matches!(
                    item.state,
                    QueueState::Cancelled { .. } | QueueState::Failed { .. }
                ) && item.target.new_work_identity().as_ref() == Some(&identity)
            }) {
                return Err(QueueError::Invalid(format!(
                    "Work {} is already queued",
                    identity.key()
                )));
            }
        }
        let item = QueueItem {
            id: new_id("q-"),
            revision: 1,
            title,
            prompt,
            target: add.target,
            created_by: actor.clone(),
            created_at: now,
            updated_at: now,
            state: QueueState::Waiting,
            missing_since: None,
            retry: RetryState::default(),
        };
        let mut next = Self::cloned(&inner);
        next.items.push(item.clone());
        next.request_keys.push(RequestKeyRecord {
            key,
            item_id: item.id.clone(),
            fingerprint: print,
            at: now,
        });
        next.prune(now);
        self.commit(&mut inner, next)?;
        Ok(item)
    }

    /// Run `change` on a waiting item the actor may manage, at the
    /// revision the caller saw.
    async fn change_waiting(
        &self,
        actor: &QueueActor,
        id: &str,
        expected_revision: Option<u64>,
        action: &'static str,
        now: OffsetDateTime,
        change: impl FnOnce(&mut Inner, usize) -> Result<(), QueueError>,
    ) -> Result<QueueItem, QueueError> {
        let mut inner = self.inner.lock().await;
        inner.check_writable()?;
        let index = inner.index(id)?;
        let item = &inner.items[index];
        if !actor.may_manage(item) {
            return Err(QueueError::Forbidden(
                "an agent may only change the items it queued".into(),
            ));
        }
        if let Some(expected) = expected_revision {
            if expected != item.revision {
                return Err(QueueError::Conflict {
                    id: id.into(),
                    current: item.revision,
                });
            }
        }
        if !matches!(item.state, QueueState::Waiting) {
            return Err(QueueError::WrongState {
                id: id.into(),
                state: item.state.label(),
                action,
            });
        }
        let mut next = Self::cloned(&inner);
        change(&mut next, index)?;
        // The change may have moved the item.
        let index = next.index(id)?;
        next.items[index].touch(now);
        let item = next.items[index].clone();
        self.commit(&mut inner, next)?;
        Ok(item)
    }

    pub async fn edit(
        &self,
        actor: &QueueActor,
        id: &str,
        expected_revision: Option<u64>,
        edit: QueueEdit,
        now: OffsetDateTime,
    ) -> Result<QueueItem, QueueError> {
        let title = match &edit.title {
            None => None,
            Some(title) => Some(clean_title(Some(title))?),
        };
        let prompt = edit
            .prompt
            .as_deref()
            .map(|prompt| clean_text(prompt, MAX_PROMPT_BYTES, "prompt"))
            .transpose()?;
        if let Some(target) = &edit.target {
            validate_target(target)?;
        }
        self.change_waiting(
            actor,
            id,
            expected_revision,
            "only a waiting item can be edited",
            now,
            |inner, index| {
                if let Some(identity) = edit
                    .target
                    .as_ref()
                    .and_then(QueueTarget::new_work_identity)
                {
                    let taken = inner.items.iter().any(|other| {
                        other.id != id
                            && !matches!(
                                other.state,
                                QueueState::Cancelled { .. } | QueueState::Failed { .. }
                            )
                            && other.target.new_work_identity().as_ref() == Some(&identity)
                    });
                    if taken {
                        return Err(QueueError::Invalid(format!(
                            "Work {} is already queued",
                            identity.key()
                        )));
                    }
                }
                let item = &mut inner.items[index];
                if let Some(title) = title {
                    item.title = title;
                }
                if let Some(prompt) = prompt {
                    item.prompt = prompt;
                }
                if let Some(target) = edit.target {
                    item.target = target;
                    item.missing_since = None;
                    item.retry = RetryState::default();
                }
                Ok(())
            },
        )
        .await
    }

    /// Move a waiting item to `to`, a 0-based place among the items not
    /// finished (clamped).
    pub async fn move_to(
        &self,
        actor: &QueueActor,
        id: &str,
        expected_revision: Option<u64>,
        to: usize,
        now: OffsetDateTime,
    ) -> Result<QueueItem, QueueError> {
        self.change_waiting(
            actor,
            id,
            expected_revision,
            "only a waiting item can be moved",
            now,
            |inner, index| {
                let item = inner.items.remove(index);
                // Places count only unfinished items; finished ones keep
                // their slots after the place they precede.
                let live: Vec<usize> = inner
                    .items
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| !item.state.is_finished())
                    .map(|(i, _)| i)
                    .collect();
                let at = live
                    .get(to)
                    .copied()
                    .unwrap_or_else(|| live.last().map_or(inner.items.len(), |last| last + 1));
                inner.items.insert(at, item);
                Ok(())
            },
        )
        .await
    }

    pub async fn cancel(
        &self,
        actor: &QueueActor,
        id: &str,
        expected_revision: Option<u64>,
        now: OffsetDateTime,
    ) -> Result<QueueItem, QueueError> {
        self.change_waiting(
            actor,
            id,
            expected_revision,
            "it can no longer be cancelled",
            now,
            |inner, index| {
                inner.items[index].state = QueueState::Cancelled { at: now };
                let item = inner.items[index].clone();
                inner.release_lease_of(&item);
                Ok(())
            },
        )
        .await
    }

    /// Delete an item. A dispatching one is refused. One whose attempt may
    /// have reached the agent (uncertain, or holding an unresolved fence)
    /// needs `ack_duplicate_risk`: removing it releases the fence, and the
    /// queue may then type to that agent while it still holds this text.
    /// A delivered fence is kept; it ends when the agent takes a turn.
    pub async fn remove(
        &self,
        actor: &QueueActor,
        id: &str,
        ack_duplicate_risk: bool,
    ) -> Result<QueueItem, QueueError> {
        let mut inner = self.inner.lock().await;
        inner.check_writable()?;
        let index = inner.index(id)?;
        let item = inner.items[index].clone();
        if !actor.may_manage(&item) {
            return Err(QueueError::Forbidden(
                "an agent may only remove the items it queued".into(),
            ));
        }
        if matches!(item.state, QueueState::Dispatching { .. }) {
            return Err(QueueError::WrongState {
                id: id.into(),
                state: item.state.label(),
                action: "it is being dispatched and cannot be removed",
            });
        }
        let unresolved = inner
            .fence_of(id)
            .is_some_and(|at| inner.fences[at].kind != FenceKind::Delivered)
            || matches!(item.state, QueueState::Uncertain { .. });
        if unresolved && !ack_duplicate_risk {
            return Err(QueueError::NeedsAck(
                "this item may already have reached the agent; removing it lets the queue type to that agent again. Pass ack_duplicate_risk (CLI: --ack-duplicate-risk) to remove it anyway".into(),
            ));
        }
        let mut next = Self::cloned(&inner);
        next.items.remove(index);
        if unresolved {
            next.fences.retain(|fence| fence.item_id != id);
        }
        // A queued Work the person removes is theirs now: an ordinary
        // Work, its lease released, in the same write.
        next.release_lease_of(&item);
        self.commit(&mut inner, next)?;
        Ok(item)
    }

    /// Forget finished items: the actor's own, or every one for the person.
    pub async fn clear(&self, actor: &QueueActor) -> Result<usize, QueueError> {
        if matches!(actor, QueueActor::Unknown) {
            return Err(QueueError::Forbidden(
                "tendd could not tell who is asking; only listing is allowed".into(),
            ));
        }
        let mut inner = self.inner.lock().await;
        inner.check_writable()?;
        let mut next = Self::cloned(&inner);
        let before = next.items.len();
        next.items
            .retain(|item| !(item.state.is_finished() && actor.may_manage(item)));
        let removed = before - next.items.len();
        if removed > 0 {
            self.commit(&mut inner, next)?;
        }
        Ok(removed)
    }

    /// The person settles an uncertain attempt: it was sent (`Dispatched`,
    /// manual evidence; its fence becomes a delivered one, released at the
    /// agent's next turn), or it was not (`Waiting` again, fence released;
    /// that needs `ack_duplicate_risk`, since the text may still be in the
    /// agent's input).
    pub async fn resolve(
        &self,
        actor: &QueueActor,
        id: &str,
        sent: bool,
        ack_duplicate_risk: bool,
        now: OffsetDateTime,
    ) -> Result<QueueItem, QueueError> {
        let mut inner = self.inner.lock().await;
        inner.check_writable()?;
        let index = inner.index(id)?;
        let item = &inner.items[index];
        if !actor.may_manage(item) {
            return Err(QueueError::Forbidden(
                "an agent may only resolve the items it queued".into(),
            ));
        }
        let QueueState::Uncertain {
            attempt, resolved, ..
        } = &item.state
        else {
            return Err(QueueError::WrongState {
                id: id.into(),
                state: item.state.label(),
                action: "only an uncertain item can be resolved",
            });
        };
        if !sent && !ack_duplicate_risk {
            return Err(QueueError::NeedsAck(
                "marking it not sent lets the queue send it (and type to that agent) again; if the text did reach the agent it arrives twice. Pass ack_duplicate_risk (CLI: --ack-duplicate-risk)".into(),
            ));
        }
        let state = if sent {
            QueueState::Dispatched {
                attempt: attempt.clone(),
                at: now,
                resolved: resolved.clone(),
                evidence: Evidence {
                    manual: true,
                    ..Evidence::default()
                },
                work: item.target.new_work_identity(),
                detail: Some("marked as sent".into()),
            }
        } else {
            QueueState::Waiting
        };
        let mut next = Self::cloned(&inner);
        if sent {
            for fence in next.fences.iter_mut().filter(|fence| fence.item_id == id) {
                fence.kind = FenceKind::Delivered;
            }
        } else {
            next.fences.retain(|fence| fence.item_id != id);
        }
        let holder = attempt.clone();
        next.items[index].state = state;
        next.items[index].retry = RetryState::default();
        next.items[index].touch(now);
        let item = next.items[index].clone();
        // Marked started: released. Not started: stays suspended until the
        // next attempt takes it over.
        if sent {
            next.intend_lease(&item, &holder);
        }
        self.commit(&mut inner, next)?;
        Ok(item)
    }

    /// Make a failed or cancelled item waiting again, for a new attempt. A
    /// waiting item is returned as is. An uncertain item is not rearmed:
    /// "run now" claims it directly ([`Self::claim`] with
    /// `allow_duplicate`), so it stays uncertain unless it really goes.
    pub async fn rearm(
        &self,
        actor: &QueueActor,
        id: &str,
        now: OffsetDateTime,
    ) -> Result<QueueItem, QueueError> {
        let mut inner = self.inner.lock().await;
        inner.check_writable()?;
        let index = inner.index(id)?;
        let item = &inner.items[index];
        if !actor.may_manage(item) {
            return Err(QueueError::Forbidden(
                "an agent may only run the items it queued".into(),
            ));
        }
        match &item.state {
            QueueState::Waiting => return Ok(item.clone()),
            QueueState::Failed { reason, .. } if !reason.rearmable() => {
                return Err(QueueError::WrongState {
                    id: id.into(),
                    state: "failed",
                    action: "its Work id is taken; queue it again under a new Work id",
                })
            }
            QueueState::Failed { .. } | QueueState::Cancelled { .. } => {}
            QueueState::Uncertain { .. } => {
                return Err(QueueError::WrongState {
                    id: id.into(),
                    state: "uncertain",
                    action: "it may already have been sent; resolve it, or run it again allowing a duplicate",
                })
            }
            QueueState::Dispatching { .. } | QueueState::Dispatched { .. } => {
                return Err(QueueError::WrongState {
                    id: id.into(),
                    state: item.state.label(),
                    action: "it was already dispatched",
                })
            }
        }
        let live = inner
            .items
            .iter()
            .filter(|item| !item.state.is_finished())
            .count();
        if live >= MAX_LIVE_ITEMS {
            return Err(QueueError::Full);
        }
        let mut next = Self::cloned(&inner);
        next.items[index].state = QueueState::Waiting;
        next.items[index].retry = RetryState::default();
        next.items[index].missing_since = None;
        next.items[index].touch(now);
        let item = next.items[index].clone();
        self.commit(&mut inner, next)?;
        Ok(item)
    }

    /// Claim a waiting item for one attempt: the item becomes
    /// `Dispatching` and, for an agent target, an in-flight fence holds the
    /// session, both in one write that is on disk before this returns. An
    /// item that changed since `expected_revision` is not claimed.
    ///
    /// `allow_duplicate` also claims an uncertain item (the person accepted
    /// that it may arrive twice); its own fence is replaced.
    pub async fn claim(
        &self,
        id: &str,
        expected_revision: u64,
        resolved: Option<ResolvedTarget>,
        allow_duplicate: bool,
        now: OffsetDateTime,
    ) -> Result<Claim, QueueError> {
        let mut inner = self.inner.lock().await;
        inner.check_writable()?;
        if inner.dirty {
            return Err(QueueError::Persist(
                "an earlier outcome is not saved yet".into(),
            ));
        }
        let index = inner.index(id)?;
        let item = &inner.items[index];
        if item.revision != expected_revision {
            return Err(QueueError::Conflict {
                id: id.into(),
                current: item.revision,
            });
        }
        let claimable = match item.state {
            QueueState::Waiting => true,
            QueueState::Uncertain { .. } => allow_duplicate,
            _ => false,
        };
        if !claimable {
            return Err(QueueError::WrongState {
                id: id.into(),
                state: item.state.label(),
                action: "it is not waiting",
            });
        }
        if matches!(item.target, QueueTarget::NewWork { .. }) != resolved.is_none() {
            return Err(QueueError::Invalid(
                "an agent target is claimed against one resolved agent, a new Work against none"
                    .into(),
            ));
        }
        let mut next = Self::cloned(&inner);
        next.fences.retain(|fence| fence.item_id != id);
        let attempt = uuid::Uuid::new_v4().to_string();
        if let Some(target) = &resolved {
            if next
                .fences
                .iter()
                .any(|fence| fence.session_id == target.session_id)
            {
                return Err(QueueError::WrongState {
                    id: id.into(),
                    state: "waiting",
                    action: "an earlier queued prompt to that agent is not settled",
                });
            }
            if next.fences.len() >= MAX_FENCES {
                return Err(QueueError::WrongState {
                    id: id.into(),
                    state: "waiting",
                    action: "too many agents hold unsettled queued prompts",
                });
            }
            next.fences.push(Fence {
                session_id: target.session_id.clone(),
                pane: target.pane.clone(),
                socket: target.socket.clone(),
                item_id: id.to_string(),
                attempt: attempt.clone(),
                kind: FenceKind::InFlight,
                idle_stamp: target.idle_stamp.clone(),
                since: now,
                typed_at: None,
            });
        }
        next.items[index].state = QueueState::Dispatching {
            attempt: attempt.clone(),
            started_at: now,
            resolved,
            operation_id: None,
        };
        next.items[index].retry.retry_after = None;
        next.items[index].touch(now);
        let item = next.items[index].clone();
        self.commit(&mut inner, next)?;
        Ok(Claim { item, attempt })
    }

    /// Change the state of the item `attempt` claimed, after an effect,
    /// and its fence with it. The new state is current at once; if it
    /// cannot be saved the store stays dirty (no new claims) until
    /// [`Self::flush`] succeeds, and a restart before that reads the claim
    /// back, which recovery makes uncertain.
    pub async fn record(
        &self,
        id: &str,
        attempt: &str,
        now: OffsetDateTime,
        fence: FenceChange,
        update: impl FnOnce(&mut QueueItem),
    ) -> Result<QueueItem, QueueError> {
        let mut inner = self.inner.lock().await;
        let index = inner.index(id)?;
        let item = &mut inner.items[index];
        if item.state.attempt() != Some(attempt)
            || !matches!(item.state, QueueState::Dispatching { .. })
        {
            return Err(QueueError::WrongState {
                id: id.into(),
                state: item.state.label(),
                action: "the attempt is no longer current",
            });
        }
        update(item);
        item.touch(now);
        let item = item.clone();
        inner.intend_lease(&item, attempt);
        let owned = |fence: &Fence| fence.item_id == id && fence.attempt == attempt;
        match fence {
            FenceChange::Keep => {}
            FenceChange::Remove => inner.fences.retain(|fence| !owned(fence)),
            FenceChange::Delivered(stamp, typed_at) => {
                for fence in inner.fences.iter_mut().filter(|fence| owned(fence)) {
                    fence.kind = FenceKind::Delivered;
                    fence.idle_stamp = stamp.clone();
                    fence.typed_at = Some(typed_at);
                    fence.since = now;
                }
            }
            FenceChange::Unresolved => {
                for fence in inner.fences.iter_mut().filter(|fence| owned(fence)) {
                    fence.kind = FenceKind::Unresolved;
                    fence.since = now;
                }
            }
        }
        inner.dirty = true;
        if let Err(error) = self.write(&inner) {
            tracing::warn!(%error, "work queue outcome not saved yet; holding new claims");
        } else {
            inner.dirty = false;
        }
        self.notify(&mut inner);
        Ok(item)
    }

    /// Save a dirty store. `Ok(true)` when it is clean afterwards.
    pub async fn flush(&self) -> Result<bool, QueueError> {
        let mut inner = self.inner.lock().await;
        if !inner.dirty {
            return Ok(true);
        }
        match self.write(&inner) {
            Ok(()) => {
                inner.dirty = false;
                self.notify(&mut inner);
                Ok(true)
            }
            Err(error) => Err(QueueError::Persist(error)),
        }
    }

    /// Update waiting items between passes, without an effect: missing
    /// timers, failures found by planning. Saved like any mutation.
    pub async fn apply_plan(
        &self,
        updates: Vec<plan::ItemUpdate>,
        waits: HashMap<String, WaitReason>,
        now: OffsetDateTime,
    ) -> Result<(), QueueError> {
        let mut inner = self.inner.lock().await;
        inner.evaluated_at = Some(now);
        let changed_waits = inner.waits != waits;
        inner.waits = waits;
        if updates.is_empty() || inner.broken.is_some() {
            if changed_waits {
                self.notify(&mut inner);
            }
            return Ok(());
        }
        let mut next = Self::cloned(&inner);
        let mut changed = false;
        // Several updates may name one item at the revision it was planned
        // on; the first one bumps it.
        let planned: HashMap<String, u64> = inner
            .items
            .iter()
            .map(|item| (item.id.clone(), item.revision))
            .collect();
        for update in updates {
            let Ok(index) = next.index(&update.id) else {
                continue;
            };
            let item = &mut next.items[index];
            if planned.get(&update.id) != Some(&update.revision)
                || !matches!(item.state, QueueState::Waiting)
            {
                continue;
            }
            match update.change {
                plan::Change::MissingSince(since) => item.missing_since = since,
                plan::Change::Fail(reason, detail) => {
                    item.state = QueueState::Failed {
                        attempt: None,
                        at: now,
                        reason,
                        detail,
                    };
                }
            }
            item.touch(now);
            changed = true;
        }
        if changed {
            self.commit(&mut inner, next)?;
        } else if changed_waits {
            self.notify(&mut inner);
        }
        Ok(())
    }

    /// Put a claimed item back to waiting: the effect was refused before
    /// anything happened. `send_failure` counts toward
    /// [`MAX_SEND_FAILURES`]; `retry_after` delays the next look.
    pub async fn release(
        &self,
        id: &str,
        attempt: &str,
        retry_after: Option<OffsetDateTime>,
        send_failure: bool,
        now: OffsetDateTime,
    ) -> Result<QueueItem, QueueError> {
        self.record(id, attempt, now, FenceChange::Remove, |item| {
            item.state = QueueState::Waiting;
            item.retry.retry_after = retry_after;
            if send_failure {
                item.retry.send_failures += 1;
            } else {
                item.retry.refusals += 1;
            }
        })
        .await
    }

    /// After a restart: every attempt that was under way has an unknown
    /// outcome. Returns how many became uncertain.
    pub async fn recover(&self, now: OffsetDateTime) -> Result<usize, QueueError> {
        let mut inner = self.inner.lock().await;
        if inner.broken.is_some() {
            return Ok(0);
        }
        let mut next = Self::cloned(&inner);
        let mut count = 0;
        let mut intents = Vec::new();
        for item in &mut next.items {
            if let QueueState::Dispatching {
                attempt, resolved, ..
            } = &item.state
            {
                let reason = match item.target {
                    QueueTarget::NewWork { ref request } => format!(
                        "tendd stopped while starting Work {}; it may have partly started",
                        request.work
                    ),
                    _ => "tendd stopped while sending; check whether the agent got it".into(),
                };
                let holder = attempt.clone();
                item.state = QueueState::Uncertain {
                    attempt: attempt.clone(),
                    at: now,
                    resolved: resolved.clone(),
                    reason,
                };
                item.touch(now);
                intents.push((item.clone(), holder));
                count += 1;
            }
        }
        for (item, holder) in intents {
            next.intend_lease(&item, &holder);
        }
        for fence in &mut next.fences {
            if fence.kind == FenceKind::InFlight {
                fence.kind = FenceKind::Unresolved;
            }
        }
        if count > 0 || next.fences != inner.fences {
            self.commit(&mut inner, next)?;
        }
        Ok(count)
    }
}
