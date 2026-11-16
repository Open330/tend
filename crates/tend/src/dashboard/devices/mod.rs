//! Paired devices: a phone that may decide, and, when the operator who
//! approves it says so, also see the hosts and start new work.
//!
//! Pairing is shaped like the OAuth device authorization grant (RFC 8628),
//! with tend's own endpoints: the **device asks**, an **operator
//! approves**, and no secret passes through a link, the clipboard or
//! another app.
//!
//! 1. The app calls `POST /auth/devices/request` with its name. It gets a
//!    secret `device_code` (kept on the phone) and a short `user_code`
//!    (shown on the phone).
//! 2. An operator, signed in on the dashboard, types the user code into
//!    **access ▸ Pair a device**, sees the device's name, and approves
//!    (`POST /api/devices/approve`).
//! 3. The app, polling `POST /auth/devices/token` with its device code, gets
//!    its **device token** once, and the request is gone.
//!
//! Every device has one [`Scope`]. A `decide` token authorizes exactly the
//! decision routes (`/api/decisions*`) and the device's own `GET
//! /api/device` and `POST /api/device/forget`: read the questions and
//! evidence of every host this dashboard covers, and answer them, which can
//! tell an agent to go ahead. An `operate` token adds the device-only
//! routes in `operate_routes` (the Fleet snapshot cut down, an agent's
//! recent output, starting new work) and nothing else: no pane input,
//! generic Fleet commands, Work control, shares or access management. The
//! device asks for a scope; the approving operator grants it or less; a
//! token is never widened, and `limit` narrows it in place.
//!
//! # Secrets and limits
//!
//! - The device code and the device token are 244 random bits; only their
//!   SHA-256 is kept (the token's also on disk). The user code is 8
//!   characters (about 38 bits), accepted only from a signed-in operator.
//! - A request lives [`REQUEST_TTL`]; at most [`MAX_REQUESTS`] are pending.
//! - A device stops working after [`IDLE_EXPIRY`] unused or [`MAX_LIFETIME`]
//!   in all, whichever comes first, and when removed. Devices are their own
//!   principals: removing the operator who approved one, or rotating the
//!   dashboard token, does not unpair it. **Remove all** does.
//!
//! # Storage
//!
//! `$XDG_DATA_HOME/tend/dashboard-devices/devices.sqlite3`, owner-only, with
//! an exclusive lock and its own application id. Nothing is created until
//! the first device is paired; an existing store is opened at start.

mod operate;
mod routes;
mod storage;
#[cfg(test)]
mod tests;

pub(in crate::dashboard) use routes::{
    admin_routes, device_auth, device_routes, operate_routes, pairing_routes,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const DEFAULT_STORAGE_PATH: &str = "tend/dashboard-devices/devices.sqlite3";
const TOKEN_PREFIX: &str = "tdv_";
const REQUEST_TTL: Duration = Duration::from_secs(10 * 60);
/// How often the app may poll for its token.
const POLL_INTERVAL_SECS: u64 = 2;
const MAX_REQUESTS: usize = 32;
const MAX_DEVICES: usize = 16;
const IDLE_EXPIRY: i64 = 30 * 24 * 3600;
const MAX_LIFETIME: i64 = 90 * 24 * 3600;
/// `last_seen_at` is written back at most this often per device.
const TOUCH_INTERVAL: i64 = 3600;
const MAX_NAME_CHARS: usize = 64;
/// No vowels (no words), no 0/O/1/I.
const USER_CODE_ALPHABET: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ23456789";
const USER_CODE_LEN: usize = 8;

/// What a paired device may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::dashboard) enum Scope {
    /// Read and answer what needs the person.
    Decide,
    /// Also the hosts, agents, recent output, and starting new work.
    Operate,
}

impl Scope {
    pub(in crate::dashboard) fn as_str(self) -> &'static str {
        match self {
            Self::Decide => "decide",
            Self::Operate => "operate",
        }
    }

    /// A stored or requested value: anything but `operate` is `decide`.
    pub(in crate::dashboard) fn from_stored(value: &str) -> Self {
        if value == "operate" {
            Self::Operate
        } else {
            Self::Decide
        }
    }

    /// The route groups this scope reaches, as `GET /api/device` lists them
    /// (`decisions` first, for clients that only knew phase 1).
    pub(in crate::dashboard) fn names(self) -> &'static [&'static str] {
        match self {
            Self::Decide => &["decisions"],
            Self::Operate => &["decisions", "operate"],
        }
    }
}

/// Who approved a device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::dashboard) struct ApprovedBy {
    /// `token` (the dashboard token) or `session` (an operator sign-in).
    pub credential: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// A paired device as listed to operators. The token hash never leaves.
#[derive(Debug, Clone, Serialize)]
pub(in crate::dashboard) struct Device {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub approved_by: ApprovedBy,
    pub created_at: i64,
    pub last_seen_at: i64,
    pub scope: Scope,
    #[serde(skip)]
    token_hash: [u8; 32],
}

impl Device {
    /// The actor label decisions record: the name for people, the id so
    /// two devices with one name stay apart.
    pub(in crate::dashboard) fn label(name: &str, id: &str) -> String {
        let short = id.strip_prefix("dev_").unwrap_or(id);
        format!("app:{name}#{}", &short[..short.len().min(8)])
    }
}

/// A request authenticated with a device token, for the decision handlers'
/// actor label and audit line.
#[derive(Debug, Clone)]
pub(in crate::dashboard) struct DeviceAuth {
    pub id: String,
    pub name: String,
    pub approved_by: ApprovedBy,
    /// The scope as the registry held it when this request was admitted.
    pub scope: Scope,
}

/// A pending request as an operator sees it before approving: nothing is
/// granted by looking.
#[derive(Debug, Clone, Serialize)]
pub(in crate::dashboard) struct RequestView {
    pub name: String,
    pub platform: String,
    pub scope: Scope,
    /// Seconds since the device asked.
    pub age_secs: u64,
}

struct PendingRequest {
    device_code_hash: [u8; 32],
    user_code: String,
    name: String,
    platform: String,
    /// What the device asked for; fixed for the life of the request.
    requested: Scope,
    /// What the operator granted: at most `requested`.
    granted: Scope,
    expires: Instant,
    approved_by: Option<ApprovedBy>,
}

#[derive(Default)]
struct Registry {
    requests: Vec<PendingRequest>,
    devices: Vec<Device>,
}

/// What `POST /auth/devices/token` answers.
#[derive(Debug)]
pub(in crate::dashboard) enum Poll {
    /// Not approved yet: ask again after [`POLL_INTERVAL_SECS`].
    Pending,
    /// Approved: the device and its token, issued exactly once.
    Issued(Box<Device>, String),
    /// Unknown, expired or already issued.
    Invalid,
    /// [`MAX_DEVICES`] are paired.
    Full,
    Storage(String),
}

pub(in crate::dashboard) struct Devices {
    /// Serializes the transitions that change which devices exist (issuing
    /// a token, removing one, removing all) from the capacity check to the
    /// durable write, so memory and the store agree when each one ends and
    /// a removal can never miss an issuance in flight. Lock order: this,
    /// then `registry`, then `storage`.
    transitions: Mutex<()>,
    registry: Mutex<Registry>,
    /// `None` until the first device is paired (or the store existed at
    /// start); always `None` for an in-memory registry.
    storage: Mutex<Option<Arc<storage::Storage>>>,
    path: Option<PathBuf>,
    /// Devices with a capture in flight: one each.
    captures: Mutex<std::collections::HashSet<String>>,
}

fn sha256(value: &str) -> [u8; 32] {
    Sha256::digest(value.as_bytes()).into()
}

fn now_unix() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

fn random_secret() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn random_user_code() -> String {
    // 252: the largest multiple of the alphabet size below 256.
    let limit =
        u8::try_from(256 / USER_CODE_ALPHABET.len() * USER_CODE_ALPHABET.len()).unwrap_or(u8::MAX);
    let mut code = String::with_capacity(USER_CODE_LEN);
    while code.len() < USER_CODE_LEN {
        for byte in uuid::Uuid::new_v4().into_bytes() {
            // Rejection sampling keeps every character equally likely.
            if byte < limit && code.len() < USER_CODE_LEN {
                code.push(USER_CODE_ALPHABET[usize::from(byte) % USER_CODE_ALPHABET.len()] as char);
            }
        }
    }
    code
}

/// What a person may type for a user code: any case, with or without the
/// dash or spaces.
pub(in crate::dashboard) fn normalize_user_code(input: &str) -> String {
    input
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// Constant-time equality of two digests.
fn same(a: &[u8; 32], b: &[u8; 32]) -> bool {
    use subtle::ConstantTimeEq;
    bool::from(a.ct_eq(b))
}

/// A display name: control characters removed, trimmed, bounded.
fn clean_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !c.is_control() && *c != '#')
        .take(MAX_NAME_CHARS)
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "device".into()
    } else {
        cleaned.to_owned()
    }
}

fn clean_platform(platform: &str) -> String {
    match platform {
        "ios" | "ipados" | "android" | "windows" | "macos" | "linux" => platform.to_owned(),
        _ => "other".into(),
    }
}

fn expired(device: &Device, now: i64) -> bool {
    now.saturating_sub(device.last_seen_at) >= IDLE_EXPIRY
        || now.saturating_sub(device.created_at) >= MAX_LIFETIME
}

impl Devices {
    /// Paired devices kept in memory only (embedding callers and tests).
    #[must_use]
    pub(in crate::dashboard) fn memory() -> Arc<Self> {
        Arc::new(Self {
            transitions: Mutex::new(()),
            registry: Mutex::new(Registry::default()),
            storage: Mutex::new(None),
            path: None,
            captures: Mutex::default(),
        })
    }

    /// Devices persisted at `path` (default: the XDG data directory). An
    /// existing store is opened now; a missing one is created at the first
    /// pairing, so a daemon that never pairs writes nothing.
    pub(in crate::dashboard) async fn open(path: Option<PathBuf>) -> std::io::Result<Arc<Self>> {
        let path = path.or_else(|| crate::paths::data_dir().map(|d| d.join(DEFAULT_STORAGE_PATH)));
        let devices = Arc::new(Self {
            transitions: Mutex::new(()),
            registry: Mutex::new(Registry::default()),
            storage: Mutex::new(None),
            path: path.clone(),
            captures: Mutex::default(),
        });
        if let Some(path) = path.filter(|path| path.exists()) {
            let (storage, stored) =
                tokio::task::spawn_blocking(move || storage::Storage::open(&path))
                    .await
                    .map_err(std::io::Error::other)??;
            *devices.storage_slot() = Some(Arc::new(storage));
            devices.registry().devices = stored;
        }
        Ok(devices)
    }

    fn transition(&self) -> std::sync::MutexGuard<'_, ()> {
        self.transitions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn registry(&self) -> std::sync::MutexGuard<'_, Registry> {
        self.registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn storage_slot(&self) -> std::sync::MutexGuard<'_, Option<Arc<storage::Storage>>> {
        self.storage
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The store, created on first use. `None` for an in-memory registry.
    fn storage(&self) -> std::io::Result<Option<Arc<storage::Storage>>> {
        let mut slot = self.storage_slot();
        if slot.is_none() {
            if let Some(path) = &self.path {
                let (storage, _) = storage::Storage::open(path)?;
                *slot = Some(Arc::new(storage));
            }
        }
        Ok(slot.clone())
    }

    /// Drop expired requests and devices; the ids of devices dropped.
    fn prune(registry: &mut Registry) -> Vec<String> {
        let now = Instant::now();
        registry.requests.retain(|request| request.expires > now);
        let at = now_unix();
        let mut gone = Vec::new();
        registry.devices.retain(|device| {
            let keep = !expired(device, at);
            if !keep {
                gone.push(device.id.clone());
            }
            keep
        });
        gone
    }

    fn forget_stored(&self, ids: &[String]) {
        if ids.is_empty() {
            return;
        }
        if let Ok(Some(storage)) = self.storage() {
            for id in ids {
                if let Err(error) = storage.remove(id) {
                    tracing::warn!(%error, device = %id, "could not remove an expired device");
                }
            }
        }
    }

    fn pruned(&self) -> std::sync::MutexGuard<'_, Registry> {
        let gone = Self::prune(&mut self.registry());
        self.forget_stored(&gone);
        self.registry()
    }

    /// A device asks to be paired: `(device_code, user_code, expires_in,
    /// interval)`, or `None` when [`MAX_REQUESTS`] are pending.
    pub(in crate::dashboard) fn request(
        &self,
        name: &str,
        platform: &str,
        scope: Scope,
    ) -> Option<(String, String, u64, u64)> {
        let device_code = random_secret();
        let mut registry = self.pruned();
        if registry.requests.len() >= MAX_REQUESTS {
            return None;
        }
        let user_code = loop {
            let code = random_user_code();
            if !registry.requests.iter().any(|r| r.user_code == code) {
                break code;
            }
        };
        registry.requests.push(PendingRequest {
            device_code_hash: sha256(&device_code),
            user_code: user_code.clone(),
            name: clean_name(name),
            platform: clean_platform(platform),
            requested: scope,
            granted: Scope::Decide,
            expires: Instant::now() + REQUEST_TTL,
            approved_by: None,
        });
        Some((
            device_code,
            user_code,
            REQUEST_TTL.as_secs(),
            POLL_INTERVAL_SECS,
        ))
    }

    /// The pending, not yet approved request with this user code, as an
    /// operator sees it before deciding. Grants nothing.
    pub(in crate::dashboard) fn lookup(&self, user_code: &str) -> Option<RequestView> {
        let code = normalize_user_code(user_code);
        let registry = self.pruned();
        let request = registry
            .requests
            .iter()
            .find(|request| request.user_code == code && request.approved_by.is_none())?;
        let age =
            REQUEST_TTL.saturating_sub(request.expires.saturating_duration_since(Instant::now()));
        Some(RequestView {
            name: request.name.clone(),
            platform: request.platform.clone(),
            scope: request.requested,
            age_secs: age.as_secs(),
        })
    }

    /// An operator approves the pending request with this user code,
    /// granting `grant` or, if the device asked for less, what it asked:
    /// the device approved and the scope it gets, `None` if no request has
    /// that code (or it was approved already).
    pub(in crate::dashboard) fn approve(
        &self,
        user_code: &str,
        by: ApprovedBy,
        grant: Scope,
    ) -> Option<RequestView> {
        let code = normalize_user_code(user_code);
        let mut registry = self.pruned();
        let request = registry
            .requests
            .iter_mut()
            .find(|request| request.user_code == code && request.approved_by.is_none())?;
        request.granted = grant.min(request.requested);
        request.approved_by = Some(by);
        Some(RequestView {
            name: request.name.clone(),
            platform: request.platform.clone(),
            scope: request.granted,
            age_secs: 0,
        })
    }

    /// The device polls with its device code.
    pub(in crate::dashboard) fn poll(&self, device_code: &str) -> Poll {
        let hash = sha256(device_code.trim());
        let _transition = self.transition();
        let request = {
            let mut registry = self.pruned();
            let Some(index) = registry
                .requests
                .iter()
                .position(|request| same(&request.device_code_hash, &hash))
            else {
                return Poll::Invalid;
            };
            if registry.requests[index].approved_by.is_none() {
                return Poll::Pending;
            }
            if registry.devices.len() >= MAX_DEVICES {
                return Poll::Full;
            }
            // Taken under the lock: a token is issued once, even to two
            // polls racing with the same code.
            registry.requests.swap_remove(index)
        };
        let (request_code, request_expires, requested) = (
            request.user_code.clone(),
            request.expires,
            request.requested,
        );
        let token = format!("{TOKEN_PREFIX}{}", random_secret());
        let at = now_unix();
        let device = Device {
            id: format!("dev_{}", uuid::Uuid::new_v4().simple()),
            name: request.name,
            platform: request.platform,
            approved_by: request.approved_by.expect("checked approval"),
            created_at: at,
            last_seen_at: at,
            scope: request.granted,
            token_hash: sha256(&token),
        };
        let stored = match self.storage() {
            Ok(Some(storage)) => storage.insert(&device),
            Ok(None) => Ok(()),
            Err(error) => Err(error),
        };
        if let Err(error) = stored {
            // Nothing was issued: the approved request stays, so the next
            // poll can try again.
            self.registry().requests.push(PendingRequest {
                device_code_hash: hash,
                user_code: request_code,
                name: device.name,
                platform: device.platform,
                requested,
                granted: device.scope,
                expires: request_expires,
                approved_by: Some(device.approved_by),
            });
            return Poll::Storage(error.to_string());
        }
        self.registry().devices.push(device.clone());
        Poll::Issued(Box::new(device), token)
    }

    /// The device a `Bearer tdv_…` header names, if it is still paired.
    pub(in crate::dashboard) fn authenticate(&self, bearer: Option<&str>) -> Option<DeviceAuth> {
        let token = bearer?.strip_prefix("Bearer ")?;
        if !token.starts_with(TOKEN_PREFIX) {
            return None;
        }
        let hash = sha256(token);
        let at = now_unix();
        let (auth, touch) = {
            let mut registry = self.pruned();
            let device = registry
                .devices
                .iter_mut()
                .find(|device| same(&device.token_hash, &hash))?;
            let touch = at - device.last_seen_at >= TOUCH_INTERVAL;
            if touch {
                device.last_seen_at = at;
            }
            (
                DeviceAuth {
                    id: device.id.clone(),
                    name: device.name.clone(),
                    approved_by: device.approved_by.clone(),
                    scope: device.scope,
                },
                touch,
            )
        };
        if touch {
            if let Ok(Some(storage)) = self.storage() {
                if let Err(error) = storage.touch(&auth.id, at) {
                    tracing::warn!(%error, device = %auth.id, "could not record device use");
                }
            }
        }
        Some(auth)
    }

    pub(in crate::dashboard) fn list(&self) -> Vec<Device> {
        self.pruned().devices.clone()
    }

    /// Unpair a device; `false` when no such device exists.
    pub(in crate::dashboard) fn remove(&self, id: &str) -> std::io::Result<bool> {
        let _transition = self.transition();
        if !self.registry().devices.iter().any(|device| device.id == id) {
            return Ok(false);
        }
        if let Some(storage) = self.storage()? {
            storage.remove(id)?;
        }
        self.registry().devices.retain(|device| device.id != id);
        Ok(true)
    }

    /// Limit a device to deciding, keeping it paired. Irreversible for its
    /// token; persisted before it is reported, serialized with issuance and
    /// removal. `false` when no such device exists; `Ok(true)` also when it
    /// already could only decide.
    pub(in crate::dashboard) fn limit(&self, id: &str) -> std::io::Result<bool> {
        let _transition = self.transition();
        let Some(scope) = self
            .registry()
            .devices
            .iter()
            .find(|device| device.id == id)
            .map(|device| device.scope)
        else {
            return Ok(false);
        };
        if scope == Scope::Decide {
            return Ok(true);
        }
        if let Some(storage) = self.storage()? {
            storage.set_scope(id, Scope::Decide)?;
        }
        if let Some(device) = self
            .registry()
            .devices
            .iter_mut()
            .find(|device| device.id == id)
        {
            device.scope = Scope::Decide;
        }
        Ok(true)
    }

    /// Unpair every device and drop every pending request; how many
    /// devices were removed.
    pub(in crate::dashboard) fn remove_all(&self) -> std::io::Result<usize> {
        let _transition = self.transition();
        // Approvals still waiting for their poll are cancelled too.
        let count = {
            let mut registry = self.registry();
            registry.requests.clear();
            registry.devices.len()
        };
        if count == 0 {
            return Ok(0);
        }
        if let Some(storage) = self.storage()? {
            storage.clear()?;
        }
        self.registry().devices.clear();
        Ok(count)
    }

    /// Take this device's capture slot; `false` while one is in flight.
    pub(in crate::dashboard) fn begin_capture(&self, id: &str) -> bool {
        self.captures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id.to_owned())
    }

    pub(in crate::dashboard) fn end_capture(&self, id: &str) {
        self.captures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id);
    }

    #[cfg(test)]
    fn age_device(&self, id: &str, created_by: i64, seen_by: i64) {
        let mut registry = self.registry();
        if let Some(device) = registry.devices.iter_mut().find(|d| d.id == id) {
            device.created_at -= created_by;
            device.last_seen_at -= seen_by;
        }
    }
}
