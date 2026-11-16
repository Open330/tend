//! Home's board on the web (`GET /api/home`) and its "Looks Good" mark
//! (`POST /api/home/reviewed`): the board tendd computes once for every
//! client ([`crate::home_board`]), from the Fleet snapshot, this daemon's
//! pipeline runs, automation rules and review records, and the decision
//! queue `/api/decisions` serves.
//!
//! # Authorization
//!
//! Both are operator-only, like pane output and the decision queue: the
//! board carries what agents are working on and what they asked. The read
//! is a GET behind the operator gate (a viewer gets 403, an anonymous
//! visitor 401); the mark is a control write, so a cookie session also
//! sends the CSRF proof. A paired device (which may only decide) reaches
//! neither.

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
};
use serde::Deserialize;
use serde_json::{json, Value};

use super::server::AppState;
use crate::home_board::{self, BoardHost};

const DEFAULT_LIMIT: usize = 100;
const MAX_SUMMARY_BYTES: usize = 1024;

#[derive(Debug, Deserialize)]
pub(super) struct HomeQuery {
    #[serde(default)]
    limit: Option<usize>,
}

/// Every host's agents and panes: the Fleet snapshot when Fleet runs (it
/// always lists this host), else this host from its own registry.
async fn board_hosts(state: &AppState) -> Vec<BoardHost> {
    if let Some(fleet) = &state.fleet {
        let snapshot = fleet.store.snapshot_selected(None).await;
        if let Ok(value) = serde_json::to_value(&snapshot) {
            let hosts: Vec<BoardHost> = value
                .get("hosts")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(home_board::host_from_snapshot)
                .collect();
            if !hosts.is_empty() {
                return hosts;
            }
        }
    }
    let agents = state.store.snapshot().await;
    let scan = state.refresh_pane_scan().await;
    vec![BoardHost {
        alias: "this host".into(),
        local: true,
        state: "online".into(),
        error: None,
        agents: agents
            .iter()
            .filter_map(|a| serde_json::to_value(a).ok())
            .collect(),
        panes: scan
            .panes
            .iter()
            .filter_map(|p| serde_json::to_value(p).ok())
            .collect(),
    }]
}

pub(super) async fn board(state: &AppState, limit: usize) -> Value {
    let (hosts, decisions) = tokio::join!(
        board_hosts(state),
        super::decisions::queue_sections(state, limit)
    );
    let input = state
        .home_sources
        .gather(hosts, decisions, &state.reviews)
        .await;
    home_board::build(&input)
}

/// `GET /api/home`: the board.
pub(super) async fn board_handler(
    State(state): State<AppState>,
    Query(query): Query<HomeQuery>,
) -> Response {
    Json(board(&state, query.limit.unwrap_or(DEFAULT_LIMIT)).await).into_response()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reviewed {
    /// The unit's id as the board names it (`work:…` or `agent:…`).
    id: String,
}

fn finished_unit(board: &Value, id: &str) -> Option<Value> {
    board
        .pointer("/lanes/finished/groups")?
        .as_array()?
        .iter()
        .flat_map(|group| {
            group
                .get("items")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .find(|item| item.get("id").and_then(Value::as_str) == Some(id))
}

/// `POST /api/home/reviewed`: "Looks Good" on a finished unit. The mark is
/// the review record Tend for Mac writes (`home:` + SHA-256 of the unit's
/// id and revision), so a unit marked here is marked on the Mac too, and a
/// unit that changes after the mark shows again.
pub(super) async fn reviewed_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Reviewed>,
) -> Response {
    let (by, account) = super::decisions::operator_identity(&state, &headers, None);
    let board = board(&state, DEFAULT_LIMIT).await;
    let Some(unit) = finished_unit(&board, &body.id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "no finished unit with that id"})),
        )
            .into_response();
    };
    let source = unit.get("source").cloned().unwrap_or(Value::Null);
    let target = match source.get("kind").and_then(Value::as_str) {
        Some("work") => crate::review::ReviewTarget::Work {
            workspace: source
                .get("workspace")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            work: source
                .get("work")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            alias: None,
            pipeline_generation: None,
        },
        _ => crate::review::ReviewTarget::Session {
            provider: unit
                .pointer("/focus/kind")
                .and_then(Value::as_str)
                .map(str::to_owned),
            session_id: source
                .get("session_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        },
    };
    let summary = unit
        .get("headline")
        .and_then(Value::as_str)
        .or_else(|| unit.get("title").and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Reviewed in Home");
    let summary = truncate(summary, MAX_SUMMARY_BYTES);
    let key = unit
        .get("review_key")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let new = crate::review::NewReview {
        target,
        summary,
        anchor: None,
        idempotency_key: Some(key.clone()),
        requested_by: crate::review::ReviewActor {
            kind: crate::review::ActorKind::User,
            label: Some(by.clone()),
        },
        decision: None,
    };
    let result = state
        .reviews
        .record(new, crate::review::DispositionKind::Accepted, None)
        .await;
    let (status, outcome) = match &result {
        Ok(_) => (StatusCode::OK, "recorded".to_owned()),
        Err(error) => (StatusCode::CONFLICT, error.to_string()),
    };
    super::decisions::audit(
        state.decision_audit.as_ref(),
        json!({
            "at": time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).ok(),
            "by": by, "account": account, "action": "home_reviewed", "id": body.id, "key": key,
            "outcome": outcome, "status": status.as_u16(),
        }),
    )
    .await;
    match result {
        Ok(item) => Json(json!({"review": item, "review_key": key})).into_response(),
        Err(error) => (status, Json(json!({"error": error.to_string()}))).into_response(),
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}
