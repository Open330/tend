//! HTTP routes for device pairing.
//!
//! | Route | Who | What |
//! | --- | --- | --- |
//! | `POST /auth/devices/request` | a device | a device code (secret) and a user code (shown) |
//! | `POST /auth/devices/token` | the device holding the device code | pending, or its token once approved |
//! | `POST /api/devices/lookup` | operator | the request with this user code, granting nothing |
//! | `POST /api/devices/approve` | operator | approve the request with this user code, granting a scope up to what it asked |
//! | `GET /api/devices` | operator | paired devices |
//! | `POST /api/devices/{id}/remove` | operator | unpair one |
//! | `POST /api/devices/remove-all` | operator | unpair every device |
//! | `POST /api/devices/{id}/limit` | operator | limit a device to deciding, keeping it paired |
//! | `GET /api/device` | that device | who it is |
//! | `POST /api/device/forget` | that device | unpair itself (sign out) |
//! | `GET /api/device/fleet` | that device, `operate` | the Fleet snapshot, cut down |
//! | `POST /api/device/capture` | that device, `operate` | one agent's recent output |
//!
//! The decision routes accept a device token through the dashboard's
//! decision gate (`server.rs`), not here. Requests, approvals, issued
//! tokens and removals are written to the decision audit (never a code or
//! a token).
use super::{ApprovedBy, DeviceAuth, Poll, Scope};
use crate::dashboard::server::AppState;
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Extension, Router,
};
use serde::Deserialize;
use serde_json::json;

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

async fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub(super) async fn audit_entry(state: &AppState, entry: serde_json::Value) {
    audit(state, entry).await;
}

async fn audit(state: &AppState, entry: serde_json::Value) {
    let mut entry = entry;
    entry["at"] = json!(time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .ok());
    crate::dashboard::decisions::audit(state.decision_audit.as_ref(), entry).await;
}

/// The unauthenticated half: a device asks, then polls. Bodies are small.
pub(in crate::dashboard) fn pairing_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/auth/devices/request",
            post(request).layer(DefaultBodyLimit::max(1024)),
        )
        .route(
            "/auth/devices/token",
            post(token).layer(DefaultBodyLimit::max(1024)),
        )
        .layer(middleware::map_response(no_store))
}

/// Operator management. Mounted inside the operator write layer, so these
/// need the dashboard token or an operator session with the CSRF proof.
pub(in crate::dashboard) fn admin_routes() -> Router<AppState> {
    Router::new()
        .route("/api/devices", get(list))
        .route(
            "/api/devices/lookup",
            post(lookup).layer(DefaultBodyLimit::max(1024)),
        )
        .route(
            "/api/devices/approve",
            post(approve).layer(DefaultBodyLimit::max(1024)),
        )
        .route("/api/devices/remove-all", post(remove_all))
        .route("/api/devices/{id}/remove", post(remove))
        .route("/api/devices/{id}/limit", post(limit))
        .layer(middleware::map_response(no_store))
}

/// A device's own routes: only a device token reaches them.
pub(in crate::dashboard) fn device_routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/device", get(whoami))
        .route("/api/device/forget", post(forget))
        .layer(middleware::from_fn_with_state(state, device_only))
        .layer(middleware::map_response(no_store))
}

/// The device named by the request's bearer token, if any. Only a
/// dashboard with a token pairs devices.
pub(in crate::dashboard) fn device_auth(
    state: &AppState,
    headers: &HeaderMap,
) -> Option<DeviceAuth> {
    state.config.token.as_ref()?;
    state.devices.authenticate(
        headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
    )
}

/// The routes an `operate` device adds, each an exact path (never a
/// prefix: `/api/device` also prefixes `/api/devices`). A device without
/// `operate` gets 403, never 401, which the app reads as "removed".
pub(in crate::dashboard) fn operate_routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/device/fleet", get(super::operate::fleet))
        .route(
            "/api/device/capture",
            post(super::operate::capture).layer(DefaultBodyLimit::max(1024)),
        )
        .layer(middleware::from_fn_with_state(state.clone(), operate_only))
        .layer(middleware::from_fn_with_state(state, device_only))
        .layer(middleware::map_response(no_store))
}

async fn operate_only(State(state): State<AppState>, req: Request, next: Next) -> Response {
    match req.extensions().get::<DeviceAuth>() {
        Some(auth) if auth.scope == Scope::Operate => next.run(req).await,
        Some(auth) => {
            // A device that may only decide tried more: worth a line.
            let mut entry = super::operate::device_actor(auth);
            entry["action"] = json!("device_operate_refused");
            entry["path"] = json!(req.uri().path());
            entry["status"] = json!(403);
            audit(&state, entry).await;
            error(
                StatusCode::FORBIDDEN,
                "this device may only decide; pair it again with hosts and new work",
            )
        }
        None => StatusCode::UNAUTHORIZED.into_response(),
    }
}

async fn device_only(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let auth = device_auth(&state, req.headers()).ok_or(StatusCode::UNAUTHORIZED)?;
    req.extensions_mut().insert(auth);
    Ok(next.run(req).await)
}

#[derive(Deserialize)]
struct RequestBody {
    #[serde(default)]
    name: String,
    #[serde(default)]
    platform: String,
    /// `operate` to ask for hosts and new work; anything else is `decide`.
    #[serde(default)]
    scope: String,
}

async fn request(State(state): State<AppState>, body: Bytes) -> Response {
    if state.config.token.is_none() {
        return error(StatusCode::NOT_FOUND, "device pairing is unavailable");
    }
    let Ok(body) = serde_json::from_slice::<RequestBody>(&body) else {
        return error(StatusCode::BAD_REQUEST, "expected {\"name\": \"...\"}");
    };
    let Some((device_code, user_code, expires_in, interval)) =
        state
            .devices
            .request(&body.name, &body.platform, Scope::from_stored(&body.scope))
    else {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "too many devices are waiting to be paired; try again in a few minutes",
        );
    };
    (
        StatusCode::CREATED,
        Json(json!({
            "device_code": device_code,
            "user_code": format!("{}-{}", &user_code[..4], &user_code[4..]),
            "expires_in": expires_in,
            "interval": interval,
        })),
    )
        .into_response()
}

#[derive(Deserialize)]
struct TokenBody {
    device_code: String,
}

async fn token(State(state): State<AppState>, body: Bytes) -> Response {
    if state.config.token.is_none() {
        return error(StatusCode::NOT_FOUND, "device pairing is unavailable");
    }
    let Ok(body) = serde_json::from_slice::<TokenBody>(&body) else {
        return error(
            StatusCode::BAD_REQUEST,
            "expected {\"device_code\": \"...\"}",
        );
    };
    match state.devices.poll(&body.device_code) {
        Poll::Pending => (
            StatusCode::ACCEPTED,
            Json(json!({ "status": "pending", "interval": super::POLL_INTERVAL_SECS })),
        )
            .into_response(),
        Poll::Issued(device, token) => {
            tracing::info!(device = %device.id, name = %device.name, "dashboard device paired");
            audit(
                &state,
                json!({
                    "by": super::Device::label(&device.name, &device.id),
                    "action": "device_paired", "id": device.id, "name": device.name,
                    "platform": device.platform, "approved_by": device.approved_by,
                    "scope": device.scope, "status": 201,
                }),
            )
            .await;
            (
                StatusCode::CREATED,
                Json(json!({
                    "device": { "id": device.id, "name": device.name, "platform": device.platform },
                    "token": token,
                    "scope": device.scope.names(),
                })),
            )
                .into_response()
        }
        Poll::Invalid => error(
            StatusCode::UNAUTHORIZED,
            "this pairing request is unknown, expired or already used; start again",
        ),
        Poll::Full => error(
            StatusCode::CONFLICT,
            "this dashboard has as many paired devices as it allows; remove one first",
        ),
        Poll::Storage(message) => {
            tracing::warn!(%message, "dashboard device store failed");
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "the device store is unavailable",
            )
        }
    }
}

/// Who is approving: the token or the signed-in operator.
fn approved_by(state: &AppState, headers: &HeaderMap) -> ApprovedBy {
    match crate::dashboard::operator::authenticate(
        state.config.token.as_deref(),
        &state.operator,
        headers,
    ) {
        Some(crate::dashboard::operator::OperatorAuth::Session) => {
            let account = state.operator.account(headers);
            ApprovedBy {
                credential: "session".into(),
                issuer: account.as_ref().map(|a| a.0.clone()),
                subject: account.as_ref().map(|a| a.1.clone()),
                email: account.and_then(|a| a.2),
            }
        }
        _ => ApprovedBy {
            credential: "token".into(),
            issuer: None,
            subject: None,
            email: None,
        },
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LookupBody {
    user_code: String,
}

/// What the device behind a code is and asks for, before anything is
/// granted: the operator approves that exact request next.
async fn lookup(State(state): State<AppState>, body: Bytes) -> Response {
    let Ok(body) = serde_json::from_slice::<LookupBody>(&body) else {
        return error(StatusCode::BAD_REQUEST, "expected {\"user_code\": \"...\"}");
    };
    match state.devices.lookup(&body.user_code) {
        Some(view) => Json(json!({ "request": view })).into_response(),
        None => error(
            StatusCode::NOT_FOUND,
            "no device is waiting with that code; check it on the device",
        ),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApproveBody {
    user_code: String,
    /// What the operator grants; absent (an older dashboard page) is
    /// `decide`. The device gets this or what it asked for, whichever is
    /// less.
    #[serde(default)]
    scope: Option<String>,
}

async fn approve(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let Ok(body) = serde_json::from_slice::<ApproveBody>(&body) else {
        return error(StatusCode::BAD_REQUEST, "expected {\"user_code\": \"...\"}");
    };
    let grant = Scope::from_stored(body.scope.as_deref().unwrap_or("decide"));
    let by = approved_by(&state, &headers);
    match state.devices.approve(&body.user_code, by.clone(), grant) {
        Some(view) => {
            audit(
                &state,
                json!({
                    "by": by, "action": "device_approved", "name": view.name,
                    "platform": view.platform, "scope": view.scope, "status": 200,
                }),
            )
            .await;
            Json(json!({
                "approved": { "name": view.name, "platform": view.platform, "scope": view.scope }
            }))
            .into_response()
        }
        None => error(
            StatusCode::NOT_FOUND,
            "no device is waiting with that code; check it on the device",
        ),
    }
}

async fn list(State(state): State<AppState>) -> Response {
    Json(json!({ "devices": state.devices.list() })).into_response()
}

async fn unpair(state: &AppState, id: &str, by: serde_json::Value) -> Response {
    match state.devices.remove(id) {
        Ok(true) => {
            tracing::info!(device = %id, "dashboard device removed");
            audit(
                state,
                json!({ "by": by, "action": "device_removed", "id": id, "status": 200 }),
            )
            .await;
            Json(json!({ "removed": id })).into_response()
        }
        Ok(false) => error(StatusCode::NOT_FOUND, "no such device"),
        Err(e) => {
            tracing::warn!(error = %e, "dashboard device store failed");
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "the device store is unavailable",
            )
        }
    }
}

async fn remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let by = json!(approved_by(&state, &headers));
    unpair(&state, &id, by).await
}

async fn remove_all(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match state.devices.remove_all() {
        Ok(count) => {
            audit(
                &state,
                json!({
                    "by": approved_by(&state, &headers), "action": "devices_removed_all",
                    "count": count, "status": 200,
                }),
            )
            .await;
            Json(json!({ "removed": count })).into_response()
        }
        Err(e) => {
            tracing::warn!(error = %e, "dashboard device store failed");
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "the device store is unavailable",
            )
        }
    }
}

async fn whoami(
    State(state): State<AppState>,
    Extension(device): Extension<DeviceAuth>,
) -> Response {
    // What an `operate` device can use here beyond the hosts.
    let features: Vec<&str> = if device.scope == Scope::Operate && state.fleet.is_some() {
        vec!["capture"]
    } else {
        Vec::new()
    };
    Json(json!({
        "device": { "id": device.id, "name": device.name },
        "approved_by": device.approved_by,
        "scope": device.scope.names(),
        "features": features,
    }))
    .into_response()
}

async fn limit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    match state.devices.limit(&id) {
        Ok(true) => {
            audit(
                &state,
                json!({
                    "by": approved_by(&state, &headers), "action": "device_limited",
                    "id": id, "scope": Scope::Decide, "status": 200,
                }),
            )
            .await;
            Json(json!({ "limited": id, "scope": Scope::Decide })).into_response()
        }
        Ok(false) => error(StatusCode::NOT_FOUND, "no such device"),
        Err(e) => {
            tracing::warn!(error = %e, "dashboard device store failed");
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "the device store is unavailable",
            )
        }
    }
}

async fn forget(
    State(state): State<AppState>,
    Extension(device): Extension<DeviceAuth>,
) -> Response {
    let by = json!(super::Device::label(&device.name, &device.id));
    unpair(&state, &device.id, by).await
}
