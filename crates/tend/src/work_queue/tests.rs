use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use time::macros::datetime;
use time::OffsetDateTime;

use super::plan::{plan, Observation};
use super::*;
use crate::event::{AgentKind, RateLimitScope};
use crate::process_tree::WorkloadSummary;
use crate::tmux::PaneInfo;
use crate::{Agent, AgentState};

const NOW: OffsetDateTime = datetime!(2026-11-16 12:00 UTC);

fn agent(session: &str, pane: &str, state: AgentState) -> Agent {
    Agent {
        ask_origin: None,
        kind: AgentKind::Codex,
        session_id: session.into(),
        surface: None,
        pane: Some(pane.into()),
        tmux_socket: Some("default".into()),
        tmux_session: Some("tend".into()),
        cwd: Some("/repo".into()),
        pid: None,
        workload: WorkloadSummary::default(),
        subagents: Vec::new(),
        state,
        last_prompt: None,
        last_prompt_at: None,
        last_response: None,
        recap: None,
        ai_title: None,
        pending_command: None,
        last_notification: None,
        model: None,
        context_used_pct: None,
        cost_usd: None,
        rate_limit_5h_pct: None,
        rate_limit_5h_resets_at: None,
        rate_limit_7d_pct: None,
        rate_limit_7d_resets_at: None,
        rate_limited_until: None,
        rate_limit_scope: None,
        rate_limit_source: None,
        state_source: None,
        started_at: NOW - time::Duration::hours(1),
        last_activity_at: NOW,
        state_entered_at: NOW - time::Duration::minutes(5),
        last_prompt_origin: None,
        last_event_at: None,
        last_response_at: None,
        awaiting_reply: None,
    }
}

fn pane(id: &str, workspace: &str, work: &str, alias: Option<&str>) -> PaneInfo {
    serde_json::from_value(serde_json::json!({
        "pane_id": id,
        "session_id": "$1",
        "session": "tend",
        "window_id": "@1",
        "window_name": work,
        "window_index": "1",
        "pane_index": "0",
        "tty": "/dev/ttys001",
        "current_command": "codex",
        "title": "",
        "pane_pid": 0,
        "socket": "default",
        "workspace_id": workspace,
        "work_id": work,
        "agent_alias": alias,
    }))
    .expect("pane fixture")
}

struct World {
    agents: Vec<Agent>,
    panes: Vec<PaneInfo>,
    needs_person: HashSet<String>,
    draining: HashSet<String>,
    workers: HashSet<String>,
    launch_in_flight: bool,
    max_busy: usize,
    frozen: bool,
    now: OffsetDateTime,
}

impl World {
    fn new(agents: Vec<Agent>) -> Self {
        Self {
            agents,
            panes: Vec::new(),
            needs_person: HashSet::new(),
            draining: HashSet::new(),
            workers: HashSet::new(),
            launch_in_flight: false,
            max_busy: 3,
            frozen: false,
            now: NOW,
        }
    }

    fn obs(&self) -> Observation<'_> {
        Observation {
            now: self.now,
            agents: &self.agents,
            panes: &self.panes,
            needs_person: &self.needs_person,
            draining_panes: &self.draining,
            workers: &self.workers,
            launch_in_flight: self.launch_in_flight,
            max_busy: self.max_busy,
            frozen: self.frozen,
        }
    }
}

fn item(id: &str, target: QueueTarget) -> QueueItem {
    QueueItem {
        id: id.into(),
        revision: 1,
        title: None,
        prompt: format!("prompt for {id}"),
        target,
        created_by: QueueActor::Operator,
        created_at: NOW,
        updated_at: NOW,
        state: QueueState::Waiting,
        missing_since: None,
        retry: RetryState::default(),
    }
}

fn to_agent(session: &str) -> QueueTarget {
    QueueTarget::Agent {
        session_id: session.into(),
        label: None,
    }
}

fn new_work(work: &str) -> QueueTarget {
    QueueTarget::NewWork {
        request: WorkUpRequest {
            work: work.into(),
            external: None,
            pipeline: Some("pair".into()),
            workspace: Some("web".into()),
            cwd: Some("/repo".into()),
            skill: None,
            body: None,
            context: None,
            no_ticket: true,
            dry_run: false,
            new_only: false,
            queue_lease: None,
            host: None,
        },
    }
}

fn go_ids(plan: &plan::Plan) -> Vec<&str> {
    plan.go.iter().map(|(id, _, _)| id.as_str()).collect()
}

// ---------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------

#[test]
fn an_idle_agent_gets_the_first_item_only() {
    let world = World::new(vec![agent("s1", "%1", AgentState::Idle)]);
    let items = vec![item("a", to_agent("s1")), item("b", to_agent("s1"))];
    let plan = plan(&items, &[], &world.obs());
    assert_eq!(go_ids(&plan), ["a"]);
    assert_eq!(plan.waits["b"], WaitReason::Behind);
    let target = plan.go[0].2.as_ref().expect("resolved");
    assert_eq!(target.pane, "%1");
    assert_eq!(
        target.idle_stamp.state_entered_at,
        NOW - time::Duration::minutes(5)
    );
}

#[test]
fn busy_human_blocked_limited_and_draining_agents_get_nothing() {
    let cases: Vec<(Agent, WaitReason)> = vec![
        (agent("s", "%1", AgentState::Working), WaitReason::AgentBusy),
        (
            agent("s", "%1", AgentState::Starting),
            WaitReason::AgentBusy,
        ),
        (
            agent("s", "%1", AgentState::WaitingInput),
            WaitReason::AgentNeedsYou,
        ),
        (
            agent("s", "%1", AgentState::WaitingChoice),
            WaitReason::AgentNeedsYou,
        ),
        (agent("s", "%1", AgentState::Error), WaitReason::AgentError),
        (
            {
                let mut a = agent("s", "%1", AgentState::Error);
                a.rate_limit_scope = Some(RateLimitScope::FiveHour);
                a
            },
            WaitReason::AgentRateLimited,
        ),
        (
            {
                // An Idle row with an active cap whose reset is unknown.
                let mut a = agent("s", "%1", AgentState::Idle);
                a.rate_limit_scope = Some(RateLimitScope::Unknown);
                a
            },
            WaitReason::AgentRateLimited,
        ),
        (
            {
                let mut a = agent("s", "%1", AgentState::Idle);
                a.rate_limit_scope = Some(RateLimitScope::FiveHour);
                a.rate_limited_until = Some(NOW + time::Duration::minutes(30));
                a
            },
            WaitReason::AgentRateLimited,
        ),
        (
            {
                let mut a = agent("s", "%1", AgentState::Idle);
                a.awaiting_reply = Some(crate::state::AwaitingReply {
                    question: "Should I push?".into(),
                    since: NOW,
                });
                a
            },
            WaitReason::AgentNeedsYou,
        ),
    ];
    for (row, expected) in cases {
        let state = row.state;
        let world = World::new(vec![row]);
        let decided = plan(&[item("a", to_agent("s"))], &[], &world.obs());
        assert!(decided.go.is_empty(), "{state:?} must not get a prompt");
        assert_eq!(decided.waits["a"], expected, "{state:?}");
    }

    let mut world = World::new(vec![agent("s", "%1", AgentState::Idle)]);
    world.needs_person.insert("s".into());
    let decided = plan(&[item("a", to_agent("s"))], &[], &world.obs());
    assert_eq!(decided.waits["a"], WaitReason::AgentNeedsYou);

    let mut world = World::new(vec![agent("s", "%1", AgentState::Idle)]);
    world.draining.insert("%1".into());
    let decided = plan(&[item("a", to_agent("s"))], &[], &world.obs());
    assert_eq!(decided.waits["a"], WaitReason::Draining);
}

#[test]
fn a_passed_reset_no_longer_blocks() {
    let mut row = agent("s", "%1", AgentState::Idle);
    row.rate_limit_scope = Some(RateLimitScope::FiveHour);
    row.rate_limited_until = Some(NOW - time::Duration::minutes(1));
    let world = World::new(vec![row]);
    assert_eq!(
        go_ids(&plan(&[item("a", to_agent("s"))], &[], &world.obs())),
        ["a"]
    );
}

fn fence(item: &str, row: &Agent, kind: FenceKind) -> Fence {
    Fence {
        session_id: row.session_id.clone(),
        pane: row.pane.clone().unwrap(),
        socket: row.tmux_socket.clone(),
        item_id: item.into(),
        attempt: "t1".into(),
        kind,
        idle_stamp: IdleStamp::of(row),
        since: NOW,
        typed_at: Some(NOW),
    }
}

#[test]
fn a_delivered_fence_holds_the_agent_until_the_same_session_takes_a_turn() {
    let row = agent("s", "%1", AgentState::Idle);
    let fences = vec![fence("a", &row, FenceKind::Delivered)];
    let items = vec![item("b", to_agent("s"))];

    // The hook has not moved the row yet: still Idle with the same stamp.
    let world = World::new(vec![row.clone()]);
    let first = plan(&items, &fences, &world.obs());
    assert!(first.go.is_empty());
    assert_eq!(first.waits["b"], WaitReason::Reserved);

    // Gone from the registry: no evidence, still held (and missing).
    let world = World::new(vec![]);
    assert!(fences[0].holds(&world.agents));

    // Back with the same stamp: still held.
    let world = World::new(vec![row.clone()]);
    assert_eq!(
        plan(&items, &fences, &world.obs()).waits["b"],
        WaitReason::Reserved
    );

    // Another session on the same pane is no evidence for this one.
    let mut other = row.clone();
    other.session_id = "other".into();
    other.state_entered_at = NOW + time::Duration::hours(1);
    assert!(fences[0].holds(&[row.clone(), other]));

    // It ran a turn and is idle again.
    let mut later = row;
    later.state_entered_at = NOW + time::Duration::minutes(2);
    later.last_prompt_at = Some(NOW + time::Duration::seconds(1));
    let world = World::new(vec![later]);
    assert_eq!(go_ids(&plan(&items, &fences, &world.obs())), ["b"]);
}

#[test]
fn unresolved_and_in_flight_fences_hold_whatever_the_row_shows() {
    let row = agent("s", "%1", AgentState::Idle);
    let mut later = row.clone();
    later.state_entered_at = NOW + time::Duration::hours(1);
    later.last_prompt_at = Some(NOW + time::Duration::minutes(30));
    let world = World::new(vec![later]);
    for kind in [FenceKind::Unresolved, FenceKind::InFlight] {
        let fences = vec![fence("a", &row, kind)];
        let decided = plan(&[item("b", to_agent("s"))], &fences, &world.obs());
        assert_eq!(decided.waits["b"], WaitReason::Reserved, "{kind:?}");
    }
}

#[test]
fn a_fence_on_any_member_holds_a_work_target() {
    let a = agent("sa", "%1", AgentState::Idle);
    let b = agent("sb", "%2", AgentState::Idle);
    let mut world = World::new(vec![a.clone(), b.clone()]);
    world.panes = vec![
        pane("%1", "web", "W1", Some("impl")),
        pane("%2", "web", "W1", Some("review")),
    ];
    let target = QueueTarget::Work {
        identity: crate::work::WorkIdentity::new("web", "W1"),
        alias: Some("impl".into()),
    };
    let fences = vec![fence("x", &b, FenceKind::Unresolved)];
    let decided = plan(&[item("w", target.clone())], &fences, &world.obs());
    assert_eq!(decided.waits["w"], WaitReason::Reserved);
    assert_eq!(
        go_ids(&plan(&[item("w", target)], &[], &world.obs())),
        ["w"]
    );
}

#[test]
fn a_missing_target_waits_out_the_grace_then_fails() {
    let mut world = World::new(vec![]);
    let mut waiting = item("a", to_agent("gone"));
    let first = plan(std::slice::from_ref(&waiting), &[], &world.obs());
    assert_eq!(first.waits["a"], WaitReason::AgentMissing);
    assert_eq!(
        first.updates[0].change,
        plan::Change::MissingSince(Some(NOW))
    );

    waiting.missing_since = Some(NOW);
    world.now = NOW + MISSING_GRACE;
    let later = plan(&[waiting.clone()], &[], &world.obs());
    assert!(matches!(
        later.updates[0].change,
        plan::Change::Fail(FailReason::TargetGone, _)
    ));

    // Back before the grace ran out: the timer is cleared.
    world.now = NOW + time::Duration::minutes(1);
    world.agents = vec![agent("gone", "%4", AgentState::Working)];
    let back = plan(&[waiting], &[], &world.obs());
    assert_eq!(back.updates[0].change, plan::Change::MissingSince(None));
    assert_eq!(back.waits["a"], WaitReason::AgentBusy);
}

#[test]
fn a_stopped_or_synthetic_row_is_missing() {
    let world = World::new(vec![
        agent("s", "%1", AgentState::Stopped),
        agent("synthetic-1", "%2", AgentState::Idle),
    ]);
    let plan = plan(
        &[item("a", to_agent("s")), item("b", to_agent("synthetic-1"))],
        &[],
        &world.obs(),
    );
    assert_eq!(plan.waits["a"], WaitReason::AgentMissing);
    assert_eq!(plan.waits["b"], WaitReason::AgentMissing);
}

#[test]
fn an_agents_item_reaches_only_workers_or_itself() {
    let mut world = World::new(vec![
        agent("primary", "%1", AgentState::Idle),
        agent("worker", "%2", AgentState::Idle),
        agent("me", "%3", AgentState::Idle),
    ]);
    world.workers.insert("worker".into());
    let by_agent = |id: &str, target: &str| {
        let mut queued = item(id, to_agent(target));
        queued.created_by = QueueActor::Agent {
            session_id: "me".into(),
        };
        queued
    };
    let plan = plan(
        &[
            by_agent("p", "primary"),
            by_agent("w", "worker"),
            by_agent("m", "me"),
        ],
        &[],
        &world.obs(),
    );
    assert_eq!(go_ids(&plan), ["w", "m"]);
    assert!(plan.updates.iter().any(|update| update.id == "p"
        && matches!(
            update.change,
            plan::Change::Fail(FailReason::TargetNotWorker, _)
        )));
}

#[test]
fn a_work_target_needs_every_member_idle_and_one_named_agent() {
    let identity = crate::work::WorkIdentity::new("web", "ENG-1");
    let mut world = World::new(vec![
        agent("impl", "%1", AgentState::Idle),
        agent("review", "%2", AgentState::Working),
    ]);
    world.panes = vec![
        pane("%1", "web", "ENG-1", Some("impl")),
        pane("%2", "web", "ENG-1", Some("review")),
    ];
    let named = item(
        "a",
        QueueTarget::Work {
            identity: identity.clone(),
            alias: Some("impl".into()),
        },
    );
    let first = plan(std::slice::from_ref(&named), &[], &world.obs());
    assert_eq!(first.waits["a"], WaitReason::AgentBusy);

    world.agents[1].state = AgentState::Idle;
    let ready = plan(std::slice::from_ref(&named), &[], &world.obs());
    assert_eq!(ready.go[0].2.as_ref().unwrap().session_id, "impl");

    let unnamed = item(
        "b",
        QueueTarget::Work {
            identity,
            alias: None,
        },
    );
    let ambiguous = plan(&[unnamed], &[], &world.obs());
    assert_eq!(ambiguous.waits["b"], WaitReason::WorkAmbiguous);
}

#[test]
fn new_work_starts_one_at_a_time_under_the_throttle() {
    let mut world = World::new(vec![
        agent("a", "%1", AgentState::Working),
        agent("b", "%2", AgentState::Working),
    ]);
    let items = vec![item("x", new_work("w1")), item("y", new_work("w2"))];
    let plan1 = plan(&items, &[], &world.obs());
    assert_eq!(go_ids(&plan1), ["x"]);
    assert_eq!(plan1.waits["y"], WaitReason::Launching);

    world.launch_in_flight = true;
    let plan2 = plan(&items, &[], &world.obs());
    assert!(plan2.go.is_empty());

    world.launch_in_flight = false;
    world
        .agents
        .push(agent("c", "%3", AgentState::WaitingInput));
    let plan3 = plan(&items, &[], &world.obs());
    assert_eq!(plan3.waits["x"], WaitReason::NoSlot);
}

#[test]
fn a_frozen_store_or_a_retry_delay_holds_everything() {
    let mut world = World::new(vec![agent("s", "%1", AgentState::Idle)]);
    world.frozen = true;
    let held = plan(&[item("a", to_agent("s"))], &[], &world.obs());
    assert_eq!(held.waits["a"], WaitReason::StoreUnsaved);

    world.frozen = false;
    let mut delayed = item("a", to_agent("s"));
    delayed.retry.retry_after = Some(NOW + time::Duration::seconds(30));
    assert_eq!(
        plan(&[delayed], &[], &world.obs()).waits["a"],
        WaitReason::RetryLater
    );
}

#[test]
fn repeated_send_failures_fail_the_item() {
    let world = World::new(vec![agent("s", "%1", AgentState::Idle)]);
    let mut refused = item("a", to_agent("s"));
    refused.retry.send_failures = MAX_SEND_FAILURES;
    let plan = plan(&[refused], &[], &world.obs());
    assert!(plan.go.is_empty());
    assert!(matches!(
        plan.updates[0].change,
        plan::Change::Fail(FailReason::PaneRefusedInput, _)
    ));
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct FlakyDisk {
    fail: AtomicBool,
    last: std::sync::Mutex<Option<Vec<u8>>>,
}

#[derive(Debug)]
struct Flaky(Arc<FlakyDisk>);

impl QueuePersist for Flaky {
    fn write(&self, bytes: &[u8]) -> Result<(), String> {
        if self.0.fail.load(Ordering::SeqCst) {
            return Err("disk full".into());
        }
        *self.0.last.lock().unwrap() = Some(bytes.to_vec());
        Ok(())
    }
}

impl FlakyDisk {
    /// The queue as the last successful write left it: what a restarted
    /// tendd would read.
    fn reload(&self) -> Arc<WorkQueueStore> {
        let bytes = self.last.lock().unwrap().clone().unwrap_or_default();
        let file: QueueFile = serde_json::from_slice(&bytes).expect("file");
        WorkQueueStore::with_persist(
            Box::new(NoPersist),
            Inner {
                items: file.items,
                request_keys: file.request_keys,
                fences: file.fences,
                lease_intents: file.lease_intents,
                ..Inner::default()
            },
        )
    }
}

fn add(prompt: &str, target: QueueTarget) -> QueueAdd {
    QueueAdd {
        request_fingerprint: None,
        request_key: uuid::Uuid::new_v4().to_string(),
        title: None,
        prompt: prompt.into(),
        target,
    }
}

#[tokio::test]
async fn add_is_idempotent_by_request_key() {
    let store = WorkQueueStore::in_memory();
    let request = add("do it", to_agent("s"));
    let first = store
        .add(&QueueActor::Operator, request.clone(), NOW)
        .await
        .unwrap();
    let again = store
        .add(&QueueActor::Operator, request.clone(), NOW)
        .await
        .unwrap();
    assert_eq!(first.id, again.id);
    assert_eq!(store.items().await.len(), 1);

    let mut different = request;
    different.prompt = "something else".into();
    assert!(matches!(
        store.add(&QueueActor::Operator, different, NOW).await,
        Err(QueueError::RequestKeyConflict(_))
    ));
}

#[tokio::test]
async fn bad_input_is_refused_before_anything_is_stored() {
    let store = WorkQueueStore::in_memory();
    for bad in [
        add("   ", to_agent("s")),
        add("ok", to_agent("synthetic-x")),
        add("x\u{1b}[2J", to_agent("s")),
    ] {
        assert!(store.add(&QueueActor::Operator, bad, NOW).await.is_err());
    }
    let mut remote = new_work("w1");
    if let QueueTarget::NewWork { request } = &mut remote {
        request.host = Some("build".into());
    }
    assert!(store
        .add(&QueueActor::Operator, add("go", remote), NOW)
        .await
        .is_err());
    assert!(store
        .add(&QueueActor::Unknown, add("go", to_agent("s")), NOW)
        .await
        .is_err());
    assert!(store.items().await.is_empty());

    store
        .add(&QueueActor::Operator, add("one", new_work("w1")), NOW)
        .await
        .unwrap();
    assert!(store
        .add(&QueueActor::Operator, add("two", new_work("w1")), NOW)
        .await
        .is_err());
}

#[tokio::test]
async fn claim_is_on_disk_before_it_returns_and_a_restart_makes_it_uncertain() {
    fn row_again() -> Agent {
        agent("s", "%1", AgentState::Idle)
    }
    let disk = Arc::new(FlakyDisk::default());
    let store = WorkQueueStore::with_items(Box::new(Flaky(disk.clone())), Vec::new());
    let queued = store
        .add(&QueueActor::Operator, add("hello", to_agent("s")), NOW)
        .await
        .unwrap();
    let row = agent("s", "%1", AgentState::Idle);
    let target = ResolvedTarget {
        session_id: "s".into(),
        pane: "%1".into(),
        socket: None,
        idle_stamp: IdleStamp::of(&row),
    };
    let claim = store
        .claim(&queued.id, queued.revision, Some(target), false, NOW)
        .await
        .unwrap();

    // tendd dies here, after the claim and maybe after typing.
    let restarted = disk.reload();
    assert_eq!(restarted.recover(NOW).await.unwrap(), 1);
    let item = restarted.get(&queued.id).await.unwrap();
    let QueueState::Uncertain { attempt, .. } = &item.state else {
        panic!("expected uncertain, got {:?}", item.state);
    };
    assert_eq!(attempt, &claim.attempt);
    // Never repeated on its own: the planner sees nothing waiting.
    let world = World::new(vec![row]);
    assert!(plan(
        &restarted.items().await,
        &restarted.fences().await,
        &world.obs()
    )
    .go
    .is_empty());
    // Its fence came back unresolved: the agent stays held.
    let fences = restarted.fences().await;
    assert_eq!(fences.len(), 1);
    assert_eq!(fences[0].kind, FenceKind::Unresolved);
    // Run-now never rearms it; only a claim that accepts a duplicate takes
    // it, replacing its own fence.
    assert!(restarted
        .rearm(&QueueActor::Operator, &queued.id, NOW)
        .await
        .is_err());
    let current = restarted.get(&queued.id).await.unwrap();
    let target = ResolvedTarget {
        session_id: "s".into(),
        pane: "%1".into(),
        socket: None,
        idle_stamp: IdleStamp::of(&row_again()),
    };
    assert!(restarted
        .claim(
            &queued.id,
            current.revision,
            Some(target.clone()),
            false,
            NOW
        )
        .await
        .is_err());
    let again = restarted
        .claim(&queued.id, current.revision, Some(target), true, NOW)
        .await
        .unwrap();
    assert_ne!(again.attempt, claim.attempt);
    let fences = restarted.fences().await;
    assert_eq!(fences.len(), 1);
    assert_eq!(fences[0].kind, FenceKind::InFlight);
    assert_eq!(fences[0].attempt, again.attempt);
}

#[tokio::test]
async fn a_claim_that_cannot_be_saved_does_not_happen() {
    let disk = Arc::new(FlakyDisk::default());
    let store = WorkQueueStore::with_items(Box::new(Flaky(disk.clone())), Vec::new());
    let queued = store
        .add(&QueueActor::Operator, add("hello", new_work("w1")), NOW)
        .await
        .unwrap();
    disk.fail.store(true, Ordering::SeqCst);
    assert!(matches!(
        store
            .claim(&queued.id, queued.revision, None, false, NOW)
            .await,
        Err(QueueError::Persist(_))
    ));
    assert_eq!(
        store.get(&queued.id).await.unwrap().state,
        QueueState::Waiting
    );
}

#[tokio::test]
async fn an_unsaved_outcome_freezes_claims_until_flushed() {
    let disk = Arc::new(FlakyDisk::default());
    let store = WorkQueueStore::with_items(Box::new(Flaky(disk.clone())), Vec::new());
    let a = store
        .add(&QueueActor::Operator, add("a", new_work("w1")), NOW)
        .await
        .unwrap();
    let b = store
        .add(&QueueActor::Operator, add("b", new_work("w2")), NOW)
        .await
        .unwrap();
    let claim = store
        .claim(&a.id, a.revision, None, false, NOW)
        .await
        .unwrap();
    disk.fail.store(true, Ordering::SeqCst);
    store
        .record(&a.id, &claim.attempt, NOW, FenceChange::Keep, |item| {
            item.state = QueueState::Failed {
                attempt: Some(claim.attempt.clone()),
                at: NOW,
                reason: FailReason::LaunchRefused,
                detail: "boom".into(),
            };
        })
        .await
        .unwrap();
    assert!(store.frozen().await);
    assert!(store.snapshot(true).await.unsaved);
    let b_now = store.get(&b.id).await.unwrap();
    assert!(store
        .claim(&b.id, b_now.revision, None, false, NOW)
        .await
        .is_err());
    // A restart now would find `a` still dispatching on disk: uncertain,
    // never eligible again.
    let restarted = disk.reload();
    restarted.recover(NOW).await.unwrap();
    assert!(matches!(
        restarted.get(&a.id).await.unwrap().state,
        QueueState::Uncertain { .. }
    ));

    disk.fail.store(false, Ordering::SeqCst);
    assert!(store.flush().await.unwrap());
    assert!(!store.frozen().await);
    assert!(store
        .claim(&b.id, b_now.revision, None, false, NOW)
        .await
        .is_ok());
}

#[tokio::test]
async fn cancel_during_dispatch_is_refused_and_the_attempt_finishes() {
    let store = WorkQueueStore::in_memory();
    let queued = store
        .add(&QueueActor::Operator, add("hello", new_work("w1")), NOW)
        .await
        .unwrap();
    let claim = store
        .claim(&queued.id, queued.revision, None, false, NOW)
        .await
        .unwrap();
    assert!(matches!(
        store
            .cancel(&QueueActor::Operator, &queued.id, None, NOW)
            .await,
        Err(QueueError::WrongState { .. })
    ));
    assert!(store
        .remove(&QueueActor::Operator, &queued.id, true)
        .await
        .is_err());
    let done = store
        .record(&queued.id, &claim.attempt, NOW, FenceChange::Keep, |item| {
            item.state = QueueState::Dispatched {
                attempt: claim.attempt.clone(),
                at: NOW,
                resolved: None,
                evidence: Evidence::default(),
                work: None,
                detail: None,
            };
        })
        .await
        .unwrap();
    assert_eq!(done.state.label(), "dispatched");
}

#[tokio::test]
async fn edits_and_claims_check_the_revision() {
    let store = WorkQueueStore::in_memory();
    let queued = store
        .add(&QueueActor::Operator, add("hello", to_agent("s")), NOW)
        .await
        .unwrap();
    let edited = store
        .edit(
            &QueueActor::Operator,
            &queued.id,
            Some(queued.revision),
            QueueEdit {
                prompt: Some("hello again".into()),
                ..QueueEdit::default()
            },
            NOW,
        )
        .await
        .unwrap();
    assert_eq!(edited.revision, queued.revision + 1);
    // Planned on the old revision: not claimed.
    assert!(matches!(
        store
            .claim(&queued.id, queued.revision, None, false, NOW)
            .await,
        Err(QueueError::Conflict { .. })
    ));
    assert!(matches!(
        store
            .edit(
                &QueueActor::Operator,
                &queued.id,
                Some(queued.revision),
                QueueEdit::default(),
                NOW
            )
            .await,
        Err(QueueError::Conflict { .. })
    ));
}

#[tokio::test]
async fn agents_manage_only_their_own_items() {
    let store = WorkQueueStore::in_memory();
    let mine = QueueActor::Agent {
        session_id: "me".into(),
    };
    let theirs = QueueActor::Agent {
        session_id: "other".into(),
    };
    let queued = store
        .add(&mine, add("hello", to_agent("w")), NOW)
        .await
        .unwrap();
    assert!(matches!(
        store.cancel(&theirs, &queued.id, None, NOW).await,
        Err(QueueError::Forbidden(_))
    ));
    assert!(store
        .cancel(&QueueActor::Unknown, &queued.id, None, NOW)
        .await
        .is_err());
    store.cancel(&mine, &queued.id, None, NOW).await.unwrap();
    // Clear by another agent leaves it; the person's clear removes it.
    assert_eq!(store.clear(&theirs).await.unwrap(), 0);
    assert_eq!(store.clear(&QueueActor::Operator).await.unwrap(), 1);
}

#[tokio::test]
async fn move_reorders_among_unfinished_items() {
    let store = WorkQueueStore::in_memory();
    let mut ids = Vec::new();
    for name in ["a", "b", "c"] {
        ids.push(
            store
                .add(&QueueActor::Operator, add(name, to_agent(name)), NOW)
                .await
                .unwrap()
                .id,
        );
    }
    store
        .move_to(&QueueActor::Operator, &ids[2], None, 0, NOW)
        .await
        .unwrap();
    let order: Vec<_> = store
        .snapshot(false)
        .await
        .items
        .into_iter()
        .map(|view| (view.item.prompt, view.position))
        .collect();
    assert_eq!(
        order,
        [
            ("c".to_string(), Some(0)),
            ("a".to_string(), Some(1)),
            ("b".to_string(), Some(2))
        ]
    );
}

#[tokio::test]
async fn resolve_settles_an_uncertain_attempt() {
    let disk = Arc::new(FlakyDisk::default());
    let store = WorkQueueStore::with_items(Box::new(Flaky(disk.clone())), Vec::new());
    let queued = store
        .add(&QueueActor::Operator, add("hello", to_agent("s")), NOW)
        .await
        .unwrap();
    let row = agent("s", "%1", AgentState::Idle);
    let target = ResolvedTarget {
        session_id: "s".into(),
        pane: "%1".into(),
        socket: None,
        idle_stamp: IdleStamp::of(&row),
    };
    store
        .claim(&queued.id, queued.revision, Some(target), false, NOW)
        .await
        .unwrap();
    let restarted = disk.reload();
    restarted.recover(NOW).await.unwrap();
    // "Not sent" needs the acknowledgement; "sent" does not.
    assert!(matches!(
        restarted
            .resolve(&QueueActor::Operator, &queued.id, false, false, NOW)
            .await,
        Err(QueueError::NeedsAck(_))
    ));
    let sent = restarted
        .resolve(&QueueActor::Operator, &queued.id, true, false, NOW)
        .await
        .unwrap();
    let QueueState::Dispatched { evidence, .. } = sent.state else {
        panic!("expected dispatched");
    };
    assert!(evidence.manual);
    // Marked sent, the fence is a delivered one: released by the agent's
    // next provider prompt, not before.
    let fences = restarted.fences().await;
    assert_eq!(fences[0].kind, FenceKind::Delivered);
    assert_eq!(
        restarted
            .release_delivered(std::slice::from_ref(&row))
            .await
            .unwrap(),
        0
    );
    let mut prompted = row;
    // A provider prompt reported late, for a turn before the claim: no.
    prompted.last_prompt_at = Some(NOW - time::Duration::minutes(1));
    assert_eq!(
        restarted
            .release_delivered(&[prompted.clone()])
            .await
            .unwrap(),
        0
    );
    prompted.last_prompt_at = Some(NOW + time::Duration::minutes(1));
    assert_eq!(restarted.release_delivered(&[prompted]).await.unwrap(), 1);
    assert!(restarted.fences().await.is_empty());
}

#[test]
fn an_unreadable_file_makes_the_queue_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(WORK_QUEUE_FILENAME);
    std::fs::write(
        &path,
        b"{\"version\":1,\"items\":[{\"state\":\"teleported\"}]}",
    )
    .unwrap();
    let store = WorkQueueStore::load(Some(path.clone()));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime.block_on(async {
        assert!(store.snapshot(true).await.broken.is_some());
        assert!(matches!(
            store
                .add(&QueueActor::Operator, add("x", to_agent("s")), NOW)
                .await,
            Err(QueueError::Broken(_))
        ));
    });
    // Left untouched for the person to repair.
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("teleported"));
}

#[test]
fn the_file_round_trips_through_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tend").join(WORK_QUEUE_FILENAME);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let id = runtime.block_on(async {
        let store = WorkQueueStore::load(Some(path.clone()));
        store
            .add(
                &QueueActor::Operator,
                add("persist me", new_work("w1")),
                NOW,
            )
            .await
            .unwrap()
            .id
    });
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let reopened = WorkQueueStore::load(Some(path));
    runtime.block_on(async {
        let item = reopened.get(&id).await.expect("survives a restart");
        assert_eq!(item.prompt, "persist me");
    });
}

#[test]
fn work_ids_are_slugged_and_unique() {
    let a = make_work_id("Fix the login timeout!", NOW);
    let b = make_work_id("Fix the login timeout!", NOW);
    assert!(a.starts_with("Q-FIX-THE-LOGIN-TIMEOUT-20261116-"), "{a}");
    assert_ne!(a, b);
    assert!(make_work_id("한국어만", NOW).starts_with("Q-QUEUED-"));
    for id in [&a, &b] {
        assert!(crate::pipeline_run::is_queue_work_id(id), "{id}");
    }
    for ordinary in [
        "ENG-123",
        "Q-ENG-123",
        "Q--0123456789AB",
        "Q-FIX-0123456789AB",
        "Q-FIX-2026111-0123456789AB",
        "FIX-20261116-0123456789AB",
    ] {
        assert!(
            !crate::pipeline_run::is_queue_work_id(ordinary),
            "{ordinary}"
        );
    }
}

// ---------------------------------------------------------------------------
// Fences
// ---------------------------------------------------------------------------

fn target_of(row: &Agent) -> ResolvedTarget {
    ResolvedTarget {
        session_id: row.session_id.clone(),
        pane: row.pane.clone().unwrap(),
        socket: row.tmux_socket.clone(),
        idle_stamp: IdleStamp::of(row),
    }
}

/// Send one item to `row` and record it delivered (submitted, no hook
/// confirmation), as the executor does.
async fn deliver_one(store: &WorkQueueStore, row: &Agent, prompt: &str) -> QueueItem {
    let queued = store
        .add(
            &QueueActor::Operator,
            add(prompt, to_agent(&row.session_id)),
            NOW,
        )
        .await
        .unwrap();
    let claim = store
        .claim(
            &queued.id,
            queued.revision,
            Some(target_of(row)),
            false,
            NOW,
        )
        .await
        .unwrap();
    let attempt = claim.attempt.clone();
    store
        .record(
            &queued.id,
            &claim.attempt,
            NOW,
            FenceChange::Delivered(IdleStamp::of(row), NOW),
            |item| {
                item.state = QueueState::Dispatched {
                    attempt,
                    at: NOW,
                    resolved: Some(target_of(row)),
                    evidence: Evidence {
                        typed: true,
                        submitted: true,
                        confirmed: None,
                        manual: false,
                    },
                    work: None,
                    detail: None,
                };
            },
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn fences_outlive_the_finished_history_and_clearing() {
    let store = WorkQueueStore::in_memory();
    let rows: Vec<Agent> = (0..150)
        .map(|n| agent(&format!("s{n}"), &format!("%{n}"), AgentState::Idle))
        .collect();
    for row in &rows {
        deliver_one(&store, row, &format!("to {}", row.session_id)).await;
    }
    // One more add trims the finished history to 100 items...
    store
        .add(&QueueActor::Operator, add("later", to_agent("s0")), NOW)
        .await
        .unwrap();
    let finished = store
        .snapshot(true)
        .await
        .items
        .iter()
        .filter(|view| view.item.state.is_finished())
        .count();
    assert_eq!(finished, MAX_FINISHED_ITEMS);
    // ...and every agent typed to is still held, trimmed item or not.
    assert_eq!(store.fences().await.len(), 150);
    assert_eq!(store.clear(&QueueActor::Operator).await.unwrap(), 100);
    assert_eq!(store.fences().await.len(), 150);
    let world = World::new(rows.clone());
    let fences = store.fences().await;
    let decided = plan(&store.items().await, &fences, &world.obs());
    assert_eq!(decided.waits.values().next(), Some(&WaitReason::Reserved));
}

#[tokio::test]
async fn only_a_later_provider_prompt_of_the_same_session_releases_a_delivered_fence() {
    let store = WorkQueueStore::in_memory();
    let row = agent("s", "%1", AgentState::Idle);
    deliver_one(&store, &row, "hello").await;

    // Gone from the registry, then back with the same stamp.
    assert_eq!(store.release_delivered(&[]).await.unwrap(), 0);
    assert_eq!(
        store
            .release_delivered(std::slice::from_ref(&row))
            .await
            .unwrap(),
        0
    );
    // Timing, screen or inventory repair moves the state, not the prompt.
    let mut repaired = row.clone();
    repaired.state = AgentState::Working;
    repaired.state_entered_at = NOW + time::Duration::minutes(3);
    repaired.state_source = None;
    assert_eq!(store.release_delivered(&[repaired]).await.unwrap(), 0);
    // A prompt by another session on the same pane.
    let mut neighbour = agent("other", "%1", AgentState::Working);
    neighbour.last_prompt_at = Some(NOW + time::Duration::minutes(1));
    assert_eq!(store.release_delivered(&[neighbour]).await.unwrap(), 0);
    // A stopped row of the same session.
    let mut stopped = row.clone();
    stopped.state = AgentState::Stopped;
    stopped.last_prompt_at = Some(NOW + time::Duration::minutes(1));
    assert_eq!(store.release_delivered(&[stopped]).await.unwrap(), 0);
    assert_eq!(store.fences().await.len(), 1);
    // The provider reports a prompt after the queue typed.
    let mut prompted = row;
    prompted.last_prompt_at = Some(NOW + time::Duration::seconds(1));
    assert_eq!(store.release_delivered(&[prompted]).await.unwrap(), 1);
    assert!(store.fences().await.is_empty());
}

#[tokio::test]
async fn removing_or_unsending_an_uncertain_item_needs_the_acknowledgement() {
    let store = WorkQueueStore::in_memory();
    let row = agent("s", "%1", AgentState::Idle);
    let queued = store
        .add(&QueueActor::Operator, add("hello", to_agent("s")), NOW)
        .await
        .unwrap();
    let claim = store
        .claim(
            &queued.id,
            queued.revision,
            Some(target_of(&row)),
            false,
            NOW,
        )
        .await
        .unwrap();
    let attempt = claim.attempt.clone();
    store
        .record(
            &queued.id,
            &claim.attempt,
            NOW,
            FenceChange::Unresolved,
            |item| {
                item.state = QueueState::Uncertain {
                    attempt,
                    at: NOW,
                    resolved: Some(target_of(&row)),
                    reason: "typed, Enter not confirmed".into(),
                };
            },
        )
        .await
        .unwrap();
    // Clearing finished items never touches it or its fence.
    assert_eq!(store.clear(&QueueActor::Operator).await.unwrap(), 0);
    assert!(matches!(
        store.remove(&QueueActor::Operator, &queued.id, false).await,
        Err(QueueError::NeedsAck(_))
    ));
    assert!(matches!(
        store
            .resolve(&QueueActor::Operator, &queued.id, false, false, NOW)
            .await,
        Err(QueueError::NeedsAck(_))
    ));
    assert_eq!(store.fences().await.len(), 1);
    store
        .remove(&QueueActor::Operator, &queued.id, true)
        .await
        .unwrap();
    assert!(store.fences().await.is_empty());
}

#[test]
fn only_the_two_pre_launch_refusals_are_clean_launch_failures() {
    assert!(launch_refusal("work_exists_before_start: Work X already exists").is_some());
    assert!(launch_refusal("1: work_exists: Work ws/X already has a Run").is_some());
    assert!(
        launch_refusal("tend work up failed: work_exists: Work ws/X already has a Run").is_some()
    );
    assert!(launch_refusal("tend work up failed: 0: work_exists_before_start: Work X").is_some());
    assert!(launch_refusal("  work_exists: Work ws/X already has a window").is_some());
    for other in [
        "work_reserved: Work X is being started by the Work queue",
        "Work ws/X gained a window from elsewhere while it was being started",
        "launch codex: tmux split-window failed",
        "tend work up exceeded 600s",
        "something work_exists: in the middle",
        "tend work up failed: launch codex: work_exists: later in the line",
        "",
    ] {
        assert!(launch_refusal(other).is_none(), "{other}");
    }
}

/// Lease changes ride on the queue write that caused them and outlive the
/// item: clearing, trimming and a restart keep them until applied.
#[tokio::test]
async fn lease_intents_are_written_with_outcomes_and_outlive_the_item() {
    use crate::pipeline_run::QueueLeaseState;
    let disk = Arc::new(FlakyDisk::default());
    let store = WorkQueueStore::with_items(Box::new(Flaky(disk.clone())), Vec::new());
    let queued = store
        .add(&QueueActor::Operator, add("start it", new_work("w1")), NOW)
        .await
        .unwrap();
    let claim = store
        .claim(&queued.id, queued.revision, None, false, NOW)
        .await
        .unwrap();
    assert!(
        store.lease_intents().await.is_empty(),
        "none while launching"
    );
    let attempt = claim.attempt.clone();
    store
        .record(&queued.id, &claim.attempt, NOW, FenceChange::Keep, |item| {
            item.state = QueueState::Dispatched {
                attempt,
                at: NOW,
                resolved: None,
                evidence: Evidence::default(),
                work: None,
                detail: None,
            };
        })
        .await
        .unwrap();
    let intents = store.lease_intents().await;
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0].state, QueueLeaseState::Released);
    assert_eq!(intents[0].holder.as_deref(), Some(claim.attempt.as_str()));
    assert_eq!(store.clear(&QueueActor::Operator).await.unwrap(), 1);
    let restarted = disk.reload();
    assert_eq!(restarted.lease_intents().await, intents);
    restarted.lease_applied(&intents[0]).await.unwrap();
    assert!(restarted.lease_intents().await.is_empty());
}

#[tokio::test]
async fn a_stale_remove_cannot_touch_a_newer_attempt() {
    use crate::pipeline_run::QueueLeaseState;
    let disk = Arc::new(FlakyDisk::default());
    let store = WorkQueueStore::with_items(Box::new(Flaky(disk.clone())), Vec::new());
    let queued = store
        .add(&QueueActor::Operator, add("start it", new_work("w1")), NOW)
        .await
        .unwrap();
    let first = store
        .claim(&queued.id, queued.revision, None, false, NOW)
        .await
        .unwrap();
    // A restart: uncertain, its lease to be suspended for that attempt.
    let store = disk.reload();
    store.recover(NOW).await.unwrap();
    let intents = store.lease_intents().await;
    assert_eq!(intents[0].state, QueueLeaseState::Suspended);
    assert_eq!(intents[0].holder.as_deref(), Some(first.attempt.as_str()));
    // run-now with the duplicate accepted takes it again...
    let current = store.get(&queued.id).await.unwrap();
    let second = store
        .claim(&queued.id, current.revision, None, true, NOW)
        .await
        .unwrap();
    assert_ne!(second.attempt, first.attempt);
    // ...so a remove made from the old (uncertain) view is refused, with
    // nothing written for the lease.
    assert!(matches!(
        store.remove(&QueueActor::Operator, &queued.id, true).await,
        Err(QueueError::WrongState { .. })
    ));
    assert_eq!(store.lease_intents().await, intents);
}

#[tokio::test]
async fn cancelling_or_removing_a_queued_work_releases_its_lease() {
    use crate::pipeline_run::QueueLeaseState;
    let store = WorkQueueStore::in_memory();
    let queued = store
        .add(&QueueActor::Operator, add("start it", new_work("w1")), NOW)
        .await
        .unwrap();
    store
        .cancel(&QueueActor::Operator, &queued.id, None, NOW)
        .await
        .unwrap();
    let intents = store.lease_intents().await;
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0].state, QueueLeaseState::Released);
    assert_eq!(intents[0].holder, None);
}

#[test]
fn work_paths_are_normalised_like_work_up() {
    assert_eq!(
        QueueTargetSpec::work_path("  Web / eng-1 @Review ").unwrap(),
        QueueTargetSpec::Work {
            workspace: "web".into(),
            work: "ENG-1".into(),
            alias: Some("Review".into()),
        }
    );
    for bad in ["ENG-1", "web/", "/ENG-1", "web/ENG-1@", "  /  "] {
        assert!(QueueTargetSpec::work_path(bad).is_err(), "{bad}");
    }
    assert_eq!(
        normalized_identity(" Web ", " eng-1 "),
        crate::work::WorkIdentity::new("web", "ENG-1")
    );
}
