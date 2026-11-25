//! Home's board on the web (`GET /api/home`), its "Looks Good" mark
//! (`POST /api/home/reviewed`) and the record of what the person typed to an
//! agent from Home (`POST /api/home/instruction`): the board tendd computes once for every
//! client ([`crate::home_board`]), from the Fleet snapshot, this daemon's
//! pipeline runs, automation rules and review records, and the decision
//! queue `/api/decisions` serves.
//!
//! # Authorization
//!
//! All three are operator-only, like pane output and the decision queue: the
//! board carries what agents are working on and what they asked. The read
//! is a GET behind the operator gate (a viewer gets 403, an anonymous
//! visitor 401); the mark and the record are control writes, so a cookie session also
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

/// Tend for Mac reads other hosts' queues with the same limit
/// (`FleetDecisions.dashboardQueueLimit`), so it never links to an item
/// this page leaves out.
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
pub(in crate::dashboard) async fn shared_board(state: &AppState, limit: usize) -> Value {
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
/// Home's control writes: Looks Good and the record of an instruction.
pub(super) fn write_routes() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/api/home/reviewed", axum::routing::post(reviewed_handler))
        .route(
            "/api/home/instruction",
            axum::routing::post(instruction_handler),
        )
}

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
///
/// An `operate` device marks the same way (`POST /api/device/home/reviewed`,
/// PARITY P6): the same board check, the same record, its own name in the
/// record and the audit.
pub(super) async fn reviewed_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    device: Option<axum::Extension<super::devices::DeviceAuth>>,
    Json(body): Json<Reviewed>,
) -> Response {
    let (by, account) = super::decisions::operator_identity(&state, &headers, device.as_deref());
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

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum InstructionKind {
    /// An answer to an agent waiting on the person (Your turn).
    Answer,
    /// A finished result sent back with more to do (Review's Send Back).
    SendBack,
}

impl InstructionKind {
    /// The Mac's idempotency-key prefixes (`FleetDecisionRecord`), which the
    /// history tells the kinds apart by.
    fn prefix(self) -> &'static str {
        match self {
            Self::Answer => "home-answer:",
            Self::SendBack => "home-sendback:",
        }
    }

    fn fallback_summary(self) -> &'static str {
        match self {
            Self::Answer => "Answered in Home",
            Self::SendBack => "Sent back from Home",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Instruction {
    /// The unit's id as the board names it.
    id: String,
    /// The session the words were typed to: one of the unit's agents.
    session_id: String,
    /// When that agent entered the state the words answered, as the board
    /// showed it; part of the key, so the same words to a later wait are
    /// another decision.
    #[serde(default)]
    state_entered_at: Option<String>,
    kind: InstructionKind,
    /// What was typed.
    text: String,
    /// The question the words answered, as the person saw it: the record's
    /// summary.
    #[serde(default)]
    question: Option<String>,
}

/// Every unit on the board, whatever its lane.
fn any_unit(board: &Value, id: &str) -> Option<Value> {
    let lanes = board.get("lanes")?.as_object()?;
    lanes
        .values()
        .filter_map(|lane| lane.get("groups").and_then(Value::as_array))
        .flatten()
        .filter_map(|group| group.get("items").and_then(Value::as_array))
        .flatten()
        .find(|item| item.get("id").and_then(Value::as_str) == Some(id))
        .cloned()
}

/// The unit's agent (or helper) running `session`, if it has one.
fn unit_agent<'a>(unit: &'a Value, session: &str) -> Option<&'a Value> {
    ["agents", "helpers"]
        .iter()
        .filter_map(|field| unit.get(*field).and_then(Value::as_array))
        .flatten()
        .chain(unit.get("focus").filter(|focus| focus.is_object()))
        .find(|agent| agent.get("session_id").and_then(Value::as_str) == Some(session))
}

/// Text the review store accepts (`TendRecordText.bounded` on the Mac): no
/// control characters but newline and tab, trimmed, at most `max` bytes,
/// cut on a character boundary with an ellipsis.
fn record_text(text: &str, max: usize) -> String {
    let cleaned: String = text
        .chars()
        .filter(|c| *c == '\n' || *c == '\t' || !c.is_control())
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.len() <= max {
        return cleaned.to_owned();
    }
    let ellipsis = "\u{2026}";
    let mut out = truncate(cleaned, max - ellipsis.len());
    out.push_str(ellipsis);
    out
}

/// The Mac's key for one instruction (`FleetDecisionRecord.instructionKey`):
/// the kind's prefix and SHA-256 of the session, the wait it answered and
/// the words, so a retry of the same send records once.
fn instruction_key(kind: InstructionKind, session: &str, since: &str, text: &str) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;
    let digest = Sha256::digest(format!("{session}\n{since}\n{text}").as_bytes());
    digest
        .iter()
        .fold(String::from(kind.prefix()), |mut key, byte| {
            let _ = write!(key, "{byte:02x}");
            key
        })
}

/// `POST /api/home/instruction`: records what the person typed to a unit's
/// agent from Home (an answer, or Send Back on a finished result) as the
/// decision Tend for Mac records: a review claimed and `returned` in one
/// step, the words as its note. The words themselves go to the pane first
/// (`POST /api/panes/{pane}/prompt`); this only records them, so a failed
/// record never unsends anything. The unit and the agent must be on the
/// board tendd builds now; the record's target is the unit's Work, else
/// that agent's session.
pub(super) async fn instruction_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Instruction>,
) -> Response {
    let (by, account) = super::decisions::operator_identity(&state, &headers, None);
    let note = record_text(&body.text, MAX_SUMMARY_BYTES);
    let board = board(&state, DEFAULT_LIMIT).await;
    let unit = any_unit(&board, &body.id);
    let agent = unit
        .as_ref()
        .and_then(|unit| unit_agent(unit, &body.session_id).cloned());
    let (status, outcome, result) = match (&unit, &agent) {
        _ if note.is_empty() => (
            StatusCode::BAD_REQUEST,
            "nothing was typed".to_owned(),
            None,
        ),
        (None, _) => (
            StatusCode::NOT_FOUND,
            "no unit with that id".to_owned(),
            None,
        ),
        (Some(_), None) => (
            StatusCode::NOT_FOUND,
            "that session is not one of this unit's agents".to_owned(),
            None,
        ),
        (Some(unit), Some(agent)) => {
            let since = body.state_entered_at.as_deref().unwrap_or("");
            let key = instruction_key(body.kind, &body.session_id, since, &body.text);
            let question = body
                .question
                .as_deref()
                .map(|q| record_text(q, MAX_SUMMARY_BYTES))
                .filter(|q| !q.is_empty())
                .unwrap_or_else(|| body.kind.fallback_summary().to_owned());
            let mut review = new_review(unit, &key, &by);
            review.summary = question;
            if !matches!(review.target, crate::review::ReviewTarget::Work { .. }) {
                review.target = crate::review::ReviewTarget::Session {
                    provider: agent.get("kind").and_then(Value::as_str).map(str::to_owned),
                    session_id: body.session_id.clone(),
                };
            }
            match state
                .reviews
                .record(
                    review,
                    crate::review::DispositionKind::Returned,
                    Some(note.clone()),
                )
                .await
            {
                Ok(item) => (StatusCode::OK, "recorded".to_owned(), Some((item, key))),
                Err(error) => (StatusCode::CONFLICT, error.to_string(), None),
            }
        }
    };
    super::decisions::audit(
        state.decision_audit.as_ref(),
        json!({
            "at": time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).ok(),
            "by": by, "account": account, "action": "home_instruction", "id": body.id,
            "session_id": body.session_id, "kind": body.kind.prefix(),
            "outcome": outcome, "status": status.as_u16(),
        }),
    )
    .await;
    match result {
        Some((item, key)) => Json(json!({"review": item, "key": key})).into_response(),
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
