//! What an `operate` device adds: device-only routes, each with its own
//! minimal answer. The operator routes they resemble stay as they are and
//! never accept a device token.
//!
//! `GET /api/device/fleet` is `/api/fleet` cut down to what the app shows:
//! the same JSON shape, so one decoder reads both, without SSH targets,
//! labels, annotations, capabilities, backends or panes without an agent.
//!
//! `POST /api/device/capture {host, session_id}` is the recent output of the
//! pane of one agent in the current snapshot. The server resolves the pane
//! from the snapshot (a pane without an agent cannot be named) and runs
//! exactly `FleetOperation::Capture` through the Fleet manager, so the host
//! lookup and `[fleet] capture_policy` are the operators' own. The answer is
//! the sanitized text only, bounded; one capture per device at a time; each
//! one, refused or not, is an audit line.
use super::DeviceAuth;
use crate::dashboard::server::AppState;
use crate::fleet::FleetOperation;
use crate::work_control::{WorkUpError, WorkUpRequest};
use axum::{
    body::Bytes,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    Extension,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::time::Duration;

/// Most lines and bytes of output a capture answers with.
const CAPTURE_LINES: usize = 400;
const CAPTURE_BYTES: usize = 64 * 1024;
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(10);

/// A host's fields the app reads.
const HOST_FIELDS: &[&str] = &[
    "alias",
    "local",
    "mode",
    "state",
    "tend_version",
    "error",
    "last_seen_at",
];

/// An agent's fields the app reads.
const AGENT_FIELDS: &[&str] = &[
    "kind",
    "agent_session_id",
    "pane",
    "tmux_session",
    "cwd",
    "state",
    "awaiting_reply",
    "last_prompt",
    "last_response",
    "last_notification",
    "recap",
    "ai_title",
    "model",
    "started_at",
    "state_entered_at",
    "last_activity_at",
];

/// Longest text field passed on, in bytes; the app shows less.
const MAX_TEXT_BYTES: usize = 16 * 1024;

/// `awaiting_reply`'s fields the app reads.
const AWAITING_FIELDS: &[&str] = &["question", "since"];

/// A selected value: a string capped at [`MAX_TEXT_BYTES`]; an object only
/// through its own field list (`awaiting_reply`); anything else as is
/// when it is a scalar, dropped when it is a structure nobody listed.
fn bounded(field: &str, value: &Value) -> Option<Value> {
    match value {
        Value::String(text) if text.len() > MAX_TEXT_BYTES => {
            let mut end = MAX_TEXT_BYTES;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            Some(Value::String(format!("{}…", &text[..end])))
        }
        Value::Object(inner) if field == "awaiting_reply" => {
            Some(Value::Object(pick(inner, AWAITING_FIELDS)))
        }
        Value::Object(_) | Value::Array(_) => None,
        other => Some(other.clone()),
    }
}

fn pick(source: &Map<String, Value>, fields: &[&str]) -> Map<String, Value> {
    let mut out = Map::new();
    for field in fields {
        if let Some(value) = source.get(*field).and_then(|value| bounded(field, value)) {
            out.insert((*field).to_owned(), value);
        }
    }
    out
}

/// The snapshot as a device sees it.
pub(super) fn cut(snapshot: &Value) -> Value {
    let hosts = snapshot
        .get("hosts")
        .and_then(Value::as_array)
        .map(|hosts| {
            hosts
                .iter()
                .filter_map(Value::as_object)
                .map(|host| {
                    let mut out = pick(host, HOST_FIELDS);
                    let agents: Vec<Value> = host
                        .get("remote")
                        .and_then(|remote| remote.get("agents"))
                        .and_then(Value::as_array)
                        .map(|agents| {
                            agents
                                .iter()
                                .filter_map(Value::as_object)
                                .map(|agent| Value::Object(pick(agent, AGENT_FIELDS)))
                                .collect()
                        })
                        .unwrap_or_default();
                    out.insert("remote".into(), json!({ "agents": agents }));
                    Value::Object(out)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json!({ "generated_at": snapshot.get("generated_at"), "hosts": hosts })
}

pub(super) async fn fleet(State(state): State<AppState>) -> Response {
    let Some(fleet) = &state.fleet else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "fleet manager is not enabled" })),
        )
            .into_response();
    };
    let snapshot = fleet.store.snapshot_selected(None).await;
    match serde_json::to_value(&snapshot) {
        Ok(value) => Json(cut(&value)).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

fn refuse(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CaptureBody {
    host: String,
    session_id: String,
}

/// The last lines of `text`, at most [`CAPTURE_LINES`] and
/// [`CAPTURE_BYTES`], cut at a line or character boundary.
pub(super) fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.trim_end_matches('\n').lines().collect();
    let mut out = lines[lines.len().saturating_sub(CAPTURE_LINES)..].join("\n");
    if out.len() > CAPTURE_BYTES {
        let mut start = out.len() - CAPTURE_BYTES;
        while !out.is_char_boundary(start) {
            start += 1;
        }
        out = out[start..].to_owned();
        if let Some(newline) = out.find('\n') {
            out = out[newline + 1..].to_owned();
        }
    }
    out
}

/// The one pane that is this agent's. With a socket, the pane on that
/// endpoint (compared as the backend identifies endpoints: a tmux short
/// name and its full path are one server; rmux and cmux keep full paths).
/// Without one, a pane id that exists once. Anything else is ambiguous.
pub(super) fn agent_pane<'a>(
    panes: &'a [crate::tmux::PaneInfo],
    pane_id: &str,
    socket: Option<&str>,
) -> Option<&'a crate::tmux::PaneInfo> {
    let mut candidates = panes.iter().filter(|pane| {
        pane.pane_id == pane_id
            && match socket {
                Some(socket) => pane.socket.as_deref().is_some_and(|observed| {
                    crate::backend::pane_endpoints_match(Some(pane_id), socket, observed)
                }),
                None => true,
            }
    });
    let pane = candidates.next()?;
    candidates.next().is_none().then_some(pane)
}

/// Releases a device's capture slot when the capture ends.
struct Slot<'a>(&'a super::Devices, String);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.0.end_capture(&self.1);
    }
}

/// Who a device line names: the label, the device and who approved it.
pub(super) fn device_actor(device: &DeviceAuth) -> Value {
    json!({
        "by": super::Device::label(&device.name, &device.id),
        "account": { "credential": "device", "device": device.id, "approved_by": device.approved_by },
        "scope": device.scope,
    })
}

pub(super) async fn capture(
    State(state): State<AppState>,
    Extension(device): Extension<DeviceAuth>,
    body: Result<Bytes, axum::extract::rejection::BytesRejection>,
) -> Response {
    // A body refused before it was read (too large) is an attempt too.
    let parsed = match &body {
        Ok(bytes) => {
            serde_json::from_slice::<CaptureBody>(bytes).map_err(|_| StatusCode::BAD_REQUEST)
        }
        Err(rejection) => Err(rejection.status()),
    };
    let outcome = match &parsed {
        Ok(body) => capture_for(&state, &device, body).await,
        Err(status) => Err((
            *status,
            "expected {\"host\": \"...\", \"session_id\": \"...\"}".to_owned(),
        )),
    };
    let (host, session) = parsed.as_ref().map_or((None, None), |body| {
        (Some(&body.host), Some(&body.session_id))
    });
    let status = match &outcome {
        Ok(_) => StatusCode::OK,
        Err((status, _)) => *status,
    };
    let mut entry = device_actor(&device);
    entry["action"] = json!("device_capture");
    entry["host"] = json!(host);
    entry["session_id"] = json!(session);
    entry["status"] = json!(status.as_u16());
    super::routes::audit_entry(&state, entry).await;
    match outcome {
        Ok(text) => Json(json!({ "capture": text })).into_response(),
        Err((status, message)) => refuse(status, &message),
    }
}

async fn capture_for(
    state: &AppState,
    device: &DeviceAuth,
    body: &CaptureBody,
) -> Result<String, (StatusCode, String)> {
    let Some(fleet) = &state.fleet else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "fleet manager is not enabled".into(),
        ));
    };
    let snapshot = fleet.store.snapshot_selected(None).await;
    let not_found = || {
        (
            StatusCode::NOT_FOUND,
            "no agent with that session on that host".to_owned(),
        )
    };
    let host = snapshot
        .hosts
        .iter()
        .find(|host| host.alias == body.host)
        .ok_or_else(not_found)?;
    let remote = host.remote.as_ref().ok_or_else(not_found)?;
    let agent = remote
        .agents
        .iter()
        .find(|agent| !body.session_id.is_empty() && agent.session_id == body.session_id)
        .ok_or_else(not_found)?;
    let pane_id = agent.pane.as_deref().ok_or_else(not_found)?;
    let pane = agent_pane(&remote.panes, pane_id, agent.tmux_socket.as_deref()).ok_or((
        StatusCode::CONFLICT,
        "this agent's pane is not in the host's latest snapshot, or cannot be told apart from another; try again shortly, or read it on a computer".to_owned(),
    ))?;
    let kind = crate::backend::pane_id_host_kind(&pane.pane_id).ok_or_else(not_found)?;
    let key = crate::topology::PaneKey::from_pane(kind, pane);
    let Some(_slot) = state
        .devices
        .begin_capture(&device.id)
        .then(|| Slot(&state.devices, device.id.clone()))
    else {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "this device is already reading output; try again in a moment".into(),
        ));
    };
    // Recorded before the host is asked, so an interrupted capture still
    // leaves a line naming the device and the agent.
    let mut intent = device_actor(device);
    intent["action"] = json!("device_capture_requested");
    intent["host"] = json!(host.alias);
    intent["session_id"] = json!(body.session_id);
    intent["pane"] = json!(pane.pane_id);
    super::routes::audit_entry(state, intent).await;
    let result = fleet
        .execute(
            host.alias.clone(),
            FleetOperation::Capture { pane: key },
            CAPTURE_TIMEOUT,
        )
        .await
        .map_err(|error| (StatusCode::CONFLICT, error))?;
    if !result.accepted {
        return Err((
            StatusCode::CONFLICT,
            result
                .message
                .unwrap_or_else(|| "the host refused the capture".into()),
        ));
    }
    Ok(tail(result.capture.as_deref().unwrap_or("")))
}

// ---------------------------------------------------------------------------
// Starting new work.
//
// `GET /api/device/work-options?host=` and `POST /api/device/work`. The
// device names a host, one of the projects listed for it, one of its
// pipelines, and what to do. The server mints the Work id (`phone-…`), so a
// device can never reach an existing Work, and also launches with
// `new_only`, which tendd refuses atomically for an id that exists. The
// `WorkUpRequest` is built here with nothing the device did not choose
// from a list: no workspace, skill, context, ticket or dry run. A start is
// reserved durably before dispatch, audited before and after, and never
// dispatched twice for one key.

/// Longest request text a device may send, in bytes.
const MAX_REQUEST_BYTES: usize = 8 * 1024;
/// Most project folders offered per host.
const MAX_PROJECTS: usize = 12;
const OPTIONS_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OptionsQuery {
    host: String,
}

/// What `tend work options --json` says, as far as a device start reads it.
#[derive(Deserialize, Default)]
struct HostOptions {
    #[serde(default)]
    pipelines: Vec<OptionPipeline>,
    #[serde(default)]
    routes: Vec<OptionRoute>,
}

#[derive(Deserialize)]
struct OptionPipeline {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    agents: Vec<Value>,
}

#[derive(Deserialize)]
struct OptionRoute {
    #[serde(rename = "match", default)]
    pattern: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    worktree: Value,
    #[serde(default)]
    prepare: Value,
}

impl OptionRoute {
    /// A route that would choose the agents' folder itself.
    fn moves_the_agents(&self) -> bool {
        let set = |value: &Value| !matches!(value, Value::Null | Value::Bool(false));
        self.cwd.as_deref().is_some_and(|cwd| !cwd.is_empty())
            || set(&self.worktree)
            || set(&self.prepare)
    }

    fn matches(&self, work: &str) -> bool {
        regex::RegexBuilder::new(&self.pattern)
            .case_insensitive(true)
            .build()
            // A pattern tend could not use matches nothing there either.
            .is_ok_and(|pattern| pattern.is_match(work))
    }
}

/// A host a device may start work on: in the snapshot, this one or a
/// control-mode Fleet host, online.
struct Target {
    alias: String,
    local: bool,
    projects: Vec<String>,
}

fn target(
    snapshot: &crate::fleet::FleetSnapshot,
    alias: &str,
) -> Result<Target, (StatusCode, String)> {
    let host = snapshot
        .hosts
        .iter()
        .find(|host| host.alias == alias)
        .ok_or((StatusCode::NOT_FOUND, "no such host".to_owned()))?;
    if !host.local
        && (host.mode != crate::fleet::HostAccessMode::Control
            || host.state != crate::fleet::FleetHostState::Online)
    {
        return Err((
            StatusCode::CONFLICT,
            "work can start only on a control-mode host that is online".into(),
        ));
    }
    // Folders agents on that host work in, newest first.
    let mut agents: Vec<_> = host
        .remote
        .as_ref()
        .map(|remote| remote.agents.iter().filter(|a| a.pane.is_some()).collect())
        .unwrap_or_default();
    agents.sort_by_key(|agent| std::cmp::Reverse(agent.last_activity_at));
    let mut projects = Vec::new();
    for cwd in agents.iter().filter_map(|agent| agent.cwd.as_deref()) {
        let absolute = cwd.starts_with('/') || cwd.get(1..3) == Some(":\\");
        if absolute && !cwd.contains("..") && !projects.iter().any(|p: &String| p == cwd) {
            projects.push(cwd.to_owned());
        }
        if projects.len() == MAX_PROJECTS {
            break;
        }
    }
    Ok(Target {
        alias: host.alias.clone(),
        local: host.local,
        projects,
    })
}

/// `tend work options --json` on that host, exactly: no stdin, no other
/// argument.
async fn host_options(
    fleet: &crate::fleet::FleetRuntime,
    alias: &str,
) -> Result<HostOptions, (StatusCode, String)> {
    let result = fleet
        .execute(
            alias.to_owned(),
            FleetOperation::WorkCommand {
                args: vec!["work".into(), "options".into(), "--json".into()],
                stdin: None,
            },
            OPTIONS_TIMEOUT,
        )
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error))?;
    if result.command_exit_code != Some(0) {
        return Err((
            StatusCode::BAD_GATEWAY,
            "the host could not list its work options".into(),
        ));
    }
    serde_json::from_str(result.command_stdout.as_deref().unwrap_or("")).map_err(|_| {
        (
            StatusCode::BAD_GATEWAY,
            "the host's work options could not be read".into(),
        )
    })
}

pub(super) async fn work_options(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<OptionsQuery>,
) -> Response {
    let Some(fleet) = &state.fleet else {
        return refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            "fleet manager is not enabled",
        );
    };
    let snapshot = fleet.store.snapshot_selected(None).await;
    let target = match target(&snapshot, &query.host) {
        Ok(target) => target,
        Err((status, message)) => return refuse(status, &message),
    };
    let options = match host_options(fleet, &target.alias).await {
        Ok(options) => options,
        Err((status, message)) => return refuse(status, &message),
    };
    Json(json!({
        "allow_work_start": state.config.allow_work_start,
        "pipelines": options.pipelines.iter().map(|p| json!({
            "name": p.name, "description": p.description, "agents": p.agents.len(),
        })).collect::<Vec<_>>(),
        "projects": target.projects,
    }))
    .into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkBody {
    idempotency_key: String,
    host: String,
    project: String,
    pipeline: String,
    request: String,
}

/// The Work id a device start gets: never one the device chose.
fn mint_work_id() -> String {
    let date = time::OffsetDateTime::now_utc();
    let random = uuid::Uuid::new_v4().simple().to_string();
    format!(
        "phone-{:02}{:02}-{}",
        u8::from(date.month()),
        date.day(),
        &random[..12]
    )
}

fn fingerprint(body: &WorkBody) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    for part in [&body.host, &body.project, &body.pipeline, &body.request] {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    hash.finalize().to_vec()
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn started_answer(start: &super::Start) -> Response {
    let status = match start.state.as_str() {
        "reserved" => StatusCode::ACCEPTED,
        _ => StatusCode::OK,
    };
    (
        status,
        Json(json!({ "state": start.state, "work": start.work, "detail": start.detail })),
    )
        .into_response()
}

pub(super) async fn work(
    State(state): State<AppState>,
    Extension(device): Extension<DeviceAuth>,
    body: Result<Bytes, axum::extract::rejection::BytesRejection>,
) -> Response {
    let who = device_actor(&device);
    let audit_refusal = |status: StatusCode, message: &str, host: Option<&str>| {
        let mut entry = who.clone();
        entry["action"] = json!("device_work_refused");
        entry["host"] = json!(host);
        entry["reason"] = json!(message);
        entry["status"] = json!(status.as_u16());
        entry
    };
    let parsed = match &body {
        Ok(bytes) => serde_json::from_slice::<WorkBody>(bytes).map_err(|_| StatusCode::BAD_REQUEST),
        Err(rejection) => Err(rejection.status()),
    };
    let body = match parsed {
        Ok(body) => body,
        Err(status) => {
            let message = "expected {idempotency_key, host, project, pipeline, request}";
            super::routes::audit_entry(&state, audit_refusal(status, message, None)).await;
            return refuse(status, message);
        }
    };

    // A key already used answers with what happened then, whatever today's
    // settings are: the caller needs the outcome most when it is unsure.
    let print = fingerprint(&body);
    if uuid::Uuid::parse_str(&body.idempotency_key).is_ok() {
        match state.devices.find_start(&device.id, &body.idempotency_key) {
            Some(start) if start.fingerprint == print => return started_answer(&start),
            Some(_) => {
                return refuse(
                    StatusCode::CONFLICT,
                    "this idempotency_key was used for another request",
                )
            }
            None => {}
        }
    }
    // The one id this start can launch, checked against the routes as is.
    let work = mint_work_id();
    match prepare(&state, &body, &work).await {
        Err((status, message)) => {
            super::routes::audit_entry(&state, audit_refusal(status, &message, Some(&body.host)))
                .await;
            refuse(status, &message)
        }
        Ok((fleet_target, pipeline)) => {
            dispatch(
                &state,
                &device,
                who,
                body,
                fleet_target,
                pipeline,
                work,
                print,
            )
            .await
        }
    }
}

/// Every check before a reservation: the gate, the body, the host, the
/// pipeline, the project and the routes.
async fn prepare(
    state: &AppState,
    body: &WorkBody,
    work: &str,
) -> Result<(Target, String), (StatusCode, String)> {
    if !state.config.allow_work_start {
        return Err((
            StatusCode::NOT_IMPLEMENTED,
            "starting work is disabled; set [dashboard] allow_work_start = true to enable it"
                .into(),
        ));
    }
    if uuid::Uuid::parse_str(&body.idempotency_key).is_err() {
        return Err((
            StatusCode::BAD_REQUEST,
            "idempotency_key must be a UUID".into(),
        ));
    }
    let request = body.request.trim();
    if request.is_empty() || body.request.len() > MAX_REQUEST_BYTES {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("request must be 1 to {MAX_REQUEST_BYTES} bytes"),
        ));
    }
    let Some(fleet) = &state.fleet else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "fleet manager is not enabled".into(),
        ));
    };
    let snapshot = fleet.store.snapshot_selected(None).await;
    let target = target(&snapshot, &body.host)?;
    if !target.projects.contains(&body.project) {
        return Err((
            StatusCode::BAD_REQUEST,
            "project must be one of the folders listed for this host".into(),
        ));
    }
    let options = host_options(fleet, &target.alias).await?;
    if !options.pipelines.iter().any(|p| p.name == body.pipeline) {
        return Err((
            StatusCode::BAD_REQUEST,
            "pipeline must be one of the host's pipelines".into(),
        ));
    }
    // A route that would match this start's own id and choose the folder
    // itself refuses the start.
    if options
        .routes
        .iter()
        .any(|route| route.moves_the_agents() && route.matches(work))
    {
        return Err((
            StatusCode::CONFLICT,
            "a [[route]] on this host would choose the agents' folder for phone starts; start this one from a computer".into(),
        ));
    }
    Ok((target, body.pipeline.clone()))
}

#[allow(clippy::too_many_arguments)] // one start, every part named
async fn dispatch(
    state: &AppState,
    device: &DeviceAuth,
    who: Value,
    body: WorkBody,
    target: Target,
    pipeline: String,
    work: String,
    print: Vec<u8>,
) -> Response {
    let reservation =
        match state
            .devices
            .reserve_start(&device.id, &body.idempotency_key, &print, &work)
        {
            Ok(reservation) => reservation,
            Err(error) => {
                tracing::warn!(%error, "device start could not be reserved");
                return refuse(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "the device store is unavailable; nothing was started",
                );
            }
        };
    let start = match reservation {
        super::Reservation::New(start) => start,
        super::Reservation::Replay(start) => return started_answer(&start),
        super::Reservation::Conflict => {
            return refuse(
                StatusCode::CONFLICT,
                "this idempotency_key was used for another request",
            )
        }
        super::Reservation::Busy => {
            return refuse(
                StatusCode::TOO_MANY_REQUESTS,
                "this device is already starting work",
            )
        }
        super::Reservation::RateLimited => {
            return refuse(
                StatusCode::TOO_MANY_REQUESTS,
                "this device started too much work in the last hour",
            )
        }
    };
    let mut intent = who.clone();
    intent["action"] = json!("device_work_requested");
    intent["idempotency_key"] = json!(body.idempotency_key);
    intent["fingerprint"] = json!(hex(&print));
    intent["host"] = json!(target.alias);
    intent["project"] = json!(body.project);
    intent["pipeline"] = json!(pipeline);
    intent["work"] = json!(work);
    intent["request_bytes"] = json!(body.request.len());
    intent["at"] = json!(time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .ok());
    if !crate::dashboard::decisions::try_audit(state.decision_audit.as_ref(), intent).await {
        state.devices.finish_start(
            &device.id,
            &body.idempotency_key,
            "refused",
            Some("audit unavailable"),
        );
        return refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            "the audit could not be written; nothing was started",
        );
    }
    run_and_record(state, &device.id, who, &body, &target, pipeline, start).await
}

/// Run the reserved start and record how it ended, in the store and the
/// audit.
async fn run_and_record(
    state: &AppState,
    device: &str,
    who: Value,
    body: &WorkBody,
    target: &Target,
    pipeline: String,
    start: super::Start,
) -> Response {
    let work = start.work.clone();
    // Every field, so a new one is a decision here, not a default.
    let request = WorkUpRequest {
        work: work.clone(),
        external: None,
        pipeline: Some(pipeline),
        workspace: None,
        cwd: Some(std::path::PathBuf::from(&body.project)),
        skill: None,
        body: Some(body.request.trim().to_owned()),
        context: None,
        no_ticket: true,
        dry_run: false,
        new_only: true,
        queue_lease: None,
        route_key: None,
        host: (!target.local).then(|| target.alias.clone()),
    };
    let outcome = match &state.device_work_up {
        Some(run) => run(request).await,
        None => crate::work_control::execute_work_up(&request, None).await,
    };
    // Only a request refused before anything ran is "failed". Any other
    // error can come after the Run was registered or an agent launched (a
    // nonzero exit, a lost answer): it may have started.
    let (end, detail) = match &outcome {
        Ok(_) => ("started", None),
        Err(WorkUpError::Invalid(message)) => ("failed", Some(message.clone())),
        Err(error) => ("unknown", Some(error.to_string())),
    };
    state
        .devices
        .finish_start(device, &body.idempotency_key, end, detail.as_deref());
    let mut result = who;
    result["action"] = json!("device_work_finished");
    result["idempotency_key"] = json!(body.idempotency_key);
    result["host"] = json!(target.alias);
    result["work"] = json!(work);
    result["state"] = json!(end);
    result["detail"] = json!(detail);
    super::routes::audit_entry(state, result).await;
    started_answer(&super::Start {
        state: end.into(),
        detail,
        ..start
    })
}

/// A unit's fields an `operate` device reads: no pane ids or sockets (a
/// capture names the agent by host and session).
const UNIT_FIELDS: &[&str] = &[
    "id",
    "source",
    "title",
    "repo",
    "lane",
    "stages",
    "headline",
    "since",
    "last_change",
    "shown_time",
    "host_aliases",
    "agent_kinds",
    "handled_limit",
    "orphan_helper",
    "probable_helper",
    "empty_session",
    "recent",
    "reviewed",
    "requested_by_person",
];

/// An agent reference's fields a device reads.
const AGENT_REF_FIELDS: &[&str] = &[
    "host",
    "local",
    "session_id",
    "kind",
    "name",
    "owner",
    "state",
    "status",
    "needs_attention",
    "state_entered_at",
];

/// A backlog record's fields a device reads (no ticket reference).
const BACKLOG_FIELDS: &[&str] = &[
    "workspace",
    "work",
    "title",
    "line",
    "priority",
    "pipeline",
    "project",
];

/// A Text (a string, or `{key, …}` the app words) or a list of stages:
/// passed through when small, strings capped like every other field.
fn bounded_tree(value: &Value) -> Value {
    match value {
        Value::String(_) => bounded("", value).unwrap_or(Value::Null),
        Value::Array(items) => Value::Array(items.iter().map(bounded_tree).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), bounded_tree(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn agent_ref(value: &Value) -> Value {
    value.as_object().map_or(Value::Null, |agent| {
        let mut out = Map::new();
        for field in AGENT_REF_FIELDS {
            if let Some(v) = agent.get(*field) {
                out.insert((*field).to_owned(), bounded_tree(v));
            }
        }
        Value::Object(out)
    })
}

fn device_unit(item: &Map<String, Value>) -> Value {
    let mut out = Map::new();
    out.insert("kind".into(), json!("unit"));
    for field in UNIT_FIELDS {
        if let Some(v) = item.get(*field) {
            out.insert((*field).to_owned(), bounded_tree(v));
        }
    }
    for list in ["agents", "helpers"] {
        let refs: Vec<Value> = item
            .get(list)
            .and_then(Value::as_array)
            .map(|agents| agents.iter().map(agent_ref).collect())
            .unwrap_or_default();
        out.insert(list.into(), Value::Array(refs));
    }
    out.insert(
        "focus".into(),
        item.get("focus").map_or(Value::Null, agent_ref),
    );
    Value::Object(out)
}

/// A decision exactly as the queue lists it (the shared decoder reads it,
/// and the device decides what it read, never a cut copy; the queue bounds
/// its own entries as `/api/decisions` serves them), plus the host it is
/// on, whether it can be decided from here, and its project.
fn device_decision(item: &Map<String, Value>) -> Value {
    let mut entry = item
        .get("entry")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for field in ["host", "actionable", "repo", "system"] {
        if let Some(v) = item.get(field) {
            entry.insert(field.to_owned(), bounded_tree(v));
        }
    }
    Value::Object(entry)
}

/// The board as an `operate` device sees it: units with only the fields
/// the app shows and no pane ids or sockets in their agent references, decisions as the queue lists
/// them (`kind` `human_request` / `review`), units as `kind: unit`, and the
/// decided items from the history for Finished.
pub(super) fn device_board(board: &Value, history: Option<&Value>) -> Value {
    let lane = |name: &str| {
        let groups: Vec<Value> = board
            .pointer(&format!("/lanes/{name}/groups"))
            .and_then(Value::as_array)
            .map(|groups| {
                groups
                    .iter()
                    .map(|group| {
                        let items: Vec<Value> = group
                            .get("items")
                            .and_then(Value::as_array)
                            .map(|items| {
                                items
                                    .iter()
                                    .filter_map(Value::as_object)
                                    .map(|item| match item.get("type").and_then(Value::as_str) {
                                        Some("decision") => device_decision(item),
                                        _ => device_unit(item),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        json!({"project": group.get("project"), "system": group.get("system"), "items": items})
                    })
                    .collect()
            })
            .unwrap_or_default();
        json!({"count": board.pointer(&format!("/lanes/{name}/count")), "groups": groups})
    };
    let hosts: Vec<Value> = board
        .get("hosts")
        .and_then(Value::as_array)
        .map(|hosts| {
            hosts
                .iter()
                .filter_map(Value::as_object)
                .map(|host| {
                    Value::Object(pick(
                        host,
                        &["alias", "local", "state", "error", "mode", "tend_version"],
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let decided: Vec<Value> = history
        .and_then(|h| h.get("hosts"))
        .and_then(Value::as_array)
        .map(|hosts| {
            hosts
                .iter()
                .map(|host| {
                    json!({
                        "alias": host.get("alias"),
                        "local": host.get("local"),
                        "unavailable": host.get("unavailable"),
                        // More exist than this page.
                        "truncated": host.pointer("/history/truncated"),
                        // The history's own items, as /api/decisions/history
                        // serves them.
                        "items": host.pointer("/history/items"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    // Planned Work, to look at: a device cannot start one (that would name
    // an existing Work, which only the operator routes do).
    let backlog: Vec<Value> = board
        .get("backlog")
        .and_then(Value::as_array)
        .map(|records| {
            records
                .iter()
                .filter_map(Value::as_object)
                .map(|record| Value::Object(pick(record, BACKLOG_FIELDS)))
                .collect()
        })
        .unwrap_or_default();
    json!({
        "generated_at": board.get("generated_at"),
        "hosts": hosts,
        "backlog": backlog,
        "lanes": {"needs_you": lane("needs_you"), "running": lane("running"), "finished": lane("finished")},
        "decided": decided,
        "decision_hosts": board.get("decision_hosts").map(bounded_tree),
        "agent_count": board.get("agent_count"),
        // Numbers only (apps/shared/Design/COUNTS.md): the lanes above may
        // be cut to fit, these never are.
        "counts": board.get("counts"),
    })
}

/// `GET /api/device/home`: Home's board for an `operate` device.
pub(super) async fn home(State(state): State<AppState>) -> Response {
    let (board, history) = tokio::join!(
        crate::dashboard::home::shared_board(&state, 100),
        crate::dashboard::decisions::history_json(&state, 100, false)
    );
    Json(device_board(&board, Some(&history))).into_response()
}
