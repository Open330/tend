//! Push against a fake APNs endpoint: an HTTP/2 cleartext server in the
//! test that records each request and answers what the test says.
use super::payload::{collapse_id, item_ref, Kind};
use super::*;
use crate::dashboard::devices::{ApprovedBy, Devices, Poll, PushRegistered, Scope};
use crate::dashboard::server::router;
use crate::dashboard::DashboardConfig;
use crate::review::{ActorKind, NewReview, ReviewActor, ReviewStore, ReviewTarget};
use crate::state::Store;
use crate::tmux::scanner::PaneCache;
use axum::body::{to_bytes, Body};
use axum::http::{header, Request as HttpRequest, StatusCode};
use axum::Router;
use base64::Engine as _;
use p256::ecdsa::{signature::Verifier, SigningKey, VerifyingKey};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, Ordering};
use tower::ServiceExt;

const TOKEN: &str = "s3cret";
const PHONE_A: &str = "AABBCCDDEEFF00112233445566778899aabbccddeeff00112233445566778899";
const PHONE_B: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const START: i64 = 1_800_000_000;

// ---- the fake APNs endpoint ------------------------------------------------

#[derive(Debug, Clone)]
struct Recorded {
    path: String,
    headers: BTreeMap<String, String>,
    body: Value,
}

/// An answer for the first request whose path starts with `prefix`.
type Answer = (String, u16, String);

/// Run once on receipt of the next request (given its path), before it is
/// answered: what changes while a send waits for Apple.
type Hook = Box<dyn FnOnce(&str) + Send>;

#[derive(Clone)]
struct Fake {
    url: String,
    requests: Arc<Mutex<Vec<Recorded>>>,
    answers: Arc<Mutex<Vec<Answer>>>,
    before_response: Arc<Mutex<Option<Hook>>>,
}

impl Fake {
    async fn start() -> Self {
        use http_body_util::{BodyExt, Full};
        use hyper::body::Bytes;
        use hyper_util::rt::{TokioExecutor, TokioIo};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests: Arc<Mutex<Vec<Recorded>>> = Arc::default();
        let answers: Arc<Mutex<Vec<Answer>>> = Arc::default();
        let before_response: Arc<Mutex<Option<Hook>>> = Arc::default();
        let hook = before_response.clone();
        let (rec, ans) = (requests.clone(), answers.clone());
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let (rec, ans, hook) = (rec.clone(), ans.clone(), hook.clone());
                let service = hyper::service::service_fn(
                    move |req: hyper::Request<hyper::body::Incoming>| {
                        let (rec, ans, hook) = (rec.clone(), ans.clone(), hook.clone());
                        async move {
                            let (parts, body) = req.into_parts();
                            let bytes = body
                                .collect()
                                .await
                                .map(http_body_util::Collected::to_bytes)
                                .unwrap_or_default();
                            let path = parts.uri.path().to_owned();
                            let headers = parts
                                .headers
                                .iter()
                                .map(|(k, v)| {
                                    (k.as_str().to_owned(), v.to_str().unwrap_or("").to_owned())
                                })
                                .collect();
                            rec.lock().unwrap().push(Recorded {
                                path: path.clone(),
                                headers,
                                body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
                            });
                            let callback = hook.lock().unwrap().take();
                            if let Some(callback) = callback {
                                callback(&path);
                            }
                            let (status, answer) = {
                                let mut answers = ans.lock().unwrap();
                                match answers
                                    .iter()
                                    .position(|(p, ..)| path.starts_with(p.as_str()))
                                {
                                    Some(i) => {
                                        let (_, status, body) = answers.remove(i);
                                        (status, body)
                                    }
                                    None => (200, String::new()),
                                }
                            };
                            Ok::<_, std::convert::Infallible>(
                                hyper::Response::builder()
                                    .status(status)
                                    .body(Full::new(Bytes::from(answer)))
                                    .unwrap(),
                            )
                        }
                    },
                );
                tokio::spawn(async move {
                    let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        Self {
            url,
            requests,
            answers,
            before_response,
        }
    }

    fn answer(&self, prefix: &str, status: u16, body: Value) {
        self.answers
            .lock()
            .unwrap()
            .push((prefix.to_owned(), status, body.to_string()));
    }

    fn take(&self) -> Vec<Recorded> {
        std::mem::take(&mut *self.requests.lock().unwrap())
    }
}

// ---- keys, push, the app ---------------------------------------------------

fn test_key() -> SigningKey {
    SigningKey::from_slice(&[0x42; 32]).unwrap()
}

#[cfg(unix)]
fn key_pem() -> String {
    use p256::pkcs8::{EncodePrivateKey, LineEnding};
    test_key().to_pkcs8_pem(LineEnding::LF).unwrap().to_string()
}

/// A clock that moves one second each time it is read, from `START`.
fn ticking_clock() -> (apns::Clock, Arc<AtomicI64>) {
    let at = Arc::new(AtomicI64::new(START));
    let read = at.clone();
    (Arc::new(move || read.fetch_add(1, Ordering::SeqCst)), at)
}

fn sender(fake: &Fake, clock: apns::Clock) -> apns::Apns {
    let token = apns::ProviderToken::new(test_key(), "ABC123DEFG".into(), "728FW73BS8".into());
    apns::Apns::with_endpoint(&fake.url, "dev.tend.ios".into(), token, clock)
}

fn push_on(fake: &Fake, environments: &[Environment], allow_content: bool) -> Arc<Push> {
    let (clock, _) = ticking_clock();
    Push::on(sender(fake, clock), environments.to_vec(), allow_content)
}

struct Harness {
    app: Router,
    state: AppState,
    audit: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

#[allow(clippy::unused_async)] // called like the async harness
async fn harness_with(push: Arc<Push>, devices: Option<Arc<Devices>>) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let audit = dir.path().join("decisions.jsonl");
    let mut cfg = DashboardConfig::loopback_default();
    cfg.token = Some(TOKEN.into());
    let mut state = AppState::new(
        Store::shared(),
        Arc::new(cfg),
        Arc::new(PaneCache::new(Duration::from_secs(60))),
        crate::session::PtySessionBackend::shared(),
    )
    .with_reviews(ReviewStore::in_memory())
    .with_decision_audit(Some(audit.clone()))
    .with_push(push);
    if let Some(devices) = devices {
        state = state.with_devices(devices);
    }
    Harness {
        app: router(state.clone()),
        state,
        audit,
        _dir: dir,
    }
}

async fn harness(push: Arc<Push>) -> Harness {
    harness_with(push, None).await
}

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    bearer: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    call_raw(app, method, uri, bearer, body.map(|b| b.to_string()), &[]).await
}

async fn call_raw(
    app: &Router,
    method: &str,
    uri: &str,
    bearer: Option<&str>,
    body: Option<String>,
    extra: &[(&str, &str)],
) -> (StatusCode, Value) {
    let mut request = HttpRequest::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(bearer) = bearer {
        request = request.header(header::AUTHORIZATION, format!("Bearer {bearer}"));
    }
    for (name, value) in extra {
        request = request.header(*name, *value);
    }
    let body = body.map_or_else(Body::empty, Body::from);
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

/// Pair a device over HTTP: `(device token, device id)`.
async fn pair(app: &Router, name: &str) -> (String, String) {
    let (status, asked) = call(
        app,
        "POST",
        "/auth/devices/request",
        None,
        Some(json!({"name": name, "platform": "ios"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
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
        Some(json!({"device_code": asked["device_code"]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    (
        issued["token"].as_str().unwrap().to_owned(),
        issued["device"]["id"].as_str().unwrap().to_owned(),
    )
}

/// Pair a device through the registry directly.
fn pair_direct(devices: &Devices, name: &str) -> String {
    let by = ApprovedBy {
        credential: "token".into(),
        issuer: None,
        subject: None,
        email: None,
    };
    let (code, user, ..) = devices.request(name, "ios", Scope::Decide).unwrap();
    devices.approve(&user, by, Scope::Decide).unwrap();
    let Poll::Issued(device, _) = devices.poll(&code) else {
        panic!("not issued")
    };
    device.id
}

async fn register(
    app: &Router,
    device: &str,
    token: &str,
    environment: &str,
    content: bool,
) -> (StatusCode, Value) {
    call(
        app,
        "POST",
        "/api/device/push",
        Some(device),
        Some(json!({"token": token, "environment": environment, "content": content})),
    )
    .await
}

async fn review(state: &AppState, summary: &str) -> String {
    state
        .reviews
        .create(NewReview {
            target: ReviewTarget::Session {
                provider: None,
                session_id: "s".into(),
            },
            summary: summary.into(),
            anchor: None,
            idempotency_key: None,
            requested_by: ReviewActor {
                kind: ActorKind::Agent,
                label: Some("claude".into()),
            },
            decision: None,
        })
        .await
        .unwrap()
        .id
}

/// An agent blocked in its pane with `message`.
async fn waiting(state: &AppState, session_id: &str, message: &str) {
    use crate::event::{AgentEvent, AgentId, AgentKind, NotificationLevel};
    let id = AgentId {
        tmux_socket: None,
        kind: AgentKind::ClaudeCode,
        session_id: session_id.into(),
        surface: None,
        pane: Some(format!("%{}", session_id.len() * 7 + message.len())),
        cwd: None,
    };
    let at = time::OffsetDateTime::now_utc();
    state
        .store
        .apply(&AgentEvent::Started { id: id.clone(), at })
        .await;
    state
        .store
        .apply(&AgentEvent::NotificationFired {
            id,
            level: NotificationLevel::NeedsInput,
            message: message.into(),
            at,
        })
        .await;
}

async fn decide(app: &Router, id: &str, extra: &[(&str, &str)]) {
    let (status, body) = call_raw(
        app,
        "POST",
        &format!("/api/decisions/reviews/{id}"),
        Some(TOKEN),
        Some(json!({"disposition": "accepted"}).to_string()),
        extra,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

fn s(secs: u64) -> Duration {
    Duration::from_secs(secs)
}

async fn tick(h: &Harness, at: u64) {
    h.state.push.tick_at(&h.state, s(at)).await;
}

fn unb64(text: &str) -> Vec<u8> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(text)
        .unwrap()
}

/// The JWT's header and claims, after checking its signature.
fn verified_jwt(jwt: &str) -> (Value, Value) {
    let parts: Vec<&str> = jwt.split('.').collect();
    assert_eq!(parts.len(), 3);
    let signature = p256::ecdsa::Signature::from_slice(&unb64(parts[2])).unwrap();
    VerifyingKey::from(&test_key())
        .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
        .expect("signed by the key");
    (
        serde_json::from_slice(&unb64(parts[0])).unwrap(),
        serde_json::from_slice(&unb64(parts[1])).unwrap(),
    )
}

fn audit_lines(h: &Harness) -> Vec<Value> {
    std::fs::read_to_string(&h.audit)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

// ---- registration ----------------------------------------------------------

/// The app compares this with its own SHA-256 of token NUL environment
/// NUL channel NUL (`LifecycleTests` in apps/tend-ios has the same vector).
#[test]
fn the_registration_fingerprint_is_a_fixed_function_of_token_environment_and_channel() {
    let row = crate::dashboard::devices::PushRow {
        device: "dev_1".into(),
        token: PHONE_A.to_ascii_lowercase(),
        environment: "production".into(),
        content: false,
        channel: "ch_0123".into(),
        ref_key: vec![0; 32],
        generation: 1,
        updated_at: 0,
    };
    assert_eq!(row.registration(), "344b519bd7247b4fe517b9edaf77801e");
}

#[tokio::test]
async fn the_device_read_says_what_this_device_registered() {
    let fake = Fake::start().await;
    let h = harness(push_on(&fake, &[Environment::Production], true)).await;
    let (device, _) = pair(&h.app, "phone").await;
    let (other, _) = pair(&h.app, "tablet").await;
    let (_, me) = call(&h.app, "GET", "/api/device", Some(&device), None).await;
    assert_eq!(me["push_registration"], Value::Null);
    let (status, body) = register(&h.app, &device, PHONE_A, "production", true).await;
    assert_eq!(status, StatusCode::OK);
    let row = h
        .state
        .devices
        .push_of(me["device"]["id"].as_str().unwrap())
        .unwrap();
    assert_eq!(row.channel, body["channel"].as_str().unwrap());
    let (_, me) = call(&h.app, "GET", "/api/device", Some(&device), None).await;
    assert_eq!(
        me["push_registration"],
        json!({"registration": row.registration(), "content": true})
    );
    // Another device sees only its own.
    let (_, them) = call(&h.app, "GET", "/api/device", Some(&other), None).await;
    assert_eq!(them["push_registration"], Value::Null);
    call(
        &h.app,
        "POST",
        "/api/device/push/remove",
        Some(&device),
        None,
    )
    .await;
    let (_, me) = call(&h.app, "GET", "/api/device", Some(&device), None).await;
    assert_eq!(me["push_registration"], Value::Null);
}

#[tokio::test]
async fn push_off_stores_nothing_and_says_so() {
    let h = harness(Push::off(None)).await;
    let (device, _) = pair(&h.app, "phone").await;
    let (status, body) = register(&h.app, &device, PHONE_A, "production", false).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"enabled": false, "reason": "off"}));
    assert!(!h.state.devices.has_push());
    let (_, me) = call(&h.app, "GET", "/api/device", Some(&device), None).await;
    assert_eq!(
        me["push"],
        json!({"enabled": false, "content_allowed": false, "environments": []})
    );
}

#[tokio::test]
async fn an_environment_the_key_is_not_for_is_refused_and_nothing_is_sent() {
    let fake = Fake::start().await;
    let h = harness(push_on(&fake, &[Environment::Production], true)).await;
    let (device, _) = pair(&h.app, "phone").await;
    let (status, body) = register(&h.app, &device, PHONE_A, "development", false).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"enabled": false, "reason": "environment"}));
    assert!(!h.state.devices.has_push());
    tick(&h, 0).await;
    review(&h.state, "new").await;
    tick(&h, 15).await;
    assert!(fake.take().is_empty());
    let (_, me) = call(&h.app, "GET", "/api/device", Some(&device), None).await;
    assert_eq!(
        me["push"],
        json!({"enabled": true, "content_allowed": true, "environments": ["production"]})
    );
}

#[tokio::test]
async fn registration_bodies_are_bounded_strict_and_case_blind() {
    let fake = Fake::start().await;
    let h = harness(push_on(&fake, &[Environment::Production], false)).await;
    let (device, id) = pair(&h.app, "phone").await;
    // Over 1 KiB.
    let big = json!({"token": "a".repeat(1100), "environment": "production"}).to_string();
    let (status, _) = call_raw(
        &h.app,
        "POST",
        "/api/device/push",
        Some(&device),
        Some(big),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    // Unknown fields, bad hex, wrong lengths, unknown environments.
    for body in [
        json!({"token": PHONE_A, "environment": "production", "content": false, "extra": 1}),
        json!({"token": "zz".repeat(32), "environment": "production"}),
        json!({"token": "ab".repeat(15), "environment": "production"}),
        json!({"token": "ab".repeat(101), "environment": "production"}),
        json!({"token": "abc", "environment": "production"}),
        json!({"token": PHONE_A, "environment": "staging"}),
        json!({"environment": "production"}),
    ] {
        let (status, _) = call(
            &h.app,
            "POST",
            "/api/device/push",
            Some(&device),
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }
    assert!(!h.state.devices.has_push());
    // Only a device token reaches the route.
    let (status, _) = register(&h.app, TOKEN, PHONE_A, "production", false).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // Mixed-case hex is one token: the second registration keeps the
    // channel and ref key.
    let (status, first) = register(&h.app, &device, PHONE_A, "production", true).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["enabled"], true);
    assert_eq!(first["content"], false, "the host does not allow content");
    assert!(first["channel"].as_str().unwrap().starts_with("ch_"));
    assert_eq!(first["channel"].as_str().unwrap().len(), 3 + 32);
    let key = base64::engine::general_purpose::STANDARD
        .decode(first["ref_key"].as_str().unwrap())
        .unwrap();
    assert_eq!(key.len(), 32);
    let (_, second) = register(
        &h.app,
        &device,
        &PHONE_A.to_lowercase(),
        "production",
        false,
    )
    .await;
    assert_eq!(second["channel"], first["channel"]);
    assert_eq!(second["ref_key"], first["ref_key"]);
    let targets = h.state.devices.push_targets();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].token, PHONE_A.to_lowercase());
    // The audit names the device, environment and flag, never the token.
    let lines = std::fs::read_to_string(&h.audit).unwrap();
    assert!(!lines.to_lowercase().contains(&PHONE_A.to_lowercase()));
    let registered: Vec<Value> = audit_lines(&h)
        .into_iter()
        .filter(|line| line["action"] == "push_register")
        .collect();
    assert_eq!(registered.len(), 2);
    assert_eq!(registered[0]["id"], id.as_str());
    assert_eq!(registered[0]["environment"], "production");
    // Removing it.
    let (status, removed) = call(
        &h.app,
        "POST",
        "/api/device/push/remove",
        Some(&device),
        None,
    )
    .await;
    assert_eq!(
        (status, removed["removed"].clone()),
        (StatusCode::OK, json!(true))
    );
    assert!(!h.state.devices.has_push());
    let lines = std::fs::read_to_string(&h.audit).unwrap();
    assert!(lines.contains("\"push_remove\""));
}

// ---- payloads --------------------------------------------------------------

#[tokio::test]
async fn without_content_a_payload_names_no_item_and_refs_match() {
    let fake = Fake::start().await;
    let h = harness(push_on(&fake, &[Environment::Production], true)).await;
    let (device, _) = pair(&h.app, "phone").await;
    let (_, reg) = register(&h.app, &device, PHONE_A, "production", false).await;
    let ref_key = base64::engine::general_purpose::STANDARD
        .decode(reg["ref_key"].as_str().unwrap())
        .unwrap();
    waiting(&h.state, "already-waiting", "old").await;
    tick(&h, 0).await;
    assert!(fake.take().is_empty(), "the baseline announces nothing");
    // A human-readable and a path-like session id, and a review.
    let readable = "ios-cache-fix-login";
    let pathlike = "/Users/dev/work/tend/.claude/sess-1";
    waiting(&h.state, readable, "Approve the deploy to prod?").await;
    waiting(&h.state, pathlike, "Which branch?").await;
    let review_id = review(&h.state, "check the secret migration").await;
    tick(&h, 15).await;
    let sent = fake.take();
    assert_eq!(sent.len(), 1, "one alert per host per send");
    let alert = &sent[0];
    let raw = alert.body.to_string();
    for secret in [
        readable,
        pathlike,
        "deploy",
        "branch",
        "secret migration",
        review_id.as_str(),
        "claude",
        "/Users",
    ] {
        assert!(!raw.contains(secret), "{secret} in {raw}");
    }
    assert_eq!(alert.body["aps"]["category"], "TEND_OPEN");
    assert_eq!(alert.body["aps"]["alert"]["loc-args"], json!(["3"]));
    assert_eq!(alert.body["aps"]["alert"]["body"], "3 new");
    assert_eq!(
        alert.body["aps"]["alert"]["title"],
        "Tend: this host needs you"
    );
    assert_eq!(alert.body["aps"]["badge"], 4);
    assert_eq!(alert.body["aps"]["thread-id"], "this host");
    assert_eq!(alert.body["tend"]["v"], 1);
    assert_eq!(alert.body["tend"]["channel"], reg["channel"]);
    // The newest is one of the three; its ref is the HMAC over it.
    let refs: Vec<String> = [
        (Kind::Waiting, readable),
        (Kind::Waiting, pathlike),
        (Kind::Review, review_id.as_str()),
    ]
    .iter()
    .map(|(kind, id)| item_ref(&ref_key, "this host", *kind, id))
    .collect();
    assert!(refs.contains(&alert.body["tend"]["ref"].as_str().unwrap().to_owned()));
    // The request itself.
    assert_eq!(
        alert.path,
        format!("/production/3/device/{}", PHONE_A.to_lowercase())
    );
    assert_eq!(alert.headers["apns-topic"], "dev.tend.ios");
    assert_eq!(alert.headers["apns-push-type"], "alert");
    assert_eq!(alert.headers["apns-priority"], "10");
    assert_eq!(alert.headers["apns-collapse-id"], collapse_id("this host"));
    let expiration: i64 = alert.headers["apns-expiration"].parse().unwrap();
    assert!(
        (START + 6 * 3600..START + 6 * 3600 + 10).contains(&expiration),
        "{expiration}"
    );
    let jwt = alert.headers["authorization"]
        .strip_prefix("bearer ")
        .unwrap();
    let (header, claims) = verified_jwt(jwt);
    assert_eq!(header, json!({"alg": "ES256", "kid": "ABC123DEFG"}));
    assert_eq!(claims["iss"], "728FW73BS8");
    assert!(claims["iat"].as_i64().unwrap() >= START);
}

#[tokio::test]
async fn content_is_sent_only_when_both_sides_allow_it_at_send_time() {
    let fake = Fake::start().await;
    // The host allows content; one phone asks for it, one does not.
    let h = harness(push_on(&fake, &[Environment::Production], true)).await;
    let (asks, _) = pair(&h.app, "asks").await;
    let (quiet, _) = pair(&h.app, "quiet").await;
    let (_, reg) = register(&h.app, &asks, PHONE_A, "production", true).await;
    assert_eq!(reg["content"], true);
    register(&h.app, &quiet, PHONE_B, "production", false).await;
    tick(&h, 0).await;
    let id = review(&h.state, "## Ship *the* `migration`?\nsecond line").await;
    tick(&h, 15).await;
    let sent = fake.take();
    assert_eq!(sent.len(), 2);
    let to = |token: &str| {
        sent.iter()
            .find(|r| r.path.ends_with(&token.to_lowercase()))
            .unwrap()
            .body
            .clone()
    };
    let with = to(PHONE_A);
    assert_eq!(with["aps"]["alert"]["body"], "Ship the migration?");
    assert_eq!(with["aps"]["alert"]["subtitle"], "claude");
    assert_eq!(with["aps"]["category"], "TEND_REVIEW");
    assert!(with["aps"]["alert"].get("loc-key").is_none());
    let without = to(PHONE_B);
    assert_eq!(without["aps"]["category"], "TEND_OPEN");
    assert!(!without.to_string().contains("migration"));
    assert!(!without.to_string().contains(&id));
}

#[tokio::test]
async fn a_host_that_does_not_allow_content_never_sends_it() {
    let fake = Fake::start().await;
    let h = harness(push_on(&fake, &[Environment::Production], false)).await;
    let (device, _) = pair(&h.app, "phone").await;
    let (_, reg) = register(&h.app, &device, PHONE_A, "production", true).await;
    assert_eq!(reg["content"], false);
    tick(&h, 0).await;
    review(&h.state, "the text").await;
    tick(&h, 15).await;
    let sent = fake.take();
    assert_eq!(sent.len(), 1);
    assert!(!sent[0].body.to_string().contains("the text"));
    assert_eq!(sent[0].body["aps"]["category"], "TEND_OPEN");
}

// ---- the provider token ----------------------------------------------------

#[test]
fn the_provider_token_verifies_and_is_renewed_after_40_minutes() {
    let mut token = apns::ProviderToken::new(test_key(), "ABC123DEFG".into(), "728FW73BS8".into());
    let first = token.get(START, false);
    let (header, claims) = verified_jwt(&first);
    assert_eq!(header, json!({"alg": "ES256", "kid": "ABC123DEFG"}));
    assert_eq!(claims, json!({"iss": "728FW73BS8", "iat": START}));
    assert_eq!(token.get(START + 39 * 60, false), first);
    let renewed = token.get(START + 40 * 60, false);
    assert_ne!(renewed, first);
    assert_eq!(verified_jwt(&renewed).1["iat"], START + 40 * 60);
}

#[tokio::test]
async fn anexpired_provider_token_is_signed_again_and_retried_once() {
    let fake = Fake::start().await;
    let h = harness(push_on(&fake, &[Environment::Production], false)).await;
    let (device, _) = pair(&h.app, "phone").await;
    register(&h.app, &device, PHONE_A, "production", false).await;
    tick(&h, 0).await;
    review(&h.state, "x").await;
    fake.answer(
        "/production",
        403,
        json!({"reason": "ExpiredProviderToken"}),
    );
    tick(&h, 15).await;
    let sent = fake.take();
    assert_eq!(sent.len(), 2, "one retry");
    let first = sent[0].headers["authorization"].clone();
    let second = sent[1].headers["authorization"].clone();
    assert_ne!(first, second, "a new token");
    verified_jwt(second.strip_prefix("bearer ").unwrap());
    // Accepted: nothing pending.
    review(&h.state, "y").await;
    tick(&h, 50).await;
    assert_eq!(fake.take().len(), 1);
}

#[tokio::test]
async fn a_bad_environment_key_turns_off_only_that_environment() {
    let fake = Fake::start().await;
    let h = harness(push_on(
        &fake,
        &[Environment::Production, Environment::Development],
        false,
    ))
    .await;
    let (prod, _) = pair(&h.app, "release").await;
    let (dev, _) = pair(&h.app, "debug").await;
    register(&h.app, &prod, PHONE_A, "production", false).await;
    register(&h.app, &dev, PHONE_B, "development", false).await;
    tick(&h, 0).await;
    review(&h.state, "x").await;
    fake.answer(
        "/development",
        403,
        json!({"reason": "BadEnvironmentKeyIdInToken"}),
    );
    tick(&h, 15).await;
    assert_eq!(fake.take().len(), 2);
    review(&h.state, "y").await;
    tick(&h, 50).await;
    let sent = fake.take();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].path.starts_with("/production/"));
    let (_, list) = call(&h.app, "GET", "/api/devices", Some(TOKEN), None).await;
    assert_eq!(list["push"]["refused_environments"], json!(["development"]));
    // The registration is kept: a restart with a fixed key reaches it.
    assert_eq!(h.state.devices.push_targets().len(), 2);
}

// ---- Apple's answers and the races ------------------------------------------

#[tokio::test]
async fn a_410_drops_the_registration_and_is_audited_without_the_token() {
    let fake = Fake::start().await;
    let h = harness(push_on(&fake, &[Environment::Production], false)).await;
    let (device, id) = pair(&h.app, "phone").await;
    register(&h.app, &device, PHONE_A, "production", false).await;
    tick(&h, 0).await;
    review(&h.state, "x").await;
    fake.answer(
        "/production",
        410,
        json!({"reason": "Unregistered", "timestamp": (START + 3600) * 1000}),
    );
    tick(&h, 15).await;
    assert_eq!(fake.take().len(), 1);
    assert!(!h.state.devices.has_push());
    let dropped = audit_lines(&h)
        .into_iter()
        .find(|line| line["action"] == "push_dropped")
        .unwrap();
    assert_eq!(dropped["id"], id.as_str());
    assert_eq!(dropped["reason"], "Unregistered");
    let lines = std::fs::read_to_string(&h.audit).unwrap();
    assert!(!lines.to_lowercase().contains(&PHONE_A.to_lowercase()));
    // Nothing more is sent to it.
    review(&h.state, "y").await;
    tick(&h, 50).await;
    assert!(fake.take().is_empty());
}

#[tokio::test]
async fn bad_device_token_and_not_for_topic_drop_it_too() {
    for reason in ["BadDeviceToken", "DeviceTokenNotForTopic"] {
        let fake = Fake::start().await;
        let h = harness(push_on(&fake, &[Environment::Production], false)).await;
        let (device, _) = pair(&h.app, "phone").await;
        register(&h.app, &device, PHONE_A, "production", false).await;
        tick(&h, 0).await;
        review(&h.state, "x").await;
        fake.answer("/production", 400, json!({"reason": reason}));
        tick(&h, 15).await;
        assert!(!h.state.devices.has_push(), "{reason}");
    }
}

/// A late `410` or bad token answer, to a send made before the row changed,
/// never deletes the newer row.
#[tokio::test]
async fn late_answers_never_delete_a_newer_registration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("devices.sqlite3");
    let devices = Devices::open(Some(path.clone())).await.unwrap();
    let a = pair_direct(&devices, "a");
    let b = pair_direct(&devices, "b");
    let stored = |devices: &Devices| -> Vec<PushRow> { devices.push_targets() };
    let row = |r: PushRegistered| match r {
        PushRegistered::Stored(row) => row,
        other => panic!("{other:?}"),
    };
    let token_1 = "aa".repeat(32);
    let token_2 = "bb".repeat(32);

    // Rotated: the old token's answer keeps the new row.
    let old = row(devices
        .register_push(&a, &token_1, "production", false)
        .unwrap());
    let new = row(devices
        .register_push(&a, &token_2, "production", false)
        .unwrap());
    assert_ne!(old.channel, new.channel, "a new token mints a new channel");
    assert!(!devices.drop_push(&old, None).unwrap());
    assert_eq!(stored(&devices), vec![new.clone()]);

    // Registered again with the same token: a new generation, same channel.
    let again = row(devices
        .register_push(&a, &token_2, "production", true)
        .unwrap());
    assert_eq!(
        (again.channel.clone(), again.ref_key.clone()),
        (new.channel.clone(), new.ref_key.clone())
    );
    assert!(again.generation > new.generation);
    assert!(!devices.drop_push(&new, None).unwrap());
    assert_eq!(stored(&devices), vec![again.clone()]);

    // Moved to another device (a phone that paired again).
    let moved = row(devices
        .register_push(&b, &token_2, "production", false)
        .unwrap());
    assert_ne!(moved.channel, again.channel);
    assert_ne!(moved.ref_key, again.ref_key);
    assert!(!devices.drop_push(&again, None).unwrap());
    assert_eq!(stored(&devices), vec![moved.clone()]);

    // Removed and registered again.
    assert!(devices.remove_push(&b).unwrap());
    let back = row(devices
        .register_push(&b, &token_2, "production", false)
        .unwrap());
    assert!(!devices.drop_push(&moved, None).unwrap());
    assert_eq!(stored(&devices), vec![back.clone()]);

    // A 410 whose timestamp (ms) is before the row's update keeps it, and
    // so does one in the same millisecond; one after it drops it.
    let at = back.updated_at;
    assert!(at > 1_000_000_000_000, "stored in milliseconds: {at}");
    assert!(!devices.drop_push(&back, Some(at - 1)).unwrap());
    assert!(!devices.drop_push(&back, Some(at)).unwrap(), "the same ms");
    assert_eq!(stored(&devices).len(), 1);
    assert!(devices.drop_push(&back, Some(at + 1)).unwrap());
    assert!(stored(&devices).is_empty());

    // The store agrees after a restart.
    let kept = row(devices
        .register_push(&a, &token_1, "development", false)
        .unwrap());
    drop(devices);
    let reopened = Devices::open(Some(path)).await.unwrap();
    assert_eq!(reopened.push_targets(), vec![kept.clone()]);
    // The counter keeps growing past what was stored.
    let next = row(reopened
        .register_push(&a, &token_1, "development", false)
        .unwrap());
    assert!(next.generation > kept.generation);
}

#[tokio::test]
async fn a_late_410_after_a_reregistration_keeps_it_end_to_end() {
    let fake = Fake::start().await;
    let h = harness(push_on(&fake, &[Environment::Production], false)).await;
    let (device, _) = pair(&h.app, "phone").await;
    register(&h.app, &device, PHONE_A, "production", false).await;
    let sent_with = h.state.devices.push_targets().remove(0);
    // The phone launches again meanwhile, then the 410 for the earlier send
    // arrives with a timestamp from before that.
    register(&h.app, &device, PHONE_A, "production", false).await;
    let outcome = h
        .state
        .push
        .settle(
            &h.state,
            &Planned {
                send: tracker::Send::Badge {
                    device: sent_with.device.clone(),
                    generation: sent_with.generation,
                    badge: 0,
                },
                row: sent_with.clone(),
                request: apns::Request {
                    environment: Environment::Production,
                    token: sent_with.token.clone(),
                    payload: json!({}),
                    collapse_id: None,
                    alert: false,
                },
            },
            apns::Outcome::Unregistered(Some(sent_with.updated_at - 60_000)),
        )
        .await;
    assert_eq!(outcome, tracker::Result::Dropped);
    assert!(h.state.devices.has_push(), "the newer row survives");
    assert!(!audit_lines(&h)
        .iter()
        .any(|line| line["action"] == "push_dropped"));
}

// ---- devices -----------------------------------------------------------------

#[tokio::test]
async fn a_removed_orexpired_device_is_never_sent_to_and_its_row_goes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("devices.sqlite3");
    let devices = Devices::open(Some(path.clone())).await.unwrap();
    let fake = Fake::start().await;
    let h = harness_with(
        push_on(&fake, &[Environment::Production], false),
        Some(devices.clone()),
    )
    .await;
    let (kept, _) = pair(&h.app, "kept").await;
    let (removed, removed_id) = pair(&h.app, "removed").await;
    let (expired, expired_id) = pair(&h.app, "expired").await;
    let (signed_out, _) = pair(&h.app, "signed-out").await;
    let tokens = [
        "01".repeat(32),
        "02".repeat(32),
        "03".repeat(32),
        "04".repeat(32),
    ];
    for (device, token) in [&kept, &removed, &expired, &signed_out].iter().zip(&tokens) {
        let (status, _) = register(&h.app, device, token, "production", false).await;
        assert_eq!(status, StatusCode::OK);
    }
    let push_rows = || -> i64 {
        rusqlite::Connection::open(&path)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM device_push", [], |row| row.get(0))
            .unwrap()
    };
    assert_eq!(push_rows(), 4);
    let (status, _) = call(
        &h.app,
        "POST",
        &format!("/api/devices/{removed_id}/remove"),
        Some(TOKEN),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &h.app,
        "POST",
        "/api/device/forget",
        Some(&signed_out),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    devices.age_device(&expired_id, 0, 31 * 24 * 3600);
    tick(&h, 0).await;
    review(&h.state, "x").await;
    tick(&h, 15).await;
    let sent = fake.take();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].path.ends_with(&tokens[0]));
    assert_eq!(
        push_rows(),
        1,
        "removed, signed out and expired rows are deleted"
    );
    // Remove all takes the rest.
    let (status, _) = call(&h.app, "POST", "/api/devices/remove-all", Some(TOKEN), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(push_rows(), 0);
    assert!(!devices.has_push());
}

#[tokio::test]
async fn limiting_a_device_keeps_its_registration() {
    let dir = tempfile::tempdir().unwrap();
    let devices = Devices::open(Some(dir.path().join("d.sqlite3")))
        .await
        .unwrap();
    let id = pair_direct(&devices, "phone");
    devices
        .register_push(&id, &"0a".repeat(32), "production", false)
        .unwrap();
    assert!(devices.limit(&id).unwrap());
    assert_eq!(devices.push_targets().len(), 1);
}

#[tokio::test]
async fn a_row_left_by_an_older_tendds_removal_is_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("devices.sqlite3");
    let devices = Devices::open(Some(path.clone())).await.unwrap();
    let id = pair_direct(&devices, "phone");
    let other = pair_direct(&devices, "other");
    devices
        .register_push(&id, &"0a".repeat(32), "production", false)
        .unwrap();
    devices
        .register_push(&other, &"0b".repeat(32), "production", false)
        .unwrap();
    drop(devices);
    // An older tendd knows nothing of device_push: it deletes the device only.
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute("DELETE FROM devices WHERE id = ?1", [&id])
        .unwrap();
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 2, "the schema version stays 2");
    drop(connection);
    let reopened = Devices::open(Some(path.clone())).await.unwrap();
    let rows = reopened.push_targets();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].device, other);
    let left: i64 = rusqlite::Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM device_push WHERE device_id = ?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn the_16_row_cap_holds_with_orphans_deleted_first() {
    let devices = Devices::memory();
    let mut ids = Vec::new();
    for i in 0..16 {
        let id = pair_direct(&devices, &format!("d{i}"));
        let token = format!("{i:02x}").repeat(32);
        assert!(matches!(
            devices
                .register_push(&id, &token, "production", false)
                .unwrap(),
            PushRegistered::Stored(_)
        ));
        ids.push(id);
    }
    assert_eq!(devices.push_targets().len(), 16);
    // One expires; its row is an orphan until the next read.
    devices.age_device(&ids[0], 0, 31 * 24 * 3600);
    let late = pair_direct(&devices, "late");
    assert!(matches!(
        devices
            .register_push(&late, &"ff".repeat(32), "production", false)
            .unwrap(),
        PushRegistered::Stored(_)
    ));
    let rows = devices.push_targets();
    assert_eq!(rows.len(), 16);
    assert!(!rows.iter().any(|row| row.device == ids[0]));
}

#[tokio::test]
async fn a_token_moves_and_a_repair_gets_a_new_channel_and_ref_key() {
    let fake = Fake::start().await;
    let h = harness(push_on(&fake, &[Environment::Production], false)).await;
    let (old, _) = pair(&h.app, "phone").await;
    let (_, before) = register(&h.app, &old, PHONE_A, "production", false).await;
    // The same phone pairs again and registers the same token.
    let (new, new_id) = pair(&h.app, "phone").await;
    let (_, after) = register(&h.app, &new, PHONE_A, "production", false).await;
    assert_ne!(after["channel"], before["channel"]);
    assert_ne!(after["ref_key"], before["ref_key"]);
    let rows = h.state.devices.push_targets();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].device, new_id);
    tick(&h, 0).await;
    review(&h.state, "x").await;
    tick(&h, 15).await;
    let sent = fake.take();
    assert_eq!(sent.len(), 1, "notified once");
    // A payload from the old pairing fails the new pairing's channel check.
    assert_eq!(sent[0].body["tend"]["channel"], after["channel"]);
    assert_ne!(sent[0].body["tend"]["channel"], before["channel"]);
}

// ---- sending rules end to end -------------------------------------------------

#[tokio::test]
async fn a_429_keeps_the_pending_set_and_badge_updates_go_down_to_zero() {
    let fake = Fake::start().await;
    let h = harness(push_on(&fake, &[Environment::Production], false)).await;
    let (device, _) = pair(&h.app, "phone").await;
    register(&h.app, &device, PHONE_A, "production", false).await;
    tick(&h, 0).await;
    let first = review(&h.state, "x").await;
    let second = review(&h.state, "y").await;
    fake.answer("/production", 429, json!({"reason": "TooManyRequests"}));
    tick(&h, 15).await;
    assert_eq!(fake.take().len(), 1);
    // Backing off for 30 s: nothing.
    tick(&h, 30).await;
    assert!(fake.take().is_empty());
    // One decided meanwhile: the rebuilt alert counts one.
    decide(&h.app, &first, &[]).await;
    tick(&h, 45).await;
    let sent = fake.take();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].body["aps"]["alert"]["body"], "1 new");
    assert_eq!(sent[0].body["aps"]["badge"], 1);
    // Decided: a badge-only update to 0, priority 5, expiration 0, no
    // collapse id; refused once, then retried a minute later.
    decide(&h.app, &second, &[]).await;
    fake.answer("/production", 500, json!({"reason": "InternalServerError"}));
    tick(&h, 60).await;
    let sent = fake.take();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].body, json!({"aps": {"badge": 0}}));
    assert_eq!(sent[0].headers["apns-priority"], "5");
    assert_eq!(sent[0].headers["apns-expiration"], "0");
    assert!(!sent[0].headers.contains_key("apns-collapse-id"));
    tick(&h, 75).await;
    assert!(fake.take().is_empty(), "backing off and within the minute");
    tick(&h, 120).await;
    assert_eq!(fake.take().len(), 1);
    tick(&h, 240).await;
    assert!(fake.take().is_empty(), "accepted: nothing more");
}

#[tokio::test]
async fn no_registration_reads_nothing_and_sends_nothing() {
    let fake = Fake::start().await;
    let h = harness(push_on(&fake, &[Environment::Production], false)).await;
    review(&h.state, "x").await;
    tick(&h, 0).await;
    tick(&h, 15).await;
    assert!(fake.take().is_empty());
}

// ---- key files and config -------------------------------------------------------

#[cfg(unix)]
fn write_key(dir: &std::path::Path, name: &str, contents: &str, mode: u32) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, contents).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    path
}

#[cfg(unix)]
fn push_toml(key_path: &std::path::Path) -> toml::Value {
    toml::Value::try_from(json!({
        "enabled": true, "key_path": key_path.display().to_string(),
        "key_id": "ABC123DEFG", "team_id": "728FW73BS8",
    }))
    .unwrap()
}

#[cfg(unix)]
#[test]
fn a_good_key_loads_and_every_unsafe_one_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let pem = key_pem();
    let uid = nix::unistd::getuid().as_raw();
    let good = write_key(dir.path(), "good.p8", &pem, 0o600);
    assert!(key::load(&good).is_ok());
    let read_only = write_key(dir.path(), "ro.p8", &pem, 0o400);
    assert!(key::load(&read_only).is_ok());
    let wide = write_key(dir.path(), "wide.p8", &pem, 0o644);
    let error = key::load(&wide).unwrap_err();
    assert!(error.contains("permissions"), "{error}");
    let link = dir.path().join("link.p8");
    std::os::unix::fs::symlink(&good, &link).unwrap();
    let error = key::load(&link).unwrap_err();
    assert!(error.contains("symlink"), "{error}");
    // Owned by someone else: the owner check runs on the descriptor's
    // uid (chown needs root, so the expected uid is moved instead).
    let error = key::load_as(&good, uid.wrapping_add(1)).unwrap_err();
    assert!(error.contains("owned"), "{error}");
    let big = write_key(dir.path(), "big.p8", &"A".repeat(5000), 0o600);
    let error = key::load(&big).unwrap_err();
    assert!(error.contains("larger"), "{error}");
    let garbage = write_key(
        dir.path(),
        "garbage.p8",
        "-----BEGIN PRIVATE KEY-----\nnot a key secret-ish\n-----END PRIVATE KEY-----\n",
        0o600,
    );
    let error = key::load(&garbage).unwrap_err();
    assert!(!error.contains("secret-ish"), "never the contents: {error}");
    let error = key::load(dir.path()).unwrap_err();
    assert!(error.contains("regular"), "{error}");
    let error = key::load(&dir.path().join("missing.p8")).unwrap_err();
    assert!(!error.contains("BEGIN"));
    // A key's contents never appear in an error.
    for error in [key::load(&wide).unwrap_err(), key::load(&big).unwrap_err()] {
        assert!(!error.contains(pem.lines().nth(1).unwrap()));
    }
}

#[cfg(unix)]
#[test]
fn a_fifo_with_no_writer_is_refused_without_blocking() {
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("fifo.p8");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(made.success());
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fifo, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    // The whole start, under a deadline: the open must not wait for a writer.
    let config = push_toml(&fifo);
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let push = Push::start(Some(&config));
        let _ = send.send((push.enabled(), push.reason.clone()));
    });
    let (enabled, reason) = receive
        .recv_timeout(Duration::from_secs(5))
        .expect("the start did not block on the FIFO");
    assert!(!enabled);
    assert!(reason.unwrap().contains("regular"));
}

#[cfg(unix)]
#[tokio::test]
async fn a_bad_push_config_leaves_push_off_and_the_dashboard_serving() {
    let dir = tempfile::tempdir().unwrap();
    let wide = write_key(dir.path(), "wide.p8", &key_pem(), 0o644);
    let good = write_key(dir.path(), "good.p8", &key_pem(), 0o600);
    let with = |extra: Value| {
        let mut base = json!({
            "enabled": true, "key_path": good.display().to_string(),
            "key_id": "ABC123DEFG", "team_id": "728FW73BS8",
        });
        for (k, v) in extra.as_object().unwrap() {
            if v.is_null() {
                base.as_object_mut().unwrap().remove(k);
            } else {
                base[k] = v.clone();
            }
        }
        toml::Value::try_from(base).unwrap()
    };
    let cases = [
        (push_toml(&wide), "permissions"),
        (with(json!({"key_id": "short"})), "key_id"),
        (with(json!({"team_id": null})), "team_id"),
        (
            with(json!({"environments": ["staging"]})),
            "unknown environment",
        ),
        (with(json!({"environments": []})), "empty"),
        (
            with(json!({"endpoint": "https://evil.invalid"})),
            "invalid [dashboard.push]",
        ),
        (
            with(json!({"key_path": dir.path().join("none.p8").display().to_string()})),
            "cannot be opened",
        ),
        (with(json!({"topic": "a b"})), "topic"),
    ];
    for (config, expected) in cases {
        let push = Push::start(Some(&config));
        assert!(!push.enabled());
        let reason = push.reason.clone().unwrap();
        assert!(reason.contains(expected), "{reason} for {config}");
    }
    // Not enabled, or absent: off with no reason.
    assert!(Push::start(None).reason.is_none());
    assert!(Push::start(Some(&with(json!({"enabled": false}))))
        .reason
        .is_none());
    // A good one is on, with the defaults.
    let push = Push::start(Some(&with(json!({}))));
    assert!(push.enabled());
    assert_eq!(push.environments, vec![Environment::Production]);
    assert!(!push.allow_content());
    // Tilde-expanded.
    let parsed = settings(Some(&with(json!({"key_path": "~/k.p8"}))))
        .unwrap()
        .unwrap();
    assert!(!parsed.key_path.starts_with("~"));
    assert_eq!(parsed.topic, "dev.tend.ios");
    // The dashboard serves with push off, and says why to operators.
    let h = harness(Push::start(Some(&push_toml(&wide)))).await;
    let (device, _) = pair(&h.app, "phone").await;
    let (status, me) = call(&h.app, "GET", "/api/device", Some(&device), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["push"]["enabled"], false);
    let (_, list) = call(&h.app, "GET", "/api/devices", Some(TOKEN), None).await;
    assert_eq!(list["push"]["enabled"], false);
    assert!(list["push"]["reason"]
        .as_str()
        .unwrap()
        .contains("permissions"));
    let (_, reg) = register(&h.app, &device, PHONE_A, "production", false).await;
    assert_eq!(reg["reason"], "off");
}

// ---- secrets -------------------------------------------------------------------

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn no_log_line_or_error_holds_the_token_path_or_the_authorization() {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let fake = Fake::start().await;
    let h = harness(push_on(
        &fake,
        &[Environment::Production, Environment::Development],
        false,
    ))
    .await;
    let (a, _) = pair(&h.app, "a").await;
    let (b, _) = pair(&h.app, "b").await;
    register(&h.app, &a, PHONE_A, "production", false).await;
    register(&h.app, &b, PHONE_B, "development", false).await;
    tick(&h, 0).await;
    review(&h.state, "x").await;
    fake.answer("/production", 500, json!({"reason": "InternalServerError"}));
    fake.answer(
        "/development",
        403,
        json!({"reason": "InvalidProviderToken"}),
    );
    tick(&h, 15).await;
    fake.answer("/production", 410, json!({"reason": "Unregistered"}));
    tick(&h, 60).await;
    // A connection that fails: the error is its kind only.
    let (clock, _) = ticking_clock();
    let dead = apns::Apns::with_endpoint(
        "http://127.0.0.1:9",
        "dev.tend.ios".into(),
        apns::ProviderToken::new(test_key(), "ABC123DEFG".into(), "728FW73BS8".into()),
        clock,
    );
    let outcome = dead
        .send(&apns::Request {
            environment: Environment::Production,
            token: PHONE_A.to_lowercase(),
            payload: json!({}),
            collapse_id: None,
            alert: true,
        })
        .await;
    let apns::Outcome::Retry(reason) = &outcome else {
        panic!("{outcome:?}")
    };
    let mut everything = format!("{reason} {outcome:?}");
    everything.push_str(&String::from_utf8_lossy(&captured.0.lock().unwrap()));
    everything.push_str(&std::fs::read_to_string(&h.audit).unwrap());
    let _ = std::fmt::Write::write_fmt(
        &mut everything,
        format_args!("{:?}", h.state.devices.push_targets()),
    );
    let lower = everything.to_lowercase();
    assert!(!lower.contains(&PHONE_A.to_lowercase()), "{everything}");
    assert!(!lower.contains(&PHONE_B.to_lowercase()));
    assert!(!lower.contains("/3/device"));
    assert!(!lower.contains("bearer "));
    assert!(!lower.contains("authorization"));
    // What was logged happened: the refused environment, the dropped token.
    assert_eq!(
        h.state.push.operator_view()["refused_environments"],
        json!(["development"])
    );
    assert!(audit_lines(&h)
        .iter()
        .any(|line| line["action"] == "push_dropped"));
}

// ---- the decision audit -----------------------------------------------------------

#[tokio::test]
async fn a_decision_from_a_notification_is_audited_as_such() {
    let h = harness(Push::off(None)).await;
    let one = review(&h.state, "one").await;
    let two = review(&h.state, "two").await;
    let three = review(&h.state, "three").await;
    decide(&h.app, &one, &[("x-tend-via", "notification")]).await;
    decide(&h.app, &two, &[("x-tend-via", "notification; evil")]).await;
    decide(&h.app, &three, &[]).await;
    let via: BTreeMap<String, Value> = audit_lines(&h)
        .into_iter()
        .filter(|line| line["action"] == "review")
        .map(|line| (line["id"].as_str().unwrap().to_owned(), line["via"].clone()))
        .collect();
    assert_eq!(via[&one], json!("notification"));
    assert_eq!(via[&two], Value::Null);
    assert_eq!(via[&three], Value::Null);
}

#[tokio::test]
async fn reviewer_removing_a_later_target_during_send_must_stop_it() {
    let fake = Fake::start().await;
    let h = harness(push_on(&fake, &[Environment::Production], true)).await;
    let (a, aid) = pair(&h.app, "a").await;
    let (b, bid) = pair(&h.app, "b").await;
    register(&h.app, &a, PHONE_A, "production", true).await;
    register(&h.app, &b, PHONE_B, "production", true).await;
    tick(&h, 0).await;
    review(&h.state, "SECRET REVIEW BODY").await;
    // The test endpoint mutates state on receipt of the first request,
    // before returning its response and allowing the second send: it
    // removes the registration of whichever device was not sent to first.
    *fake.before_response.lock().unwrap() = Some(Box::new({
        let devices = h.state.devices.clone();
        move |path: &str| {
            let later = if path.ends_with(&PHONE_A.to_lowercase()) {
                bid
            } else {
                aid
            };
            devices.remove_push(&later).unwrap();
        }
    }));
    tick(&h, 15).await;
    let sent = fake.take();
    assert_eq!(
        sent.len(),
        1,
        "a removed registration was still sent its prebuilt content: {sent:?}"
    );
}

#[tokio::test]
async fn reviewer_same_registration_retry_must_not_erase_pending() {
    let fake = Fake::start().await;
    let h = harness(push_on(&fake, &[Environment::Production], false)).await;
    let (a, _) = pair(&h.app, "a").await;
    register(&h.app, &a, PHONE_A, "production", false).await;
    tick(&h, 0).await;
    review(&h.state, "RETRY ME").await;
    fake.answer("/production", 429, json!({"reason":"TooManyRequests"}));
    tick(&h, 15).await;
    assert_eq!(fake.take().len(), 1);
    register(&h.app, &a, PHONE_A, "production", false).await;
    tick(&h, 50).await;
    assert_eq!(
        fake.take().len(),
        1,
        "same-registration refresh discarded pending without a 200"
    );
}

#[tokio::test]
async fn a_content_opt_out_during_an_earlier_send_holds_for_the_later_one() {
    let fake = Fake::start().await;
    let h = harness(push_on(&fake, &[Environment::Production], true)).await;
    let (a, aid) = pair(&h.app, "a").await;
    let (b, bid) = pair(&h.app, "b").await;
    register(&h.app, &a, PHONE_A, "production", true).await;
    register(&h.app, &b, PHONE_B, "production", true).await;
    tick(&h, 0).await;
    review(&h.state, "SECRET REVIEW BODY").await;
    // While the first send waits for Apple, the other phone turns its
    // content switch off (the same token and environment, registered again).
    let later_token = Arc::new(Mutex::new(String::new()));
    *fake.before_response.lock().unwrap() = Some(Box::new({
        let devices = h.state.devices.clone();
        let later_token = later_token.clone();
        move |path: &str| {
            let (later, token) = if path.ends_with(&PHONE_A.to_lowercase()) {
                (bid, PHONE_B.to_lowercase())
            } else {
                (aid, PHONE_A.to_lowercase())
            };
            devices
                .register_push(&later, &token, "production", false)
                .unwrap();
            *later_token.lock().unwrap() = token;
        }
    }));
    tick(&h, 15).await;
    let sent = fake.take();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].body["aps"]["alert"]["body"], "SECRET REVIEW BODY");
    // The later phone still gets its alert, from the same pending set, and
    // without the text it opted out of.
    tick(&h, 45).await;
    let sent = fake.take();
    assert_eq!(sent.len(), 1, "{sent:?}");
    let later_token = later_token.lock().unwrap().clone();
    assert!(sent[0].path.ends_with(&later_token));
    assert!(
        !sent[0].body.to_string().contains("SECRET"),
        "{:?}",
        sent[0].body
    );
    assert_eq!(sent[0].body["aps"]["category"], "TEND_OPEN");
}

// Push needs a key file read with unix checks; it is off on other systems.
#[cfg(unix)]
#[tokio::test]
async fn a_failed_start_leaves_no_push_sender_and_a_stopped_one_ends() {
    use std::sync::atomic::Ordering::SeqCst;
    // The only test that starts the sender: `RUNNING` is process-wide.
    let dir = tempfile::tempdir().unwrap();
    let key = write_key(dir.path(), "key.p8", &key_pem(), 0o600);
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut cfg = DashboardConfig::loopback_default();
    cfg.bind = taken.local_addr().unwrap();
    cfg.push = Some(push_toml(&key));
    let serve = |cfg: DashboardConfig, shutdown| {
        crate::dashboard::server::serve(
            Arc::new(cfg),
            Store::shared(),
            Arc::new(PaneCache::new(Duration::from_secs(60))),
            crate::session::PtySessionBackend::shared(),
            Vec::new(),
            crate::dashboard::server::DashboardRuntimeConfig::default(),
            shutdown,
        )
    };
    let (stop, shutdown) = tokio::sync::broadcast::channel(1);
    serve(cfg.clone(), shutdown)
        .await
        .expect_err("the address is taken");
    tokio::task::yield_now().await;
    assert_eq!(
        RUNNING.load(SeqCst),
        0,
        "a push sender outlived a failed start"
    );

    // Started, then shut down: the sender stops with the server.
    drop(taken);
    cfg.bind = "127.0.0.1:0".parse().unwrap();
    let (stop2, shutdown) = tokio::sync::broadcast::channel(1);
    let server = tokio::spawn(serve(cfg, shutdown));
    let started = async {
        while RUNNING.load(SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(10), started)
        .await
        .expect("the sender starts");
    stop2.send(()).unwrap();
    server.await.unwrap().unwrap();
    let stopped = async {
        while RUNNING.load(SeqCst) != 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(10), stopped)
        .await
        .expect("the sender stops with the server");
    drop(stop);
}
