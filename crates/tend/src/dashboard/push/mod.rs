//! Push notifications to paired iOS devices, sent by this host straight to
//! Apple with the publishing team's key (`[dashboard.push]`).
//!
//! A phone registers its APNs token (`POST /api/device/push`); every
//! [`TICK`] the sender reads exactly what Your turn shows (the decision
//! queue of every host this dashboard covers) and tells each registered
//! phone what is new. By default a notification carries the host alias,
//! counts and an opaque `ref`; the item's text only when the host's
//! `allow_content` and the phone's `content` both allow it at send time.
//!
//! A bad push config never stops tendd or the dashboard: push stays off,
//! the reason is logged once at start, and `GET /api/device` says push is
//! off. The key never appears in a log or an error, and neither does a
//! device token or the request path that carries it.
//!
//! - [`key`]: opening and checking the `.p8`.
//! - [`apns`]: the HTTP/2 client and the ES256 provider token.
//! - [`payload`]: payloads, the `ref`, the collapse id, the canonical line.
//! - [`tracker`]: when to notify whom, as a pure state machine.
mod apns;
mod key;
mod payload;
#[cfg(test)]
mod tests;
mod tracker;

use crate::dashboard::devices::PushRow;
use crate::dashboard::server::AppState;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How often the queue is read while anyone is registered.
pub(in crate::dashboard) const TICK: Duration = Duration::from_secs(15);
/// The queue limit the sender reads with, as Your turn's sources do.
const QUEUE_LIMIT: usize = 500;
const DEFAULT_TOPIC: &str = "dev.tend.ios";

/// An APNs environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::dashboard) enum Environment {
    /// `TestFlight` and store builds: `api.push.apple.com`.
    Production,
    /// Debug builds on a device: `api.sandbox.push.apple.com`.
    Development,
}

impl Environment {
    pub(in crate::dashboard) fn parse(value: &str) -> Option<Self> {
        match value {
            "production" => Some(Self::Production),
            "development" => Some(Self::Development),
            _ => None,
        }
    }

    pub(in crate::dashboard) fn as_str(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Development => "development",
        }
    }
}

/// `[dashboard.push]` as written.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PushToml {
    #[serde(default)]
    enabled: bool,
    key_path: Option<String>,
    key_id: Option<String>,
    team_id: Option<String>,
    topic: Option<String>,
    environments: Option<Vec<String>>,
    #[serde(default)]
    allow_content: bool,
}

/// What a valid section resolves to, before the key is read.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Settings {
    key_path: std::path::PathBuf,
    key_id: String,
    team_id: String,
    topic: String,
    environments: Vec<Environment>,
    allow_content: bool,
}

fn apple_id(value: Option<&str>, name: &str) -> Result<String, String> {
    let value = value.ok_or_else(|| format!("dashboard.push.{name} is missing"))?;
    if value.len() == 10 && value.chars().all(|c| c.is_ascii_alphanumeric()) {
        Ok(value.to_ascii_uppercase())
    } else {
        Err(format!(
            "dashboard.push.{name} must be the 10 characters from Apple's developer site"
        ))
    }
}

/// `Ok(None)` when push is not enabled. Never reads the key.
fn settings(raw: Option<&toml::Value>) -> Result<Option<Settings>, String> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let section: PushToml = raw.clone().try_into().map_err(|error: toml::de::Error| {
        format!("invalid [dashboard.push]: {}", error.message())
    })?;
    if !section.enabled {
        return Ok(None);
    }
    let key_path = section
        .key_path
        .filter(|path| !path.trim().is_empty())
        .ok_or("dashboard.push.key_path is missing")?;
    let key_id = apple_id(section.key_id.as_deref(), "key_id")?;
    let team_id = apple_id(section.team_id.as_deref(), "team_id")?;
    let topic = section.topic.unwrap_or_else(|| DEFAULT_TOPIC.into());
    if topic.is_empty()
        || topic.len() > 155
        || !topic
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
    {
        return Err("dashboard.push.topic must be the app's bundle identifier".into());
    }
    let names = section
        .environments
        .unwrap_or_else(|| vec!["production".into()]);
    let mut environments = Vec::new();
    for name in &names {
        let environment = Environment::parse(name).ok_or_else(|| {
            format!(
                "dashboard.push.environments: unknown environment {name:?} \
                 (use \"production\" and/or \"development\")"
            )
        })?;
        if !environments.contains(&environment) {
            environments.push(environment);
        }
    }
    if environments.is_empty() {
        return Err("dashboard.push.environments is empty".into());
    }
    Ok(Some(Settings {
        key_path: crate::accounts::expand_home(&key_path).into(),
        key_id,
        team_id,
        topic,
        environments,
        allow_content: section.allow_content,
    }))
}

/// Push as this dashboard runs it: on, or off with why.
pub(in crate::dashboard) struct Push {
    /// `None` while push is off.
    sender: Option<apns::Apns>,
    /// Why push is off: `None` when it is on, or was never enabled.
    reason: Option<String>,
    environments: Vec<Environment>,
    allow_content: bool,
    /// Environments Apple refused the provider token for, until restart.
    disabled: Mutex<BTreeSet<Environment>>,
    tracker: Mutex<tracker::Tracker>,
    origin: Instant,
}

/// A send planned, and the registration it goes to.
struct Planned {
    send: tracker::Send,
    row: PushRow,
    request: apns::Request,
}

impl Push {
    /// Push off: not configured (`reason` `None`) or refused (with why).
    pub(in crate::dashboard) fn off(reason: Option<String>) -> Arc<Self> {
        Arc::new(Self {
            sender: None,
            reason,
            environments: Vec::new(),
            allow_content: false,
            disabled: Mutex::default(),
            tracker: Mutex::default(),
            origin: Instant::now(),
        })
    }

    fn on(sender: apns::Apns, environments: Vec<Environment>, allow_content: bool) -> Arc<Self> {
        Arc::new(Self {
            sender: Some(sender),
            reason: None,
            environments,
            allow_content,
            disabled: Mutex::default(),
            tracker: Mutex::default(),
            origin: Instant::now(),
        })
    }

    /// Push from `[dashboard.push]`. Anything wrong leaves it off, with the
    /// reason logged once, here.
    pub(in crate::dashboard) fn start(raw: Option<&toml::Value>) -> Arc<Self> {
        match Self::configure(raw) {
            Ok(Some(push)) => {
                let environments: Vec<&str> =
                    push.environments.iter().map(|e| e.as_str()).collect();
                tracing::info!(
                    environments = ?environments,
                    allow_content = push.allow_content,
                    "dashboard push notifications on"
                );
                push
            }
            Ok(None) => Self::off(None),
            Err(reason) => {
                tracing::warn!(%reason, "dashboard push notifications are off");
                Self::off(Some(reason))
            }
        }
    }

    fn configure(raw: Option<&toml::Value>) -> Result<Option<Arc<Self>>, String> {
        let Some(settings) = settings(raw)? else {
            return Ok(None);
        };
        let key = key::load(&settings.key_path)?;
        let token = apns::ProviderToken::new(key, settings.key_id, settings.team_id);
        let sender = apns::Apns::new(settings.topic, token)?;
        Ok(Some(Self::on(
            sender,
            settings.environments,
            settings.allow_content,
        )))
    }

    pub(in crate::dashboard) fn enabled(&self) -> bool {
        self.sender.is_some()
    }

    /// Whether a registration for `environment` is accepted.
    pub(in crate::dashboard) fn allows(&self, environment: Environment) -> bool {
        self.enabled() && self.environments.contains(&environment)
    }

    pub(in crate::dashboard) fn allow_content(&self) -> bool {
        self.enabled() && self.allow_content
    }

    /// `GET /api/device`'s `push`.
    pub(in crate::dashboard) fn device_view(&self) -> Value {
        json!({
            "enabled": self.enabled(),
            "content_allowed": self.allow_content(),
            "environments": self
                .environments
                .iter()
                .filter(|_| self.enabled())
                .map(|e| e.as_str())
                .collect::<Vec<_>>(),
        })
    }

    /// `GET /api/devices`'s `push`, for **access ▸ Devices**.
    pub(in crate::dashboard) fn operator_view(&self) -> Value {
        let disabled: Vec<&str> = self.disabled_list().iter().map(|e| e.as_str()).collect();
        json!({
            "enabled": self.enabled(),
            "reason": self.reason,
            "content_allowed": self.allow_content(),
            "environments": self.environments.iter().map(|e| e.as_str()).collect::<Vec<_>>(),
            "refused_environments": disabled,
        })
    }

    fn disabled_list(&self) -> std::sync::MutexGuard<'_, BTreeSet<Environment>> {
        self.disabled
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn tracker(&self) -> std::sync::MutexGuard<'_, tracker::Tracker> {
        self.tracker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    /// The registrations to send to now: live devices, in an environment
    /// configured and not refused.
    fn targets(&self, state: &AppState) -> Vec<PushRow> {
        let disabled = self.disabled_list().clone();
        state
            .devices
            .push_targets()
            .into_iter()
            .filter(|row| {
                Environment::parse(&row.environment)
                    .is_some_and(|e| self.environments.contains(&e) && !disabled.contains(&e))
            })
            .collect()
    }

    /// One read of the queue and the sends it calls for, at `now`.
    pub(in crate::dashboard) async fn tick(&self, state: &AppState) {
        self.tick_at(state, self.now()).await;
    }

    async fn tick_at(&self, state: &AppState, now: Duration) {
        let Some(sender) = &self.sender else {
            return;
        };
        if !state.devices.has_push() {
            return;
        }
        let rows = self.targets(state);
        if rows.is_empty() {
            return;
        }
        let sections = super::decisions::queue_sections(state, QUEUE_LIMIT).await;
        let reads: Vec<tracker::HostRead> = sections.iter().map(host_read).collect();
        // Consent is read again here, after the queue: a registration that
        // changed meanwhile is the one sent to.
        let targets: Vec<tracker::Target> = self
            .targets(state)
            .iter()
            .map(|row| tracker::Target {
                operate: operates(state, &row.device),
                device: row.device.clone(),
                registration: row.registration(),
                generation: row.generation,
                content: row.content,
            })
            .collect();
        // Home's Your turn per host, from this same queue read: the badge
        // of a device that may operate (apps/shared/Design/COUNTS.md).
        let home_turn = super::home::your_turn_by_host(state, &sections).await;
        let sends = {
            let mut tracker = self.tracker();
            tracker.observe(now, &reads, &targets);
            tracker.observe_home(&home_turn, &reads);
            tracker.plan(now, &targets, self.allow_content)
        };
        for send in sends {
            // Each request is built only now, after every earlier send has
            // been answered: a registration removed, moved, refreshed or
            // opted out of content meanwhile, an environment Apple refused,
            // or a back-off since, is honoured.
            let Some(planned) = self.prepare(state, now, send) else {
                continue;
            };
            let outcome = sender.send(&planned.request).await;
            let result = self.settle(state, &planned, outcome).await;
            self.tracker().record(now, &planned.send, result);
        }
    }

    /// The request for `send`, built from the registration as it is now,
    /// or `None` when it must not go out: the device no longer holds the
    /// generation it was planned for (removed, signed out, its token moved,
    /// registered again), its environment is off or refused, or the device
    /// or the sender has backed off. The content is decided again from the
    /// host's `allow_content` and the row's current `content`.
    fn prepare(&self, state: &AppState, now: Duration, mut send: tracker::Send) -> Option<Planned> {
        let (device, generation) = send.target();
        let row = self
            .targets(state)
            .into_iter()
            .find(|row| row.device == device && row.generation == generation)?;
        let environment = Environment::parse(&row.environment)?;
        let tracker = self.tracker();
        if !tracker.still_due(now, &send) {
            return None;
        }
        let (payload, collapse_id, alert) = match &mut send {
            tracker::Send::Alert {
                host,
                count,
                newest,
                content,
                badge,
                ..
            } => {
                *content = (self.allow_content && row.content)
                    .then(|| tracker.content(host, newest))
                    .flatten();
                *badge = tracker.badge_for(operates(state, &row.device));
                let item = payload::item_ref(&row.ref_key, host, newest.kind, &newest.id);
                (
                    payload::alert(host, *count, *badge, &row.channel, &item, content.as_ref()),
                    Some(payload::collapse_id(host)),
                    true,
                )
            }
            tracker::Send::Badge { badge, .. } => {
                // The scope as it is now: a device narrowed to deciding
                // keeps its registration, and its tab counts queue rows.
                *badge = tracker.badge_for(operates(state, &row.device));
                (payload::badge(*badge), None, false)
            }
        };
        drop(tracker);
        let request = apns::Request {
            environment,
            token: row.token.clone(),
            payload,
            collapse_id,
            alert,
        };
        Some(Planned { send, row, request })
    }

    /// What Apple's answer means for the registration and the sender.
    async fn settle(
        &self,
        state: &AppState,
        planned: &Planned,
        outcome: apns::Outcome,
    ) -> tracker::Result {
        use apns::Outcome;
        let row = &planned.row;
        let environment = planned.request.environment;
        let (apple_ms, reason) = match outcome {
            Outcome::Accepted => return tracker::Result::Accepted,
            Outcome::Unregistered(ms) => (ms, "Unregistered"),
            Outcome::BadToken(reason) => (None, reason),
            Outcome::TooManyTokenUpdates => {
                tracing::warn!(
                    environment = environment.as_str(),
                    "APNs refused a provider token renewal; backing off"
                );
                return tracker::Result::BackOffAll;
            }
            Outcome::ProviderToken(reason) => {
                if self.disabled_list().insert(environment) {
                    tracing::warn!(
                        environment = environment.as_str(),
                        %reason,
                        "APNs refused the provider token; push to this environment is off until tendd restarts"
                    );
                }
                return tracker::Result::Dropped;
            }
            Outcome::Retry(reason) | Outcome::Refused(reason) => {
                tracing::debug!(device = %row.device, %reason, "APNs did not accept a notification");
                return tracker::Result::Retry;
            }
        };
        match state.devices.drop_push(row, apple_ms) {
            Ok(true) => {
                tracing::info!(device = %row.device, reason, "push registration dropped");
                crate::dashboard::devices::audit_entry(
                    state,
                    json!({
                        "by": "push", "action": "push_dropped", "id": row.device,
                        "environment": row.environment, "content": row.content,
                        "reason": reason, "status": 200,
                    }),
                )
                .await;
            }
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(%error, device = %row.device, "could not drop a push registration");
            }
        }
        tracker::Result::Dropped
    }
}

fn unix(value: &Value) -> i64 {
    value
        .as_str()
        .and_then(|text| {
            time::OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339).ok()
        })
        .map_or(0, time::OffsetDateTime::unix_timestamp)
}

fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// One queue entry as the sender keys it. Unanswerable questions are not
/// in the person's turn, and an unknown kind is skipped.
fn entry(item: &Value) -> Option<tracker::Entry> {
    use payload::Kind;
    let (kind, id, body, label, since) = match item["kind"].as_str()? {
        "human_request" => {
            if !item["unanswerable"].is_null() {
                return None;
            }
            (
                Kind::Request,
                text(&item["request_id"])?,
                text(&item["body"]).unwrap_or_default(),
                text(&item["from_alias"])
                    .or_else(|| text(&item["from_pane"]))
                    .unwrap_or_default(),
                unix(&item["created_at"]),
            )
        }
        "review" => (
            Kind::Review,
            text(&item["id"])?,
            text(&item["summary"]).unwrap_or_default(),
            text(&item["requested_by"]["label"])
                .or_else(|| text(&item["requested_by"]["kind"]))
                .unwrap_or_default(),
            unix(&item["requested_at"]),
        ),
        "agent_waiting" => (
            Kind::Waiting,
            text(&item["session_id"])?,
            text(&item["message"]).unwrap_or_default(),
            text(&item["agent_kind"]).unwrap_or_default(),
            unix(&item["since"]),
        ),
        _ => return None,
    };
    Some(tracker::Entry {
        kind,
        id,
        text: body,
        label,
        since,
    })
}

/// A host's section of `queue_sections` as the tracker reads it: readable
/// when a source answered (`covers` is not empty), complete when also not
/// truncated and nothing was unavailable.
/// Whether the device may operate now (its badge is Home's Your turn).
fn operates(state: &AppState, device: &str) -> bool {
    state
        .devices
        .list()
        .iter()
        .any(|d| d.id == device && d.scope == super::devices::Scope::Operate)
}

fn host_read(section: &Value) -> tracker::HostRead {
    let readable = section["covers"]
        .as_array()
        .is_some_and(|covers| !covers.is_empty());
    let unavailable = section["unavailable"]
        .as_array()
        .is_some_and(|list| !list.is_empty())
        || section["queue"]["unavailable"]
            .as_array()
            .is_some_and(|list| !list.is_empty());
    let complete = readable && !unavailable && section["queue"]["truncated"] != json!(true);
    tracker::HostRead {
        host: section["alias"].as_str().unwrap_or("this host").to_owned(),
        readable,
        complete,
        entries: if readable {
            section["queue"]["items"]
                .as_array()
                .map(|items| items.iter().filter_map(entry).collect())
                .unwrap_or_default()
        } else {
            Vec::new()
        },
    }
}

/// The running sender. Dropping it stops the sender, so its lifetime ends
/// with the dashboard's on every exit, an error and a panic included.
pub(in crate::dashboard) struct Worker(tokio::task::JoinHandle<()>);

impl Drop for Worker {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Senders running in this process (tests).
#[cfg(test)]
static RUNNING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Tick [`Push`] every [`TICK`] while it is on, until the returned
/// [`Worker`] is dropped. `None` when push is off.
pub(in crate::dashboard) fn spawn(state: AppState) -> Option<Worker> {
    if !state.push.enabled() {
        return None;
    }
    Some(Worker(tokio::spawn(async move {
        #[cfg(test)]
        let _running = {
            struct Running;
            impl Drop for Running {
                fn drop(&mut self) {
                    RUNNING.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                }
            }
            RUNNING.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Running
        };
        let mut interval = tokio::time::interval(TICK);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            let push = state.push.clone();
            push.tick(&state).await;
        }
    })))
}
