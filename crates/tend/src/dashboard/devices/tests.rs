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
    let (status, asked) = call(
        app,
        "POST",
        "/auth/devices/request",
        None,
        Some(json!({"name": name, "platform": "ios"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{asked}");
    let device_code = asked["device_code"].as_str().unwrap().to_owned();
    let (status, _) = call(
        app,
        "POST",
        "/api/devices/approve",
        Some(TOKEN),
        Some(json!({"user_code": asked["user_code"]})),
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
    let (code, user, ..) = devices.request("phone", "ios").unwrap();
    devices
        .approve(
            &user,
            ApprovedBy {
                credential: "token".into(),
                issuer: None,
                subject: None,
                email: None,
            },
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
    for (method, uri) in [
        ("GET", "/api/agents"),
        ("GET", "/api/access"),
        ("GET", "/api/panes"),
        ("GET", "/api/works"),
        ("GET", "/api/collaboration"),
        ("GET", "/api/events"),
        ("GET", "/api/panes/%251/output"),
        ("GET", "/api/panes/%251/output/stream"),
        ("POST", "/api/panes/%251/prompt"),
        ("POST", "/api/panes/%251/abort"),
        ("POST", "/api/fleet/mini/command"),
        ("POST", "/api/work-control/prompt"),
        ("POST", "/api/work-control/up"),
        ("POST", "/api/terminal-sessions/x/input"),
        ("GET", "/api/operators"),
        ("GET", "/api/shares"),
        ("GET", "/api/devices"),
        ("POST", "/api/devices/approve"),
        ("POST", "/api/devices/remove-all"),
        ("POST", &format!("/api/devices/{device}/remove")),
    ] {
        let (status, _) = call(&app, method, uri, Some(&token), Some(json!({}))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri}");
    }
    // And the operator token is not a device: /api/device is for devices.
    let (status, _) = call(&app, "GET", "/api/device", Some(TOKEN), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
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
        let (code, user, ..) = devices.request(name, "ios").unwrap();
        devices.approve(&user, by.clone()).unwrap();
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
    let (code, user, ..) = devices.request("phone", "ios").unwrap();
    devices
        .approve(
            &user,
            ApprovedBy {
                credential: "session".into(),
                issuer: Some("https://id".into()),
                subject: Some("u1".into()),
                email: Some("a@b".into()),
            },
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
    let (code, user, ..) = devices.request(name, "ios").unwrap();
    devices.approve(&user, token_by()).unwrap();
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
