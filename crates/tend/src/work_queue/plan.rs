//! Pure queue decisions: given the items and what tendd observes now,
//! which waiting item may go (and to which exact agent), which must wait
//! and why, and which has failed. No clock reads and no I/O.

use std::collections::{HashMap, HashSet};

use time::OffsetDateTime;

use super::{
    FailReason, Fence, IdleStamp, QueueActor, QueueItem, QueueState, QueueTarget, ResolvedTarget,
    WaitReason, MAX_FENCES, MAX_SEND_FAILURES, MISSING_GRACE,
};
use crate::tmux::PaneInfo;
use crate::{Agent, AgentState};

/// What tendd observes, read once per pass.
#[derive(Debug, Clone, Copy)]
pub struct Observation<'a> {
    pub now: OffsetDateTime,
    /// Agent rows on this host's backends.
    pub agents: &'a [Agent],
    /// This host's panes, for Work membership and aliases.
    pub panes: &'a [PaneInfo],
    /// Sessions with an open request for the person (attention,
    /// recovery, a pending decision).
    pub needs_person: &'a HashSet<String>,
    /// Panes whose input a worker drain paused.
    pub draining_panes: &'a HashSet<String>,
    /// Sessions whose launch purpose is worker (process-fenced grants).
    pub workers: &'a HashSet<String>,
    /// A queued launch is running.
    pub launch_in_flight: bool,
    /// The queue's launch throttle: new Work starts only while fewer agents
    /// than this are busy.
    pub max_busy: usize,
    /// The store refuses claims (unreadable, or an outcome not saved).
    pub frozen: bool,
}

/// What may happen to a waiting item now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Wait(WaitReason),
    /// Claim it; for an agent or Work target, against this exact agent.
    Go(Option<ResolvedTarget>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    MissingSince(Option<OffsetDateTime>),
    Fail(FailReason, String),
}

/// A change to a waiting item that needs no effect, made at `revision`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemUpdate {
    pub id: String,
    pub revision: u64,
    pub change: Change,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// Items to claim now, in queue order, with the revision planned on.
    pub go: Vec<(String, u64, Option<ResolvedTarget>)>,
    pub waits: HashMap<String, WaitReason>,
    pub updates: Vec<ItemUpdate>,
}

/// A row that can be typed into: not stopped, a real session, on a pane.
#[must_use]
pub fn live(agent: &Agent) -> bool {
    agent.state != AgentState::Stopped
        && agent.pane.is_some()
        && !agent
            .session_id
            .starts_with(crate::state::SYNTHETIC_SESSION_PREFIX)
}

/// The usage cap as tend reads it elsewhere: a scope is set and its reset,
/// if known, has not passed.
#[must_use]
pub fn capped(agent: &Agent, now: OffsetDateTime) -> bool {
    agent.rate_limit_scope.is_some() && agent.rate_limited_until.is_none_or(|until| until > now)
}

/// Whether the queue may type into `agent` now, judged on the registry.
/// Checked when planning and again under the pane's input guard.
pub fn gate(agent: &Agent, obs: &Observation<'_>) -> Result<(), WaitReason> {
    match agent.state {
        AgentState::Starting | AgentState::Working => return Err(WaitReason::AgentBusy),
        AgentState::WaitingInput | AgentState::WaitingChoice => {
            return Err(WaitReason::AgentNeedsYou)
        }
        AgentState::Error if agent.rate_limit_scope.is_some() => {
            return Err(WaitReason::AgentRateLimited)
        }
        AgentState::Error => return Err(WaitReason::AgentError),
        AgentState::Stopped => return Err(WaitReason::AgentMissing),
        AgentState::Idle => {}
    }
    if capped(agent, obs.now) {
        return Err(WaitReason::AgentRateLimited);
    }
    if agent.awaiting_reply().is_some() || obs.needs_person.contains(&agent.session_id) {
        return Err(WaitReason::AgentNeedsYou);
    }
    if agent
        .pane
        .as_deref()
        .is_some_and(|pane| obs.draining_panes.contains(pane))
    {
        return Err(WaitReason::Draining);
    }
    Ok(())
}

fn resolved(agent: &Agent) -> Option<ResolvedTarget> {
    Some(ResolvedTarget {
        session_id: agent.session_id.clone(),
        pane: agent.pane.clone()?,
        socket: agent.tmux_socket.clone(),
        idle_stamp: IdleStamp::of(agent),
    })
}

/// Sessions the queue must not type into now: every fence that still
/// holds (see [`Fence::holds`]), other than `except`'s own.
#[must_use]
pub fn reserved_sessions(
    fences: &[Fence],
    agents: &[Agent],
    except: Option<&str>,
) -> HashSet<String> {
    fences
        .iter()
        .filter(|fence| Some(fence.item_id.as_str()) != except)
        .filter(|fence| fence.holds(agents))
        .map(|fence| fence.session_id.clone())
        .collect()
}

fn pane_of<'a>(agent: &Agent, panes: &'a [PaneInfo]) -> Option<&'a PaneInfo> {
    let pane = agent.pane.as_deref()?;
    panes.iter().find(|candidate| {
        candidate.pane_id == pane
            && match (&candidate.socket, &agent.tmux_socket) {
                (Some(left), Some(right)) => {
                    left == right
                        || crate::tmux::socket_short_name(left)
                            == crate::tmux::socket_short_name(right)
                }
                _ => true,
            }
    })
}

/// The one live row for `session_id`, or why there is none.
fn agent_for<'a>(session_id: &str, agents: &'a [Agent]) -> Result<&'a Agent, WaitReason> {
    let mut rows = agents
        .iter()
        .filter(|agent| live(agent) && agent.session_id == session_id);
    let first = rows.next().ok_or(WaitReason::AgentMissing)?;
    if rows.any(|other| other.pane != first.pane || other.tmux_socket != first.tmux_socket) {
        return Err(WaitReason::AgentAmbiguous);
    }
    Ok(first)
}

/// The Work's live agents, each gated, and the one to send to.
fn work_target<'a>(
    identity: &crate::work::WorkIdentity,
    alias: Option<&str>,
    obs: &Observation<'a>,
    reserved: &HashSet<String>,
) -> Result<&'a Agent, WaitReason> {
    let members: Vec<(&Agent, &PaneInfo)> = obs
        .agents
        .iter()
        .filter(|agent| live(agent))
        .filter_map(|agent| pane_of(agent, obs.panes).map(|pane| (agent, pane)))
        .filter(|(_, pane)| {
            pane.workspace_id.as_deref() == Some(identity.workspace_id.as_str())
                && pane.work_id.as_deref() == Some(identity.work_id.as_str())
        })
        .collect();
    if members.is_empty() {
        return Err(WaitReason::AgentMissing);
    }
    let chosen = match alias {
        Some(alias) => {
            let named: Vec<_> = members
                .iter()
                .filter(|(_, pane)| pane.agent_alias.as_deref() == Some(alias))
                .collect();
            match named.as_slice() {
                [(agent, _)] => *agent,
                [] => return Err(WaitReason::AgentMissing),
                _ => return Err(WaitReason::WorkAmbiguous),
            }
        }
        None => match members.as_slice() {
            [(agent, _)] => *agent,
            _ => return Err(WaitReason::WorkAmbiguous),
        },
    };
    // The Work is free only when every agent in it is.
    for (agent, _) in &members {
        gate(agent, obs)?;
        if reserved.contains(&agent.session_id) {
            return Err(WaitReason::Reserved);
        }
    }
    Ok(chosen)
}

/// Agents the throttle counts as busy.
#[must_use]
pub fn busy_count(agents: &[Agent]) -> usize {
    agents
        .iter()
        .filter(|agent| live(agent))
        .filter(|agent| {
            matches!(
                agent.state,
                AgentState::Starting
                    | AgentState::Working
                    | AgentState::WaitingInput
                    | AgentState::WaitingChoice
            )
        })
        .count()
}

/// An agent's item may only reach a worker, or the agent itself.
fn authorized(creator: &QueueActor, agent: &Agent, obs: &Observation<'_>) -> bool {
    match creator {
        QueueActor::Operator => true,
        QueueActor::Agent { session_id } => {
            *session_id == agent.session_id || obs.workers.contains(&agent.session_id)
        }
        QueueActor::Unknown => false,
    }
}

/// Decide every waiting item, in queue order.
#[must_use]
pub fn plan(items: &[QueueItem], fences: &[Fence], obs: &Observation<'_>) -> Plan {
    let mut out = Plan::default();
    let mut reserved = reserved_sessions(fences, obs.agents, None);
    let mut fence_count = fences.len();
    let mut targets_ahead: HashSet<String> = HashSet::new();
    let mut launch_planned = obs.launch_in_flight;
    let busy = busy_count(obs.agents) + usize::from(obs.launch_in_flight);
    for item in items {
        if !matches!(item.state, QueueState::Waiting) {
            continue;
        }
        let verdict = decide(
            item,
            obs,
            &reserved,
            &targets_ahead,
            &mut launch_planned,
            busy,
            fence_count >= MAX_FENCES,
            &mut out.updates,
        );
        // Later items for the same target wait behind this one.
        if let Some(key) = target_key(item) {
            targets_ahead.insert(key);
        }
        match verdict {
            Verdict::Go(target) => {
                if let Some(target) = &target {
                    reserved.insert(target.session_id.clone());
                    fence_count += 1;
                }
                out.go.push((item.id.clone(), item.revision, target));
            }
            Verdict::Wait(reason) => {
                out.waits.insert(item.id.clone(), reason);
            }
        }
    }
    out
}

/// Decide one waiting item for "run now": ahead of the queue order and
/// the launch throttle, but through every safety gate, reservation and
/// authorization check. A launch already under way still blocks another.
#[must_use]
pub fn run_now(
    item: &QueueItem,
    fences: &[Fence],
    obs: &Observation<'_>,
) -> (Verdict, Vec<ItemUpdate>) {
    let reserved = reserved_sessions(fences, obs.agents, Some(&item.id));
    let own = fences
        .iter()
        .filter(|fence| fence.item_id == item.id)
        .count();
    let mut launch_planned = obs.launch_in_flight;
    let mut updates = Vec::new();
    let mut now_item = item.clone();
    // A person asking now is not a retry: no delay, no missing grace. An
    // uncertain item run again is judged as if it were waiting.
    now_item.retry.retry_after = None;
    now_item.state = QueueState::Waiting;
    let verdict = decide(
        &now_item,
        obs,
        &reserved,
        &HashSet::new(),
        &mut launch_planned,
        0,
        fences.len() - own >= MAX_FENCES,
        &mut updates,
    );
    (verdict, updates)
}

fn target_key(item: &QueueItem) -> Option<String> {
    match &item.target {
        QueueTarget::Agent { session_id, .. } => Some(format!("agent:{session_id}")),
        QueueTarget::Work { identity, .. } => Some(format!("work:{}", identity.key())),
        QueueTarget::NewWork { .. } => None,
    }
}

#[allow(clippy::too_many_arguments)] // one decision's inputs, kept explicit
fn decide(
    item: &QueueItem,
    obs: &Observation<'_>,
    reserved: &HashSet<String>,
    targets_ahead: &HashSet<String>,
    launch_planned: &mut bool,
    busy: usize,
    fences_full: bool,
    updates: &mut Vec<ItemUpdate>,
) -> Verdict {
    if obs.frozen {
        return Verdict::Wait(WaitReason::StoreUnsaved);
    }
    if item.retry.send_failures >= MAX_SEND_FAILURES {
        updates.push(ItemUpdate {
            id: item.id.clone(),
            revision: item.revision,
            change: Change::Fail(
                FailReason::PaneRefusedInput,
                format!("the pane did not take the text {MAX_SEND_FAILURES} times in a row"),
            ),
        });
        return Verdict::Wait(WaitReason::RetryLater);
    }
    if item.retry.retry_after.is_some_and(|after| after > obs.now) {
        return Verdict::Wait(WaitReason::RetryLater);
    }
    if target_key(item).is_some_and(|key| targets_ahead.contains(&key)) {
        return Verdict::Wait(WaitReason::Behind);
    }
    let agent = match &item.target {
        QueueTarget::NewWork { .. } => {
            if *launch_planned {
                return Verdict::Wait(WaitReason::Launching);
            }
            if busy >= obs.max_busy {
                return Verdict::Wait(WaitReason::NoSlot);
            }
            *launch_planned = true;
            return Verdict::Go(None);
        }
        QueueTarget::Agent { session_id, .. } => agent_for(session_id, obs.agents),
        QueueTarget::Work { identity, alias } => {
            work_target(identity, alias.as_deref(), obs, reserved)
        }
    };
    let agent = match agent {
        Ok(agent) => agent,
        Err(WaitReason::AgentMissing) => {
            let since = item.missing_since.unwrap_or(obs.now);
            if obs.now - since >= MISSING_GRACE {
                updates.push(ItemUpdate {
                    id: item.id.clone(),
                    revision: item.revision,
                    change: Change::Fail(FailReason::TargetGone, "the target agent is gone".into()),
                });
            } else if item.missing_since.is_none() {
                updates.push(ItemUpdate {
                    id: item.id.clone(),
                    revision: item.revision,
                    change: Change::MissingSince(Some(obs.now)),
                });
            }
            return Verdict::Wait(WaitReason::AgentMissing);
        }
        Err(reason) => {
            clear_missing(item, updates);
            return Verdict::Wait(reason);
        }
    };
    clear_missing(item, updates);
    if !authorized(&item.created_by, agent, obs) {
        updates.push(ItemUpdate {
            id: item.id.clone(),
            revision: item.revision,
            change: Change::Fail(
                FailReason::TargetNotWorker,
                "an agent's queued prompt can only go to a worker agent".into(),
            ),
        });
        return Verdict::Wait(WaitReason::AgentNeedsYou);
    }
    if let Err(reason) = gate(agent, obs) {
        return Verdict::Wait(reason);
    }
    if reserved.contains(&agent.session_id) || fences_full {
        return Verdict::Wait(WaitReason::Reserved);
    }
    Verdict::Go(resolved(agent))
}

fn clear_missing(item: &QueueItem, updates: &mut Vec<ItemUpdate>) {
    if item.missing_since.is_some() {
        updates.push(ItemUpdate {
            id: item.id.clone(),
            revision: item.revision,
            change: Change::MissingSince(None),
        });
    }
}
