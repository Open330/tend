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
        mode: None,
        tend_version: None,
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

/// The board, built now: everything gathered, then the finished units'
/// reviewed marks looked up by their exact keys.
pub(in crate::dashboard) async fn board(state: &AppState, limit: usize) -> Value {
    let (hosts, decisions) = tokio::join!(
        board_hosts(state),
        super::decisions::queue_sections(state, limit)
    );
    let mut input = state.home_sources.gather(hosts, decisions).await;
    let keys = home_board::finished_keys(&home_board::build(&input));
    input.reviewed = home_board::accepted(&state.reviews, keys).await;
    home_board::build(&input)
}

/// The board for a read: the one built in the last
/// [`home_board::BOARD_REUSE`] when there is one, so readers polling
/// together build it once.
async fn shared_board(state: &AppState, limit: usize) -> Value {
    let mut last = state.home_sources.last_board.lock().await;
    if let Some((at, board)) = last.as_ref() {
        if limit == DEFAULT_LIMIT && at.elapsed() < home_board::BOARD_REUSE {
            return board.clone();
        }
    }
    let board = board(state, limit).await;
    if limit == DEFAULT_LIMIT {
        *last = Some((std::time::Instant::now(), board.clone()));
    }
    board
}

/// `GET /api/home`: the board.
pub(super) async fn board_handler(
    State(state): State<AppState>,
    Query(query): Query<HomeQuery>,
) -> Response {
    Json(shared_board(&state, query.limit.unwrap_or(DEFAULT_LIMIT)).await).into_response()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reviewed {
    /// The unit's id as the board names it (`work:…` or `agent:…`).
    id: String,
    /// The `review_key` of the result the person looked at. A unit that
    /// changed since (another turn finished) has another key and is
    /// refused (409), so a mark never accepts a result nobody saw.
    review_key: String,
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

fn refusal(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

/// `POST /api/home/reviewed`: "Looks Good" on a finished unit. The mark is
/// the review record Tend for Mac writes (`home:` + SHA-256 of the unit's
/// id and revision), so a unit marked here is marked on the Mac too, and a
/// unit that changes after the mark shows again. The unit, its target and
/// its key come from the board tendd builds now; the person's `review_key`
/// only has to match it.
pub(super) async fn reviewed_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Reviewed>,
) -> Response {
    let (by, account) = super::decisions::operator_identity(&state, &headers, None);
    let board = board(&state, DEFAULT_LIMIT).await;
    let (status, outcome, result) = match finished_unit(&board, &body.id) {
        None => (
            StatusCode::NOT_FOUND,
            "no finished unit with that id".to_owned(),
            None,
        ),
        Some(unit) => {
            let key = unit
                .get("review_key")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            if key == body.review_key {
                match state
                    .reviews
                    .record(
                        new_review(&unit, &key, &by),
                        crate::review::DispositionKind::Accepted,
                        None,
                    )
                    .await
                {
                    Ok(item) => (StatusCode::OK, "recorded".to_owned(), Some((item, key))),
                    Err(error) => (StatusCode::CONFLICT, error.to_string(), None),
                }
            } else {
                (
                    StatusCode::CONFLICT,
                    "this result changed since it was shown; look again".to_owned(),
                    None,
                )
            }
        }
    };
    super::decisions::audit(
        state.decision_audit.as_ref(),
        json!({
            "at": time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).ok(),
            "by": by, "account": account, "action": "home_reviewed", "id": body.id,
            "key": body.review_key, "outcome": outcome, "status": status.as_u16(),
        }),
    )
    .await;
    *state.home_sources.last_board.lock().await = None;
    match result {
        Some((item, key)) => Json(json!({"review": item, "review_key": key})).into_response(),
        None => refusal(status, &outcome),
    }
}

/// The review record for a unit: on its Work or its agent's session,
/// summarized by its line (or title), under the Mac's key.
fn new_review(unit: &Value, key: &str, by: &str) -> crate::review::NewReview {
    let source = unit.get("source").cloned().unwrap_or(Value::Null);
    let field = |name: &str| {
        source
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    let target = if source.get("kind").and_then(Value::as_str) == Some("work") {
        crate::review::ReviewTarget::Work {
            workspace: field("workspace"),
            work: field("work"),
            alias: None,
            pipeline_generation: None,
        }
    } else {
        crate::review::ReviewTarget::Session {
            provider: unit
                .pointer("/focus/kind")
                .and_then(Value::as_str)
                .map(str::to_owned),
            session_id: field("session_id"),
        }
    };
    let summary = unit
        .get("headline")
        .and_then(Value::as_str)
        .or_else(|| unit.get("title").and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Reviewed in Home");
    crate::review::NewReview {
        target,
        summary: truncate(summary, MAX_SUMMARY_BYTES),
        anchor: None,
        idempotency_key: Some(key.to_owned()),
        requested_by: crate::review::ReviewActor {
            kind: crate::review::ActorKind::User,
            label: Some(by.to_owned()),
        },
        decision: None,
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
