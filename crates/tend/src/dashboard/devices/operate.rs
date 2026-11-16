//! What an `operate` device adds: device-only routes, each with its own
//! minimal answer. The operator routes they resemble stay as they are and
//! never accept a device token.
//!
//! `GET /api/device/fleet` is `/api/fleet` cut down to what the app shows:
//! the same JSON shape, so one decoder reads both, without SSH targets,
//! labels, annotations, capabilities, backends or panes without an agent.
use crate::dashboard::server::AppState;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde_json::{json, Map, Value};

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

fn pick(source: &Map<String, Value>, fields: &[&str]) -> Map<String, Value> {
    let mut out = Map::new();
    for field in fields {
        if let Some(value) = source.get(*field) {
            let value = match value {
                Value::String(text) if text.len() > MAX_TEXT_BYTES => {
                    let mut end = MAX_TEXT_BYTES;
                    while !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    Value::String(format!("{}…", &text[..end]))
                }
                other => other.clone(),
            };
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
