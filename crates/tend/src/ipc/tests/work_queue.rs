//! The Work queue through a real server: the executor types into a fake
//! backend only when the agent is idle, once; an outcome it cannot know
//! becomes uncertain and is never repeated on its own; a queued Work is
//! launched under a lease nothing else can take.

use super::*;
use crate::pipeline_run::{PipelineRunRegistration, PipelineRunStore, QueueLeaseState};
use crate::work::WorkIdentity;
use crate::work_control::{
    HostModeFuture, RemoteWorkFuture, RemoteWorkRunner, WorkCommandLimits, WorkCommandOutput,
};
use crate::work_queue::{
    FenceKind, QueueAddRequest, QueueItem, QueueState, QueueTargetSpec, WaitReason, WorkQueueStore,
};

fn agent_id(session: &str, pane: &str) -> AgentId {
    AgentId {
        kind: AgentKind::Codex,
        session_id: session.into(),
        surface: None,
        pane: Some(pane.into()),
        tmux_socket: Some("default".into()),
        cwd: None,
    }
}

async fn working(store: &SharedStore, id: &AgentId) {
    store
        .apply(&AgentEvent::PromptSubmitted {
            id: id.clone(),
            prompt: "long task".into(),
            at: OffsetDateTime::now_utc(),
        })
        .await;
}

async fn idle(store: &SharedStore, id: &AgentId) {
    store
        .apply(&AgentEvent::TurnStopped {
            id: id.clone(),
            response: None,
            recap: None,
            ai_title: None,
            idle_confirmed: false,
            at: OffsetDateTime::now_utc(),
        })
        .await;
}

/// What the fake pane does with a prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane {
    /// Takes the text and Enter.
    Takes,
    /// Refuses before anything reaches it.
    Refuses,
    /// Takes the text, then reports a failure (a paste that timed out).
    PastesThenFails,
    /// Takes the text, then panics.
    PastesThenPanics,
    /// Takes the text; Enter fails.
    EnterFails,
}

/// A tmux-like backend for the queue: it shows one pane, so the caller's
/// pane observation succeeds, and records every key that lands.
struct QueuePane {
    mode: Pane,
    keys: Arc<Mutex<Vec<String>>>,
    /// Panes it shows besides the one that makes the caller observable.
    panes: Vec<PaneInfo>,
}

fn queue_pane(mode: Pane) -> (Arc<QueuePane>, Arc<Mutex<Vec<String>>>) {
    queue_pane_with(mode, Vec::new())
}

fn queue_pane_with(mode: Pane, panes: Vec<PaneInfo>) -> (Arc<QueuePane>, Arc<Mutex<Vec<String>>>) {
    let keys = Arc::new(Mutex::new(Vec::new()));
    (
        Arc::new(QueuePane {
            mode,
            keys: keys.clone(),
            panes,
        }),
        keys,
    )
}

/// A pane of Work `work` in workspace `workspace`, as tmux tags it.
fn work_pane(id: &str, workspace: &str, work: &str, alias: &str) -> PaneInfo {
    serde_json::from_value(serde_json::json!({
        "pane_id": id,
        "session_id": "$1",
        "session": "tend",
        "window_id": "@2",
        "window_name": work,
        "window_index": "2",
        "pane_index": "0",
        "tty": "/dev/ttys002",
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

impl PaneBackend for QueuePane {
    fn kind(&self) -> HostKind {
        HostKind::Tmux
    }
    fn list_panes(&self) -> Vec<PaneInfo> {
        let mut panes = self.panes.clone();
        panes.push(
            serde_json::from_value(serde_json::json!({
                "pane_id": "%9001",
                "session_id": "$1",
                "session": "tend",
                "window_id": "@1",
                "window_name": "w",
                "window_index": "1",
                "pane_index": "0",
                "tty": "/dev/ttys001",
                "current_command": "zsh",
                "title": "",
                "pane_pid": 0,
                "socket": "default",
            }))
            .expect("pane fixture"),
        );
        panes
    }
    fn resolve_pane(&self, _: &str) -> Option<PaneInfo> {
        None
    }
    fn capture_pane(&self, _: &str) -> Option<String> {
        None
    }
    fn pane_pid_map(&self) -> HashMap<u32, String> {
        HashMap::new()
    }
    fn current_pane(&self) -> Option<String> {
        None
    }
    fn focus_pane(&self, _: &str) -> bool {
        false
    }
    fn send_text(&self, _: &str, text: &str) -> bool {
        self.keys.lock().unwrap().push(text.into());
        !(text == "\r" && self.mode == Pane::EnterFails)
    }
    fn send_prompt_text_checked(
        &self,
        _: Option<&str>,
        _: &str,
        text: &str,
    ) -> crate::backend::TextSend {
        use crate::backend::TextSend;
        if self.mode == Pane::Refuses {
            return TextSend::Refused;
        }
        self.keys.lock().unwrap().push(text.into());
        match self.mode {
            Pane::PastesThenFails => TextSend::Unknown,
            Pane::PastesThenPanics => panic!("backend died after the paste"),
            _ => TextSend::Sent,
        }
    }
    fn caps(&self) -> BackendCaps {
        BackendCaps {
            send_text: true,
            ..BackendCaps::default()
        }
    }
}

async fn serve_queue(
    sock: &Path,
    store: SharedStore,
    backend: Arc<QueuePane>,
    queue: Arc<WorkQueueStore>,
) -> (broadcast::Sender<()>, tokio::task::JoinHandle<()>) {
    let server = Server::new(sock.to_path_buf(), store)
        .with_backends(vec![backend as SharedBackend])
        .with_work_queue(queue, 3);
    let (tx, rx) = broadcast::channel(1);
    let handle = tokio::spawn(async move { server.run(rx).await.unwrap() });
    wait_for_socket(sock).await;
    (tx, handle)
}

fn to_pane(pane: &str, prompt: &str) -> QueueAddRequest {
    QueueAddRequest {
        request_key: uuid::Uuid::new_v4().to_string(),
        title: None,
        prompt: prompt.into(),
        target: QueueTargetSpec::Agent {
            session_id: None,
            pane: Some(pane.into()),
            socket: None,
            label: None,
        },
    }
}

async fn wait_for(client: &Client, id: &str, done: impl Fn(&QueueItem) -> bool) -> QueueItem {
    let mut last = None;
    // Up to 10 s: a retry waits out its delay and the executor's tick.
    for _ in 0..200 {
        let snapshot = client.work_queue_list(true).await.unwrap();
        if let Some(view) = snapshot.items.iter().find(|view| view.item.id == id) {
            if done(&view.item) {
                return view.item.clone();
            }
            last = Some((view.item.state.clone(), view.wait));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("item {id} did not reach the expected state; last {last:?}");
}

/// The item's reason once it reads `expected` (reasons from a pass made
/// before the last change may show first), else the last one seen.
async fn wait_reason(client: &Client, id: &str, expected: WaitReason) -> Option<WaitReason> {
    let mut last = None;
    for _ in 0..60 {
        let snapshot = client.work_queue_list(false).await.unwrap();
        if let Some(reason) = snapshot
            .items
            .iter()
            .find(|view| view.item.id == id)
            .and_then(|view| view.wait)
        {
            last = Some(reason);
            if reason == expected {
                return last;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    last
}

#[tokio::test]
async fn a_busy_agent_gets_the_prompt_once_it_is_idle_and_only_once() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("tend-queue-busy.sock");
    let store = Store::shared();
    let id = agent_id("s1", "%1");
    working(&store, &id).await;
    let (backend, keys) = queue_pane(Pane::Takes);
    let queue = WorkQueueStore::load(Some(dir.path().join("work-queue.json")));
    let (tx, handle) = serve_queue(&sock, store.clone(), backend, queue).await;
    let client = Client::new(sock);

    let item = client
        .work_queue_add(&to_pane("%1", "next step"))
        .await
        .unwrap();
    assert_eq!(
        wait_reason(&client, &item.id, WaitReason::AgentBusy).await,
        Some(WaitReason::AgentBusy)
    );
    // Run now does not pass the gate either.
    let refused = client
        .work_queue_run_now(&item.id, None, false)
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("working"), "{refused}");
    assert!(keys.lock().unwrap().is_empty(), "nothing typed while busy");

    idle(&store, &id).await;
    let sent = wait_for(&client, &item.id, |item| item.state.label() == "dispatched").await;
    let QueueState::Dispatched {
        evidence, resolved, ..
    } = &sent.state
    else {
        panic!("{:?}", sent.state);
    };
    assert!(evidence.typed && evidence.submitted);
    assert_eq!(resolved.as_ref().unwrap().session_id, "s1");
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        *keys.lock().unwrap(),
        ["next step", "\r"],
        "typed once, then Enter, and never again"
    );

    tx.send(()).unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn a_second_item_waits_until_the_agent_reports_a_new_prompt() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("tend-queue-two.sock");
    let store = Store::shared();
    let id = agent_id("s1", "%1");
    working(&store, &id).await;
    idle(&store, &id).await;
    let (backend, keys) = queue_pane(Pane::Takes);
    let (tx, handle) =
        serve_queue(&sock, store.clone(), backend, WorkQueueStore::in_memory()).await;
    let client = Client::new(sock);

    let first = client
        .work_queue_add(&to_pane("%1", "first"))
        .await
        .unwrap();
    let second = client
        .work_queue_add(&to_pane("%1", "second"))
        .await
        .unwrap();
    wait_for(&client, &first.id, |item| {
        item.state.label() == "dispatched"
    })
    .await;
    // The fake backend fires no hook: no provider prompt since, so the
    // agent stays fenced, even when its state is repaired to Idle again.
    assert_eq!(
        wait_reason(&client, &second.id, WaitReason::Reserved).await,
        Some(WaitReason::Reserved)
    );
    idle(&store, &id).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(keys.lock().unwrap().len(), 2);

    // The provider reports the prompt and the turn ends.
    working(&store, &id).await;
    idle(&store, &id).await;
    wait_for(&client, &second.id, |item| {
        item.state.label() == "dispatched"
    })
    .await;
    assert_eq!(*keys.lock().unwrap(), ["first", "\r", "second", "\r"]);

    tx.send(()).unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn an_attempt_interrupted_by_a_restart_is_uncertain_and_not_retyped() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("work-queue.json");
    // What a tendd that died mid-send leaves on disk.
    let file = serde_json::json!({
        "version": 1,
        "items": [{
            "id": "q-1",
            "revision": 2,
            "prompt": "maybe sent",
            "target": {"kind": "agent", "session_id": "s1"},
            "created_by": {"kind": "operator"},
            "created_at": "2026-11-16T12:00:00Z",
            "updated_at": "2026-11-16T12:00:00Z",
            "state": {
                "state": "dispatching",
                "attempt": "a-1",
                "started_at": "2026-11-16T12:00:00Z",
                "resolved": {
                    "session_id": "s1",
                    "pane": "%1",
                    "idle_stamp": {"state_entered_at": "2026-11-16T11:59:00Z"}
                }
            }
        }],
        "fences": [{
            "session_id": "s1",
            "pane": "%1",
            "item_id": "q-1",
            "attempt": "a-1",
            "kind": "in_flight",
            "idle_stamp": {"state_entered_at": "2026-11-16T11:59:00Z"},
            "since": "2026-11-16T12:00:00Z"
        }]
    });
    std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
    let sock = dir.path().join("tend-queue-restart.sock");
    let store = Store::shared();
    let id = agent_id("s1", "%1");
    working(&store, &id).await;
    idle(&store, &id).await;
    let (backend, keys) = queue_pane(Pane::Takes);
    let (tx, handle) = serve_queue(
        &sock,
        store,
        backend,
        WorkQueueStore::load(Some(path.clone())),
    )
    .await;
    let client = Client::new(sock);

    let item = wait_for(&client, "q-1", |item| item.state.label() == "uncertain").await;
    assert_eq!(item.state.label(), "uncertain");
    // Another prompt for the same agent waits behind the unresolved fence.
    let later = client
        .work_queue_add(&to_pane("%1", "later"))
        .await
        .unwrap();
    assert_eq!(
        wait_reason(&client, &later.id, WaitReason::Reserved).await,
        Some(WaitReason::Reserved)
    );
    assert!(keys.lock().unwrap().is_empty(), "never retyped on its own");
    // The person must accept a possible duplicate to send it again.
    assert!(client.work_queue_run_now("q-1", None, false).await.is_err());
    let rerun = client.work_queue_run_now("q-1", None, true).await.unwrap();
    assert_eq!(rerun.state.label(), "dispatched");
    assert_eq!(*keys.lock().unwrap(), ["maybe sent", "\r"]);
    // And it is on disk.
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("\"dispatched\""));

    tx.send(()).unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn a_pane_that_refuses_before_typing_puts_the_item_back() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("tend-queue-refuse.sock");
    let store = Store::shared();
    let id = agent_id("s1", "%1");
    working(&store, &id).await;
    idle(&store, &id).await;
    let (backend, keys) = queue_pane(Pane::Refuses);
    let queue = WorkQueueStore::in_memory();
    let (tx, handle) = serve_queue(&sock, store, backend, queue.clone()).await;
    let client = Client::new(sock);

    let item = client
        .work_queue_add(&to_pane("%1", "hello"))
        .await
        .unwrap();
    let back = wait_for(&client, &item.id, |item| item.retry.send_failures >= 1).await;
    assert_eq!(back.state, QueueState::Waiting);
    assert!(back.retry.retry_after.is_some());
    assert!(keys.lock().unwrap().is_empty());
    assert!(queue.fences().await.is_empty(), "nothing reached the agent");

    tx.send(()).unwrap();
    handle.await.unwrap();
}

/// A send whose effect tendd cannot know: the text may be in the pane.
/// Uncertain, the agent fenced, never typed again on its own.
async fn uncertain_send(mode: Pane, reason: &str) {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("tend-queue-unknown.sock");
    let store = Store::shared();
    let id = agent_id("s1", "%1");
    working(&store, &id).await;
    idle(&store, &id).await;
    let (backend, keys) = queue_pane(mode);
    let queue = WorkQueueStore::in_memory();
    let (tx, handle) = serve_queue(&sock, store.clone(), backend, queue.clone()).await;
    let client = Client::new(sock);

    let item = client
        .work_queue_add(&to_pane("%1", "hello"))
        .await
        .unwrap();
    let uncertain = wait_for(&client, &item.id, |item| item.state.label() == "uncertain").await;
    let QueueState::Uncertain { reason: why, .. } = &uncertain.state else {
        unreachable!()
    };
    assert!(why.contains(reason), "{why}");
    let fences = queue.fences().await;
    assert_eq!(fences.len(), 1);
    assert_eq!(fences[0].kind, FenceKind::Unresolved);
    // Even a provider prompt and a finished turn do not release it.
    working(&store, &id).await;
    idle(&store, &id).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        keys.lock()
            .unwrap()
            .iter()
            .filter(|k| *k == "hello")
            .count(),
        1,
        "typed once"
    );
    assert_eq!(queue.fences().await.len(), 1);
    // Removing it needs the acknowledgement; then the agent is free.
    let refused = client
        .work_queue_remove(&item.id, false)
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("ack_duplicate_risk"), "{refused}");
    client.work_queue_remove(&item.id, true).await.unwrap();
    assert!(queue.fences().await.is_empty());

    tx.send(()).unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn a_paste_that_fails_after_reaching_the_pane_is_uncertain() {
    uncertain_send(Pane::PastesThenFails, "may have been typed").await;
}

#[tokio::test]
async fn a_backend_that_panics_after_the_paste_is_uncertain() {
    uncertain_send(Pane::PastesThenPanics, "may have been typed").await;
}

#[tokio::test]
async fn text_typed_without_a_confirmed_enter_is_uncertain() {
    uncertain_send(Pane::EnterFails, "Enter was not confirmed").await;
}

#[tokio::test]
async fn queue_requests_validate_and_report_their_errors() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("tend-queue-errors.sock");
    let (backend, _keys) = queue_pane(Pane::Takes);
    let (tx, handle) =
        serve_queue(&sock, Store::shared(), backend, WorkQueueStore::in_memory()).await;
    let client = Client::new(sock);

    let missing = client
        .work_queue_add(&to_pane("%9", "nobody"))
        .await
        .unwrap_err()
        .to_string();
    assert!(missing.contains("no running agent"), "{missing}");

    let no_cwd = client
        .work_queue_add(&new_work_request("k1", None))
        .await
        .unwrap_err()
        .to_string();
    assert!(no_cwd.contains("directory"), "{no_cwd}");

    let snapshot = client.work_queue_list(true).await.unwrap();
    assert!(snapshot.items.is_empty());
    let capabilities = client
        .hello(Duration::from_secs(2))
        .await
        .unwrap()
        .capabilities;
    for capability in [
        crate::work_queue::WORK_QUEUE_CAPABILITY,
        crate::pipeline_run::PIPELINE_QUEUE_LEASE_CAPABILITY,
    ] {
        assert!(capabilities.iter().any(|c| c == capability), "{capability}");
    }

    tx.send(()).unwrap();
    handle.await.unwrap();
}

// ---------------------------------------------------------------------------
// New Work
// ---------------------------------------------------------------------------

fn new_work_request(key: &str, cwd: Option<&str>) -> QueueAddRequest {
    QueueAddRequest {
        request_key: key.into(),
        title: None,
        prompt: "fix the login timeout".into(),
        target: QueueTargetSpec::NewWork {
            work: Some("login".into()),
            workspace: None,
            cwd: cwd.map(std::path::PathBuf::from),
            pipeline: None,
            external: None,
            skill: None,
            context: None,
        },
    }
}

/// What the fake `work up` does after taking its arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Launch {
    /// Registers under the lease, claims, and succeeds.
    Starts,
    /// Registers under the lease, then fails launching an agent.
    FailsAfterRegistering,
    /// Refuses at registration: the Work was started elsewhere first.
    FindsItStarted,
    /// Starts, then breaks the Run store's directory (a file where it
    /// was), so writing the lease release fails.
    StartsThenBreaksStore,
}

/// Stands in for the `tend work up` CLI against the server's Run store,
/// and tries a manual start of the same Work in the middle of it.
struct FakeWorkUp {
    runs: Arc<PipelineRunStore>,
    mode: Launch,
    manual: Mutex<Vec<String>>,
    /// The Run store's directory, for `StartsThenBreaksStore`.
    runs_dir: Option<std::path::PathBuf>,
}

fn arg<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|at| args.get(at + 1))
        .map(String::as_str)
}

impl RemoteWorkRunner for FakeWorkUp {
    fn host_mode<'a>(&'a self, _: &'a str) -> HostModeFuture<'a> {
        Box::pin(async { unreachable!("local runs never ask for a host mode") })
    }

    fn run<'a>(
        &'a self,
        _host: &'a str,
        args: Vec<String>,
        _stdin: Option<String>,
        _limits: WorkCommandLimits,
    ) -> RemoteWorkFuture<'a> {
        Box::pin(async move {
            let work = args[2].clone();
            let workspace = arg(&args, "--workspace").unwrap().to_string();
            let lease = arg(&args, "--queue-lease").map(str::to_string);
            assert!(args.iter().any(|a| a == "--new-only"));
            let identity = WorkIdentity::new(workspace.clone(), work.clone());
            let registration = PipelineRunRegistration {
                identity: identity.clone(),
                pipeline: "solo".into(),
                desired: vec![crate::pipeline::DesiredAgent {
                    alias: "impl".into(),
                    program: "codex".into(),
                    role: None,
                    task: None,
                    prompt: Some("go".into()),
                    options: Vec::new(),
                    direction: None,
                    after: Vec::new(),
                    purpose: crate::collaboration::LaunchPurpose::Primary,
                }],
                cwd: "/repo".into(),
                window_id: None,
                observed: Vec::new(),
                invalidate: Vec::new(),
            };
            // A person runs `tend work up` on the same id while the queue's
            // launch is under way: refused before anything changes.
            let manual = self.runs.register(registration.clone()).await;
            self.manual
                .lock()
                .unwrap()
                .push(manual.map_or_else(|e| e.to_string(), |_| "accepted".to_string()));
            let fail = |stderr: &str| WorkCommandOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: stderr.into(),
            };
            if self.mode == Launch::FindsItStarted {
                return Ok(fail(&format!(
                    "Error: register durable pipeline Run with tendd\n\nCaused by:\n    work_exists: Work {} already has a Run",
                    identity.key()
                )));
            }
            let run = self
                .runs
                .register_new(registration, lease.as_deref())
                .await
                .expect("the holder registers");
            let claims = self
                .runs
                .claim_ready(&identity, run.generation, lease.as_deref())
                .await
                .unwrap();
            assert_eq!(claims.len(), 1);
            if self.mode == Launch::FailsAfterRegistering {
                return Ok(fail("Error: launch codex: tmux split-window failed"));
            }
            if self.mode == Launch::StartsThenBreaksStore {
                let dir = self.runs_dir.as_ref().unwrap();
                std::fs::rename(dir, dir.with_extension("away")).unwrap();
                std::fs::write(dir, b"not a directory").unwrap();
            }
            Ok(WorkCommandOutput {
                exit_code: 0,
                stdout: serde_json::json!({"work": work, "workspace": workspace}).to_string(),
                stderr: String::new(),
            })
        })
    }
}

struct Launches {
    client: Client,
    runs: Arc<PipelineRunStore>,
    queue: Arc<WorkQueueStore>,
    fake: Arc<FakeWorkUp>,
    tx: broadcast::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
}

async fn serve_launches(dir: &Path, mode: Launch) -> Launches {
    serve_launches_on(dir, mode, PipelineRunStore::in_memory(), None).await
}

async fn serve_launches_on(
    dir: &Path,
    mode: Launch,
    runs: Arc<PipelineRunStore>,
    runs_dir: Option<std::path::PathBuf>,
) -> Launches {
    let sock = dir.join("tend-queue-launch.sock");
    let fake = Arc::new(FakeWorkUp {
        runs: runs.clone(),
        mode,
        manual: Mutex::new(Vec::new()),
        runs_dir,
    });
    let queue = WorkQueueStore::load(Some(dir.join("work-queue.json")));
    let (backend, _keys) = queue_pane(Pane::Takes);
    let mut server = Server::new(sock.clone(), Store::shared())
        .with_backends(vec![backend as SharedBackend])
        .with_pipeline_runs(runs.clone())
        .with_work_queue(queue.clone(), 3);
    server.work_up = super::super::work::WorkUpManager::with_local(sock.clone(), fake.clone());
    let (tx, rx) = broadcast::channel(1);
    let handle = tokio::spawn(async move { server.run(rx).await.unwrap() });
    wait_for_socket(&sock).await;
    Launches {
        client: Client::new(sock),
        runs,
        queue,
        fake,
        tx,
        handle,
    }
}

impl Launches {
    async fn stop(self) {
        self.tx.send(()).unwrap();
        self.handle.await.unwrap();
    }
}

fn identity_of(item: &QueueItem) -> WorkIdentity {
    item.target.new_work_identity().unwrap()
}

/// The lease once it reads `state` (the executor writes it on its next
/// pass), else panic.
async fn wait_lease(runs: &PipelineRunStore, identity: &WorkIdentity, state: QueueLeaseState) {
    for _ in 0..100 {
        if runs.lease(identity).await.map(|lease| lease.state) == Some(state) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("lease of {} never became {state:?}", identity.key());
}

#[tokio::test]
async fn a_queued_work_starts_under_its_lease_and_is_released_after() {
    let dir = tempdir().unwrap();
    let Launches {
        client,
        runs,
        fake,
        tx,
        handle,
        ..
    } = serve_launches(dir.path(), Launch::Starts).await;
    let item = client
        .work_queue_add(&new_work_request("k1", Some("/repo/webapp")))
        .await
        .unwrap();
    let identity = identity_of(&item);
    // Minted in the queue's namespace, the workspace fixed from the cwd.
    assert!(crate::pipeline_run::is_queue_work_id(&identity.work_id));
    assert!(
        identity.work_id.starts_with("Q-LOGIN-"),
        "{}",
        identity.work_id
    );
    assert_eq!(identity.workspace_id, "webapp");

    let done = wait_for(&client, &item.id, |item| item.state.label() == "dispatched").await;
    let QueueState::Dispatched { work, .. } = &done.state else {
        unreachable!()
    };
    assert_eq!(work.as_ref(), Some(&identity));
    // The manual start in the middle of the launch was refused.
    let manual = fake.manual.lock().unwrap().clone();
    assert_eq!(manual.len(), 1);
    assert!(manual[0].starts_with("work_reserved"), "{manual:?}");
    // Released: an ordinary Work from now on.
    wait_lease(&runs, &identity, QueueLeaseState::Released).await;

    tx.send(()).unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn a_launch_that_fails_after_registering_is_uncertain_and_suspended() {
    let dir = tempdir().unwrap();
    let Launches {
        client,
        runs,
        tx,
        handle,
        ..
    } = serve_launches(dir.path(), Launch::FailsAfterRegistering).await;
    let item = client
        .work_queue_add(&new_work_request("k1", Some("/repo/webapp")))
        .await
        .unwrap();
    let identity = identity_of(&item);
    let uncertain = wait_for(&client, &item.id, |item| item.state.label() == "uncertain").await;
    let QueueState::Uncertain { reason, .. } = &uncertain.state else {
        unreachable!()
    };
    assert!(
        reason.contains("may have started part of the Work"),
        "{reason}"
    );
    wait_lease(&runs, &identity, QueueLeaseState::Suspended).await;
    // Nothing reconciles it meanwhile: no claims, no manual registration.
    let run = runs.get(&identity).await.unwrap();
    for _ in 0..2 {
        assert!(runs
            .claim_ready(&identity, run.generation, None)
            .await
            .unwrap()
            .is_empty());
    }
    assert!(runs
        .register_new(
            PipelineRunRegistration {
                identity: identity.clone(),
                pipeline: run.pipeline.clone(),
                desired: run.desired.clone(),
                cwd: run.cwd.clone(),
                window_id: None,
                observed: Vec::new(),
                invalidate: Vec::new(),
            },
            None
        )
        .await
        .is_err());
    // Run-now needs the duplicate acknowledgement.
    assert!(client
        .work_queue_run_now(&item.id, None, false)
        .await
        .is_err());
    // The person checks and marks it started: released.
    client
        .work_queue_resolve(&item.id, true, false)
        .await
        .unwrap();
    wait_lease(&runs, &identity, QueueLeaseState::Released).await;

    tx.send(()).unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn a_work_started_elsewhere_first_fails_cleanly_and_is_final() {
    let dir = tempdir().unwrap();
    let Launches {
        client,
        runs,
        tx,
        handle,
        ..
    } = serve_launches(dir.path(), Launch::FindsItStarted).await;
    let item = client
        .work_queue_add(&new_work_request("k1", Some("/repo/webapp")))
        .await
        .unwrap();
    let failed = wait_for(&client, &item.id, |item| item.state.label() == "failed").await;
    let QueueState::Failed { reason, .. } = &failed.state else {
        unreachable!()
    };
    assert_eq!(*reason, crate::work_queue::FailReason::IdentityInUse);
    // Not rearmable: queue it again under a new id.
    let refused = client
        .work_queue_run_now(&item.id, None, true)
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("new Work id"), "{refused}");
    assert!(runs.lease(&identity_of(&item)).await.is_none());

    tx.send(()).unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn a_retried_new_work_add_returns_the_first_item() {
    let dir = tempdir().unwrap();
    let launches = serve_launches(dir.path(), Launch::Starts).await;
    let client = &launches.client;
    // A fresh Work id is minted per resolution; the retry still matches.
    let request = new_work_request("same-key", Some("/repo/webapp"));
    let (first, second) = tokio::join!(
        client.work_queue_add(&request),
        client.work_queue_add(&request)
    );
    let (first, second) = (first.unwrap(), second.unwrap());
    assert_eq!(first.id, second.id);
    let third = client.work_queue_add(&request).await.unwrap();
    assert_eq!(third.id, first.id);
    assert_eq!(identity_of(&third), identity_of(&first));
    // The same key for a different request is a conflict.
    let mut other = request.clone();
    other.prompt = "something else".into();
    let conflict = client.work_queue_add(&other).await.unwrap_err().to_string();
    assert!(conflict.contains("different item"), "{conflict}");
    launches.stop().await;
}

/// The release of a started Work's lease cannot be written: the item is
/// sent and may be cleared, but the release is kept in the queue file and
/// written once the Run store works again.
#[tokio::test]
async fn a_lease_release_that_failed_survives_clearing_and_is_written_later() {
    let dir = tempdir().unwrap();
    let runs_dir = dir.path().join("runs");
    let runs = PipelineRunStore::load(Some(runs_dir.join("pipeline-runs.json"))).unwrap();
    let launches = serve_launches_on(
        dir.path(),
        Launch::StartsThenBreaksStore,
        runs,
        Some(runs_dir.clone()),
    )
    .await;
    let client = &launches.client;
    let item = client
        .work_queue_add(&new_work_request("k1", Some("/repo/webapp")))
        .await
        .unwrap();
    let identity = identity_of(&item);
    wait_for(client, &item.id, |item| item.state.label() == "dispatched").await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        launches.runs.lease(&identity).await.unwrap().state,
        QueueLeaseState::Launching,
        "the release could not be written"
    );
    // Clearing the finished item keeps the pending release.
    assert_eq!(client.work_queue_clear().await.unwrap(), 1);
    assert_eq!(launches.queue.lease_intents().await.len(), 1);
    let on_disk = std::fs::read_to_string(dir.path().join("work-queue.json")).unwrap();
    assert!(on_disk.contains("lease_intents"), "{on_disk}");
    // Storage recovers: the next pass writes it and forgets it.
    std::fs::remove_file(&runs_dir).unwrap();
    std::fs::rename(runs_dir.with_extension("away"), &runs_dir).unwrap();
    client
        .work_queue_add(&to_pane("%404", "nudge a pass"))
        .await
        .ok();
    wait_lease(&launches.runs, &identity, QueueLeaseState::Released).await;
    for _ in 0..40 {
        if launches.queue.lease_intents().await.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(launches.queue.lease_intents().await.is_empty());
    launches.stop().await;
}

/// A Work target re-decided under the pane guard: another member got busy
/// while the delivery waited for the guard, so nothing is typed.
#[tokio::test]
async fn a_work_member_that_got_busy_while_waiting_for_the_guard_stops_the_send() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("tend-queue-work.sock");
    let store = Store::shared();
    let (impl_id, review_id) = (agent_id("s-impl", "%1"), agent_id("s-review", "%2"));
    for id in [&impl_id, &review_id] {
        working(&store, id).await;
        idle(&store, id).await;
    }
    let (backend, keys) = queue_pane_with(
        Pane::Takes,
        vec![
            work_pane("%1", "web", "W1", "impl"),
            work_pane("%2", "web", "W1", "review"),
        ],
    );
    let server = Server::new(sock.clone(), store.clone())
        .with_backends(vec![backend as SharedBackend])
        .with_work_queue(WorkQueueStore::in_memory(), 3);
    let collaboration = server.collaboration.clone();
    let (tx, rx) = broadcast::channel(1);
    let handle = tokio::spawn(async move { server.run(rx).await.unwrap() });
    wait_for_socket(&sock).await;
    let client = Client::new(sock);

    let held = collaboration.worker_action_guard("%1").await;
    let item = client
        .work_queue_add(&QueueAddRequest {
            request_key: "w1".into(),
            title: None,
            prompt: "please implement".into(),
            target: QueueTargetSpec::Work {
                workspace: "web".into(),
                work: "W1".into(),
                alias: Some("impl".into()),
            },
        })
        .await
        .unwrap();
    wait_for(&client, &item.id, |item| {
        item.state.label() == "dispatching"
    })
    .await;
    // The other member starts a turn while the send waits for the guard.
    working(&store, &review_id).await;
    drop(held);
    let back = wait_for(&client, &item.id, |item| item.state.label() == "waiting").await;
    assert!(back.retry.retry_after.is_some());
    assert!(keys.lock().unwrap().is_empty(), "nothing typed");
    // Once the whole Work is idle, it goes to the named member, once.
    idle(&store, &review_id).await;
    wait_for(&client, &item.id, |item| item.state.label() == "dispatched").await;
    assert_eq!(*keys.lock().unwrap(), ["please implement", "\r"]);

    tx.send(()).unwrap();
    handle.await.unwrap();
}
