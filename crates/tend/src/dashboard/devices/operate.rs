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
struct CaptureBody {
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

/// Releases a device's capture slot when the capture ends.
struct Slot<'a>(&'a super::Devices, String);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.0.end_capture(&self.1);
    }
}

pub(super) async fn capture(
    State(state): State<AppState>,
    Extension(device): Extension<DeviceAuth>,
    body: Bytes,
) -> Response {
    let parsed = serde_json::from_slice::<CaptureBody>(&body);
    let outcome = match &parsed {
        Ok(body) => capture_for(&state, &device, body).await,
        Err(_) => Err((
            StatusCode::BAD_REQUEST,
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
    super::routes::audit_entry(
        &state,
        json!({
            "by": super::Device::label(&device.name, &device.id),
            "account": { "credential": "device", "device": device.id, "approved_by": device.approved_by },
            "action": "device_capture", "scope": device.scope,
            "host": host, "session_id": session, "status": status.as_u16(),
        }),
    )
    .await;
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
    let pane = remote
        .panes
        .iter()
        .find(|pane| {
            pane.pane_id == pane_id
                && (agent.tmux_socket.is_none() || pane.socket == agent.tmux_socket)
        })
        .ok_or_else(not_found)?;
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
