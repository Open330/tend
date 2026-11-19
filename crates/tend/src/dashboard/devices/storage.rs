//! Durable paired devices. Each row holds the SHA-256 of the device token,
//! never the token, plus what the access dialog lists.
use super::{ApprovedBy, Device, PushRow, Scope, MAX_DEVICES};
use rusqlite::{params, Connection};
use std::{fs::File, io, path::Path, sync::Mutex, time::Duration};

/// `MXDV`: refuses to adopt a database written by anything else.
const APPLICATION_ID: i64 = 0x4D58_4456;
/// 1: phase 1 (every device may decide). 2: adds `scope`; a version 1
/// store is migrated in place on open, every device keeping `decide`.
/// `device_push` (push registrations) is an extra table at version 2, with
/// no foreign key or trigger: an older tendd opens the store, ignores it,
/// and can still delete a device (its row is then an orphan, deleted here).
const SCHEMA_VERSION: i64 = 2;

pub(super) struct Storage {
    connection: Mutex<Connection>,
    lock: File,
}

fn validate_database(connection: &Connection) -> io::Result<()> {
    let application: i64 = connection
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .map_err(io::Error::other)?;
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(io::Error::other)?;
    let tables: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
            [],
            |row| row.get(0),
        )
        .map_err(io::Error::other)?;
    if (application == 0 && tables != 0)
        || (application != 0
            && (application != APPLICATION_ID || !(1..=SCHEMA_VERSION).contains(&version)))
    {
        return Err(io::Error::other("not a supported tend device database"));
    }
    Ok(())
}

impl Storage {
    /// Open (creating if needed) the store and load every device. An
    /// exclusive lease on a sibling lock file keeps a second daemon out.
    pub(super) fn open(path: &Path) -> io::Result<(Self, Vec<Device>)> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            crate::dashboard::sharing::private_dir(parent)?;
        }
        let private_file = crate::dashboard::sharing::private_file;
        let lock = private_file(&path.with_extension("lock"))?;
        lock.try_lock()
            .map_err(|error| io::Error::other(format!("cannot lock device storage: {error}")))?;
        private_file(path)?;
        let connection = Connection::open(path).map_err(io::Error::other)?;
        validate_database(&connection)?;
        connection
            .busy_timeout(Duration::from_secs(2))
            .map_err(io::Error::other)?;
        connection
            .execute_batch(
                "PRAGMA synchronous=FULL; PRAGMA application_id=0x4D584456; \
                 CREATE TABLE IF NOT EXISTS devices (id TEXT PRIMARY KEY, \
                 token_hash BLOB NOT NULL UNIQUE, name TEXT NOT NULL, platform TEXT NOT NULL, \
                 approved_by TEXT NOT NULL, created_at INTEGER NOT NULL, \
                 last_seen_at INTEGER NOT NULL, scope TEXT NOT NULL DEFAULT 'decide'); \
                 CREATE TABLE IF NOT EXISTS device_starts (device_id TEXT NOT NULL, \
                 key TEXT NOT NULL, fingerprint BLOB NOT NULL, work TEXT NOT NULL, \
                 state TEXT NOT NULL, detail TEXT, created_at INTEGER NOT NULL, \
                 updated_at INTEGER NOT NULL, PRIMARY KEY (device_id, key)); \
                 CREATE TABLE IF NOT EXISTS device_push (device_id TEXT PRIMARY KEY, \
                 token TEXT NOT NULL UNIQUE, environment TEXT NOT NULL, \
                 content INTEGER NOT NULL, channel TEXT NOT NULL, ref_key BLOB NOT NULL, \
                 generation INTEGER NOT NULL, updated_at INTEGER NOT NULL);",
            )
            .map_err(io::Error::other)?;
        migrate(&connection)?;
        // A start still reserved was cut off by a restart: it may have run.
        // It becomes unknown and is never dispatched again.
        connection
            .execute(
                "UPDATE device_starts SET state = 'unknown', detail = 'tendd restarted during the start' \
                 WHERE state = 'reserved'",
                [],
            )
            .map_err(io::Error::other)?;
        let devices = {
            let mut statement = connection
                .prepare(
                    "SELECT id, token_hash, name, platform, approved_by, created_at, last_seen_at, \
                     scope FROM devices ORDER BY created_at, id",
                )
                .map_err(io::Error::other)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, String>(7)?,
                    ))
                })
                .map_err(io::Error::other)?;
            let mut devices = Vec::new();
            for row in rows {
                let (id, hash, name, platform, approved_by, created_at, last_seen_at, scope) =
                    row.map_err(io::Error::other)?;
                let token_hash: [u8; 32] = hash
                    .try_into()
                    .map_err(|_| io::Error::other("device storage holds an invalid token hash"))?;
                let approved_by: ApprovedBy = serde_json::from_str(&approved_by)
                    .map_err(|_| io::Error::other("device storage holds an invalid record"))?;
                devices.push(Device {
                    id,
                    name,
                    platform,
                    approved_by,
                    created_at,
                    last_seen_at,
                    // Anything but a known wider scope reads as `decide`.
                    scope: Scope::from_stored(&scope),
                    token_hash,
                });
            }
            devices
        };
        if devices.len() > MAX_DEVICES {
            return Err(io::Error::other("device storage exceeds supported limits"));
        }
        Ok((
            Self {
                connection: Mutex::new(connection),
                lock,
            },
            devices,
        ))
    }

    fn connection(&self) -> io::Result<std::sync::MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| io::Error::other("device storage unavailable"))
    }

    pub(super) fn insert(&self, device: &Device) -> io::Result<()> {
        let approved_by = serde_json::to_string(&device.approved_by).map_err(io::Error::other)?;
        self.connection()?
            .execute(
                "INSERT INTO devices (id, token_hash, name, platform, approved_by, created_at, \
                 last_seen_at, scope) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    device.id,
                    device.token_hash.as_slice(),
                    device.name,
                    device.platform,
                    approved_by,
                    device.created_at,
                    device.last_seen_at,
                    device.scope.as_str()
                ],
            )
            .map(drop)
            .map_err(io::Error::other)
    }

    pub(super) fn touch(&self, id: &str, at: i64) -> io::Result<()> {
        self.connection()?
            .execute(
                "UPDATE devices SET last_seen_at = ?2 WHERE id = ?1",
                params![id, at],
            )
            .map(drop)
            .map_err(io::Error::other)
    }

    /// Narrow a device's scope. Persisted before the caller reports it.
    pub(super) fn set_scope(&self, id: &str, scope: Scope) -> io::Result<()> {
        let changed = self
            .connection()?
            .execute(
                "UPDATE devices SET scope = ?2 WHERE id = ?1",
                params![id, scope.as_str()],
            )
            .map_err(io::Error::other)?;
        if changed == 1 {
            Ok(())
        } else {
            Err(io::Error::other("the device is not in storage"))
        }
    }

    /// Every device, start and push registration, in one transaction.
    pub(super) fn clear(&self) -> io::Result<()> {
        let mut connection = self.connection()?;
        let tx = connection.transaction().map_err(io::Error::other)?;
        tx.execute_batch(
            "DELETE FROM device_starts; DELETE FROM device_push; DELETE FROM devices;",
        )
        .map_err(io::Error::other)?;
        tx.commit().map_err(io::Error::other)
    }

    /// A device with its starts and its push registration, in one
    /// transaction.
    pub(super) fn remove(&self, id: &str) -> io::Result<()> {
        let mut connection = self.connection()?;
        let tx = connection.transaction().map_err(io::Error::other)?;
        tx.execute("DELETE FROM device_starts WHERE device_id = ?1", [id])
            .map_err(io::Error::other)?;
        tx.execute("DELETE FROM device_push WHERE device_id = ?1", [id])
            .map_err(io::Error::other)?;
        tx.execute("DELETE FROM devices WHERE id = ?1", [id])
            .map_err(io::Error::other)?;
        tx.commit().map_err(io::Error::other)
    }

    /// Every push registration, for loading at open.
    pub(super) fn push_rows(&self) -> io::Result<Vec<PushRow>> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT device_id, token, environment, content, channel, ref_key, generation, \
                 updated_at FROM device_push",
            )
            .map_err(io::Error::other)?;
        let rows = statement
            .query_map([], |row| {
                Ok(PushRow {
                    device: row.get(0)?,
                    token: row.get(1)?,
                    environment: row.get(2)?,
                    content: row.get::<_, i64>(3)? != 0,
                    channel: row.get(4)?,
                    ref_key: row.get(5)?,
                    generation: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })
            .map_err(io::Error::other)?;
        rows.collect::<Result<_, _>>().map_err(io::Error::other)
    }

    /// Store `row` as its device's registration, in one transaction: that
    /// device's earlier row, and any row holding the same token (a phone
    /// that paired again), are replaced.
    pub(super) fn put_push(&self, row: &PushRow) -> io::Result<()> {
        let mut connection = self.connection()?;
        let tx = connection.transaction().map_err(io::Error::other)?;
        tx.execute(
            "DELETE FROM device_push WHERE device_id = ?1 OR token = ?2",
            params![row.device, row.token],
        )
        .map_err(io::Error::other)?;
        tx.execute(
            "INSERT INTO device_push (device_id, token, environment, content, channel, \
             ref_key, generation, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                row.device,
                row.token,
                row.environment,
                i64::from(row.content),
                row.channel,
                row.ref_key,
                row.generation,
                row.updated_at
            ],
        )
        .map_err(io::Error::other)?;
        tx.commit().map_err(io::Error::other)
    }

    /// Delete the push rows of these devices, in one transaction.
    pub(super) fn remove_push(&self, devices: &[String]) -> io::Result<()> {
        let mut connection = self.connection()?;
        let tx = connection.transaction().map_err(io::Error::other)?;
        for device in devices {
            tx.execute("DELETE FROM device_push WHERE device_id = ?1", [device])
                .map_err(io::Error::other)?;
        }
        tx.commit().map_err(io::Error::other)
    }

    /// Delete a registration only while it is exactly the one a send used.
    pub(super) fn remove_push_if(&self, sent: &PushRow) -> io::Result<()> {
        self.connection()?
            .execute(
                "DELETE FROM device_push WHERE device_id = ?1 AND token = ?2 \
                 AND environment = ?3 AND generation = ?4",
                params![sent.device, sent.token, sent.environment, sent.generation],
            )
            .map(drop)
            .map_err(io::Error::other)
    }

    /// Every start kept, for loading at open.
    pub(super) fn starts(&self) -> io::Result<Vec<super::Start>> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT device_id, key, fingerprint, work, state, detail, created_at, updated_at \
                 FROM device_starts",
            )
            .map_err(io::Error::other)?;
        let rows = statement
            .query_map([], |row| {
                Ok(super::Start {
                    device: row.get(0)?,
                    key: row.get(1)?,
                    fingerprint: row.get(2)?,
                    work: row.get(3)?,
                    state: row.get(4)?,
                    detail: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })
            .map_err(io::Error::other)?;
        rows.collect::<Result<_, _>>().map_err(io::Error::other)
    }

    /// Reserve a start; fails if (device, key) is already there.
    pub(super) fn reserve(&self, start: &super::Start) -> io::Result<()> {
        self.connection()?
            .execute(
                "INSERT INTO device_starts VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    start.device,
                    start.key,
                    start.fingerprint,
                    start.work,
                    start.state,
                    start.detail,
                    start.created_at,
                    start.updated_at
                ],
            )
            .map(drop)
            .map_err(io::Error::other)
    }

    pub(super) fn finish_start(
        &self,
        device: &str,
        key: &str,
        state: &str,
        detail: Option<&str>,
        at: i64,
    ) -> io::Result<()> {
        self.connection()?
            .execute(
                "UPDATE device_starts SET state = ?3, detail = ?4, updated_at = ?5 \
                 WHERE device_id = ?1 AND key = ?2",
                params![device, key, state, detail, at],
            )
            .map(drop)
            .map_err(io::Error::other)
    }

    /// Drop starts that settled before `settled`. A reserved or unknown
    /// start is never dropped (only with its device).
    pub(super) fn prune_starts(&self, settled: i64) -> io::Result<()> {
        self.connection()?
            .execute(
                "DELETE FROM device_starts WHERE state IN ('started', 'failed', 'refused') \
                 AND created_at < ?1",
                [settled],
            )
            .map(drop)
            .map_err(io::Error::other)
    }
}

/// Bring a store to [`SCHEMA_VERSION`]. A version 1 table has no `scope`
/// column: it is added with `decide` for every row.
fn migrate(connection: &Connection) -> io::Result<()> {
    let has_scope: bool = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('devices') WHERE name = 'scope'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(io::Error::other)?
        > 0;
    if !has_scope {
        connection
            .execute_batch("ALTER TABLE devices ADD COLUMN scope TEXT NOT NULL DEFAULT 'decide';")
            .map_err(io::Error::other)?;
    }
    connection
        .execute_batch(&format!("PRAGMA user_version={SCHEMA_VERSION};"))
        .map_err(io::Error::other)
}

impl Drop for Storage {
    fn drop(&mut self) {
        let _ = self.lock.unlock();
    }
}
