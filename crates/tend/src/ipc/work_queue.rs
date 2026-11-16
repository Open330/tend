//! The Work queue inside tendd: the `work_queue_*` requests and the one
//! executor that dispatches items.
//!
//! The executor is the only place an item is claimed and acted on; "run
//! now" is a message to it, so a manual dispatch and an automatic one share
//! the claim path. It types prompts with the same sequence as
//! `send_prompt` (namespace backend, the pane's input guard, endpoint and
//! session authority, the idle gate, `deliver_prompt_holding`) and starts
//! Work through the same `WorkUpManager` as `work_up`. See
//! [`crate::work_queue`] for the contract.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use time::OffsetDateTime;
use tokio::sync::{broadcast, mpsc, oneshot};

use super::dispatch::ServerCtx;
use super::topology::{resolve_backend, send_prompt_endpoint};
use super::work::{StartRefusal, WorkUpOperationState};
use crate::prompt_delivery::{
    deliver_prompt_holding, idle_gate, lock_pane, DeliveryRefused, PromptDelivery,
};
use crate::tmux::PaneInfo;
use crate::work::WorkIdentity;
use crate::work_control::WorkUpRequest;
use crate::work_queue::plan::{self, Observation, Verdict};
use crate::work_queue::{
    Evidence, FailReason, FenceChange, IdleStamp, QueueActor, QueueAddRequest, QueueEdit,
    QueueEditRequest, QueueError, QueueItem, QueueState, QueueTarget, QueueTargetSpec,
    ResolvedTarget, WaitReason, WorkQueueStore,
};
use crate::{Agent, PromptOrigin};

/// How often the executor looks again with nothing else waking it: reset
/// times passing, grace timers, a launch finishing.
const TICK: Duration = Duration::from_secs(5);
/// Bursts of registry changes are folded into one pass.
const SETTLE: Duration = Duration::from_millis(200);
/// Pane scans (Work membership, aliases) are reused this long.
const PANE_CACHE: Duration = Duration::from_secs(3);
/// After the gate refused under the pane guard (the row changed between
/// planning and typing), look again soon.
const REFUSED_RETRY: time::Duration = time::Duration::seconds(2);
const DRAINING_RETRY: time::Duration = time::Duration::seconds(10);
const BUSY_RETRY: time::Duration = time::Duration::seconds(15);

/// "Run now", handed to the executor.
pub(super) struct RunNow {
    pub(super) actor: QueueActor,
    pub(super) id: String,
    pub(super) expected_revision: Option<u64>,
    pub(super) allow_duplicate: bool,
    pub(super) reply: oneshot::Sender<Result<QueueItem, String>>,
}

/// The queue state one server shares between its handlers and executor.
pub(super) struct QueueRuntime {
    pub(super) store: Arc<WorkQueueStore>,
    pub(super) commands: mpsc::Sender<RunNow>,
    pub(super) max_busy: usize,
}

impl QueueRuntime {
    pub(super) fn new(
        store: Arc<WorkQueueStore>,
        max_busy: usize,
    ) -> (Arc<Self>, mpsc::Receiver<RunNow>) {
        let (commands, receiver) = mpsc::channel(16);
        (
            Arc::new(Self {
                store,
                commands,
                max_busy: max_busy.max(1),
            }),
            receiver,
        )
    }
}

fn now() -> OffsetDateTime {
    OffsetDateTime::now_utc()
}

/// Who is asking, for queue authorization. Unlike the Work declarer this
/// never falls back to the person when the caller cannot be placed.
pub(super) async fn actor(
    ctx: &ServerCtx,
    caller: &mut super::identity::CollaborationConnectionActor,
) -> QueueActor {
    if caller.caller_pid.is_none() {
        return QueueActor::Unknown;
    }
    caller.observe_pane(&ctx.backends).await;
    // The person only on a successful observation: a failed or borrowed
    // scan cannot tell an agent from the person, so it may only read.
    if !caller.pane_scan_ok {
        return QueueActor::Unknown;
    }
    let Some(pane) = caller.observed_pane.clone() else {
        // No pane found. With no panes at all in the scan while tracked
        // agents sit on panes, the scan failed rather than came back empty.
        let scan_empty = caller.request_panes.as_ref().is_none_or(Vec::is_empty);
        if scan_empty && ctx.store.snapshot().await.iter().any(plan::live) {
            return QueueActor::Unknown;
        }
        return QueueActor::Operator;
    };
    ctx.store
        .by_pane(&pane)
        .await
        .into_iter()
        .filter(|agent| agent.state != crate::AgentState::Stopped)
        .filter(|agent| {
            !agent
                .session_id
                .starts_with(crate::state::SYNTHETIC_SESSION_PREFIX)
        })
        .max_by_key(|agent| agent.last_activity_at)
        .map_or(QueueActor::Operator, |agent| QueueActor::Agent {
            session_id: agent.session_id,
        })
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

fn item_response(result: Result<QueueItem, QueueError>) -> super::Response {
    match result {
        Ok(item) => super::Response {
            work_queue_item: Some(Box::new(item)),
            ..super::Response::ok()
        },
        Err(error) => super::Response::err(error.to_string()),
    }
}

#[allow(clippy::too_many_lines)] // one arm per queue request
pub(super) async fn response(
    ctx: &ServerCtx,
    actor: QueueActor,
    body: super::RequestBody,
) -> (&'static str, super::Response) {
    use super::RequestBody as R;
    let queue = &ctx.work_queue.store;
    match body {
        R::WorkQueueList { include_finished } => (
            "work_queue_list",
            super::Response {
                work_queue: Some(Box::new(queue.snapshot(include_finished).await)),
                ..super::Response::ok()
            },
        ),
        R::WorkQueueAdd { item } => ("work_queue_add", add(ctx, &actor, item).await),
        R::WorkQueueEdit {
            id,
            expected_revision,
            edit,
        } => (
            "work_queue_edit",
            edit_item(ctx, &actor, &id, expected_revision, edit).await,
        ),
        R::WorkQueueMove {
            id,
            expected_revision,
            to,
        } => (
            "work_queue_move",
            item_response(
                queue
                    .move_to(&actor, &id, expected_revision, to, now())
                    .await,
            ),
        ),
        R::WorkQueueCancel {
            id,
            expected_revision,
        } => (
            "work_queue_cancel",
            item_response(queue.cancel(&actor, &id, expected_revision, now()).await),
        ),
        R::WorkQueueRemove {
            id,
            ack_duplicate_risk,
        } => (
            "work_queue_remove",
            item_response(queue.remove(&actor, &id, ack_duplicate_risk).await),
        ),
        // The store writes any lease change with the item in one write; the
        // executor applies it to the Run store on its next pass.
        R::WorkQueueResolve {
            id,
            sent,
            ack_duplicate_risk,
        } => (
            "work_queue_resolve",
            item_response(
                queue
                    .resolve(&actor, &id, sent, ack_duplicate_risk, now())
                    .await,
            ),
        ),
        R::WorkQueueClear => (
            "work_queue_clear",
            match queue.clear(&actor).await {
                Ok(count) => super::Response {
                    work_queue_cleared: Some(count),
                    ..super::Response::ok()
                },
                Err(error) => super::Response::err(error.to_string()),
            },
        ),
        R::WorkQueueRunNow {
            id,
            expected_revision,
            allow_duplicate,
        } => {
            let (reply, answer) = oneshot::channel();
            let sent = ctx
                .work_queue
                .commands
                .send(RunNow {
                    actor,
                    id,
                    expected_revision,
                    allow_duplicate,
                    reply,
                })
                .await;
            let response = match sent {
                Err(_) => super::Response::err("the queue executor is not running"),
                Ok(()) => match answer.await {
                    Ok(Ok(item)) => super::Response {
                        work_queue_item: Some(Box::new(item)),
                        ..super::Response::ok()
                    },
                    Ok(Err(error)) => super::Response::err(error),
                    Err(_) => super::Response::err("the queue executor stopped"),
                },
            };
            ("work_queue_run_now", response)
        }
        other => unreachable!("dispatch routed {other:?} to work_queue"),
    }
}

async fn add(ctx: &ServerCtx, actor: &QueueActor, request: QueueAddRequest) -> super::Response {
    // A retry is answered from the key before anything is resolved again:
    // a new Work gets a fresh id on every resolution, and an agent target
    // may be gone by the time a lost reply is retried. Two retries racing
    // both resolve, and the store keeps the first.
    let print = crate::work_queue::request_fingerprint(&request, actor);
    if let Some(existing) = ctx
        .work_queue
        .store
        .existing_for_key(&request.request_key, &print)
        .await
    {
        return item_response(existing);
    }
    let target = match resolve_target(ctx, actor, request.target, &request.prompt, None).await {
        Ok(target) => target,
        Err(error) => return super::Response::err(error),
    };
    item_response(
        ctx.work_queue
            .store
            .add(
                actor,
                crate::work_queue::QueueAdd {
                    request_key: request.request_key,
                    title: request.title,
                    prompt: request.prompt,
                    target,
                    request_fingerprint: Some(print),
                },
                now(),
            )
            .await,
    )
}

async fn edit_item(
    ctx: &ServerCtx,
    actor: &QueueActor,
    id: &str,
    expected_revision: Option<u64>,
    edit: QueueEditRequest,
) -> super::Response {
    let target = match edit.target {
        None => None,
        Some(spec) => {
            let prompt = edit.prompt.clone().unwrap_or_default();
            match resolve_target(ctx, actor, spec, &prompt, Some(id)).await {
                Ok(target) => Some(target),
                Err(error) => return super::Response::err(error),
            }
        }
    };
    item_response(
        ctx.work_queue
            .store
            .edit(
                actor,
                id,
                expected_revision,
                QueueEdit {
                    title: edit.title,
                    prompt: edit.prompt,
                    target,
                },
                now(),
            )
            .await,
    )
}

/// Agent rows tendd can type into from here: their pane's namespace has an
/// observed backend that can send text.
async fn local_agents(ctx: &ServerCtx) -> Vec<Agent> {
    ctx.store
        .snapshot()
        .await
        .into_iter()
        .filter(|agent| {
            agent.pane.as_deref().is_some_and(|pane| {
                resolve_backend(&ctx.backends, pane).is_ok_and(|backend| backend.caps().send_text)
            })
        })
        .collect()
}

async fn scan_panes(ctx: &ServerCtx) -> Vec<PaneInfo> {
    let backends = ctx.backends.clone();
    let scan = tokio::task::spawn_blocking(move || {
        backends
            .iter()
            .flat_map(|backend| backend.list_panes())
            .collect::<Vec<_>>()
    });
    tokio::time::timeout(Duration::from_secs(3), scan)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default()
}

/// Whether anything already uses a Work id: a durable record, a pipeline
/// run, a live pane, or another queued item. Checked across workspaces so
/// a queued Work can never be mistaken for, or merged into, another.
async fn work_id_in_use(ctx: &ServerCtx, work: &str, except: Option<&str>) -> bool {
    let records = ctx.work_store.lock().await.records();
    if records
        .iter()
        .any(|record| record.identity.work_id.eq_ignore_ascii_case(work))
    {
        return true;
    }
    if ctx
        .pipeline_runs
        .list()
        .await
        .iter()
        .any(|run| run.identity.work_id.eq_ignore_ascii_case(work))
    {
        return true;
    }
    if scan_panes(ctx).await.iter().any(|pane| {
        pane.work_id
            .as_deref()
            .is_some_and(|id| id.eq_ignore_ascii_case(work))
    }) {
        return true;
    }
    ctx.work_queue.store.items().await.iter().any(|item| {
        Some(item.id.as_str()) != except
            && !matches!(
                item.state,
                QueueState::Cancelled { .. } | QueueState::Failed { .. }
            )
            && matches!(&item.target, QueueTarget::NewWork { request } if request.work.eq_ignore_ascii_case(work))
    })
}

/// Turn a client's target into a stored one, checking what can only be
/// checked here: the agent exists on this host, an agent caller may use
/// it, and a new Work id is free.
#[allow(clippy::too_many_lines)] // one arm per target kind
async fn resolve_target(
    ctx: &ServerCtx,
    actor: &QueueActor,
    spec: QueueTargetSpec,
    prompt: &str,
    except: Option<&str>,
) -> Result<QueueTarget, String> {
    match spec {
        QueueTargetSpec::NewWork {
            work,
            workspace,
            cwd,
            pipeline,
            external,
            skill,
            context,
        } => {
            let cwd = cwd.ok_or("a queued Work needs the absolute directory to start in")?;
            if !cwd.is_absolute() {
                return Err("a queued Work needs an absolute directory".into());
            }
            // The identity is fixed here, the way `work up` would normalise
            // it, and handed to the launch explicitly, so nothing resolved
            // later (a route's workspace template) can move it.
            // Always a fresh id in the queue's namespace; a name the caller
            // gave only shapes it.
            let hint = work
                .as_deref()
                .map(str::trim)
                .filter(|w| !w.is_empty())
                .unwrap_or(prompt);
            let work = crate::work_queue::make_work_id(hint, now());
            let workspace = crate::work_queue::queued_workspace(workspace.as_deref(), &cwd)?;
            if work_id_in_use(ctx, &work, except).await {
                return Err(format!(
                    "Work id {work} is already in use; queue it under a new id"
                ));
            }
            let no_ticket = external.as_deref().is_none_or(|e| e.trim().is_empty());
            Ok(QueueTarget::NewWork {
                request: WorkUpRequest {
                    work,
                    external,
                    pipeline,
                    workspace: Some(workspace),
                    cwd: Some(cwd),
                    skill,
                    body: None,
                    context,
                    no_ticket,
                    dry_run: false,
                    new_only: true,
                    queue_lease: None,
                    host: None,
                },
            })
        }
        QueueTargetSpec::Agent {
            session_id,
            pane,
            socket,
            label,
        } => {
            let agents = local_agents(ctx).await;
            let rows: Vec<&Agent> = agents
                .iter()
                .filter(|agent| plan::live(agent))
                .filter(|agent| session_id.as_deref().is_none_or(|s| agent.session_id == s))
                .filter(|agent| {
                    pane.as_deref()
                        .is_none_or(|p| agent.pane.as_deref() == Some(p))
                })
                .filter(|agent| {
                    socket.as_deref().is_none_or(|socket| {
                        agent.tmux_socket.as_deref().is_some_and(|endpoint| {
                            crate::backend::pane_endpoints_match(
                                agent.pane.as_deref(),
                                endpoint,
                                socket,
                            )
                        })
                    })
                })
                .collect();
            if session_id.is_none() && pane.is_none() {
                return Err("name the agent by session id or pane".into());
            }
            let sessions: HashSet<&str> = rows.iter().map(|a| a.session_id.as_str()).collect();
            let session = match sessions.into_iter().collect::<Vec<_>>().as_slice() {
                [one] => (*one).to_string(),
                [] => {
                    return Err(
                        "no running agent on this host matches that target (tracked, on a pane that accepts input)"
                            .into(),
                    )
                }
                _ => return Err("that pane holds several agents; name the session id".into()),
            };
            if let QueueActor::Agent { session_id: me } = actor {
                if *me != session && !ctx.collaboration.worker_sessions().await.contains(&session) {
                    return Err(
                        "an agent can queue prompts only for worker agents or itself; this one is a protected primary"
                            .into(),
                    );
                }
            }
            Ok(QueueTarget::Agent {
                session_id: session,
                label,
            })
        }
        QueueTargetSpec::Work {
            workspace,
            work,
            alias,
        } => Ok(QueueTarget::Work {
            identity: WorkIdentity::new(workspace, work),
            alias,
        }),
    }
}

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

/// The queued launch under way.
struct InFlight {
    id: String,
    attempt: String,
    operation_id: String,
    /// The attempt took over a Run an earlier attempt of this item left
    /// suspended, so even a "Work exists" refusal may follow effects.
    took_over: bool,
}

struct Executor {
    ctx: ServerCtx,
    queue: Arc<WorkQueueStore>,
    max_busy: usize,
    launch: Option<InFlight>,
    panes: Vec<PaneInfo>,
    panes_at: Option<Instant>,
}

/// Start the executor. It recovers interrupted attempts, then passes over
/// the queue on every registry or queue change and every few seconds.
pub(super) fn spawn(
    ctx: ServerCtx,
    mut commands: mpsc::Receiver<RunNow>,
    mut shutdown: broadcast::Receiver<()>,
) -> tokio::task::JoinHandle<()> {
    // Subscribe before recovery reads, so no change is lost in between.
    let mut agents = ctx.store.subscribe_changes();
    let queue = Arc::clone(&ctx.work_queue.store);
    let mut changes = queue.subscribe();
    let max_busy = ctx.work_queue.max_busy;
    tokio::spawn(async move {
        match queue.recover(now()).await {
            Ok(0) => {}
            Ok(count) => tracing::warn!(
                count,
                "work queue: attempts interrupted by a restart are uncertain"
            ),
            Err(error) => tracing::error!(%error, "work queue: recovery could not be saved"),
        }
        let mut executor = Executor {
            ctx,
            queue,
            max_busy,
            launch: None,
            panes: Vec::new(),
            panes_at: None,
        };
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            executor.pass().await;
            tokio::select! {
                biased;
                _ = shutdown.recv() => {
                    let _ = executor.queue.flush().await;
                    break;
                }
                command = commands.recv() => {
                    let Some(command) = command else { break };
                    let result = executor.run_now(&command).await;
                    let _ = command.reply.send(result);
                    continue;
                }
                agent_change = agents.changed() => if agent_change.is_err() { break },
                queue_change = changes.changed() => if queue_change.is_err() { break },
                _ = tick.tick() => {}
            }
            tokio::time::sleep(SETTLE).await;
        }
    })
}

/// Backoff for a pane that did not take the text.
fn send_backoff(failures: u32) -> time::Duration {
    let seconds = 5_i64.saturating_mul(3_i64.saturating_pow(failures.min(5)));
    time::Duration::seconds(seconds.min(300))
}

impl Executor {
    async fn observe_inputs(
        &mut self,
        items: &[QueueItem],
    ) -> (
        Vec<Agent>,
        HashSet<String>,
        HashSet<String>,
        HashSet<String>,
    ) {
        let agents = local_agents(&self.ctx).await;
        let wants_panes = items.iter().any(|item| {
            matches!(item.state, QueueState::Waiting)
                && matches!(item.target, QueueTarget::Work { .. })
        });
        if wants_panes && self.panes_at.is_none_or(|at| at.elapsed() >= PANE_CACHE) {
            self.panes = scan_panes(&self.ctx).await;
            self.panes_at = Some(Instant::now());
        }
        let needs_person = self.ctx.collaboration.sessions_awaiting_person().await;
        let workers = self.ctx.collaboration.worker_sessions().await;
        let mut draining = HashSet::new();
        for agent in agents.iter().filter(|agent| plan::live(agent)) {
            if let Some(pane) = agent.pane.as_deref() {
                if self
                    .ctx
                    .collaboration
                    .worker_input_blocked(pane, agent.tmux_socket.as_deref())
                    .await
                {
                    draining.insert(pane.to_string());
                }
            }
        }
        (agents, needs_person, draining, workers)
    }

    async fn pass(&mut self) {
        self.poll_launch().await;
        if let Err(error) = self.queue.flush().await {
            tracing::warn!(%error, "work queue: outcome still not saved");
        }
        let items = self.queue.items().await;
        // Leases follow the items' durable states; until every one is
        // written, nothing new is claimed.
        let leases_settled = self.apply_lease_intents().await;
        if !items
            .iter()
            .any(|item| matches!(item.state, QueueState::Waiting))
        {
            let _ = self
                .queue
                .apply_plan(Vec::new(), HashMap::new(), now())
                .await;
            return;
        }
        let (agents, needs_person, draining, workers) = self.observe_inputs(&items).await;
        if let Err(error) = self.queue.release_delivered(&agents).await {
            tracing::warn!(%error, "work queue: released fences not saved");
        }
        let fences = self.queue.fences().await;
        let frozen = self.queue.frozen().await || !leases_settled;
        let at = now();
        let decided = {
            let obs = Observation {
                now: at,
                agents: &agents,
                panes: &self.panes,
                needs_person: &needs_person,
                draining_panes: &draining,
                workers: &workers,
                launch_in_flight: self.launch.is_some(),
                max_busy: self.max_busy,
                frozen,
            };
            plan::plan(&items, &fences, &obs)
        };
        if let Err(error) = self
            .queue
            .apply_plan(decided.updates, decided.waits, at)
            .await
        {
            tracing::warn!(%error, "work queue: planning changes not saved");
            return;
        }
        for (id, revision, target) in decided.go {
            if let Err(error) = self.execute(&id, revision, target, false).await {
                tracing::debug!(%id, %error, "work queue: item not dispatched this pass");
            }
        }
    }

    async fn run_now(&mut self, command: &RunNow) -> Result<QueueItem, String> {
        let current = self
            .queue
            .get(&command.id)
            .await
            .ok_or_else(|| format!("no queued item {}", command.id))?;
        if !command.actor.may_manage_item(&current) {
            return Err(
                QueueError::Forbidden("an agent may only run the items it queued".into())
                    .to_string(),
            );
        }
        if let Some(expected) = command.expected_revision {
            if expected != current.revision {
                return Err(QueueError::Conflict {
                    id: command.id.clone(),
                    current: current.revision,
                }
                .to_string());
            }
        }
        // An uncertain item is claimed as it is, so it stays uncertain unless
        // it really goes; anything else is made waiting first.
        let uncertain = matches!(current.state, QueueState::Uncertain { .. });
        if uncertain && !command.allow_duplicate {
            return Err(QueueError::WrongState {
                id: command.id.clone(),
                state: "uncertain",
                action: "it may already have been sent; resolve it, or run it again allowing a duplicate",
            }
            .to_string());
        }
        let item = if uncertain {
            current
        } else {
            self.queue
                .rearm(&command.actor, &command.id, now())
                .await
                .map_err(|error| error.to_string())?
        };
        let items = self.queue.items().await;
        let leases_settled = self.apply_lease_intents().await;
        let (agents, needs_person, draining, workers) = self.observe_inputs(&items).await;
        let fences = self.queue.fences().await;
        let frozen = self.queue.frozen().await || !leases_settled;
        let at = now();
        let (verdict, updates) = {
            let obs = Observation {
                now: at,
                agents: &agents,
                panes: &self.panes,
                needs_person: &needs_person,
                draining_panes: &draining,
                workers: &workers,
                launch_in_flight: self.launch.is_some(),
                max_busy: self.max_busy,
                frozen,
            };
            plan::run_now(&item, &fences, &obs)
        };
        if let Some(fail) = updates
            .iter()
            .find(|update| matches!(update.change, plan::Change::Fail(..)))
        {
            let plan::Change::Fail(_, detail) = &fail.change else {
                unreachable!()
            };
            return Err(detail.clone());
        }
        match verdict {
            Verdict::Wait(reason) => Err(format!("not sent now: {}", wait_text(reason))),
            Verdict::Go(target) => {
                self.execute(&item.id, item.revision, target, uncertain)
                    .await
                    .map_err(|error| error.to_string())?;
                self.queue
                    .get(&item.id)
                    .await
                    .ok_or_else(|| format!("no queued item {}", item.id))
            }
        }
    }

    /// Write the lease changes the queue recorded with its outcomes to the
    /// Run store, then forget them. `false` while one could not be written;
    /// the caller then claims nothing, as for an unsaved queue outcome.
    async fn apply_lease_intents(&self) -> bool {
        let mut settled = true;
        for intent in self.queue.lease_intents().await {
            let written = self
                .ctx
                .pipeline_runs
                .settle_lease(
                    &intent.identity,
                    &intent.item_id,
                    intent.holder.as_deref(),
                    intent.state,
                )
                .await;
            match written {
                Ok(()) => {
                    if let Err(error) = self.queue.lease_applied(&intent).await {
                        tracing::warn!(%error, "work queue: applied lease not recorded");
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "work queue: lease not saved; holding new claims");
                    settled = false;
                }
            }
        }
        settled
    }

    /// Claim one item (on disk first), then act.
    async fn execute(
        &mut self,
        id: &str,
        revision: u64,
        target: Option<ResolvedTarget>,
        allow_duplicate: bool,
    ) -> Result<(), QueueError> {
        let claim = self
            .queue
            .claim(id, revision, target.clone(), allow_duplicate, now())
            .await?;
        match (&claim.item.target, target) {
            (QueueTarget::NewWork { request }, _) => {
                self.launch(&claim.item, &claim.attempt, request.clone())
                    .await;
            }
            (_, Some(target)) => self.deliver(&claim.item, &claim.attempt, target).await,
            (_, None) => {
                self.queue
                    .release(
                        id,
                        &claim.attempt,
                        Some(now() + REFUSED_RETRY),
                        false,
                        now(),
                    )
                    .await?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)] // lease, refusals and start are one claimed launch
    async fn launch(&mut self, item: &QueueItem, attempt: &str, mut request: WorkUpRequest) {
        let failed = |reason: FailReason, detail: String| {
            let attempt = attempt.to_string();
            move |item: &mut QueueItem| {
                item.state = QueueState::Failed {
                    attempt: Some(attempt),
                    at: now(),
                    reason,
                    detail,
                };
            }
        };
        let identity = WorkIdentity::new(
            request.workspace.clone().unwrap_or_default(),
            request.work.clone(),
        );
        let took_over = self
            .ctx
            .pipeline_runs
            .get(&identity)
            .await
            .and_then(|run| run.queue_lease)
            .is_some_and(|lease| lease.item_id == item.id);
        let in_use = !took_over && work_id_in_use_except_queue(&self.ctx, &request.work).await;
        let acquired = if in_use {
            Ok(())
        } else {
            self.ctx
                .pipeline_runs
                .acquire_lease(&identity, &item.id, attempt)
                .await
        };
        if let Err(error) = &acquired {
            if !matches!(error, crate::pipeline_run::PipelineRunError::Exists(_)) {
                // Not a verdict on the Work: the store could not be written.
                tracing::warn!(%error, "work queue: lease not taken; trying later");
                let _ = self
                    .queue
                    .release(&item.id, attempt, Some(now() + BUSY_RETRY), false, now())
                    .await;
                return;
            }
        }
        if in_use || acquired.is_err() {
            let _ = self
                .queue
                .record(
                    &item.id,
                    attempt,
                    now(),
                    FenceChange::Keep,
                    failed(
                        FailReason::IdentityInUse,
                        format!(
                            "Work id {} is already in use; nothing was started",
                            request.work
                        ),
                    ),
                )
                .await;
            return;
        }
        request.body = Some(item.prompt.clone());
        request.new_only = true;
        request.queue_lease = Some(attempt.to_string());
        match self.ctx.work_up.start_typed(request).await {
            Ok(operation) => {
                let operation_id = operation.operation_id.clone();
                let _ = self
                    .queue
                    .record(&item.id, attempt, now(), FenceChange::Keep, |item| {
                        if let QueueState::Dispatching {
                            operation_id: op, ..
                        } = &mut item.state
                        {
                            *op = Some(operation_id.clone());
                        }
                    })
                    .await;
                self.launch = Some(InFlight {
                    id: item.id.clone(),
                    attempt: attempt.to_string(),
                    operation_id,
                    took_over,
                });
                self.poll_launch().await;
            }
            Err(StartRefusal::Busy(reason)) => {
                tracing::debug!(%reason, "work queue: launch deferred");
                let _ = self
                    .queue
                    .release(&item.id, attempt, Some(now() + BUSY_RETRY), false, now())
                    .await;
            }
            Err(StartRefusal::Invalid(reason)) if took_over => {
                // Nothing new ran, but the earlier attempt may have left
                // part of the Work: still the person's call.
                let attempt_id = attempt.to_string();
                let _ = self
                    .queue
                    .record(&item.id, attempt, now(), FenceChange::Keep, |item| {
                        item.state = QueueState::Uncertain {
                            attempt: attempt_id,
                            at: now(),
                            resolved: None,
                            reason: format!(
                                "could not start it again ({reason}); the Work may be partly started"
                            ),
                        };
                    })
                    .await;
            }
            Err(StartRefusal::Invalid(reason)) => {
                let _ = self
                    .queue
                    .record(
                        &item.id,
                        attempt,
                        now(),
                        FenceChange::Keep,
                        failed(FailReason::LaunchRefused, reason),
                    )
                    .await;
            }
        }
    }

    async fn poll_launch(&mut self) {
        let Some(flight) = &self.launch else {
            return;
        };
        let operation = self.ctx.work_up.status(&flight.operation_id).await;
        let at = now();
        let attempt = flight.attempt.clone();
        let took_over = flight.took_over;
        let state = match operation {
            Some(operation) if operation.state == WorkUpOperationState::Running => return,
            Some(operation) if operation.state == WorkUpOperationState::Succeeded => {
                let work = operation.result.as_ref().and_then(|result| {
                    Some(WorkIdentity::new(
                        result.get("workspace")?.as_str()?,
                        result.get("work")?.as_str()?,
                    ))
                });
                QueueState::Dispatched {
                    attempt: attempt.clone(),
                    at,
                    resolved: None,
                    evidence: Evidence::default(),
                    work,
                    detail: Some(operation.message),
                }
            }
            Some(operation) => launch_failure(&attempt, at, &operation.message, took_over),
            None => QueueState::Uncertain {
                attempt: attempt.clone(),
                at,
                resolved: None,
                reason: "tendd lost track of the launch; it may have partly started".into(),
            },
        };
        let id = flight.id.clone();
        self.launch = None;
        // The record carries the lease change (released, or suspended) in
        // the same write; the next pass applies it to the Run store.
        if let Err(error) = self
            .queue
            .record(&id, &attempt, at, FenceChange::Keep, |item| {
                item.state = state;
            })
            .await
        {
            tracing::warn!(%error, "work queue: launch outcome not recorded");
        }
    }

    /// Type the prompt, re-checking everything under the pane's input guard.
    #[allow(clippy::too_many_lines)] // one guarded delivery, every check in order
    async fn deliver(&mut self, item: &QueueItem, attempt: &str, target: ResolvedTarget) {
        let store = Arc::clone(&self.queue);
        let release = |retry: time::Duration, failure: bool| {
            let queue = Arc::clone(&store);
            let id = item.id.clone();
            let attempt = attempt.to_string();
            async move {
                let _ = queue
                    .release(&id, &attempt, Some(now() + retry), failure, now())
                    .await;
            }
        };
        let fail = |reason: FailReason, detail: String| {
            let queue = Arc::clone(&store);
            let id = item.id.clone();
            let attempt = attempt.to_string();
            async move {
                let _ = queue
                    .record(&id, &attempt, now(), FenceChange::Remove, |item| {
                        item.state = QueueState::Failed {
                            attempt: Some(attempt.clone()),
                            at: now(),
                            reason,
                            detail,
                        };
                    })
                    .await;
            }
        };
        let pane = target.pane.clone();
        let backend = match resolve_backend(&self.ctx.backends, &pane) {
            Ok(backend) if backend.caps().send_text => backend.clone(),
            _ => {
                return fail(
                    FailReason::NotLocal,
                    format!("pane {pane} cannot take input from this host"),
                )
                .await
            }
        };
        let guard = lock_pane(&self.ctx.collaboration, &pane).await;
        let agents = self.ctx.store.by_pane(&pane).await;
        let Some(row) = agents
            .iter()
            .find(|agent| plan::live(agent) && agent.session_id == target.session_id)
            .cloned()
        else {
            return release(REFUSED_RETRY, false).await;
        };
        let socket = match send_prompt_endpoint(
            &pane,
            backend.kind(),
            &agents,
            target.socket.as_deref(),
            Some(&target.session_id),
        )
        .and_then(|socket| {
            idle_gate(&pane, &agents, socket.as_deref(), Some(&target.session_id))?;
            Ok(socket)
        }) {
            Ok(socket) => socket,
            Err(error) => {
                tracing::debug!(%error, "work queue: gate refused under the pane guard");
                return release(REFUSED_RETRY, false).await;
            }
        };
        // Everything the plan checked, again, now.
        let needs_person = self.ctx.collaboration.sessions_awaiting_person().await;
        let workers = self.ctx.collaboration.worker_sessions().await;
        let mut draining = HashSet::new();
        if self
            .ctx
            .collaboration
            .worker_input_blocked(&pane, socket.as_deref())
            .await
        {
            draining.insert(pane.clone());
        }
        let obs = Observation {
            now: now(),
            agents: &agents,
            panes: &[],
            needs_person: &needs_person,
            draining_panes: &draining,
            workers: &workers,
            launch_in_flight: false,
            max_busy: self.max_busy,
            frozen: false,
        };
        if let Err(reason) = plan::gate(&row, &obs) {
            let retry = if reason == WaitReason::Draining {
                DRAINING_RETRY
            } else {
                REFUSED_RETRY
            };
            return release(retry, false).await;
        }
        if let QueueActor::Agent { session_id } = &item.created_by {
            if *session_id != row.session_id && !workers.contains(&row.session_id) {
                return fail(
                    FailReason::TargetNotWorker,
                    "an agent's queued prompt can only go to a worker agent".into(),
                )
                .await;
            }
        }
        // The whole target decided again from fresh reads, still under the
        // guard: for a Work, every member's gates, its membership and the
        // chosen alias, which may have changed while this waited for the
        // guard. Anything but the same agent puts the item back.
        if !self.still_the_target(item, &target).await {
            tracing::debug!(id = %item.id, "work queue: target changed under the pane guard");
            return release(REFUSED_RETRY, false).await;
        }
        let stamp = IdleStamp::of(&row);
        let typed_at = now();
        let origin = match item.created_by {
            QueueActor::Operator => PromptOrigin::PersonViaTend,
            _ => PromptOrigin::Peer,
        };
        let delivered = deliver_prompt_holding(
            guard,
            &self.ctx.collaboration,
            &self.ctx.store,
            &agents,
            backend,
            PromptDelivery {
                pane: &pane,
                socket: socket.as_deref(),
                text: &item.prompt,
                submit: true,
                origin: Some(origin),
            },
        )
        .await;
        match delivered {
            Err(DeliveryRefused::WorkerDraining) => release(DRAINING_RETRY, false).await,
            Err(DeliveryRefused::NotSent) => {
                release(send_backoff(item.retry.send_failures), true).await;
            }
            Err(DeliveryRefused::Unknown) => {
                let resolved = ResolvedTarget {
                    socket,
                    idle_stamp: stamp,
                    ..target
                };
                self.uncertain(
                    item,
                    attempt,
                    resolved,
                    "the pane did not confirm the text; it may have been typed. Check the agent",
                )
                .await;
            }
            Ok(delivered) if !delivered.submitted => {
                let resolved = ResolvedTarget {
                    socket,
                    idle_stamp: stamp,
                    ..target
                };
                self.uncertain(
                    item,
                    attempt,
                    resolved,
                    "the text was typed but Enter was not confirmed; check the agent",
                )
                .await;
            }
            Ok(delivered) => {
                let confirmed = delivered.confirm().await;
                let resolved = ResolvedTarget {
                    socket,
                    idle_stamp: stamp.clone(),
                    ..target
                };
                let attempt_id = attempt.to_string();
                if let Err(error) = self
                    .queue
                    .record(
                        &item.id,
                        attempt,
                        now(),
                        // The provider's own hook reported this text: the
                        // composer is clear, nothing to hold back.
                        if confirmed == Some(true) {
                            FenceChange::Remove
                        } else {
                            FenceChange::Delivered(stamp, typed_at)
                        },
                        |item| {
                            item.state = QueueState::Dispatched {
                                attempt: attempt_id,
                                at: now(),
                                resolved: Some(resolved),
                                evidence: Evidence {
                                    typed: true,
                                    submitted: true,
                                    confirmed,
                                    manual: false,
                                },
                                work: None,
                                detail: None,
                            };
                        },
                    )
                    .await
                {
                    tracing::warn!(%error, "work queue: delivery not recorded");
                }
            }
        }
    }

    /// Whether planning `item` now, from fresh reads (panes scanned again),
    /// picks exactly `target`. Its own in-flight fence is not counted.
    async fn still_the_target(&mut self, item: &QueueItem, target: &ResolvedTarget) -> bool {
        self.panes_at = None;
        let items = vec![QueueItem {
            state: QueueState::Waiting,
            ..item.clone()
        }];
        let (agents, needs_person, draining, workers) = self.observe_inputs(&items).await;
        let fences = self.queue.fences().await;
        let obs = Observation {
            now: now(),
            agents: &agents,
            panes: &self.panes,
            needs_person: &needs_person,
            draining_panes: &draining,
            workers: &workers,
            launch_in_flight: false,
            max_busy: self.max_busy,
            frozen: false,
        };
        let (verdict, _) = plan::run_now(&items[0], &fences, &obs);
        matches!(
            verdict,
            Verdict::Go(Some(now)) if now.session_id == target.session_id && now.pane == target.pane
        )
    }

    /// An attempt whose effect is unknown: kept, with its fence, until a
    /// person settles it.
    async fn uncertain(
        &self,
        item: &QueueItem,
        attempt: &str,
        resolved: ResolvedTarget,
        reason: &str,
    ) {
        let attempt_id = attempt.to_string();
        let reason = reason.to_string();
        if let Err(error) = self
            .queue
            .record(&item.id, attempt, now(), FenceChange::Unresolved, |item| {
                item.state = QueueState::Uncertain {
                    attempt: attempt_id,
                    at: now(),
                    resolved: Some(resolved),
                    reason,
                };
            })
            .await
        {
            tracing::warn!(%error, "work queue: uncertain outcome not recorded");
        }
    }
}

/// A failed `work up` operation. Only the two refusals the new-only launch
/// makes before starting any agent are a clean failure; anything else may
/// have started part of the Work, so it is uncertain.
fn launch_failure(attempt: &str, at: OffsetDateTime, message: &str, took_over: bool) -> QueueState {
    match crate::work_queue::launch_refusal(message).filter(|_| !took_over) {
        Some(detail) => QueueState::Failed {
            attempt: Some(attempt.to_string()),
            at,
            reason: FailReason::IdentityInUse,
            detail: detail.to_string(),
        },
        None => QueueState::Uncertain {
            attempt: attempt.to_string(),
            at,
            resolved: None,
            reason: format!("the launch failed and may have started part of the Work: {message}"),
        },
    }
}

/// [`work_id_in_use`] for a claimed item: its own queue entry does not
/// count.
async fn work_id_in_use_except_queue(ctx: &ServerCtx, work: &str) -> bool {
    let records = ctx.work_store.lock().await.records();
    records
        .iter()
        .any(|record| record.identity.work_id.eq_ignore_ascii_case(work))
        || ctx
            .pipeline_runs
            .list()
            .await
            .iter()
            .any(|run| run.identity.work_id.eq_ignore_ascii_case(work))
        || scan_panes(ctx).await.iter().any(|pane| {
            pane.work_id
                .as_deref()
                .is_some_and(|id| id.eq_ignore_ascii_case(work))
        })
}

/// A short reason for refusals worded for people.
#[must_use]
pub fn wait_text(reason: WaitReason) -> &'static str {
    match reason {
        WaitReason::AgentBusy => "the agent is working",
        WaitReason::AgentNeedsYou => "the agent is waiting for you",
        WaitReason::AgentRateLimited => "the agent is at its usage limit",
        WaitReason::AgentError => "the agent is in an error state",
        WaitReason::AgentMissing => "the agent is not running",
        WaitReason::AgentAmbiguous => "the agent shows on more than one pane",
        WaitReason::WorkAmbiguous => "the Work has several agents; name one",
        WaitReason::Draining => "the agent's input is paused by a worker drain",
        WaitReason::Reserved => "an earlier queued prompt to this agent has not been picked up yet",
        WaitReason::Behind => "an earlier item for the same agent goes first",
        WaitReason::NoSlot => "too many agents are busy",
        WaitReason::Launching => "another queued Work is starting",
        WaitReason::RetryLater => "it will be tried again shortly",
        WaitReason::StoreUnsaved => "tendd could not save the queue",
    }
}
