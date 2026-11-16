use super::*;
use crate::dashboard::server::{router, AppState};
use crate::dashboard::DashboardConfig;
use crate::review::{ActorKind, NewReview, ReviewActor, ReviewStore, ReviewTarget};
use crate::state::Store;
use crate::tmux::scanner::PaneCache;
use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

const TOKEN: &str = "s3cret";

fn state(token: Option<&str>) -> AppState {
    let mut cfg = DashboardConfig::loopback_default();
    cfg.token = token.map(str::to_owned);
    AppState::new(
        Store::shared(),
        Arc::new(cfg),
        Arc::new(PaneCache::new(Duration::from_secs(60))),
        crate::session::PtySessionBackend::shared(),
    )
}

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    bearer: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(bearer) = bearer {
        request = request.header(header::AUTHORIZATION, format!("Bearer {bearer}"));
    }
    let body = body.map_or_else(Body::empty, |b| Body::from(b.to_string()));
    let response = app
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// Request, approve with the operator token, and collect the device token.
async fn pair(app: &Router, name: &str) -> (String, String) {
    pair_with(app, name, None, None).await
}

/// `pair`, asking for and granting the given scopes (absent: the field is
/// not sent, as a phase 1 app and dashboard page do).
async fn pair_with(
    app: &Router,
    name: &str,
    ask: Option<&str>,
    grant: Option<&str>,
) -> (String, String) {
    let mut body = json!({"name": name, "platform": "ios"});
    if let Some(ask) = ask {
        body["scope"] = json!(ask);
    }
    let (status, asked) = call(app, "POST", "/auth/devices/request", None, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{asked}");
    let device_code = asked["device_code"].as_str().unwrap().to_owned();
    let mut approval = json!({"user_code": asked["user_code"]});
    if let Some(grant) = grant {
        approval["scope"] = json!(grant);
    }
    let (status, _) = call(
        app,
        "POST",
        "/api/devices/approve",
        Some(TOKEN),
        Some(approval),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, issued) = call(
        app,
        "POST",
        "/auth/devices/token",
        None,
        Some(json!({"device_code": device_code})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{issued}");
    (
        issued["token"].as_str().unwrap().to_owned(),
        issued["device"]["id"].as_str().unwrap().to_owned(),
    )
}

async fn review_store() -> (ReviewStore, String) {
    let reviews = ReviewStore::in_memory();
    let item = reviews
        .create(NewReview {
            target: ReviewTarget::Session {
                provider: None,
                session_id: "s".into(),
            },
            summary: "check the migration".into(),
            anchor: None,
            idempotency_key: None,
            requested_by: ReviewActor {
                kind: ActorKind::Agent,
                label: None,
            },
            decision: None,
        })
        .await
        .unwrap();
    (reviews, item.id)
}

#[tokio::test]
async fn a_device_asks_an_operator_approves_and_the_token_is_issued_once() {
    let app = router(state(Some(TOKEN)));
    let (status, asked) = call(
        &app,
        "POST",
        "/auth/devices/request",
        None,
        Some(json!({"name": "Jiun's iPhone", "platform": "ios"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_code = asked["user_code"].as_str().unwrap();
    assert_eq!(user_code.len(), 9, "XXXX-XXXX");
    assert_eq!(asked["interval"], 2);
    let device_code = json!({"device_code": asked["device_code"]});
    // Not approved yet.
    let (status, body) = call(
        &app,
        "POST",
        "/auth/devices/token",
        None,
        Some(device_code.clone()),
    )
    .await;
    assert_eq!(
        (status, body["status"].clone()),
        (StatusCode::ACCEPTED, json!("pending"))
    );
    // Only an operator approves; a typed code may differ in case and dash.
    let typed = user_code.replace('-', " ").to_lowercase();
    let (status, _) = call(
        &app,
        "POST",
        "/api/devices/approve",
        None,
        Some(json!({"user_code": typed})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, approved) = call(
        &app,
        "POST",
        "/api/devices/approve",
        Some(TOKEN),
        Some(json!({"user_code": typed})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(approved["approved"]["name"], "Jiun's iPhone");
    // Approving twice finds nothing to approve.
    let (status, _) = call(
        &app,
        "POST",
        "/api/devices/approve",
        Some(TOKEN),
        Some(json!({"user_code": typed})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, issued) = call(
        &app,
        "POST",
        "/auth/devices/token",
        None,
        Some(device_code.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let token = issued["token"].as_str().unwrap();
    assert!(token.starts_with("tdv_"));
    // Issued once.
    let (status, _) = call(&app, "POST", "/auth/devices/token", None, Some(device_code)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, me) = call(&app, "GET", "/api/device", Some(token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["device"]["name"], "Jiun's iPhone");
    assert_eq!(me["approved_by"]["credential"], "token");
    let (_, listed) = call(&app, "GET", "/api/devices", Some(TOKEN), None).await;
    assert_eq!(listed["devices"].as_array().unwrap().len(), 1);
    assert!(listed.to_string().find("token_hash").is_none());
}

#[tokio::test]
async fn racing_polls_get_one_token() {
    let devices = Devices::memory();
    let (code, user, ..) = devices.request("phone", "ios", Scope::Decide).unwrap();
    devices
        .approve(
            &user,
            ApprovedBy {
                credential: "token".into(),
                issuer: None,
                subject: None,
                email: None,
            },
            Scope::Decide,
        )
        .unwrap();
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let devices = devices.clone();
            let code = code.clone();
            std::thread::spawn(move || matches!(devices.poll(&code), Poll::Issued(..)))
        })
        .collect();
    let issued = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .filter(|issued| *issued)
        .count();
    assert_eq!(issued, 1);
    assert_eq!(devices.list().len(), 1);
}

#[tokio::test]
async fn a_device_token_decides_and_nothing_else() {
    let (reviews, id) = review_store().await;
    let dir = tempfile::tempdir().unwrap();
    let audit = dir.path().join("decisions.jsonl");
    let app = router(
        state(Some(TOKEN))
            .with_reviews(reviews.clone())
            .with_decision_audit(Some(audit.clone())),
    );
    let (token, device) = pair(&app, "phone").await;
    let (status, queue) = call(&app, "GET", "/api/decisions", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        queue["hosts"][0]["queue"]["items"][0]["summary"],
        "check the migration"
    );
    let (status, _) = call(&app, "GET", "/api/decisions/history", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, decided) = call(
        &app,
        "POST",
        &format!("/api/decisions/reviews/{id}"),
        Some(&token),
        Some(json!({"disposition": "accepted", "note": "fine"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{decided}");
    let label = Device::label("phone", &device);
    assert!(label.starts_with("app:phone#"), "{label}");
    assert_eq!(
        decided["review"]["disposition"]["by"]["label"],
        label.as_str()
    );
    let lines = std::fs::read_to_string(&audit).unwrap();
    let review_line = lines
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|line| line["action"] == "review")
        .unwrap();
    assert_eq!(review_line["by"], label.as_str());
    assert_eq!(review_line["account"]["credential"], "device");
    assert_eq!(review_line["account"]["device"], device.as_str());
    assert!(!lines.contains(&token), "the audit never holds a token");
    assert!(lines.contains("\"device_paired\"") && lines.contains("\"device_approved\""));

    // Every other operator surface refuses it, reads included.
    refuses_every_operator_route(&app, &token, &device).await;
    // The operate routes say why (403), and never "removed" (401).
    let (status, _) = call(&app, "GET", "/api/device/fleet", Some(&token), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // And the operator token is not a device: /api/device is for devices.
    let (status, _) = call(&app, "GET", "/api/device", Some(TOKEN), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// Every operator route answers a device token, whatever its scope, as if
/// it had no credential.
async fn refuses_every_operator_route(app: &Router, token: &str, device: &str) {
    for (method, uri) in [
        ("GET", "/api/agents".to_owned()),
        ("GET", "/api/access".to_owned()),
        ("GET", "/api/fleet".to_owned()),
        ("GET", "/api/panes".to_owned()),
        ("GET", "/api/works".to_owned()),
        ("GET", "/api/collaboration".to_owned()),
        ("GET", "/api/events".to_owned()),
        ("GET", "/api/panes/%251/output".to_owned()),
        ("GET", "/api/panes/%251/output/stream".to_owned()),
        ("POST", "/api/panes/%251/prompt".to_owned()),
        ("POST", "/api/panes/%251/abort".to_owned()),
        ("POST", "/api/fleet/mini/command".to_owned()),
        ("POST", "/api/fleet/local/command".to_owned()),
        ("POST", "/api/work-control/prompt".to_owned()),
        ("POST", "/api/work-control/up".to_owned()),
        ("POST", "/api/work-control/abort".to_owned()),
        ("PUT", "/api/work-metadata".to_owned()),
        ("POST", "/api/terminal-sessions/x/input".to_owned()),
        ("POST", "/api/terminal-sessions/x/terminate".to_owned()),
        ("GET", "/api/operators".to_owned()),
        ("GET", "/api/shares".to_owned()),
        ("GET", "/api/devices".to_owned()),
        ("POST", "/api/devices/lookup".to_owned()),
        ("POST", "/api/devices/approve".to_owned()),
        ("POST", "/api/devices/remove-all".to_owned()),
        ("POST", format!("/api/devices/{device}/remove")),
        ("POST", format!("/api/devices/{device}/limit")),
    ] {
        let (status, _) = call(app, method, &uri, Some(token), Some(json!({}))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri}");
    }
}

#[tokio::test]
async fn an_operate_device_gets_its_routes_and_no_operator_route() {
    let app = router(state(Some(TOKEN)));
    let (token, device) = pair_with(&app, "phone", Some("operate"), Some("operate")).await;
    let (status, me) = call(&app, "GET", "/api/device", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["scope"], json!(["decisions", "operate"]));
    // Past the gate: this test state has no Fleet manager, so 503, not 403.
    let (status, _) = call(&app, "GET", "/api/device/fleet", Some(&token), None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let (status, _) = call(&app, "GET", "/api/decisions", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    refuses_every_operator_route(&app, &token, &device).await;
    // A route is matched exactly: nothing under /api/device/ is a prefix match.
    let (status, _) = call(&app, "GET", "/api/device/fleet/x", Some(&token), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_operator_grants_at_most_what_was_asked_and_at_most_what_was_ticked() {
    let app = router(state(Some(TOKEN)));
    for (ask, grant, scope) in [
        (Some("operate"), None, json!(["decisions"])),
        (Some("operate"), Some("decide"), json!(["decisions"])),
        (Some("decide"), Some("operate"), json!(["decisions"])),
        (None, Some("operate"), json!(["decisions"])),
        (Some("admin"), Some("admin"), json!(["decisions"])),
        (
            Some("operate"),
            Some("operate"),
            json!(["decisions", "operate"]),
        ),
    ] {
        let (token, _) = pair_with(&app, "phone", ask, grant).await;
        let (_, me) = call(&app, "GET", "/api/device", Some(&token), None).await;
        assert_eq!(me["scope"], scope, "asked {ask:?}, granted {grant:?}");
    }
}

#[tokio::test]
async fn looking_a_code_up_grants_nothing() {
    let app = router(state(Some(TOKEN)));
    let (_, asked) = call(
        &app,
        "POST",
        "/auth/devices/request",
        None,
        Some(json!({"name": "phone", "platform": "ios", "scope": "operate"})),
    )
    .await;
    let code = asked["user_code"].as_str().unwrap().to_owned();
    // Only an operator may look.
    let (status, _) = call(
        &app,
        "POST",
        "/api/devices/lookup",
        None,
        Some(json!({"user_code": code})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, seen) = call(
        &app,
        "POST",
        "/api/devices/lookup",
        Some(TOKEN),
        Some(json!({"user_code": code.to_lowercase().replace('-', "")})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{seen}");
    assert_eq!(seen["request"]["name"], "phone");
    assert_eq!(seen["request"]["scope"], "operate");
    let (status, _) = call(
        &app,
        "POST",
        "/auth/devices/token",
        None,
        Some(json!({"device_code": asked["device_code"]})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "still pending after a lookup");
    let (status, _) = call(
        &app,
        "POST",
        "/api/devices/lookup",
        Some(TOKEN),
        Some(json!({"user_code": code, "scope": "operate"})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a lookup takes nothing else"
    );
}

#[tokio::test]
async fn limit_to_deciding_narrows_at_once_and_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("devices.sqlite3");
    let audit = dir.path().join("decisions.jsonl");
    let devices = Devices::open(Some(path.clone())).await.unwrap();
    let app = router(
        state(Some(TOKEN))
            .with_devices(devices)
            .with_decision_audit(Some(audit.clone())),
    );
    let (token, device) = pair_with(&app, "phone", Some("operate"), Some("operate")).await;
    let (status, _) = call(&app, "GET", "/api/device/fleet", Some(&token), None).await;
    assert_ne!(status, StatusCode::FORBIDDEN);
    // A device cannot limit (or widen) itself.
    let (status, _) = call(
        &app,
        "POST",
        &format!("/api/devices/{device}/limit"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = call(
        &app,
        "POST",
        &format!("/api/devices/{device}/limit"),
        Some(TOKEN),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(&app, "GET", "/api/device/fleet", Some(&token), None).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "narrowed on the next request"
    );
    let (status, _) = call(&app, "GET", "/api/decisions", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK, "still paired, still decides");
    let (status, _) = call(&app, "POST", "/api/devices/nope/limit", Some(TOKEN), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(std::fs::read_to_string(&audit)
        .unwrap()
        .contains("\"device_limited\""));
    drop(app);
    let reopened = Devices::open(Some(path)).await.unwrap();
    let listed = reopened.list();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].scope, Scope::Decide);
}

#[tokio::test]
async fn a_phase_1_store_opens_with_every_device_deciding() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("devices.sqlite3");
    {
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "PRAGMA application_id=0x4D584456; PRAGMA user_version=1; \
                 CREATE TABLE devices (id TEXT PRIMARY KEY, token_hash BLOB NOT NULL UNIQUE, \
                 name TEXT NOT NULL, platform TEXT NOT NULL, approved_by TEXT NOT NULL, \
                 created_at INTEGER NOT NULL, last_seen_at INTEGER NOT NULL);",
            )
            .unwrap();
        let now = now_unix();
        connection
            .execute(
                "INSERT INTO devices VALUES ('dev_old', ?1, 'old phone', 'ios', ?2, ?3, ?3)",
                rusqlite::params![
                    sha256("tdv_old").as_slice(),
                    serde_json::to_string(&token_by()).unwrap(),
                    now
                ],
            )
            .unwrap();
    }
    let devices = Devices::open(Some(path.clone())).await.unwrap();
    let auth = devices.authenticate(Some("Bearer tdv_old")).unwrap();
    assert_eq!(auth.scope, Scope::Decide);
    drop(devices);
    let connection = rusqlite::Connection::open(&path).unwrap();
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 2);
    // A value this code does not know is never more than deciding.
    connection
        .execute("UPDATE devices SET scope = 'root'", [])
        .unwrap();
    drop(connection);
    let devices = Devices::open(Some(path)).await.unwrap();
    assert_eq!(
        devices.authenticate(Some("Bearer tdv_old")).unwrap().scope,
        Scope::Decide
    );
}

#[test]
fn the_device_fleet_leaves_out_what_the_app_does_not_show() {
    let full = json!({
        "generated_at": "2026-11-16T00:00:00Z",
        "hosts": [{
            "alias": "mini", "local": false, "mode": "control", "state": "online",
            "ssh_target": "jiun@mini", "labels": {"a": "b"}, "annotations": {},
            "capabilities": ["capture"], "tend_version": "0.10.20",
            "remote": {
                "agents": [{
                    "kind": "claude_code", "agent_session_id": "s1", "pane": "%1",
                    "tmux_socket": "/tmp/x", "pid": 42, "cwd": "/w/tend", "state": "working",
                    "last_response": "x".repeat(40_000), "workload": {"cpu": 1},
                    "awaiting_reply": {"question": "q".repeat(50_000), "since": "t", "extra": {"a": 1}},
                    "model": {"nested": "not a scalar"},
                }],
                "panes": [{"pane_id": "%2", "current_path": "/secret"}],
                "backends": [{"kind": "tmux"}],
            },
        }],
    });
    let cut = super::operate::cut(&full);
    let host = &cut["hosts"][0];
    assert_eq!(host["alias"], "mini");
    assert_eq!(host["tend_version"], "0.10.20");
    for gone in ["ssh_target", "labels", "annotations", "capabilities"] {
        assert!(host.get(gone).is_none(), "{gone}");
    }
    assert!(host["remote"].get("panes").is_none());
    assert!(host["remote"].get("backends").is_none());
    let agent = &host["remote"]["agents"][0];
    assert_eq!(agent["agent_session_id"], "s1");
    assert_eq!(agent["cwd"], "/w/tend");
    for gone in ["tmux_socket", "pid", "workload"] {
        assert!(agent.get(gone).is_none(), "{gone}");
    }
    assert!(agent["last_response"].as_str().unwrap().len() <= 16 * 1024 + 4);
    // Nested text is bounded too, and only listed nested fields pass.
    assert!(agent["awaiting_reply"]["question"].as_str().unwrap().len() <= 16 * 1024 + 4);
    assert_eq!(agent["awaiting_reply"]["since"], "t");
    assert!(agent["awaiting_reply"].get("extra").is_none());
    assert!(
        agent.get("model").is_none(),
        "a structure nobody listed is dropped"
    );
    assert!(cut.to_string().len() < 64 * 1024);
}

#[tokio::test]
async fn removal_and_sign_out_end_a_device_at_once() {
    let app = router(state(Some(TOKEN)));
    let (one, one_id) = pair(&app, "one").await;
    let (two, _) = pair(&app, "two").await;
    let (three, _) = pair(&app, "three").await;
    let (status, _) = call(
        &app,
        "POST",
        &format!("/api/devices/{one_id}/remove"),
        Some(TOKEN),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(&app, "GET", "/api/decisions", Some(&one), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // Sign out from the device itself.
    let (status, _) = call(&app, "POST", "/api/device/forget", Some(&two), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(&app, "GET", "/api/device", Some(&two), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, body) = call(&app, "POST", "/api/devices/remove-all", Some(TOKEN), None).await;
    assert_eq!(
        (status, body["removed"].clone()),
        (StatusCode::OK, json!(1))
    );
    let (status, _) = call(&app, "GET", "/api/device", Some(&three), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[test]
fn devices_expire_when_idle_or_old() {
    let devices = Devices::memory();
    let by = ApprovedBy {
        credential: "token".into(),
        issuer: None,
        subject: None,
        email: None,
    };
    let issue = |name: &str| {
        let (code, user, ..) = devices.request(name, "ios", Scope::Decide).unwrap();
        devices.approve(&user, by.clone(), Scope::Decide).unwrap();
        match devices.poll(&code) {
            Poll::Issued(device, token) => (device.id, token),
            other => panic!("{other:?}"),
        }
    };
    let (idle, idle_token) = issue("idle");
    let (old, old_token) = issue("old");
    let (fresh, fresh_token) = issue("fresh");
    devices.age_device(&idle, IDLE_EXPIRY, IDLE_EXPIRY);
    // Used every day, but paired past the maximum lifetime.
    devices.age_device(&old, MAX_LIFETIME, 0);
    devices.age_device(&fresh, IDLE_EXPIRY - 60, IDLE_EXPIRY - 60);
    let bearer = |token: &str| format!("Bearer {token}");
    assert!(devices.authenticate(Some(&bearer(&idle_token))).is_none());
    assert!(devices.authenticate(Some(&bearer(&old_token))).is_none());
    assert!(devices.authenticate(Some(&bearer(&fresh_token))).is_some());
    assert_eq!(devices.list().len(), 1);
    // Not a device token at all.
    assert!(devices.authenticate(Some("Bearer s3cret")).is_none());
    assert!(
        devices.authenticate(Some(&fresh_token)).is_none(),
        "needs the Bearer scheme"
    );
}

#[tokio::test]
async fn pending_requests_are_bounded_and_need_a_dashboard_token() {
    let app = router(state(Some(TOKEN)));
    for _ in 0..MAX_REQUESTS {
        let (status, _) = call(
            &app,
            "POST",
            "/auth/devices/request",
            None,
            Some(json!({"name": "x"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }
    let (status, _) = call(
        &app,
        "POST",
        "/auth/devices/request",
        None,
        Some(json!({"name": "x"})),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    // A wrong device code never slows down anyone else.
    for _ in 0..20 {
        let (status, _) = call(
            &app,
            "POST",
            "/auth/devices/token",
            None,
            Some(json!({"device_code": "nope"})),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let app = router(state(None));
    let (status, _) = call(
        &app,
        "POST",
        "/auth/devices/request",
        None,
        Some(json!({"name": "x"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(&app, "GET", "/api/decisions", Some("tdv_x"), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[test]
fn names_and_codes_are_cleaned() {
    assert_eq!(clean_name("  \u{1b}[31mphone#2\n "), "[31mphone2");
    assert_eq!(clean_name(""), "device");
    assert_eq!(clean_name(&"x".repeat(200)).chars().count(), MAX_NAME_CHARS);
    assert_eq!(clean_platform("ios"), "ios");
    assert_eq!(clean_platform("<script>"), "other");
    assert_eq!(normalize_user_code(" bcdf-ghjk "), "BCDFGHJK");
    for _ in 0..200 {
        let code = random_user_code();
        assert_eq!(code.len(), USER_CODE_LEN);
        assert!(code.bytes().all(|b| USER_CODE_ALPHABET.contains(&b)));
    }
}

#[tokio::test]
async fn the_store_is_created_at_the_first_pairing_and_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("devices/devices.sqlite3");
    let devices = Devices::open(Some(path.clone())).await.unwrap();
    assert!(!path.exists(), "nothing is written before a device pairs");
    let (code, user, ..) = devices.request("phone", "ios", Scope::Decide).unwrap();
    devices
        .approve(
            &user,
            ApprovedBy {
                credential: "session".into(),
                issuer: Some("https://id".into()),
                subject: Some("u1".into()),
                email: Some("a@b".into()),
            },
            Scope::Decide,
        )
        .unwrap();
    let Poll::Issued(device, token) = devices.poll(&code) else {
        panic!("not issued")
    };
    assert!(path.exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let stored = std::fs::read(&path).unwrap();
    assert!(
        !stored.windows(token.len()).any(|w| w == token.as_bytes()),
        "only the hash is stored"
    );
    // A second daemon cannot open it while the first holds it.
    assert!(storage::Storage::open(&path).is_err());
    drop(devices);
    let reopened = Devices::open(Some(path)).await.unwrap();
    let auth = reopened
        .authenticate(Some(&format!("Bearer {token}")))
        .unwrap();
    assert_eq!(auth.id, device.id);
    assert_eq!(auth.approved_by.email.as_deref(), Some("a@b"));
}

fn token_by() -> ApprovedBy {
    ApprovedBy {
        credential: "token".into(),
        issuer: None,
        subject: None,
        email: None,
    }
}

/// An approved request's device code.
fn approved(devices: &Devices, name: &str) -> String {
    let (code, user, ..) = devices.request(name, "ios", Scope::Decide).unwrap();
    devices.approve(&user, token_by(), Scope::Decide).unwrap();
    code
}

async fn store_with(paired: usize) -> (tempfile::TempDir, PathBuf, Arc<Devices>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("devices.sqlite3");
    let devices = Devices::open(Some(path.clone())).await.unwrap();
    for i in 0..paired {
        let code = approved(&devices, &format!("d{i}"));
        assert!(matches!(devices.poll(&code), Poll::Issued(..)));
    }
    (dir, path, devices)
}

fn stored_rows(path: &std::path::Path) -> i64 {
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM devices", [], |row| row.get(0))
        .unwrap()
}

#[tokio::test]
async fn remove_all_never_misses_a_token_being_issued() {
    let (_dir, path, devices) = store_with(1).await;
    let code = approved(&devices, "late");
    // Hold the store so the poll stops between taking its request and
    // writing the device; remove all arrives meanwhile.
    let slot = devices.storage_slot();
    let poller = {
        let devices = devices.clone();
        std::thread::spawn(move || devices.poll(&code))
    };
    std::thread::sleep(Duration::from_millis(100));
    let remover = {
        let devices = devices.clone();
        std::thread::spawn(move || devices.remove_all().unwrap())
    };
    std::thread::sleep(Duration::from_millis(100));
    drop(slot);
    let issued = poller.join().unwrap();
    let count = remover.join().unwrap();
    // Remove all ran after the issuance finished, and took it with it.
    let Poll::Issued(_, token) = issued else {
        panic!("{issued:?}")
    };
    assert_eq!(count, 2);
    assert!(devices
        .authenticate(Some(&format!("Bearer {token}")))
        .is_none());
    assert!(devices.list().is_empty());
    assert_eq!(stored_rows(&path), 0);
}

#[tokio::test]
async fn remove_all_cancels_approvals_not_yet_collected() {
    let devices = Devices::memory();
    let code = approved(&devices, "phone");
    devices.remove_all().unwrap();
    assert!(matches!(devices.poll(&code), Poll::Invalid));
}

#[tokio::test]
async fn parallel_issuance_never_exceeds_the_device_limit() {
    let (_dir, path, devices) = store_with(MAX_DEVICES - 1).await;
    let codes = [approved(&devices, "a"), approved(&devices, "b")];
    let slot = devices.storage_slot();
    let polls: Vec<_> = codes
        .into_iter()
        .map(|code| {
            let devices = devices.clone();
            std::thread::spawn(move || devices.poll(&code))
        })
        .collect();
    std::thread::sleep(Duration::from_millis(100));
    drop(slot);
    let results: Vec<Poll> = polls.into_iter().map(|p| p.join().unwrap()).collect();
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Poll::Issued(..)))
            .count(),
        1
    );
    assert_eq!(
        results.iter().filter(|r| matches!(r, Poll::Full)).count(),
        1
    );
    assert_eq!(devices.list().len(), MAX_DEVICES);
    drop(devices);
    assert_eq!(stored_rows(&path), i64::try_from(MAX_DEVICES).unwrap());
    // And the store still opens.
    assert_eq!(
        Devices::open(Some(path)).await.unwrap().list().len(),
        MAX_DEVICES
    );
}

/// A Fleet with one control host whose snapshot has an agent in `%3` and
/// a shell in `%4`; the fake relay answers `capture` with 1,000 lines and
/// records the pane keys it was asked for.
async fn fleet_with_an_agent() -> (
    crate::fleet::FleetRuntime,
    Arc<std::sync::Mutex<Vec<crate::topology::PaneKey>>>,
) {
    use crate::fleet::{FleetCommandResult, FleetOperation};
    let store = Arc::new(crate::fleet::FleetStore::new());
    let now = "2026-11-16T00:00:00Z";
    let pane = |id: &str| {
        json!({
            "pane_id": id, "session_id": "$1", "session": "work", "window_id": "@1",
            "window_name": "w", "window_index": "0", "pane_index": "0", "tty": "/dev/ttys1",
            "current_command": "node", "title": "t", "current_path": "/w", "pane_pid": 42,
            "socket": "default"
        })
    };
    let host: crate::fleet::FleetHostSnapshot = serde_json::from_value(json!({
        "alias": "mini", "local": false, "ssh_target": "jiun@mini", "labels": {}, "annotations": {},
        "mode": "control", "state": "online",
        "remote": {
            "revision": 1, "observed_at": now, "sessions": [], "backends": [],
            "panes": [pane("%3"), pane("%4")],
            "agents": [{
                "kind": "claude_code", "agent_session_id": "s-agent", "pane": "%3",
                "tmux_socket": "default", "state": "working", "started_at": now,
                "last_activity_at": now, "state_entered_at": now
            }]
        }
    }))
    .unwrap();
    store.upsert_host(host).await;
    let (runtime, mut commands) = crate::fleet::FleetRuntime::new(store);
    let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = asked.clone();
    tokio::spawn(async move {
        while let Some(command) = commands.recv().await {
            let result = match command.operation {
                FleetOperation::Capture { pane } => {
                    seen.lock().unwrap().push(pane);
                    let text = (0..1000)
                        .map(|i| format!("line {i}"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    Ok(FleetCommandResult::capture(Some(text)))
                }
                other => Err(format!("a device route sent {other:?}")),
            };
            let _ = command.reply.send(result);
        }
    });
    (runtime, asked)
}

#[tokio::test]
async fn an_operate_device_reads_one_agents_output_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let audit = dir.path().join("decisions.jsonl");
    let (runtime, asked) = fleet_with_an_agent().await;
    let app = router(
        state(Some(TOKEN))
            .with_fleet(Some(runtime))
            .with_decision_audit(Some(audit.clone())),
    );
    let (token, device) = pair_with(&app, "phone", Some("operate"), Some("operate")).await;
    let (_, me) = call(&app, "GET", "/api/device", Some(&token), None).await;
    assert_eq!(me["features"], json!(["capture", "work"]));
    let body = |session: &str| Some(json!({"host": "mini", "session_id": session}));

    let (status, out) = call(
        &app,
        "POST",
        "/api/device/capture",
        Some(&token),
        body("s-agent"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let text = out["capture"].as_str().unwrap();
    assert_eq!(text.lines().count(), 400, "bounded to the last lines");
    assert!(text.ends_with("line 999"));
    let keys = asked.lock().unwrap().clone();
    assert_eq!(keys.len(), 1);
    assert_eq!(
        keys[0].pane_id, "%3",
        "the agent's pane, resolved by the server"
    );

    // A pane without an agent, another host, an unknown session, extra
    // fields: refused before anything is sent.
    for (request, expected) in [
        (body("%4"), StatusCode::NOT_FOUND),
        (
            Some(json!({"host": "elsewhere", "session_id": "s-agent"})),
            StatusCode::NOT_FOUND,
        ),
        (body(""), StatusCode::NOT_FOUND),
        (
            Some(json!({"host": "mini", "session_id": "s-agent", "pane": "%4"})),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let (status, _) = call(
            &app,
            "POST",
            "/api/device/capture",
            Some(&token),
            request.clone(),
        )
        .await;
        assert_eq!(status, expected, "{request:?}");
    }
    assert_eq!(
        asked.lock().unwrap().len(),
        1,
        "no other capture reached the host"
    );

    // Too large to read: refused, and still recorded.
    let big = Some(json!({"host": "mini", "session_id": "x".repeat(4096)}));
    let (status, _) = call(&app, "POST", "/api/device/capture", Some(&token), big).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    // A decide device may not; the audit names every attempt.
    let (decide, _) = pair_with(&app, "other", None, None).await;
    let (status, _) = call(
        &app,
        "POST",
        "/api/device/capture",
        Some(&decide),
        body("s-agent"),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let lines: Vec<Value> = std::fs::read_to_string(&audit)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect();
    let action = |name: &str| {
        lines
            .iter()
            .filter(|l| l["action"] == name)
            .cloned()
            .collect::<Vec<_>>()
    };
    let captures = action("device_capture");
    assert_eq!(captures.len(), 6);
    assert_eq!(captures[0]["status"], 200);
    assert_eq!(captures[0]["account"]["device"], device.as_str());
    assert_eq!(captures[5]["status"], 413);
    let requested = action("device_capture_requested");
    assert_eq!(requested.len(), 1, "an intent line before the one dispatch");
    assert_eq!(requested[0]["pane"], "%3");
    assert_eq!(action("device_operate_refused").len(), 1);
    assert!(
        !serde_json::to_string(&lines).unwrap().contains("line 999"),
        "the audit never holds output"
    );
}

#[test]
fn capture_tails_are_bounded_in_lines_and_bytes() {
    let long = vec!["é".repeat(20_000); 10].join(
        "
",
    );
    let out = super::operate::tail(&long);
    assert!(out.len() <= 64 * 1024);
    assert!(out.chars().all(|c| c == 'é' || c == '\n'));
    assert_eq!(super::operate::tail("a\nb\n"), "a\nb");
}

#[test]
fn an_agents_pane_is_told_apart_by_its_endpoint_or_not_at_all() {
    let pane = |id: &str, socket: Option<&str>| -> crate::tmux::PaneInfo {
        serde_json::from_value(json!({
            "pane_id": id, "session_id": "$1", "session": "s", "window_id": "@1",
            "window_name": "w", "window_index": "0", "pane_index": "0", "tty": "/dev/ttys1",
            "current_command": "zsh", "title": "t", "current_path": "/", "pane_pid": 1,
            "socket": socket
        }))
        .unwrap()
    };
    let panes = [
        pane("%3", Some("private-shell")),
        pane("%3", Some("default")),
    ];
    let pick = |socket| super::operate::agent_pane(&panes, "%3", socket).map(|p| p.socket.clone());
    // No socket on the agent: two %3 panes, so neither.
    assert_eq!(pick(None), None);
    assert_eq!(pick(Some("default")), Some(Some("default".into())));
    assert_eq!(pick(Some("elsewhere")), None);
    // One %3 pane: an agent without a socket gets it.
    let single = [pane("%3", Some("default"))];
    assert!(super::operate::agent_pane(&single, "%3", None).is_some());
    // A pane that names no socket does not match an agent that names one.
    let unnamed = [pane("%3", None)];
    assert!(super::operate::agent_pane(&unnamed, "%3", Some("default")).is_none());
}

// ---------------------------------------------------------------------------
// Starting new work from a device.

type Launched = Arc<std::sync::Mutex<Vec<crate::work_control::WorkUpRequest>>>;

/// A Fleet with one control host, `mini`, whose agent works in `/w/tend`;
/// its relay lists the pipeline `solo` (and `routes`), and a stand-in
/// `work up` answers `outcome` and records what it was asked to start.
async fn work_app(
    allow: bool,
    routes: Value,
    outcome: fn() -> Result<Value, crate::work_control::WorkUpError>,
    audit: Option<std::path::PathBuf>,
    devices: Option<Arc<Devices>>,
) -> (Router, Launched) {
    use crate::fleet::{FleetCommandResult, FleetOperation};
    let store = Arc::new(crate::fleet::FleetStore::new());
    let now = "2026-11-16T00:00:00Z";
    let host: crate::fleet::FleetHostSnapshot = serde_json::from_value(json!({
        "alias": "mini", "local": false, "ssh_target": "jiun@mini", "labels": {}, "annotations": {},
        "mode": "control", "state": "online",
        "remote": {
            "revision": 1, "observed_at": now, "sessions": [], "backends": [], "panes": [],
            "agents": [{
                "kind": "claude_code", "agent_session_id": "s1", "pane": "%3", "cwd": "/w/tend",
                "state": "idle", "started_at": now, "last_activity_at": now, "state_entered_at": now
            }]
        }
    }))
    .unwrap();
    store.upsert_host(host).await;
    let (runtime, mut commands) = crate::fleet::FleetRuntime::new(store);
    tokio::spawn(async move {
        while let Some(command) = commands.recv().await {
            let result = match command.operation {
                FleetOperation::WorkCommand { args, stdin } => {
                    assert_eq!(args, ["work", "options", "--json"]);
                    assert!(stdin.is_none());
                    let mut result = FleetCommandResult::accepted("options");
                    result.command_exit_code = Some(0);
                    result.command_stdout = Some(
                        json!({
                            "config_path": "/home/x/.config/tend/config.toml",
                            "pipelines": [{"name": "solo", "description": "one implementer",
                                           "agents": [{"prompt": "secret prompt"}]}],
                            "routes": routes,
                        })
                        .to_string(),
                    );
                    Ok(result)
                }
                other => Err(format!("a device route sent {other:?}")),
            };
            let _ = command.reply.send(result);
        }
    });
    let launched: Launched = Arc::default();
    let seen = launched.clone();
    let runner: super::WorkUpRunner = Arc::new(move |request| {
        seen.lock().unwrap().push(request);
        Box::pin(async move { outcome() })
    });
    let mut cfg = DashboardConfig::loopback_default();
    cfg.token = Some(TOKEN.to_owned());
    cfg.allow_work_start = allow;
    let mut state = AppState::new(
        Store::shared(),
        Arc::new(cfg),
        Arc::new(PaneCache::new(Duration::from_secs(60))),
        crate::session::PtySessionBackend::shared(),
    )
    .with_fleet(Some(runtime))
    .with_work_up(runner)
    .with_decision_audit(audit);
    if let Some(devices) = devices {
        state = state.with_devices(devices);
    }
    (router(state), launched)
}

#[allow(clippy::unnecessary_wraps)] // the runner's shape
fn started() -> Result<Value, crate::work_control::WorkUpError> {
    Ok(json!({"work": "x"}))
}

fn timed_out() -> Result<Value, crate::work_control::WorkUpError> {
    Err(crate::work_control::WorkUpError::Timeout)
}

fn start_body(key: &str) -> Value {
    json!({"idempotency_key": key, "host": "mini", "project": "/w/tend",
           "pipeline": "solo", "request": "fix the login timeout"})
}

const KEY: &str = "6f2b1b8e-3a5f-4f62-9a55-3c1f0c7b9d11";
const KEY2: &str = "0b7f6a1c-2d3e-4f5a-8b9c-0d1e2f3a4b5c";

#[tokio::test]
async fn work_options_list_only_what_a_phone_chooses_from() {
    let (app, _) = work_app(true, json!([]), started, None, None).await;
    let (token, _) = pair_with(&app, "phone", Some("operate"), Some("operate")).await;
    let (status, options) = call(
        &app,
        "GET",
        "/api/device/work-options?host=mini",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{options}");
    assert_eq!(options["allow_work_start"], true);
    assert_eq!(options["projects"], json!(["/w/tend"]));
    assert_eq!(options["pipelines"][0]["name"], "solo");
    assert_eq!(options["pipelines"][0]["agents"], 1);
    let text = options.to_string();
    assert!(
        !text.contains("config_path") && !text.contains("secret prompt"),
        "{text}"
    );
    let (_, me) = call(&app, "GET", "/api/device", Some(&token), None).await;
    assert_eq!(me["features"], json!(["capture", "work"]));
}

#[tokio::test]
async fn a_device_starts_only_new_work_built_by_the_server() {
    let dir = tempfile::tempdir().unwrap();
    let audit = dir.path().join("decisions.jsonl");
    let (app, launched) = work_app(true, json!([]), started, Some(audit.clone()), None).await;
    let (token, device) = pair_with(&app, "phone", Some("operate"), Some("operate")).await;
    let (status, first) = call(
        &app,
        "POST",
        "/api/device/work",
        Some(&token),
        Some(start_body(KEY)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["state"], "started");
    let work = first["work"].as_str().unwrap().to_owned();
    assert!(work.starts_with("phone-"), "{work}");
    {
        let sent = launched.lock().unwrap();
        assert_eq!(sent.len(), 1);
        let request = &sent[0];
        assert_eq!(request.work, work, "the server mints the id");
        assert!(request.new_only && request.no_ticket && !request.dry_run);
        assert_eq!(
            request.cwd.as_deref(),
            Some(std::path::Path::new("/w/tend"))
        );
        assert_eq!(request.pipeline.as_deref(), Some("solo"));
        assert_eq!(request.host.as_deref(), Some("mini"));
        assert_eq!(request.body.as_deref(), Some("fix the login timeout"));
        assert!(
            request.workspace.is_none() && request.skill.is_none() && request.context.is_none()
        );
        assert!(
            request.external.is_none()
                && request.queue_lease.is_none()
                && request.route_key.is_none()
        );
    }
    // A retry with the same key gets the same start, and nothing runs again.
    let (status, again) = call(
        &app,
        "POST",
        "/api/device/work",
        Some(&token),
        Some(start_body(KEY)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["work"], work.as_str());
    assert_eq!(launched.lock().unwrap().len(), 1);
    // The same key for another request is refused.
    let mut other = start_body(KEY);
    other["request"] = json!("something else");
    let (status, _) = call(&app, "POST", "/api/device/work", Some(&token), Some(other)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    // Intent before, result after, never the request text.
    let lines = std::fs::read_to_string(&audit).unwrap();
    let actions: Vec<String> = lines
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|l| l["account"]["device"] == device.as_str())
        .map(|l| l["action"].as_str().unwrap_or("").to_owned())
        .collect();
    assert_eq!(actions, ["device_work_requested", "device_work_finished"]);
    assert!(!lines.contains("fix the login timeout"));
}

#[tokio::test]
async fn every_way_a_start_is_refused_runs_nothing() {
    let routes =
        json!([{"match": "^phone-", "cwd": "/elsewhere", "worktree": false, "prepare": false}]);
    for (allow, routes, body, expected) in [
        (
            false,
            json!([]),
            start_body(KEY),
            StatusCode::NOT_IMPLEMENTED,
        ),
        (
            true,
            json!([]),
            json!({"idempotency_key": KEY, "host": "mini", "project": "/w/other", "pipeline": "solo", "request": "x"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            true,
            json!([]),
            json!({"idempotency_key": KEY, "host": "mini", "project": "/w/tend/../etc", "pipeline": "solo", "request": "x"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            true,
            json!([]),
            json!({"idempotency_key": KEY, "host": "mini", "project": "/w/tend", "pipeline": "triad", "request": "x"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            true,
            json!([]),
            json!({"idempotency_key": KEY, "host": "mini", "project": "/w/tend", "pipeline": "solo", "request": "x", "workspace": "w"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            true,
            json!([]),
            json!({"idempotency_key": "not-a-uuid", "host": "mini", "project": "/w/tend", "pipeline": "solo", "request": "x"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            true,
            json!([]),
            json!({"idempotency_key": KEY, "host": "mini", "project": "/w/tend", "pipeline": "solo", "request": "  "}),
            StatusCode::BAD_REQUEST,
        ),
        (
            true,
            json!([]),
            json!({"idempotency_key": KEY, "host": "nowhere", "project": "/w/tend", "pipeline": "solo", "request": "x"}),
            StatusCode::NOT_FOUND,
        ),
        (true, routes, start_body(KEY), StatusCode::CONFLICT),
    ] {
        let (app, launched) = work_app(allow, routes, started, None, None).await;
        let (token, _) = pair_with(&app, "phone", Some("operate"), Some("operate")).await;
        let (status, answer) = call(
            &app,
            "POST",
            "/api/device/work",
            Some(&token),
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, expected, "{body} → {answer}");
        assert!(launched.lock().unwrap().is_empty(), "{body} ran");
    }
    // A decide device, and an operate device limited to deciding, may not.
    let (app, launched) = work_app(true, json!([]), started, None, None).await;
    let (decide, _) = pair_with(&app, "phone", None, None).await;
    let (status, _) = call(
        &app,
        "POST",
        "/api/device/work",
        Some(&decide),
        Some(start_body(KEY)),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (token, id) = pair_with(&app, "phone 2", Some("operate"), Some("operate")).await;
    call(
        &app,
        "POST",
        &format!("/api/devices/{id}/limit"),
        Some(TOKEN),
        None,
    )
    .await;
    let (status, _) = call(
        &app,
        "POST",
        "/api/device/work",
        Some(&token),
        Some(start_body(KEY)),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(launched.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_unwritable_audit_starts_nothing() {
    let dir = tempfile::tempdir().unwrap();
    // A directory where the audit file should be: the line cannot be written.
    let audit = dir.path().join("decisions.jsonl");
    std::fs::create_dir(&audit).unwrap();
    let (app, launched) = work_app(true, json!([]), started, Some(audit), None).await;
    let (token, _) = pair_with(&app, "phone", Some("operate"), Some("operate")).await;
    let (status, _) = call(
        &app,
        "POST",
        "/api/device/work",
        Some(&token),
        Some(start_body(KEY)),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(launched.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_uncertain_start_is_never_dispatched_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("devices.sqlite3");
    let devices = Devices::open(Some(path.clone())).await.unwrap();
    let (app, launched) = work_app(true, json!([]), timed_out, None, Some(devices)).await;
    let (token, device) = pair_with(&app, "phone", Some("operate"), Some("operate")).await;
    let (_, first) = call(
        &app,
        "POST",
        "/api/device/work",
        Some(&token),
        Some(start_body(KEY)),
    )
    .await;
    assert_eq!(first["state"], "unknown", "a timeout may have started it");
    let (_, again) = call(
        &app,
        "POST",
        "/api/device/work",
        Some(&token),
        Some(start_body(KEY)),
    )
    .await;
    assert_eq!(again["state"], "unknown");
    assert_eq!(launched.lock().unwrap().len(), 1, "never dispatched twice");
    drop(app);

    // A start cut off while reserved (tendd stopped mid-dispatch) comes
    // back unknown after a restart, and is not dispatched either.
    let devices = Devices::open(Some(path.clone())).await.unwrap();
    let print = vec![1u8; 32];
    assert!(matches!(
        devices
            .reserve_start(&device, KEY2, &print, "phone-1116-aaaaaaaaaaaa")
            .unwrap(),
        super::Reservation::New(_)
    ));
    // Busy: one start per device in flight.
    assert!(matches!(
        devices
            .reserve_start(&device, "another", &print, "phone-1116-bbbbbbbbbbbb")
            .unwrap(),
        super::Reservation::Busy
    ));
    drop(devices);
    let reopened = Devices::open(Some(path)).await.unwrap();
    match reopened
        .reserve_start(&device, KEY2, &print, "phone-1116-cccccccccccc")
        .unwrap()
    {
        super::Reservation::Replay(start) => {
            assert_eq!(start.state, "unknown");
            assert_eq!(start.work, "phone-1116-aaaaaaaaaaaa");
        }
        other => panic!("expected the earlier start, got {other:?}"),
    }
}
