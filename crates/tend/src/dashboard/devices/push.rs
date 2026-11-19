//! Push registrations: one per device, at most one device per APNs token.
//!
//! Register, move, remove and device removal take `transitions` and change
//! the store in one transaction, so they happen in one order. Every change
//! takes the next `generation`, from a counter that only grows (it starts
//! past every stored row and is never reused in this process), so a late
//! answer from Apple to an older send can tell that the row has changed
//! since. The token is never logged: [`PushRow`]'s `Debug` leaves it out.
use super::{storage::Storage, Device, Devices, MAX_DEVICES};
use sha2::{Digest, Sha256};
use std::io;

/// A device's push registration.
#[derive(Clone, PartialEq, Eq)]
pub(in crate::dashboard) struct PushRow {
    pub device: String,
    /// The APNs device token, lower-case hex.
    pub token: String,
    /// `production` or `development`.
    pub environment: String,
    /// The device asked for the item's text (the host's `allow_content`
    /// still has to allow it, at send time).
    pub content: bool,
    /// `ch_` and 128 random bits as hex, minted with the registration.
    pub channel: String,
    /// 256 random bits: the key of the payload's opaque `ref`.
    pub ref_key: Vec<u8>,
    pub generation: i64,
    /// When this registration last changed, in unix milliseconds: compared
    /// exactly with the timestamp of Apple's `410`.
    pub updated_at: i64,
}

impl std::fmt::Debug for PushRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the token or the ref key.
        f.debug_struct("PushRow")
            .field("device", &self.device)
            .field("environment", &self.environment)
            .field("content", &self.content)
            .field("generation", &self.generation)
            .field("updated_at", &self.updated_at)
            .finish_non_exhaustive()
    }
}

impl PushRow {
    /// The logical registration, as a fingerprint of the token, the
    /// environment and the channel (never the token itself). It stays the
    /// same while this device registers the same token and environment
    /// again (the channel is kept then), and changes at a token move, a
    /// removal followed by a new registration, or another environment.
    pub(in crate::dashboard) fn registration(&self) -> String {
        let mut hasher = Sha256::new();
        for part in [&self.token, &self.environment, &self.channel] {
            hasher.update(part.as_bytes());
            hasher.update([0]);
        }
        hex(&hasher.finalize()[..16])
    }
}

/// What registering gave.
#[derive(Debug)]
pub(in crate::dashboard) enum PushRegistered {
    /// Stored: the row as it is now.
    Stored(PushRow),
    /// The device is gone.
    NoDevice,
    /// [`MAX_DEVICES`] live registrations are held by other devices.
    Full,
}

#[derive(Default)]
pub(super) struct PushMirror {
    pub(super) rows: Vec<PushRow>,
    /// The last generation handed out.
    generation: i64,
}

impl PushMirror {
    /// The stored rows at open. Rows whose device is gone (removed by an
    /// older tendd) are deleted now; the counter starts past all of them.
    pub(super) fn load(storage: &Storage, devices: &[Device]) -> io::Result<Self> {
        let rows = storage.push_rows()?;
        let generation = rows.iter().map(|row| row.generation).max().unwrap_or(0);
        let (rows, orphans): (Vec<_>, Vec<_>) = rows
            .into_iter()
            .partition(|row| devices.iter().any(|device| device.id == row.device));
        if !orphans.is_empty() {
            let ids: Vec<String> = orphans.into_iter().map(|row| row.device).collect();
            storage.remove_push(&ids)?;
        }
        Ok(Self { rows, generation })
    }
}

fn now_unix_ms() -> i64 {
    i64::try_from(time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000)
        .unwrap_or(i64::MAX)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// `n` bytes (at most 32) from the OS generator: SHA-256 over enough v4
/// UUIDs (122 random bits each) to carry more than `8 n` bits.
fn random_bytes(n: usize) -> Vec<u8> {
    let mut hasher = Sha256::new();
    for _ in 0..=(n * 8).div_ceil(122) {
        hasher.update(uuid::Uuid::new_v4().as_bytes());
    }
    hasher.finalize()[..n.min(32)].to_vec()
}

impl Devices {
    pub(super) fn push_list(&self) -> std::sync::MutexGuard<'_, PushMirror> {
        self.pushes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Delete the rows of devices no longer paired (expired, or removed by
    /// an older tendd), from the store and the mirror. Under `transitions`.
    fn drop_orphans(&self, live: &[String]) {
        let orphans: Vec<String> = self
            .push_list()
            .rows
            .iter()
            .filter(|row| !live.contains(&row.device))
            .map(|row| row.device.clone())
            .collect();
        if orphans.is_empty() {
            return;
        }
        if let Ok(Some(storage)) = self.storage() {
            if let Err(error) = storage.remove_push(&orphans) {
                tracing::warn!(%error, "could not delete push registrations of removed devices");
                return;
            }
        }
        self.push_list()
            .rows
            .retain(|row| !orphans.contains(&row.device));
    }

    fn live_ids(&self) -> Vec<String> {
        self.pruned()
            .devices
            .iter()
            .map(|device| device.id.clone())
            .collect()
    }

    /// Register `token` for `device`. The channel and ref key are kept only
    /// when this device registers the same token and environment again;
    /// anything else mints new ones. A token another device holds moves
    /// here. Every registration takes a new generation.
    pub(in crate::dashboard) fn register_push(
        &self,
        device: &str,
        token: &str,
        environment: &str,
        content: bool,
    ) -> io::Result<PushRegistered> {
        let _transition = self.transition();
        let live = self.live_ids();
        if !live.iter().any(|id| id == device) {
            return Ok(PushRegistered::NoDevice);
        }
        // Orphans go before the cap is checked.
        self.drop_orphans(&live);
        let (kept, others, generation) = {
            let mirror = self.push_list();
            let kept = mirror
                .rows
                .iter()
                .find(|row| {
                    row.device == device && row.token == token && row.environment == environment
                })
                .map(|row| (row.channel.clone(), row.ref_key.clone()));
            let others = mirror
                .rows
                .iter()
                .filter(|row| row.device != device && row.token != token)
                .count();
            (kept, others, mirror.generation + 1)
        };
        if others >= MAX_DEVICES {
            return Ok(PushRegistered::Full);
        }
        let (channel, ref_key) =
            kept.unwrap_or_else(|| (format!("ch_{}", hex(&random_bytes(16))), random_bytes(32)));
        let row = PushRow {
            device: device.to_owned(),
            token: token.to_owned(),
            environment: environment.to_owned(),
            content,
            channel,
            ref_key,
            generation,
            updated_at: now_unix_ms(),
        };
        if let Some(storage) = self.storage()? {
            storage.put_push(&row)?;
        }
        let mut mirror = self.push_list();
        mirror.generation = generation;
        mirror
            .rows
            .retain(|other| other.device != device && other.token != token);
        mirror.rows.push(row.clone());
        Ok(PushRegistered::Stored(row))
    }

    /// Drop `device`'s registration; `false` when it had none.
    pub(in crate::dashboard) fn remove_push(&self, device: &str) -> io::Result<bool> {
        let _transition = self.transition();
        if !self.push_list().rows.iter().any(|row| row.device == device) {
            return Ok(false);
        }
        if let Some(storage) = self.storage()? {
            storage.remove_push(&[device.to_owned()])?;
        }
        self.push_list().rows.retain(|row| row.device != device);
        Ok(true)
    }

    /// Apple said the token a send used is no longer valid: delete the
    /// registration, but only while it still has the device, token,
    /// environment and generation the send used, and, for a `410` with
    /// Apple's timestamp (unix ms), only when the row was updated before
    /// that millisecond. `true` when it was deleted.
    pub(in crate::dashboard) fn drop_push(
        &self,
        sent: &PushRow,
        apple_ms: Option<i64>,
    ) -> io::Result<bool> {
        let _transition = self.transition();
        let current = self
            .push_list()
            .rows
            .iter()
            .find(|row| {
                row.device == sent.device
                    && row.token == sent.token
                    && row.environment == sent.environment
                    && row.generation == sent.generation
            })
            .cloned();
        let Some(current) = current else {
            return Ok(false);
        };
        if let Some(ms) = apple_ms {
            // Updated at or after Apple's timestamp: a newer registration.
            if current.updated_at >= ms {
                return Ok(false);
            }
        }
        if let Some(storage) = self.storage()? {
            storage.remove_push_if(&current)?;
        }
        self.push_list()
            .rows
            .retain(|row| !(row.device == current.device && row.generation == current.generation));
        Ok(true)
    }

    /// The registrations to send to: only rows of live, unexpired devices.
    /// Rows of devices that are gone are deleted here.
    pub(in crate::dashboard) fn push_targets(&self) -> Vec<PushRow> {
        let _transition = self.transition();
        let live = self.live_ids();
        self.drop_orphans(&live);
        self.push_list()
            .rows
            .iter()
            .filter(|row| live.contains(&row.device))
            .cloned()
            .collect()
    }

    /// Whether any registration exists (orphans included: the next
    /// [`Self::push_targets`] deletes them).
    pub(in crate::dashboard) fn has_push(&self) -> bool {
        !self.push_list().rows.is_empty()
    }
}
