//! The subject check: evidence, episodes, Keep and Close all. Fixtures are
//! shaped like the items measured on the owner's hosts
//! (`docs/proposals/STALE_QUESTIONS.md`); nothing here touches a real
//! daemon.
use super::*;
use crate::backend::HostKind;
use crate::collaboration::{CollaborationOptions, HumanAction, NewRequest, RequestKind};
use crate::event::{AgentEvent, AgentId};
use crate::provenance::StateSource;
use crate::subject_evidence::{
    HostScan, PaneScan, ScannedPane, ServerAnswer, ServerIncarnation, SubjectBinding,
};
use time::Duration;

const A: ServerIncarnation = ServerIncarnation {
    pid: 100,
    start_time: 1_000,
};
const B: ServerIncarnation = ServerIncarnation {
    pid: 200,
    start_time: 2_000,
};

fn participant(session: &str, pane: &str, socket: &str) -> Participant {
    serde_json::from_value(serde_json::json!({
        "launch_purpose": "worker", "agent_kind": "claude_code", "agent_session_id": session,
        "pane": pane, "socket": socket,
        "room": {"host": "tmux", "socket": socket, "window_id": "@1"}, "state": "idle"
    }))
    .unwrap()
}

/// A hooked row for `session` in `pane`, then put in `state`, last active
/// `idle` ago.
async fn row(session: &str, pane: &str, socket: &str, state: AgentState, idle: Duration) -> Agent {
    let store = crate::state::Store::shared();
    store
        .apply(&AgentEvent::Started {
            id: AgentId {
                kind: AgentKind::ClaudeCode,
                session_id: session.into(),
                surface: None,
                pane: Some(pane.into()),
                tmux_socket: Some(socket.into()),
                cwd: None,
            },
            at: OffsetDateTime::now_utc(),
        })
        .await;
    let mut agent = store.snapshot().await.remove(0);
    agent.state = state;
    agent.last_activity_at = OffsetDateTime::now_utc() - idle;
    agent
}

struct Pane<'a> {
    id: &'a str,
    endpoint: &'a str,
    foreground: &'a str,
    agent: Option<bool>,
}

fn pane<'a>(id: &'a str, endpoint: &'a str, foreground: &'a str, agent: Option<bool>) -> Pane<'a> {
    Pane {
        id,
        endpoint,
        foreground,
        agent,
    }
}

/// A complete tmux scan at `at` where `servers` answered, from a pass that
/// did not walk the panes' processes: only the bound incarnation counts.
fn scan(
    at: OffsetDateTime,
    servers: &[(&str, Option<ServerIncarnation>)],
    panes: &[Pane<'_>],
) -> PaneScan {
    PaneScan {
        at,
        processes_read: false,
        outside: Vec::new(),
        outside_resumed: Vec::new(),
        hosts: vec![HostScan {
            kind: HostKind::Tmux,
            complete: true,
            servers: servers
                .iter()
                .map(|(endpoint, incarnation)| ServerAnswer {
                    endpoint: (*endpoint).into(),
                    running: true,
                    incarnation: *incarnation,
                })
                .collect(),
            panes: panes
                .iter()
                .map(|p| ScannedPane {
                    pane_id: p.id.into(),
                    endpoint: Some(p.endpoint.into()),
                    foreground: p.foreground.into(),
                    agent_process: p.agent,
                    resumed: Vec::new(),
                    unnamed: Vec::new(),
                })
                .collect(),
        }],
    }
}

fn bound(
    session: &str,
    pane: &str,
    socket: &str,
    incarnation: ServerIncarnation,
) -> HashMap<String, SubjectBinding> {
    HashMap::from([(
        session.to_owned(),
        SubjectBinding {
            pane: pane.into(),
            endpoint: socket.into(),
            incarnation: Some(incarnation),
        },
    )])
}

async fn question(store: &CollaborationStore, from: &Participant) -> CollaborationRequest {
    let request = store
        .create(
            from.clone(),
            Participant::console(from.room.clone()),
            NewRequest {
                kind: RequestKind::Question,
                human_action: Some(HumanAction::Choice),
                body: "deploy?".into(),
                ..NewRequest::default()
            },
        )
        .await
        .unwrap();
    backdate(store, &request.id, Duration::days(8)).await;
    request
}

async fn get(store: &CollaborationStore, id: &str) -> CollaborationRequest {
    store.requests.read().await.get(id).cloned().unwrap()
}

async fn put(store: &CollaborationStore, row: CollaborationRequest) {
    store.commit_request_rows(&[row]).await.unwrap();
}

async fn backdate(store: &CollaborationStore, id: &str, by: Duration) {
    let mut aged = get(store, id).await;
    aged.created_at -= by;
    put(store, aged).await;
}

/// Run passes every five minutes from `hours` ago up to now, with the scan
/// `scan_at` gives for each pass.
async fn drive(
    store: &CollaborationStore,
    agents: &[Agent],
    bindings: &HashMap<String, SubjectBinding>,
    hours: i64,
    scan_at: impl Fn(OffsetDateTime) -> Option<PaneScan>,
) {
    drive_for(store, agents, bindings, Duration::hours(hours), scan_at).await;
}

/// [`drive`] over `window`.
async fn drive_for(
    store: &CollaborationStore,
    agents: &[Agent],
    bindings: &HashMap<String, SubjectBinding>,
    window: Duration,
    scan_at: impl Fn(OffsetDateTime) -> Option<PaneScan>,
) {
    let now = OffsetDateTime::now_utc();
    let mut at = now - window;
    while at <= now {
        let scan = scan_at(at);
        let inputs = SubjectInputs {
            agents,
            scan: scan.as_ref(),
            bindings,
        };
        store.check_subjects_at(&inputs, at).await.unwrap();
        at += Duration::minutes(5);
    }
}

async fn pass(
    store: &CollaborationStore,
    agents: &[Agent],
    bindings: &HashMap<String, SubjectBinding>,
    scan: Option<&PaneScan>,
) {
    let inputs = SubjectInputs {
        agents,
        scan,
        bindings,
    };
    store.check_subjects(&inputs).await.unwrap();
}

async fn pass_at(
    store: &CollaborationStore,
    agents: &[Agent],
    bindings: &HashMap<String, SubjectBinding>,
    scan: Option<&PaneScan>,
    at: OffsetDateTime,
) {
    let inputs = SubjectInputs {
        agents,
        scan,
        bindings,
    };
    store.check_subjects_at(&inputs, at).await.unwrap();
}

async fn folded(store: &CollaborationStore, id: &str) -> Option<Unanswerable> {
    store.unanswerable(&get(store, id).await, OffsetDateTime::now_utc())
}

/// Every fixture session bound on server A of `default`, as tendd's scans
/// would have stamped them while they were live.
fn everyone() -> HashMap<String, SubjectBinding> {
    [
        ("otpeek", "%42"),
        ("s5", "%5"),
        ("capped", "%6"),
        ("remote", "%1"),
        ("s7", "%7"),
        ("s8", "%8"),
        ("s9", "%9"),
        ("t1", "%9"),
        ("peer", "%2"),
        ("a", "%11"),
        ("b", "%12"),
        ("c", "%13"),
        ("d", "%14"),
        ("e", "%15"),
        ("ga", "%31"),
        ("gb", "%32"),
        ("live", "%33"),
        ("gone-sender", "%40"),
    ]
    .into_iter()
    .flat_map(|(session, pane)| bound(session, pane, "default", A))
    .collect()
}

/// The agent registry as Close all reads it inside its transaction: these
/// sessions live, every fixture session bound on A, and a fresh scan of
/// `default` on A with no panes.
async fn registry(live: &[(&str, &str)]) -> crate::state::Store {
    let registry = crate::state::Store::default();
    for (session, pane) in live {
        registry
            .apply(&AgentEvent::Started {
                id: AgentId {
                    kind: AgentKind::ClaudeCode,
                    session_id: (*session).into(),
                    surface: None,
                    pane: Some((*pane).into()),
                    tmux_socket: Some("default".into()),
                    cwd: None,
                },
                at: OffsetDateTime::now_utc(),
            })
            .await;
    }
    let observations = registry.subject_observations();
    observations.stamp_bindings(
        everyone().into_iter().collect(),
        &std::collections::HashSet::new(),
    );
    observations.set_scan(scan(
        OffsetDateTime::now_utc(),
        &[("default", Some(A))],
        &[],
    ));
    registry
}

#[allow(clippy::unnecessary_wraps)] // the shape `drive` takes
fn gone_scan(at: OffsetDateTime) -> Option<PaneScan> {
    Some(scan(at, &[("default", Some(A))], &[]))
}

#[tokio::test]
async fn an_agents_question_whose_pane_is_gone_folds_after_the_grace_and_is_never_closed() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let asker = participant("otpeek", "%42", "default");
    let q = question(&store, &asker).await;
    // Ten minutes of pane gone: listed, not folded.
    drive_for(&store, &[], &everyone(), Duration::minutes(10), gone_scan).await;
    assert!(folded(&store, &q.id).await.is_none());
    let check = get(&store, &q.id).await.subject_check.unwrap();
    assert_eq!(check.state, SubjectState::Gone);
    assert_eq!(check.causes, [GoneCause::PaneGone]);
    // The grace of it.
    let mut aged = get(&store, &q.id).await;
    let check = aged.subject_check.as_mut().unwrap();
    check.gone_since = check.gone_since.map(|at| at - Duration::minutes(6));
    put(&store, aged).await;
    pass(
        &store,
        &[],
        &everyone(),
        gone_scan(OffsetDateTime::now_utc()).as_ref(),
    )
    .await;
    let gone = folded(&store, &q.id).await.expect("folded");
    assert_eq!(gone.cause, GoneCause::PaneGone);
    assert_eq!(gone.pane.as_deref(), Some("%42"));
    // Still open: nothing closes an agent's question without a person.
    assert!(get(&store, &q.id).await.needs_human_response());
    let page = store.open_human_requests_folded(10).await.unwrap();
    assert_eq!(page.unanswerable_count, 1);
    assert!(page.folded.contains_key(&q.id));
}

#[tokio::test]
async fn a_full_day_of_five_minute_passes_folds_without_a_gap() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("otpeek", "%42", "default")).await;
    drive(&store, &[], &everyone(), 25, gone_scan).await;
    let gone = folded(&store, &q.id).await.expect("folded after 25h");
    assert_eq!(gone.cause, GoneCause::PaneGone);
}

#[tokio::test]
async fn a_pane_now_running_a_shell_is_agent_exited_but_a_wrapper_is_unknown() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let asker = participant("s3", "%3", "default");
    let q = question(&store, &asker).await;
    let stopped = row(
        "s3",
        "%3",
        "default",
        AgentState::Stopped,
        Duration::hours(2),
    )
    .await;
    let bindings = bound("s3", "%3", "default", A);
    let now = OffsetDateTime::now_utc();
    let shell = scan(
        now,
        &[("default", Some(A))],
        &[pane("%3", "default", "zsh", Some(false))],
    );
    pass(
        &store,
        std::slice::from_ref(&stopped),
        &bindings,
        Some(&shell),
    )
    .await;
    let check = get(&store, &q.id).await.subject_check.unwrap();
    assert_eq!(check.causes, [GoneCause::AgentExited]);

    let other = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&other, &asker).await;
    let wrapper = scan(
        now,
        &[("default", Some(A))],
        &[pane("%3", "default", "node", Some(false))],
    );
    pass(
        &other,
        std::slice::from_ref(&stopped),
        &bindings,
        Some(&wrapper),
    )
    .await;
    assert!(
        get(&other, &q.id).await.subject_check.is_none(),
        "a wrapper is unknown"
    );
    // Inside the inactivity floor, a shell is unknown too.
    let fresh = row(
        "s3",
        "%3",
        "default",
        AgentState::Stopped,
        Duration::minutes(5),
    )
    .await;
    pass(&other, &[fresh], &bindings, Some(&shell)).await;
    assert!(get(&other, &q.id).await.subject_check.is_none());
}

#[tokio::test]
async fn a_tmux_server_restart_that_reuses_the_pane_id_is_unknown() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("s3", "%3", "default")).await;
    let stopped = row(
        "s3",
        "%3",
        "default",
        AgentState::Stopped,
        Duration::hours(2),
    )
    .await;
    let bindings = bound("s3", "%3", "default", A);
    let now = OffsetDateTime::now_utc();
    // New server B, a new pane %3 running a shell.
    let reused = scan(
        now,
        &[("default", Some(B))],
        &[pane("%3", "default", "zsh", Some(false))],
    );
    pass(
        &store,
        std::slice::from_ref(&stopped),
        &bindings,
        Some(&reused),
    )
    .await;
    assert!(get(&store, &q.id).await.subject_check.is_none());
    // New server B, no %3: the absence says nothing about A's pane.
    let absent = scan(now, &[("default", Some(B))], &[]);
    pass(
        &store,
        std::slice::from_ref(&stopped),
        &bindings,
        Some(&absent),
    )
    .await;
    assert!(get(&store, &q.id).await.subject_check.is_none());
}

#[tokio::test]
async fn the_same_pane_id_on_two_sockets_is_told_apart() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("s3", "%3", "work")).await;
    let now = OffsetDateTime::now_utc();
    // %3 lives on `other`; `work` answered without it.
    let other_only = scan(
        now,
        &[("work", Some(A)), ("other", Some(B))],
        &[pane("%3", "other", "claude", Some(true))],
    );
    pass(
        &store,
        &[],
        &bound("s3", "%3", "work", A),
        Some(&other_only),
    )
    .await;
    let check = get(&store, &q.id).await.subject_check.unwrap();
    assert_eq!(check.causes, [GoneCause::PaneGone]);

    // E4 needs the pane id to be unique: a workload keyed by pane id
    // cannot say which of two %3 it walked.
    let second = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&second, &participant("s3", "%3", "work")).await;
    let stopped = row("s3", "%3", "work", AgentState::Stopped, Duration::hours(2)).await;
    let both = scan(
        now,
        &[("work", Some(A)), ("other", Some(B))],
        &[
            pane("%3", "work", "zsh", Some(false)),
            pane("%3", "other", "zsh", Some(false)),
        ],
    );
    pass(
        &second,
        &[stopped],
        &bound("s3", "%3", "work", A),
        Some(&both),
    )
    .await;
    assert!(get(&second, &q.id).await.subject_check.is_none());
}

#[tokio::test]
async fn a_partial_herdr_scan_or_a_failed_socket_never_shows_absence() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let herdr = question(
        &store,
        &participant(
            "h1",
            &format!("{}7", crate::backend::herdr::PANE_ID_PREFIX),
            "herdr",
        ),
    )
    .await;
    let tmux = question(&store, &participant("t1", "%9", "default")).await;
    let now = OffsetDateTime::now_utc();
    let mut partial = scan(now, &[("default", Some(A))], &[]);
    partial.hosts[0].complete = false;
    partial.hosts.push(HostScan {
        kind: HostKind::Herdr,
        complete: false,
        servers: vec![],
        panes: vec![],
    });
    pass(&store, &[], &everyone(), Some(&partial)).await;
    assert!(get(&store, &herdr.id).await.subject_check.is_none());
    assert!(
        get(&store, &tmux.id).await.subject_check.is_none(),
        "an incomplete scan"
    );
    // A complete scan in which the subject's socket did not answer.
    let elsewhere = scan(now, &[("other", Some(A))], &[]);
    pass(&store, &[], &everyone(), Some(&elsewhere)).await;
    assert!(get(&store, &tmux.id).await.subject_check.is_none());
    // A stale scan shows nothing either.
    let stale = scan(now - Duration::hours(1), &[("default", Some(A))], &[]);
    pass(&store, &[], &everyone(), Some(&stale)).await;
    assert!(get(&store, &tmux.id).await.subject_check.is_none());
}

#[tokio::test]
async fn a_live_session_anywhere_wins_after_a_resume_or_restore() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("s5", "%5", "default")).await;
    let now = OffsetDateTime::now_utc();
    // Its old pane is gone, and the session ended once.
    store
        .record_subject_facts(vec![SubjectFact::new(
            FactKind::Ended,
            "s5",
            Some("%5"),
            Some("default"),
            None,
            now - Duration::hours(3),
        )])
        .unwrap();
    pass(&store, &[], &everyone(), gone_scan(now).as_ref()).await;
    assert!(get(&store, &q.id).await.subject_check.is_some());
    // Resumed into a new pane (or `tend restore`): alive, episode over.
    let resumed = row(
        "s5",
        "%8",
        "default",
        AgentState::Idle,
        Duration::minutes(1),
    )
    .await;
    pass(
        &store,
        std::slice::from_ref(&resumed),
        &everyone(),
        gone_scan(now).as_ref(),
    )
    .await;
    assert!(get(&store, &q.id).await.subject_check.is_none());
    // The resumed run stops later without a hook: the end before the
    // resume says nothing about it, so only a scan can show it gone.
    let mut stopped = resumed;
    stopped.state = AgentState::Stopped;
    let alive_pane = scan(
        now,
        &[("default", Some(A))],
        &[pane("%8", "default", "claude", Some(true))],
    );
    pass(&store, &[stopped], &everyone(), Some(&alive_pane)).await;
    assert!(get(&store, &q.id).await.subject_check.is_none());
}

#[tokio::test]
async fn a_rate_limited_agent_is_alive() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("capped", "%6", "default")).await;
    let mut capped = row(
        "capped",
        "%6",
        "default",
        AgentState::Error,
        Duration::days(3),
    )
    .await;
    capped.rate_limit_scope = Some(crate::event::RateLimitScope::FiveHour);
    drive(&store, &[capped], &everyone(), 25, gone_scan).await;
    assert!(get(&store, &q.id).await.subject_check.is_none());
}

#[tokio::test]
async fn an_asker_on_another_host_is_never_judged() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let asker = participant("remote", "%1", "default");
    let request = store
        .create(
            asker.clone(),
            Participant::console(asker.room.clone()),
            NewRequest {
                kind: RequestKind::Question,
                human_action: Some(HumanAction::Choice),
                body: "from jiun-mini".into(),
                source_node_id: Some(crate::fleet::NodeId::generate()),
                ..NewRequest::default()
            },
        )
        .await
        .unwrap();
    backdate(&store, &request.id, Duration::days(8)).await;
    drive(&store, &[], &everyone(), 25, gone_scan).await;
    assert!(get(&store, &request.id).await.subject_check.is_none());
}

#[tokio::test]
async fn a_daemon_down_a_day_restarts_the_grace_clock_not_the_episode() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("s7", "%7", "default")).await;
    let now = OffsetDateTime::now_utc();
    // As the row was left by a daemon that stopped 24 hours ago, kept.
    let mut left = get(&store, &q.id).await;
    left.subject_check = Some(SubjectCheck {
        episode: "ep_before".into(),
        state: SubjectState::Gone,
        gone_since: Some(now - Duration::hours(50)),
        episode_since: Some(now - Duration::hours(50)),
        confirmed_at: now - Duration::hours(24),
        causes: vec![GoneCause::PaneGone],
        kept_episode: Some("ep_before".into()),
        pane: Some("%7".into()),
    });
    put(&store, left).await;
    // The first pass after start cannot fold anything.
    pass(&store, &[], &everyone(), gone_scan(now).as_ref()).await;
    let check = get(&store, &q.id).await.subject_check.unwrap();
    assert_eq!(check.episode, "ep_before", "the episode survives an outage");
    assert_eq!(
        check.kept_episode.as_deref(),
        Some("ep_before"),
        "so does Keep"
    );
    assert!(
        check.gone_since.unwrap() > now - Duration::minutes(1),
        "the grace restarts"
    );
    assert_eq!(
        check.episode_since,
        Some(now - Duration::hours(50)),
        "but not when the episode began"
    );
    assert!(folded(&store, &q.id).await.is_none());
}

/// Measured on jiun-mini (2026-11-25): a question 23 days old whose
/// asker's pane was gone since 2026-11-01 read "pane gone since
/// 2026-11-24", because every daemon restart (each upgrade) restarted the
/// clock the person reads.
#[tokio::test]
async fn restart_gaps_never_move_when_the_episode_began() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("s7", "%7", "default")).await;
    backdate(&store, &q.id, Duration::days(15)).await;
    let start = OffsetDateTime::now_utc() - Duration::days(20);
    pass_at(&store, &[], &everyone(), gone_scan(start).as_ref(), start).await;
    let began = get(&store, &q.id).await.subject_check.unwrap();
    assert_eq!(began.episode_since, Some(start));
    // Five upgrades a few days apart: each a gap of an hour with no pass.
    for day in [3, 7, 11, 15, 19] {
        let at = start + Duration::days(day);
        pass_at(&store, &[], &everyone(), gone_scan(at).as_ref(), at).await;
        let check = get(&store, &q.id).await.subject_check.unwrap();
        assert_eq!(check.episode, began.episode, "same episode");
        assert_eq!(check.gone_since, Some(at), "only the grace clock restarts");
        assert_eq!(check.episode_since, Some(start), "day {day}");
    }
    // An unknown stretch keeps it too.
    let at = start + Duration::days(19) + Duration::hours(2);
    pass_at(&store, &[], &everyone(), None, at).await;
    let check = get(&store, &q.id).await.subject_check.unwrap();
    assert_eq!(check.state, SubjectState::Unknown);
    assert_eq!(check.episode_since, Some(start));
    // Seen alive ends it.
    let alive = row("s7", "%7", "default", AgentState::Idle, Duration::ZERO).await;
    pass(&store, &[alive], &everyone(), None).await;
    assert!(get(&store, &q.id).await.subject_check.is_none());
}

/// A row written before `episode_since` takes the subject's last sign of
/// life, never before the question was asked.
#[tokio::test]
async fn a_legacy_row_takes_the_subjects_last_sign_of_life() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("s7", "%7", "default")).await;
    backdate(&store, &q.id, Duration::days(23)).await;
    let now = OffsetDateTime::now_utc();
    let mut left = get(&store, &q.id).await;
    left.subject_check = Some(SubjectCheck {
        episode: "ep_legacy".into(),
        state: SubjectState::Gone,
        gone_since: Some(now - Duration::hours(20)),
        episode_since: None,
        confirmed_at: now - Duration::hours(20),
        causes: vec![GoneCause::PaneGone],
        kept_episode: None,
        pane: Some("%7".into()),
    });
    put(&store, left.clone()).await;
    // Its row, stopped, last active 21 days ago.
    let stopped = row(
        "s7",
        "%7",
        "default",
        AgentState::Stopped,
        Duration::days(21),
    )
    .await;
    let seen = stopped.last_activity_at;
    pass(&store, &[stopped], &everyone(), gone_scan(now).as_ref()).await;
    let check = get(&store, &q.id).await.subject_check.unwrap();
    assert_eq!(check.episode, "ep_legacy");
    assert_eq!(check.episode_since, Some(seen));
    // No sign of life left: its own clock, the oldest it has.
    put(&store, left).await;
    pass(&store, &[], &everyone(), gone_scan(now).as_ref()).await;
    let check = get(&store, &q.id).await.subject_check.unwrap();
    assert_eq!(check.episode_since, Some(now - Duration::hours(20)));
}

#[tokio::test]
async fn keep_holds_through_unknown_and_a_change_of_cause_until_alive() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("s8", "%8", "default")).await;
    drive(&store, &[], &everyone(), 25, gone_scan).await;
    let gone = folded(&store, &q.id).await.expect("folded");
    let kept = store.keep_unanswerable(&q.id, &gone.episode).await.unwrap();
    assert_eq!(kept.episode, gone.episode);
    assert!(
        folded(&store, &q.id).await.is_none(),
        "kept: back in the person's turn"
    );
    // A day of unknown (failed scans), then gone again with another cause.
    drive(&store, &[], &everyone(), 24, |_| None).await;
    store
        .record_subject_facts(vec![SubjectFact::new(
            FactKind::Ended,
            "s8",
            Some("%8"),
            Some("default"),
            None,
            OffsetDateTime::now_utc(),
        )])
        .unwrap();
    drive(&store, &[], &everyone(), 25, gone_scan).await;
    let check = get(&store, &q.id).await.subject_check.unwrap();
    assert_eq!(
        check.episode, gone.episode,
        "no alive in between: same episode"
    );
    assert_eq!(check.causes, [GoneCause::Ended, GoneCause::PaneGone]);
    assert!(folded(&store, &q.id).await.is_none(), "still kept");
    // Seen alive, then gone again: a new episode Keep does not cover.
    let alive = row("s8", "%8", "default", AgentState::Idle, Duration::ZERO).await;
    pass(&store, &[alive], &everyone(), None).await;
    assert!(get(&store, &q.id).await.subject_check.is_none());
    store
        .record_subject_facts(vec![SubjectFact::new(
            FactKind::Ended,
            "s8",
            Some("%8"),
            None,
            None,
            OffsetDateTime::now_utc() + Duration::seconds(1),
        )])
        .unwrap();
    pass(&store, &[], &everyone(), None).await;
    let check = get(&store, &q.id).await.subject_check.unwrap();
    assert_ne!(check.episode, gone.episode);
    assert!(check.kept_episode.is_none());
    // Keep of an ended episode is refused.
    assert!(store.keep_unanswerable(&q.id, &gone.episode).await.is_err());
}

#[test]
fn the_state_machine_steps() {
    let now = OffsetDateTime::now_utc();
    let gone = Verdict::Gone {
        causes: vec![GoneCause::PaneGone],
        pane: Some("%1".into()),
    };
    let first = step(None, &gone, now, None, None).unwrap();
    assert_eq!(first.gone_since, Some(now));
    assert!(step(None, &Verdict::Unknown, now, None, None).is_none());
    assert!(step(Some(&first), &Verdict::Alive, now, Some(now), None).is_none());
    // A short unknown keeps the grace clock; a long one restarts only it.
    let later = now + Duration::minutes(5);
    let paused = step(Some(&first), &Verdict::Unknown, later, Some(now), None).unwrap();
    assert_eq!(paused.gone_since, Some(now));
    let much_later = now + Duration::minutes(11);
    let gap = step(Some(&first), &Verdict::Unknown, much_later, Some(now), None).unwrap();
    assert_eq!(gap.gone_since, None);
    assert_eq!(gap.episode, first.episode);
    let again = step(Some(&gap), &gone, much_later, Some(now), None).unwrap();
    assert_eq!(again.gone_since, Some(much_later));
    assert_eq!(again.episode, first.episode);
    assert_eq!(first.episode_since, Some(now));
    assert_eq!(gap.episode_since, Some(now), "a gap keeps when it began");
    assert_eq!(again.episode_since, Some(now));
}

/// The two E1/E2 facts are written at ingestion, from real hooks only.
#[tokio::test]
async fn ended_and_replaced_facts_come_from_real_hooks_only() {
    let registry = crate::state::Store::shared();
    let id = |session: &str, pane: &str| AgentId {
        kind: AgentKind::ClaudeCode,
        session_id: session.into(),
        surface: None,
        pane: Some(pane.into()),
        tmux_socket: Some("/private/tmp/tmux-501/default".into()),
        cwd: None,
    };
    let now = OffsetDateTime::now_utc();
    registry
        .apply(&AgentEvent::Started {
            id: id("old", "%4"),
            at: now,
        })
        .await;
    // The scan that binds `old` to %4 on server A.
    registry
        .record_pane_scan(scan(
            now,
            &[("default", Some(A))],
            &[pane("%4", "default", "claude", Some(true))],
        ))
        .await;
    assert_eq!(
        registry
            .subject_observations()
            .binding("old")
            .unwrap()
            .incarnation,
        Some(A)
    );
    // A synthetic start or a sweep's end writes nothing.
    registry
        .apply_observed(
            &AgentEvent::SessionEnded {
                id: id("old", "%4"),
                at: now,
            },
            StateSource::PaneInventory,
        )
        .await;
    assert!(registry.subject_observations().take_facts().is_empty());
    // A real Started of another session in the pane: E2, same incarnation.
    registry
        .apply(&AgentEvent::Started {
            id: id("new", "%4"),
            at: now,
        })
        .await;
    // Settled by the first scan after the hook, still on server A.
    registry
        .record_pane_scan(scan(
            OffsetDateTime::now_utc() + Duration::seconds(1),
            &[("default", Some(A))],
            &[pane("%4", "default", "claude", Some(true))],
        ))
        .await;
    let facts = registry.subject_observations().take_facts();
    assert_eq!(facts.len(), 1, "{facts:?}");
    assert_eq!(facts[0].kind, FactKind::Replaced);
    assert_eq!(facts[0].session_id, "old");
    assert!(facts[0].same_incarnation);
    assert_eq!(facts[0].replaced_by.as_deref(), Some("new"));
    assert_eq!(facts[0].endpoint.as_deref(), Some("default"));
    // The provider's own end hook: E1.
    registry
        .apply(&AgentEvent::SessionEnded {
            id: id("new", "%4"),
            at: now,
        })
        .await;
    let facts = registry.subject_observations().take_facts();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].kind, FactKind::Ended);
    assert_eq!(facts[0].session_id, "new");

    // The replaced subject folds on E2 alone.
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("old", "%4", "default")).await;
    let mut replaced = SubjectFact::new(
        FactKind::Replaced,
        "old",
        Some("%4"),
        Some("default"),
        Some(A),
        now,
    );
    replaced.same_incarnation = true;
    store.record_subject_facts(vec![replaced]).unwrap();
    let agents = registry.snapshot().await;
    pass(&store, &agents, &everyone(), None).await;
    let check = get(&store, &q.id).await.subject_check.unwrap();
    assert_eq!(check.causes, [GoneCause::Replaced]);
}

#[tokio::test]
async fn a_replacement_on_another_server_incarnation_never_counts() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("old", "%4", "default")).await;
    store
        .record_subject_facts(vec![SubjectFact::new(
            FactKind::Replaced,
            "old",
            Some("%4"),
            Some("default"),
            Some(A),
            OffsetDateTime::now_utc(),
        )])
        .unwrap();
    pass(&store, &[], &everyone(), None).await;
    assert!(get(&store, &q.id).await.subject_check.is_none());
}

#[tokio::test]
async fn facts_survive_a_restart_and_expire() {
    let dir = tempfile::tempdir().unwrap();
    let options = CollaborationOptions {
        path: Some(dir.path().join("mailbox.db")),
        ..CollaborationOptions::default()
    };
    let store = CollaborationStore::load(options.clone()).await.unwrap();
    let now = OffsetDateTime::now_utc();
    store
        .record_subject_facts(vec![
            SubjectFact::new(
                FactKind::Ended,
                "fresh",
                Some("%1"),
                Some("default"),
                None,
                now,
            ),
            SubjectFact::new(
                FactKind::Ended,
                "old",
                Some("%2"),
                Some("default"),
                None,
                now - Duration::days(31),
            ),
        ])
        .unwrap();
    drop(store);
    let reloaded = CollaborationStore::load(options).await.unwrap();
    let sessions: Vec<_> = reloaded
        .subject_facts()
        .into_iter()
        .map(|fact| fact.session_id)
        .collect();
    assert_eq!(sessions, ["fresh"]);
}

/// A recovery item for a request the person sent from the console, its
/// recipient's binding unavailable for `days`.
async fn console_recovery(
    store: &CollaborationStore,
    days: i64,
) -> (CollaborationRequest, CollaborationRequest) {
    let recipient = participant("peer", "%2", "default");
    let console = Participant::console(recipient.room.clone());
    let parent = store
        .create(
            console,
            recipient.clone(),
            NewRequest {
                body: "work".into(),
                ..NewRequest::default()
            },
        )
        .await
        .unwrap();
    store.claim_by_id(&recipient, &parent.id).await.unwrap();
    let mut errored = row("peer", "%2", "default", AgentState::Error, Duration::ZERO).await;
    errored.rate_limit_scope = None;
    store
        .reconcile_peer_interruptions(&[errored])
        .await
        .unwrap();
    // As measured on jiun-mbp: the recipient's binding unavailable, there
    // for 11 to 20 days.
    let mut parent = get(store, &parent.id).await;
    let info = parent.interruption.as_mut().unwrap();
    info.reason = "binding_unavailable".into();
    info.observed_at -= Duration::days(days);
    let item_id = info.action_request_id.clone().unwrap();
    put(store, parent.clone()).await;
    backdate(store, &item_id, Duration::days(days)).await;
    store.reconcile_peer_interruptions(&[]).await.unwrap();
    (get(store, &parent.id).await, get(store, &item_id).await)
}

#[tokio::test]
async fn a_console_recovery_with_only_binding_unavailable_does_not_fold_and_closes_after_a_week() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let (parent, item) = Box::pin(console_recovery(&store, 3)).await;
    assert!(item.needs_human_response());
    // No scan establishes anything: a day of passes changes nothing.
    drive(&store, &[], &everyone(), 25, |_| None).await;
    assert!(get(&store, &item.id).await.subject_check.is_none());
    assert!(folded(&store, &item.id).await.is_none());
    store.sweep_system_asks(&[]).await.unwrap();
    assert!(get(&store, &item.id).await.needs_human_response());
    // Its recipient vanished a week ago: nobody is left for the answer.
    let mut aged = get(&store, &parent.id).await;
    let info = aged.interruption.as_mut().unwrap();
    info.observed_at -= Duration::days(5);
    *info.reason_since.as_mut().expect("stamped") -= Duration::days(8);
    put(&store, aged).await;
    store.sweep_system_asks(&[]).await.unwrap();
    let closed = get(&store, &item.id).await;
    assert!(!closed.needs_human_response());
    assert_eq!(
        closed.system_ask.unwrap().closed.unwrap().reason,
        crate::collaboration::system_asks::SystemAskCloseReason::Stale
    );
    assert_eq!(get(&store, &parent.id).await.status, RequestStatus::Claimed);
}

#[tokio::test]
async fn a_console_recovery_whose_recipient_pane_is_gone_folds_and_is_not_closed_within_the_week() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let (_, item) = Box::pin(console_recovery(&store, 3)).await;
    drive(&store, &[], &everyone(), 25, gone_scan).await;
    let gone = folded(&store, &item.id).await.expect("folded");
    assert_eq!(gone.cause, GoneCause::PaneGone);
    store.sweep_system_asks(&[]).await.unwrap();
    store.reconcile_peer_interruptions(&[]).await.unwrap();
    assert!(
        get(&store, &item.id).await.needs_human_response(),
        "not closed for age within the week"
    );
    // Its recipient gone a week: tend closes it, recorded as such.
    let mut aged = get(&store, &item.id).await;
    let check = aged.subject_check.as_mut().unwrap();
    check.episode_since = check.episode_since.map(|at| at - Duration::days(7));
    put(&store, aged).await;
    store.sweep_system_asks(&[]).await.unwrap();
    let closed = get(&store, &item.id).await;
    assert_eq!(closed.status, RequestStatus::Cancelled);
    assert_eq!(
        closed.system_ask.unwrap().closed.unwrap().reason,
        super::super::system_asks::SystemAskCloseReason::SubjectGone
    );
}

#[tokio::test]
async fn closing_a_recovery_item_records_the_dismissal_on_the_original_request_only() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    store.set_close_unanswerable(true);
    let (parent, item) = Box::pin(console_recovery(&store, 3)).await;
    drive(&store, &[], &everyone(), 25, gone_scan).await;
    let gone = folded(&store, &item.id).await.unwrap();
    let before = store.requests.read().await.len();
    let outcome = store
        .close_unanswerable(
            &[CloseItem {
                request_id: item.id.clone(),
                episode: gone.episode.clone(),
            }],
            &registry(&[]).await,
            "cli",
            None,
        )
        .await
        .unwrap();
    assert_eq!(outcome.closed, std::slice::from_ref(&item.id));
    let closed = get(&store, &item.id).await;
    assert_eq!(closed.status, RequestStatus::Declined);
    assert_eq!(closed.reply.as_ref().unwrap().body, DISMISSED_BODY);
    let closure = closed.closure.unwrap();
    assert_eq!(closure.reason, ClosureReason::SubjectGone);
    assert_eq!(
        (closure.by.as_str(), closure.surface.as_str()),
        ("person", "cli")
    );
    assert_eq!(closure.episode, gone.episode);
    // The reconcile copies the decline into the original request, as for
    // any answer to a recovery item.
    store.reconcile_peer_interruptions(&[]).await.unwrap();
    let after = get(&store, &parent.id).await;
    let decision = after
        .interruption
        .as_ref()
        .unwrap()
        .decision
        .as_ref()
        .unwrap();
    assert_eq!(decision.status, RequestStatus::Declined);
    assert_eq!(decision.body, DISMISSED_BODY);
    // Its status and authority are unchanged, and nothing was restarted,
    // cancelled, resent or retried.
    assert_eq!(after.status, parent.status);
    assert_eq!(after.work_mode, parent.work_mode);
    assert_eq!(after.to, parent.to);
    assert_eq!(after.reply, parent.reply);
    assert_eq!(after.retirement, parent.retirement);
    assert_eq!(store.requests.read().await.len(), before, "no new request");
}

#[tokio::test]
async fn close_all_is_refused_while_turned_off() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("s9", "%9", "default")).await;
    drive(&store, &[], &everyone(), 25, gone_scan).await;
    let gone = folded(&store, &q.id).await.unwrap();
    let error = store
        .close_unanswerable(
            &[CloseItem {
                request_id: q.id.clone(),
                episode: gone.episode,
            }],
            &registry(&[]).await,
            "cli",
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), CLOSE_UNANSWERABLE_OFF);
    assert!(get(&store, &q.id).await.needs_human_response());
}

#[tokio::test]
async fn close_all_closes_exactly_the_list_and_loses_every_race() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    store.set_close_unanswerable(true);
    let questions = [
        question(&store, &participant("a", "%11", "default")).await,
        question(&store, &participant("b", "%12", "default")).await,
        question(&store, &participant("c", "%13", "default")).await,
        question(&store, &participant("d", "%14", "default")).await,
        question(&store, &participant("e", "%15", "default")).await,
    ];
    drive(&store, &[], &everyone(), 25, gone_scan).await;
    let mut list = Vec::new();
    for q in &questions[..4] {
        let gone = folded(&store, &q.id).await.expect("folded");
        list.push(CloseItem {
            request_id: q.id.clone(),
            episode: gone.episode,
        });
    }
    // After the list was drawn: `a` was answered, `b` kept, `c` resumed.
    let console = Participant::console(questions[0].from.room.clone());
    store
        .reply(
            &console,
            &questions[0].id,
            RequestStatus::Completed,
            "ship it".into(),
            vec![],
        )
        .await
        .unwrap();
    store
        .keep_unanswerable(&questions[1].id, &list[1].episode)
        .await
        .unwrap();
    // `e` was never in the list: it arrived after it was drawn.
    let outcome = store
        .close_unanswerable(&list, &registry(&[("c", "%20")]).await, "mac", None)
        .await
        .unwrap();
    assert_eq!(outcome.closed, [questions[3].id.clone()]);
    let reasons: HashMap<_, _> = outcome
        .skipped
        .iter()
        .map(|s| (s.request_id.as_str(), s.reason.as_str()))
        .collect();
    assert_eq!(reasons[questions[0].id.as_str()], "no longer open");
    assert_eq!(reasons[questions[1].id.as_str()], "kept");
    assert_eq!(reasons[questions[2].id.as_str()], "its subject is back");
    assert_eq!(
        get(&store, &questions[0].id).await.reply.unwrap().body,
        "ship it"
    );
    assert!(get(&store, &questions[4].id).await.needs_human_response());
    // A pair from an earlier episode is skipped too.
    let stale = store
        .close_unanswerable(
            &[CloseItem {
                request_id: questions[4].id.clone(),
                episode: "ep_earlier".into(),
            }],
            &registry(&[]).await,
            "web",
            None,
        )
        .await
        .unwrap();
    assert!(stale.closed.is_empty());
    assert_eq!(stale.skipped[0].reason, "a new episode since the list");
}

#[tokio::test]
async fn the_queue_lists_live_items_first_and_counts_every_folded_one() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let gone_a = question(&store, &participant("ga", "%31", "default")).await;
    let gone_b = question(&store, &participant("gb", "%32", "default")).await;
    drive(&store, &[], &everyone(), 25, gone_scan).await;
    // A live question asked later, by an agent that is still there.
    let live_asker = participant("live", "%33", "default");
    let live = question(&store, &live_asker).await;
    backdate(&store, &live.id, -Duration::days(7)).await;
    let alive = row("live", "%33", "default", AgentState::Idle, Duration::ZERO).await;
    pass(
        &store,
        std::slice::from_ref(&alive),
        &everyone(),
        gone_scan(OffsetDateTime::now_utc()).as_ref(),
    )
    .await;
    // A limit of one keeps the live question; the count stays exact.
    let page = store.open_human_requests_folded(1).await.unwrap();
    assert_eq!(page.requests[0].id, live.id);
    assert_eq!(page.unanswerable_count, 2);
    let queue = crate::decision_queue::assemble_queue_folded(
        &page.requests,
        &[],
        Vec::new(),
        1,
        &page.folded,
        page.unanswerable_count,
    );
    assert_eq!(queue.unanswerable_count, 2);
    let full = store.open_human_requests_folded(10).await.unwrap();
    let queue = crate::decision_queue::assemble_queue_folded(
        &full.requests,
        &[],
        Vec::new(),
        10,
        &full.folded,
        full.unanswerable_count,
    );
    let order: Vec<_> = queue
        .items
        .iter()
        .map(|item| match item {
            crate::decision_queue::PendingItem::HumanRequest(entry) => {
                (entry.request_id.clone(), entry.unanswerable.is_some())
            }
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(
        order,
        [
            (live.id.clone(), false),
            (gone_a.id.clone(), true),
            (gone_b.id.clone(), true),
        ]
    );
}

/// A reader built before these fields existed decodes the new queue, and
/// the new reader decodes the old one.
#[tokio::test]
async fn old_and_new_queue_readers_decode_each_other() {
    #[derive(Deserialize)]
    struct OldEntry {
        request_id: String,
        body: String,
    }
    #[derive(Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    enum OldItem {
        HumanRequest(OldEntry),
        #[serde(other)]
        Other,
    }
    #[derive(Deserialize)]
    struct OldQueue {
        items: Vec<OldItem>,
        #[serde(default)]
        truncated: bool,
    }
    let new = serde_json::json!({
        "items": [{
            "kind": "human_request", "request_id": "rq_1", "body": "deploy?",
            "from_session_id": "s", "from_pane": "%1", "created_at": "2026-11-01T00:00:00Z",
            "unanswerable": {"cause": "pane_gone", "episode": "ep_1",
                             "since": "2026-11-10T00:00:00Z", "pane": "%1"}
        }],
        "truncated": false,
        "unanswerable_count": 3
    });
    let old: OldQueue = serde_json::from_value(new.clone()).unwrap();
    assert!(!old.truncated);
    match &old.items[0] {
        OldItem::HumanRequest(entry) => {
            assert_eq!(
                (entry.request_id.as_str(), entry.body.as_str()),
                ("rq_1", "deploy?")
            );
        }
        OldItem::Other => panic!("decoded as another kind"),
    }
    let decoded: crate::decision_queue::PendingQueue = serde_json::from_value(new).unwrap();
    assert_eq!(decoded.unanswerable_count, 3);
    // Without the fields: absent, and not written back.
    let old_json = serde_json::json!({
        "items": [{
            "kind": "human_request", "request_id": "rq_1", "body": "deploy?",
            "from_session_id": "s", "from_pane": "%1", "created_at": "2026-11-01T00:00:00Z"
        }]
    });
    let decoded: crate::decision_queue::PendingQueue = serde_json::from_value(old_json).unwrap();
    assert_eq!(decoded.unanswerable_count, 0);
    let back = serde_json::to_value(&decoded).unwrap();
    assert!(back.get("unanswerable_count").is_none(), "{back}");
    assert!(back["items"][0].get("unanswerable").is_none(), "{back}");
    // A request row without the new fields writes none of them, and one
    // with them round-trips.
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("s", "%1", "default")).await;
    let row = serde_json::to_value(get(&store, &q.id).await).unwrap();
    assert!(row.get("subject_check").is_none() && row.get("closure").is_none());
    let episode = store.fold_for_test(&q.id, GoneCause::Ended).await;
    let row = serde_json::to_value(get(&store, &q.id).await).unwrap();
    assert_eq!(row["subject_check"]["episode"], episode.as_str());
    assert_eq!(row["subject_check"]["state"], "gone");
    assert_eq!(row["subject_check"]["causes"][0], "ended");
    assert!(row["subject_check"]["kept_episode"].is_null());
    let request: CollaborationRequest = serde_json::from_value(row).unwrap();
    assert_eq!(request.subject_check.unwrap().episode, episode);
}

/// A reply-wake item is about the original request's sender.
#[tokio::test]
async fn a_reply_wake_folds_when_the_original_sender_is_gone() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let sender = participant("gone-sender", "%40", "default");
    let recipient = participant("worker", "%41", "default");
    let parent = store
        .create(
            sender,
            recipient,
            NewRequest {
                body: "do it".into(),
                ..NewRequest::default()
            },
        )
        .await
        .unwrap();
    let wake = super::super::sender_follow::gone_sender_action(
        &parent,
        "rq_wake".into(),
        OffsetDateTime::now_utc() - Duration::days(7),
    );
    put(&store, wake).await;
    // The reply is 73 days old; the item was recreated 6.9 days ago.
    drive(&store, &[], &everyone(), 25, gone_scan).await;
    let gone = folded(&store, "rq_wake").await.expect("folded");
    assert_eq!(gone.pane.as_deref(), Some("%40"));
}

// Regressions from the review of #642.

/// After a daemon restart no binding is remembered: a tmux pane absent from
/// server B says nothing about a session that may have been bound on A.
#[tokio::test]
async fn reviewer_missing_incarnation_is_unknown() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("old", "%3", "default")).await;
    let absent = scan(OffsetDateTime::now_utc(), &[("default", Some(B))], &[]);
    pass(&store, &[], &HashMap::new(), Some(&absent)).await;
    assert!(get(&store, &q.id).await.subject_check.is_none());
    // Nor does a server that reports no incarnation, even with a binding.
    let unreported = scan(OffsetDateTime::now_utc(), &[("default", None)], &[]);
    pass(
        &store,
        &[],
        &bound("old", "%3", "default", A),
        Some(&unreported),
    )
    .await;
    assert!(get(&store, &q.id).await.subject_check.is_none());
    // A socket that says no server runs there holds no pane at all.
    let mut stopped = scan(OffsetDateTime::now_utc(), &[("default", None)], &[]);
    stopped.hosts[0].servers[0].running = false;
    pass(&store, &[], &HashMap::new(), Some(&stopped)).await;
    assert_eq!(
        get(&store, &q.id).await.subject_check.unwrap().causes,
        [GoneCause::PaneGone]
    );
}

/// rmux reports no incarnation; absence from its running endpoint counts.
#[tokio::test]
async fn an_rmux_pane_absent_from_its_running_endpoint_is_gone() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let endpoint = "/tmp/rmux-501/default";
    let q = question(&store, &participant("r1", "rmux:%3", endpoint)).await;
    let mut absent = scan(OffsetDateTime::now_utc(), &[(endpoint, None)], &[]);
    absent.hosts[0].kind = HostKind::Rmux;
    pass(&store, &[], &HashMap::new(), Some(&absent)).await;
    assert_eq!(
        get(&store, &q.id).await.subject_check.unwrap().causes,
        [GoneCause::PaneGone]
    );
}

/// A confirmation older than a few minutes folds nothing and authorizes
/// no close.
#[tokio::test]
async fn reviewer_expired_confirmation_cannot_close() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    store.set_close_unanswerable(true);
    let q = question(&store, &participant("old", "%3", "default")).await;
    let episode = store.fold_for_test(&q.id, GoneCause::PaneGone).await;
    store
        .subject
        .pass
        .lock()
        .unwrap()
        .confirmed
        .get_mut(&q.id)
        .unwrap()
        .1 -= Duration::hours(2);
    assert!(folded(&store, &q.id).await.is_none(), "not folded either");
    let page = store.open_human_requests_folded(10).await.unwrap();
    assert_eq!(page.unanswerable_count, 0);
    let result = store
        .close_unanswerable(
            &[CloseItem {
                request_id: q.id.clone(),
                episode,
            }],
            &registry(&[]).await,
            "cli",
            None,
        )
        .await
        .unwrap();
    assert!(result.closed.is_empty());
    assert_eq!(result.skipped[0].reason, "not unanswerable now");
}

/// Close all reads the evidence again inside its transaction: a resume or a
/// scan that no longer shows the subject gone stops the close, however
/// fresh the pass was.
#[tokio::test]
async fn close_all_rechecks_the_evidence_at_the_commit() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    store.set_close_unanswerable(true);
    let q = question(&store, &participant("s9", "%9", "default")).await;
    let episode = store.fold_for_test(&q.id, GoneCause::PaneGone).await;
    let item = [CloseItem {
        request_id: q.id.clone(),
        episode,
    }];
    // The pane is back on the same server.
    let back = registry(&[]).await;
    back.subject_observations().set_scan(scan(
        OffsetDateTime::now_utc(),
        &[("default", Some(A))],
        &[pane("%9", "default", "claude", Some(true))],
    ));
    let outcome = store
        .close_unanswerable(&item, &back, "cli", None)
        .await
        .unwrap();
    assert!(outcome.closed.is_empty());
    assert_eq!(
        outcome.skipped[0].reason,
        "its subject is no longer established gone"
    );
    // The newest scan is stale.
    let stale = registry(&[]).await;
    stale.subject_observations().set_scan(scan(
        OffsetDateTime::now_utc() - Duration::hours(1),
        &[("default", Some(A))],
        &[],
    ));
    let outcome = store
        .close_unanswerable(&item, &stale, "cli", None)
        .await
        .unwrap();
    assert!(outcome.closed.is_empty());
    // The session resumed elsewhere.
    let resumed = registry(&[("s9", "%30")]).await;
    let outcome = store
        .close_unanswerable(&item, &resumed, "cli", None)
        .await
        .unwrap();
    assert_eq!(outcome.skipped[0].reason, "its subject is back");
    // Gone by the evidence now: closed.
    let outcome = store
        .close_unanswerable(&item, &registry(&[]).await, "cli", None)
        .await
        .unwrap();
    assert_eq!(outcome.closed, std::slice::from_ref(&q.id));
}

/// A covered request that cannot be found is a subject nobody can show
/// gone, so the item does not fold.
#[tokio::test]
async fn reviewer_missing_covered_parent_is_unknown() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let (_, item) = Box::pin(console_recovery(&store, 3)).await;
    let mut item = get(&store, &item.id).await;
    item.system_ask
        .as_mut()
        .unwrap()
        .covers
        .push("missing_parent".into());
    put(&store, item.clone()).await;
    // `peer` is bound and its pane is gone: only the missing one is unknown.
    pass(
        &store,
        &[],
        &everyone(),
        gone_scan(OffsetDateTime::now_utc()).as_ref(),
    )
    .await;
    assert!(get(&store, &item.id).await.subject_check.is_none());
}

/// The registry of the replacement fixture: `old` bound to %4 on server A
/// by a scan two days ago, then `new` started in %4 now.
async fn replaced_after_an_old_scan() -> crate::state::SharedStore {
    let id = |session: &str| AgentId {
        kind: AgentKind::ClaudeCode,
        session_id: session.into(),
        surface: None,
        pane: Some("%4".into()),
        tmux_socket: Some("default".into()),
        cwd: None,
    };
    let now = OffsetDateTime::now_utc();
    let registry = crate::state::Store::shared();
    registry
        .apply(&AgentEvent::Started {
            id: id("old"),
            at: now - Duration::days(2),
        })
        .await;
    registry
        .record_pane_scan(scan(
            now - Duration::days(2),
            &[("default", Some(A))],
            &[pane("%4", "default", "claude", Some(true))],
        ))
        .await;
    registry
        .apply(&AgentEvent::Started {
            id: id("new"),
            at: now,
        })
        .await;
    registry
}

/// A replacement is settled by the first scan after its hook, never by an
/// older one: a server restart between the last scan and the hook makes it
/// not count.
#[tokio::test]
async fn reviewer_stale_scan_cannot_certify_replacement_incarnation() {
    // No scan since the hook: nothing is settled, so nothing counts.
    let registry = replaced_after_an_old_scan().await;
    assert!(registry.subject_observations().take_facts().is_empty());
    // The server was restarted between the old scan and the hook.
    registry
        .record_pane_scan(scan(
            OffsetDateTime::now_utc() + Duration::seconds(1),
            &[("default", Some(B))],
            &[pane("%4", "default", "claude", Some(true))],
        ))
        .await;
    let facts = registry.subject_observations().take_facts();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].kind, FactKind::Replaced);
    assert!(!facts[0].same_incarnation);
    // The same server before and after the hook: it counts.
    let registry = replaced_after_an_old_scan().await;
    registry
        .record_pane_scan(scan(
            OffsetDateTime::now_utc() + Duration::seconds(1),
            &[("default", Some(A))],
            &[pane("%4", "default", "claude", Some(true))],
        ))
        .await;
    let facts = registry.subject_observations().take_facts();
    assert!(facts[0].same_incarnation);
}

/// A failed write puts the facts back; the next pass keeps them.
#[tokio::test]
async fn facts_survive_a_failed_write() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mailbox.db");
    let store = CollaborationStore::load(CollaborationOptions {
        path: Some(path.clone()),
        ..CollaborationOptions::default()
    })
    .await
    .unwrap();
    let registry = crate::state::Store::shared();
    let observations = registry.subject_observations();
    observations.record(SubjectFact::new(
        FactKind::Ended,
        "s1",
        Some("%1"),
        Some("default"),
        None,
        OffsetDateTime::now_utc(),
    ));
    // The database is replaced under the store: the write fails.
    std::fs::rename(&path, dir.path().join("moved.db")).unwrap();
    assert!(store.keep_facts_from(observations).is_err());
    assert_eq!(observations.peek_facts().len(), 1, "requeued");
    assert!(store.subject_facts().is_empty());
    // A store over a working database keeps them, and they are drained.
    let recovered = CollaborationStore::load(CollaborationOptions {
        path: Some(dir.path().join("fresh.db")),
        ..CollaborationOptions::default()
    })
    .await
    .unwrap();
    recovered.keep_facts_from(observations).unwrap();
    assert!(observations.peek_facts().is_empty());
    assert_eq!(recovered.subject_facts()[0].session_id, "s1");
}

/// A resume that lands while Close all waits for the mailbox wins: the
/// registry is read only once the mailbox is held, and held still through
/// the commit.
#[tokio::test]
async fn reviewer_resume_after_registry_read_before_commit_must_win() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    store.set_close_unanswerable(true);
    let q = question(&store, &participant("s9", "%9", "default")).await;
    let episode = store.fold_for_test(&q.id, GoneCause::PaneGone).await;
    let registry = registry(&[]).await;
    let items = [CloseItem {
        request_id: q.id.clone(),
        episode,
    }];
    let requests_guard = store.requests.write().await;
    let mut closing = Box::pin(store.close_unanswerable(&items, &registry, "cli", None));
    tokio::select! {
        biased;
        _ = &mut closing => panic!("must wait for the held requests lock"),
        () = std::future::ready(()) => {}
    }
    registry
        .apply(&AgentEvent::Started {
            id: AgentId {
                kind: AgentKind::ClaudeCode,
                session_id: "s9".into(),
                surface: None,
                pane: Some("%30".into()),
                tmux_socket: Some("default".into()),
                cwd: None,
            },
            at: OffsetDateTime::now_utc(),
        })
        .await;
    drop(requests_guard);
    let outcome = closing.await.unwrap();
    assert!(outcome.closed.is_empty());
    assert_eq!(outcome.skipped[0].reason, "its subject is back");
    assert!(get(&store, &q.id).await.needs_human_response());
}

/// While Close all holds the registry, a hook waits for its commit.
#[tokio::test]
async fn a_hook_waits_for_close_alls_commit() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    store.set_close_unanswerable(true);
    let q = question(&store, &participant("s9", "%9", "default")).await;
    let episode = store.fold_for_test(&q.id, GoneCause::PaneGone).await;
    let registry = registry(&[]).await;
    let hold = registry.hold().await;
    let resume = AgentEvent::Started {
        id: AgentId {
            kind: AgentKind::ClaudeCode,
            session_id: "s9".into(),
            surface: None,
            pane: Some("%30".into()),
            tmux_socket: Some("default".into()),
            cwd: None,
        },
        at: OffsetDateTime::now_utc(),
    };
    let mut hook = Box::pin(registry.apply(&resume));
    tokio::select! {
        biased;
        () = &mut hook => panic!("a hook must wait while the registry is held"),
        () = std::future::ready(()) => {}
    }
    drop(hold);
    hook.await;
    // The resume landed before the close read the registry: it wins.
    let outcome = store
        .close_unanswerable(
            &[CloseItem {
                request_id: q.id.clone(),
                episode,
            }],
            &registry,
            "cli",
            None,
        )
        .await
        .unwrap();
    assert_eq!(outcome.skipped[0].reason, "its subject is back");
}

/// An answering scan that first arrives after the ten-minute window does
/// not certify a replacement.
#[tokio::test]
async fn reviewer_replacement_answer_after_ten_minutes_is_uncertified() {
    let registry = replaced_after_an_old_scan().await;
    registry
        .record_pane_scan(scan(
            OffsetDateTime::now_utc() + Duration::minutes(11),
            &[("default", Some(A))],
            &[pane("%4", "default", "claude", Some(true))],
        ))
        .await;
    let facts = registry.subject_observations().take_facts();
    assert_eq!(facts.len(), 1);
    assert!(!facts[0].same_incarnation);
}

// Bindings stored on the item ("Bindings survive a daemon restart").

/// The asker was seen bound on server A before a daemon restart and never
/// came back: its pane absent from A still folds the question, while an
/// absence from a restarted server B says nothing.
#[tokio::test]
async fn a_binding_stored_on_the_question_survives_a_daemon_restart() {
    let dir = tempfile::tempdir().unwrap();
    let options = CollaborationOptions {
        path: Some(dir.path().join("mailbox.db")),
        ..CollaborationOptions::default()
    };
    let store = CollaborationStore::load(options.clone()).await.unwrap();
    let q = question(&store, &participant("old", "%3", "default")).await;
    let present = scan(
        OffsetDateTime::now_utc(),
        &[("default", Some(A))],
        &[pane("%3", "default", "claude", Some(true))],
    );
    let memory = bound("old", "%3", "default", A);
    pass(&store, &[], &memory, Some(&present)).await;
    let row = get(&store, &q.id).await;
    assert!(row.subject_check.is_none());
    assert_eq!(row.subject_bindings.get("old"), memory.get("old"));
    drop(store);

    // The restarted daemon remembers no binding.
    let reloaded = CollaborationStore::load(options).await.unwrap();
    let restarted = scan(OffsetDateTime::now_utc(), &[("default", Some(B))], &[]);
    pass(&reloaded, &[], &HashMap::new(), Some(&restarted)).await;
    assert!(get(&reloaded, &q.id).await.subject_check.is_none());
    let absent = scan(OffsetDateTime::now_utc(), &[("default", Some(A))], &[]);
    pass(&reloaded, &[], &HashMap::new(), Some(&absent)).await;
    assert_eq!(
        get(&reloaded, &q.id).await.subject_check.unwrap().causes,
        [GoneCause::PaneGone]
    );
}

/// A pane id a restarted server reuses never re-stamps the stored
/// incarnation; a move to another pane replaces the binding.
#[tokio::test]
async fn a_stored_binding_keeps_its_first_incarnation_and_follows_a_move() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("old", "%3", "default")).await;
    let inputs_scan = scan(OffsetDateTime::now_utc(), &[("default", Some(A))], &[]);
    let stored = |row: &CollaborationRequest| row.subject_bindings.get("old").cloned().unwrap();
    // Judged against the stored binding, before any absence counts.
    let present = scan(
        OffsetDateTime::now_utc(),
        &[("default", Some(A))],
        &[pane("%3", "default", "claude", Some(true))],
    );
    pass(
        &store,
        &[],
        &bound("old", "%3", "default", A),
        Some(&present),
    )
    .await;
    // A second pass with nothing new writes nothing.
    let report = store
        .check_subjects(&SubjectInputs {
            agents: &[],
            scan: Some(&present),
            bindings: &bound("old", "%3", "default", A),
        })
        .await
        .unwrap();
    assert_eq!(report.written, 0);
    // The same pane id, now on server B: the stored binding stays on A.
    let reused = scan(
        OffsetDateTime::now_utc(),
        &[("default", Some(B))],
        &[pane("%3", "default", "claude", Some(true))],
    );
    pass(
        &store,
        &[],
        &bound("old", "%3", "default", B),
        Some(&reused),
    )
    .await;
    assert_eq!(stored(&get(&store, &q.id).await).incarnation, Some(A));
    // Seen live in another pane: that is where it is bound now.
    pass(
        &store,
        &[],
        &bound("old", "%4", "default", A),
        Some(&inputs_scan),
    )
    .await;
    let moved = stored(&get(&store, &q.id).await);
    assert_eq!((moved.pane.as_str(), moved.incarnation), ("%4", Some(A)));
}

/// `scan`, from a pass that walked every pane's processes completely and
/// found these `(pane, session)` explicit resumes.
fn walked(scan: PaneScan, resumed: &[(&str, &str)]) -> PaneScan {
    let mut scan = PaneScan {
        processes_read: true,
        ..scan
    };
    let at = scan.at;
    for host in &mut scan.hosts {
        for pane in &mut host.panes {
            pane.resumed = resumed
                .iter()
                .filter(|(id, _)| *id == pane.pane_id)
                .map(|(_, session)| (*session).to_owned())
                .collect();
            // One agent per pane, named when it was resumed by id.
            pane.unnamed = (pane.agent_process == Some(true) && pane.resumed.is_empty())
                .then(|| UnnamedAgent::process(1, Some("1"), Some(AgentKind::ClaudeCode), at))
                .into_iter()
                .collect();
        }
    }
    scan
}

/// Pass every five minutes for the grace and a little more, with no
/// binding anywhere: the asker was gone before tendd recorded one.
async fn drive_unbound(
    store: &CollaborationStore,
    agents: &[Agent],
    scan_at: impl Fn(OffsetDateTime) -> PaneScan,
) {
    drive_for(
        store,
        agents,
        &HashMap::new(),
        SUBJECT_GONE_GRACE + Duration::minutes(5),
        |at| Some(scan_at(at)),
    )
    .await;
}

#[tokio::test]
async fn an_asker_gone_before_any_binding_was_recorded_folds_from_present_state() {
    // As on jiun-mini: week-old questions whose asker has no row, asked
    // before tendd stored where it was bound.
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("01a16985", "%15", "default")).await;
    // Without a process walk, absence on an unknown incarnation says nothing.
    drive_unbound(&store, &[], |at| scan(at, &[("default", Some(B))], &[])).await;
    assert!(get(&store, &q.id).await.subject_check.is_none());
    // With one, and no process resumed on it: its pane is gone.
    drive_unbound(&store, &[], |at| {
        walked(scan(at, &[("default", Some(B))], &[]), &[])
    })
    .await;
    let gone = folded(&store, &q.id).await.expect("folded");
    assert_eq!(gone.cause, GoneCause::PaneGone);
    assert!(get(&store, &q.id).await.needs_human_response());
}

#[tokio::test]
async fn a_pane_another_named_session_holds_is_replaced_but_an_unnamed_one_shows_nothing() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("01a13356", "%42", "default")).await;
    let panes = |at| {
        walked(
            scan(
                at,
                &[("default", Some(B))],
                &[pane("%42", "default", "codex", Some(true))],
            ),
            &[],
        )
    };
    // An agent tendd cannot name yet may be the asker resumed.
    let mut unnamed = row("x", "%42", "default", AgentState::Idle, Duration::ZERO).await;
    unnamed.session_id = format!("{}7:default:%42", crate::state::SYNTHETIC_SESSION_PREFIX);
    drive_unbound(&store, &[unnamed], panes).await;
    assert!(folded(&store, &q.id).await.is_none());
    // Another session's own row there: the asker was replaced.
    let other = row(
        "01a1f248",
        "%42",
        "default",
        AgentState::Idle,
        Duration::ZERO,
    )
    .await;
    drive_unbound(&store, &[other], panes).await;
    assert_eq!(
        folded(&store, &q.id).await.expect("folded").cause,
        GoneCause::Replaced
    );
}

#[tokio::test]
async fn a_resumed_or_moved_asker_is_alive_before_its_first_prompt() {
    // As in jiun-mini's %8: `codex resume <asker>` runs, still unprompted, so
    // the pane shows only a synthetic row.
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("01a132f3", "%8", "default")).await;
    let gone_then = |at| walked(scan(at, &[("default", Some(B))], &[]), &[]);
    drive_unbound(&store, &[], gone_then).await;
    assert!(folded(&store, &q.id).await.is_some());
    let resumed = walked(
        scan(
            OffsetDateTime::now_utc(),
            &[("default", Some(B))],
            &[pane("%8", "default", "codex", Some(true))],
        ),
        &[("%8", "01a132f3")],
    );
    pass(&store, &[], &HashMap::new(), Some(&resumed)).await;
    assert!(
        get(&store, &q.id).await.subject_check.is_none(),
        "alive ends the episode"
    );
    // Swapped to another account and pane: its own live row wins.
    drive_unbound(&store, &[], gone_then).await;
    assert!(folded(&store, &q.id).await.is_some());
    let moved = row(
        "01a132f3",
        "%77",
        "default",
        AgentState::Idle,
        Duration::ZERO,
    )
    .await;
    pass(
        &store,
        &[moved],
        &HashMap::new(),
        Some(&gone_then(OffsetDateTime::now_utc())),
    )
    .await;
    assert!(get(&store, &q.id).await.subject_check.is_none());
}

#[tokio::test]
async fn a_shell_where_the_asker_ran_is_agent_exited_from_present_state() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("88ce61e3", "%142", "default")).await;
    drive_unbound(&store, &[], |at| {
        walked(
            scan(
                at,
                &[("default", Some(B))],
                &[pane("%142", "default", "zsh", Some(false))],
            ),
            &[],
        )
    })
    .await;
    assert_eq!(
        folded(&store, &q.id).await.expect("folded").cause,
        GoneCause::AgentExited
    );
}

#[tokio::test]
async fn an_agent_tendd_cannot_name_anywhere_keeps_an_unbound_asker_unknown() {
    // `claude --continue` in another pane: no id on its command line, so it
    // could be the asker. Its absent pane then shows nothing.
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("55115377", "%140", "default")).await;
    let mut unnamed = row("x", "%99", "default", AgentState::Idle, Duration::ZERO).await;
    unnamed.session_id = format!("{}7:default:%99", crate::state::SYNTHETIC_SESSION_PREFIX);
    let elsewhere = |at| {
        walked(
            scan(
                at,
                &[("default", Some(B))],
                &[pane("%99", "default", "claude", Some(true))],
            ),
            &[],
        )
    };
    drive_unbound(&store, std::slice::from_ref(&unnamed), elsewhere).await;
    assert!(get(&store, &q.id).await.subject_check.is_none());
    // No row at all for that agent process: the same.
    drive_unbound(&store, &[], elsewhere).await;
    assert!(get(&store, &q.id).await.subject_check.is_none());
    // Named, it is someone else: the asker is gone.
    let other = row(
        "someone",
        "%99",
        "default",
        AgentState::Idle,
        Duration::ZERO,
    )
    .await;
    drive_unbound(&store, &[other], elsewhere).await;
    assert_eq!(
        folded(&store, &q.id).await.expect("folded").cause,
        GoneCause::PaneGone
    );
}

#[tokio::test]
async fn a_walk_that_was_not_complete_shows_nothing_without_a_binding() {
    // A complete tmux scan beside an incomplete backend, a failed `ps` or a
    // cut-off walk: the reconcile reports `processes_read: false`.
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("01a16985", "%15", "default")).await;
    drive_unbound(&store, &[], |at| scan(at, &[("default", Some(B))], &[])).await;
    assert!(get(&store, &q.id).await.subject_check.is_none());
    // A resume found by that partial walk still counts as alive.
    let partial = scan(
        OffsetDateTime::now_utc(),
        &[("default", Some(B))],
        &[pane("%3", "default", "codex", Some(true))],
    );
    let mut partial = partial;
    partial.hosts[0].panes[0].resumed = vec!["01a16985".into()];
    let mut row_gone = get(&store, &q.id).await;
    row_gone.subject_check = Some(SubjectCheck {
        episode: "ep_x".into(),
        state: SubjectState::Gone,
        gone_since: Some(OffsetDateTime::now_utc() - Duration::hours(1)),
        episode_since: None,
        confirmed_at: OffsetDateTime::now_utc(),
        causes: vec![GoneCause::PaneGone],
        kept_episode: None,
        pane: Some("%15".into()),
    });
    put(&store, row_gone).await;
    pass(&store, &[], &HashMap::new(), Some(&partial)).await;
    assert!(
        get(&store, &q.id).await.subject_check.is_none(),
        "resumed: alive"
    );
}

#[tokio::test]
async fn one_named_agent_does_not_account_for_another_in_the_same_pane() {
    // %5 runs `codex resume other` and, beside it, `claude --continue`,
    // which could be the asker. `other`'s own row is in %5 too.
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("55115377", "%140", "default")).await;
    let other = row("other", "%5", "default", AgentState::Idle, Duration::ZERO).await;
    let shared = |unnamed: u16| {
        move |at| {
            let mut scan = walked(
                scan(
                    at,
                    &[("default", Some(B))],
                    &[pane("%5", "default", "codex", Some(true))],
                ),
                &[("%5", "other")],
            );
            scan.hosts[0].panes[0].unnamed = (0..unnamed)
                .map(|pid| {
                    UnnamedAgent::process(pid.into(), Some("1"), Some(AgentKind::ClaudeCode), at)
                })
                .collect();
            scan
        }
    };
    drive_unbound(&store, std::slice::from_ref(&other), shared(1)).await;
    assert!(get(&store, &q.id).await.subject_check.is_none());
    // Only `other` there: every agent is accounted for.
    drive_unbound(&store, &[other], shared(0)).await;
    assert!(folded(&store, &q.id).await.is_some());
}

/// `walked` with a cmux backend beside tmux, as every host has: always
/// partial, so never `complete`.
fn beside_cmux(mut scan: PaneScan) -> PaneScan {
    scan.hosts.push(HostScan {
        kind: HostKind::Cmux,
        complete: false,
        servers: Vec::new(),
        panes: Vec::new(),
    });
    scan
}

#[tokio::test]
async fn a_partial_backend_beside_tmux_does_not_stop_the_present_state_judgement() {
    // jiun-mini on 0.10.29: tmux complete, cmux partial. The asker's pane
    // is gone and nothing running could be it.
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("01a16985", "%15", "default")).await;
    drive_unbound(&store, &[], |at| {
        beside_cmux(walked(scan(at, &[("default", Some(B))], &[]), &[]))
    })
    .await;
    assert_eq!(
        folded(&store, &q.id).await.expect("folded").cause,
        GoneCause::PaneGone
    );
}

#[tokio::test]
async fn an_unnamed_agent_outside_every_pane_blocks_only_an_asker_of_its_kind() {
    // rtzr: a fresh `codex --yolo` nobody has prompted yet.
    let codex_outside = |at| {
        let mut scan = walked(scan(at, &[("default", Some(B))], &[]), &[]);
        scan.outside = vec![UnnamedAgent::process(
            492,
            Some("1"),
            Some(AgentKind::Codex),
            at,
        )];
        scan
    };
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let claude = question(&store, &participant("55115377", "%140", "default")).await;
    let mut asker = participant("01a1ca42", "%77", "default");
    asker.agent_kind = AgentKind::Codex;
    let codex = question(&store, &asker).await;
    drive_unbound(&store, &[], codex_outside).await;
    assert!(
        folded(&store, &claude.id).await.is_some(),
        "a codex is no claude"
    );
    assert!(
        folded(&store, &codex.id).await.is_none(),
        "it could be this one"
    );
    // A process whose command could not be read could be either.
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let claude = question(&store, &participant("55115377", "%140", "default")).await;
    drive_unbound(&store, &[], |at| {
        let mut scan = walked(scan(at, &[("default", Some(B))], &[]), &[]);
        scan.outside = vec![UnnamedAgent::process(7, Some("1"), None, at)];
        scan
    })
    .await;
    assert!(folded(&store, &claude.id).await.is_none());
}

#[tokio::test]
async fn an_unnamed_agent_blocks_for_at_most_the_bound() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("55115377", "%140", "default")).await;
    let first_seen = OffsetDateTime::now_utc() - UNNAMED_AGENT_BOUND - SUBJECT_GONE_GRACE;
    let outside = |since: OffsetDateTime| {
        move |at| {
            let mut scan = walked(scan(at, &[("default", Some(B))], &[]), &[]);
            scan.outside = vec![UnnamedAgent {
                since,
                ..UnnamedAgent::process(9, Some("1"), Some(AgentKind::ClaudeCode), at)
            }];
            scan
        }
    };
    // Seen just now: it may be the asker, resumed and not yet prompted.
    drive_unbound(&store, &[], outside(OffsetDateTime::now_utc())).await;
    assert!(folded(&store, &q.id).await.is_none());
    // Unnamed for longer than the bound: it no longer holds the judgement.
    drive_unbound(&store, &[], outside(first_seen)).await;
    assert!(folded(&store, &q.id).await.is_some());
    // The same for a pane whose walk did not finish.
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("55115377", "%140", "default")).await;
    let cut = |since: OffsetDateTime| {
        move |at| {
            let mut scan = walked(
                scan(
                    at,
                    &[("default", Some(B))],
                    &[pane("%9", "default", "node", Some(false))],
                ),
                &[],
            );
            scan.hosts[0].panes[0].unnamed = vec![UnnamedAgent {
                since,
                ..UnnamedAgent::unwalked("%9", Some("default"), Some(B), at)
            }];
            scan
        }
    };
    drive_unbound(&store, &[], cut(OffsetDateTime::now_utc())).await;
    assert!(folded(&store, &q.id).await.is_none());
    drive_unbound(&store, &[], cut(first_seen)).await;
    assert!(folded(&store, &q.id).await.is_some());
}

#[tokio::test]
async fn outside_every_pane_a_resume_is_alive_and_a_named_row_accounts_for_its_agent() {
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("55115377", "%140", "default")).await;
    // `claude --resume 55115377` in another terminal.
    drive_unbound(&store, &[], |at| {
        let mut scan = walked(scan(at, &[("default", Some(B))], &[]), &[]);
        scan.outside_resumed = vec!["55115377".into()];
        scan
    })
    .await;
    assert!(get(&store, &q.id).await.subject_check.is_none());
    // A fresh claude in a cmux surface whose own row tendd has: accounted.
    let mut surface = row(
        "01a1f248",
        "%1",
        "default",
        AgentState::Idle,
        Duration::ZERO,
    )
    .await;
    surface.pane = Some("cmux:surface-1".into());
    surface.tmux_socket = None;
    drive_unbound(&store, std::slice::from_ref(&surface), |at| {
        let mut scan = walked(scan(at, &[("default", Some(B))], &[]), &[]);
        scan.outside = vec![UnnamedAgent::process(
            9,
            Some("1"),
            Some(AgentKind::ClaudeCode),
            at,
        )];
        scan
    })
    .await;
    assert!(folded(&store, &q.id).await.is_some());
}

#[test]
fn a_scan_keeps_when_it_first_saw_each_unnamed_thing() {
    let t0 = OffsetDateTime::now_utc() - Duration::hours(2);
    let t1 = t0 + Duration::minutes(1);
    let observations = crate::subject_evidence::SubjectObservations::default();
    let at = |at: OffsetDateTime, start: &str| {
        let mut scan = walked(scan(at, &[("default", Some(B))], &[]), &[]);
        scan.outside = vec![
            UnnamedAgent::process(9, Some(start), Some(AgentKind::ClaudeCode), at),
            UnnamedAgent::process(10, Some("1"), Some(AgentKind::Codex), at),
            UnnamedAgent::process(11, None, Some(AgentKind::Codex), at),
        ];
        scan
    };
    observations.set_scan(at(t0, "1"));
    observations.set_scan(at(t1, "1"));
    let kept = observations.scan().unwrap();
    assert_eq!(kept.outside[0].since, t0);
    // An unreadable start is never carried: always new.
    assert_eq!(kept.outside[2].since, t1);
    // pid 9 now started at another time: another process on a reused pid.
    let t2 = t1 + Duration::minutes(1);
    observations.set_scan(at(t2, "2"));
    let kept = observations.scan().unwrap();
    assert_eq!(kept.outside[0].since, t2);
    assert_eq!(kept.outside[1].since, t0);
}

#[tokio::test]
async fn close_all_never_closes_past_an_unnamed_agent_that_could_be_the_asker() {
    // Unnamed for two hours: the question folds, but Close all, which
    // cannot be undone, still treats that agent as the asker.
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("55115377", "%140", "default")).await;
    let long_ago = OffsetDateTime::now_utc() - Duration::hours(2);
    let scan_at = move |at| {
        let mut scan = walked(scan(at, &[("default", Some(B))], &[]), &[]);
        scan.outside = vec![UnnamedAgent {
            since: long_ago,
            ..UnnamedAgent::process(9, Some("1"), Some(AgentKind::ClaudeCode), at)
        }];
        scan
    };
    drive_unbound(&store, &[], scan_at).await;
    let gone = folded(&store, &q.id).await.expect("folded");
    store.set_close_unanswerable(true);
    let close = |scan: PaneScan| {
        let store = &store;
        let item = CloseItem {
            request_id: q.id.clone(),
            episode: gone.episode.clone(),
        };
        async move {
            let registry = registry(&[]).await;
            registry.subject_observations().set_scan(scan);
            store
                .close_unanswerable(&[item], &registry, "cli", None)
                .await
                .unwrap()
        }
    };
    let outcome = close(scan_at(OffsetDateTime::now_utc())).await;
    assert!(outcome.closed.is_empty(), "{outcome:?}");
    assert!(get(&store, &q.id).await.needs_human_response());
    // Once that agent is gone too, Close all closes it.
    let outcome = close(walked(
        scan(OffsetDateTime::now_utc(), &[("default", Some(B))], &[]),
        &[],
    ))
    .await;
    assert_eq!(outcome.closed, vec![q.id.clone()]);
}

#[tokio::test]
async fn a_session_resumed_outside_every_pane_accounts_only_for_its_own_process() {
    // `claude --resume other` and an unrelated `claude --continue`, both in
    // another terminal; `other` has its live row. The second could be the
    // asker.
    let store = CollaborationStore::in_memory(CollaborationOptions::default());
    let q = question(&store, &participant("55115377", "%140", "default")).await;
    let mut other = row("other", "%1", "default", AgentState::Idle, Duration::ZERO).await;
    other.pane = None;
    other.tmux_socket = None;
    drive_unbound(&store, std::slice::from_ref(&other), |at| {
        let mut scan = walked(scan(at, &[("default", Some(B))], &[]), &[]);
        scan.outside_resumed = vec!["other".into()];
        scan.outside = vec![UnnamedAgent::process(
            9,
            Some("1"),
            Some(AgentKind::ClaudeCode),
            at,
        )];
        scan
    })
    .await;
    assert!(folded(&store, &q.id).await.is_none());
}
