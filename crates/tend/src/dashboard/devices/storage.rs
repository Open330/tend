//! Durable paired devices. Each row holds the SHA-256 of the device token,
//! never the token, plus what the access dialog lists.
use super::{ApprovedBy, Device, MAX_DEVICES};
use rusqlite::{params, Connection};
use std::{fs::File, io, path::Path, sync::Mutex, time::Duration};

/// `MXDV`: refuses to adopt a database written by anything else.
const APPLICATION_ID: i64 = 0x4D58_4456;

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
        || (application != 0 && (application != APPLICATION_ID || version != 1))
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
                "PRAGMA synchronous=FULL; PRAGMA application_id=0x4D584456; PRAGMA user_version=1; \
                 CREATE TABLE IF NOT EXISTS devices (id TEXT PRIMARY KEY, \
                 token_hash BLOB NOT NULL UNIQUE, name TEXT NOT NULL, platform TEXT NOT NULL, \
                 approved_by TEXT NOT NULL, created_at INTEGER NOT NULL, \
                 last_seen_at INTEGER NOT NULL);",
            )
            .map_err(io::Error::other)?;
        let devices = {
            let mut statement = connection
                .prepare(
                    "SELECT id, token_hash, name, platform, approved_by, created_at, last_seen_at \
                     FROM devices ORDER BY created_at, id",
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
                    ))
                })
                .map_err(io::Error::other)?;
            let mut devices = Vec::new();
            for row in rows {
                let (id, hash, name, platform, approved_by, created_at, last_seen_at) =
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
                "INSERT INTO devices VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    device.id,
                    device.token_hash.as_slice(),
                    device.name,
                    device.platform,
                    approved_by,
                    device.created_at,
                    device.last_seen_at
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

    pub(super) fn clear(&self) -> io::Result<()> {
        self.connection()?
            .execute("DELETE FROM devices", [])
            .map(drop)
            .map_err(io::Error::other)
    }

    pub(super) fn remove(&self, id: &str) -> io::Result<()> {
        self.connection()?
            .execute("DELETE FROM devices WHERE id = ?1", [id])
            .map(drop)
            .map_err(io::Error::other)
    }
}

impl Drop for Storage {
    fn drop(&mut self) {
        let _ = self.lock.unlock();
    }
}
