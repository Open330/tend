//! Durable result handoff between the provider runner and SQLite. A sealed
//! record belongs to one request and one execution generation. The caller
//! removes it only after the corresponding terminal row commits.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const DIR_NAME: &str = "headless-review-results";
const VERSION: u64 = 1;
pub(super) const MAX_ANSWER_BYTES: usize = 64 * 1024;
// JSON may expand a control byte to six ASCII bytes; leave room for metadata.
const MAX_RECORD_BYTES: usize = MAX_ANSWER_BYTES * 6 + 1024;

pub(super) struct Journal {
    directory: PathBuf,
}

impl Journal {
    pub(super) fn open(root: &Path) -> Result<Self> {
        let directory = root.join(DIR_NAME);
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt as _;
            builder.mode(0o700);
        }
        match builder.create(&directory) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err).context("create headless result journal"),
        }
        let metadata =
            fs::symlink_metadata(&directory).context("inspect headless result journal")?;
        if !metadata.is_dir() {
            bail!("headless result journal is not a real directory");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            if metadata.permissions().mode() & 0o077 != 0 {
                bail!("headless result journal must be accessible only to its owner");
            }
        }
        // Also sync on retry: a preceding open may have made the directory
        // visible, but failed to persist its entry in the parent.
        sync_directory(root)?;
        Ok(Self { directory })
    }

    /// Seal one result. An identical retry succeeds, a different result cannot
    /// overwrite a seal, and a stale generation has its own journal file.
    pub(super) fn write(
        &self,
        request_id: &str,
        generation: Uuid,
        answer: Option<&str>,
    ) -> Result<()> {
        let path = self.path(request_id, generation)?;
        let answer_bytes = answer.map_or(0, str::len);
        if answer_bytes > MAX_ANSWER_BYTES {
            bail!("headless review result exceeds journal size limit");
        }
        let record = json!({
            "version": VERSION,
            "request_id": request_id,
            "dispatch_generation": generation.to_string(),
            "answer_bytes": answer_bytes,
            "answer": answer,
            "complete": true,
        });
        let mut file = tempfile::NamedTempFile::new_in(&self.directory)
            .context("create headless result temporary file")?;
        serde_json::to_writer(&mut file, &record).context("encode headless result")?;
        file.flush().context("flush headless result")?;
        file.as_file().sync_all().context("sync headless result")?;
        match file.persist_noclobber(path) {
            Ok(_) => {}
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                if self
                    .read(request_id, generation)?
                    .as_ref()
                    .map(Option::as_deref)
                    != Some(answer)
                {
                    bail!("a different headless result is already sealed");
                }
            }
            Err(error) => return Err(error).context("seal headless result"),
        }
        sync_directory(&self.directory)
    }

    /// Missing journal file and sealed result with no answer are distinct.
    /// Partial, malformed or mismatched records are errors and remain intact.
    pub(super) fn read(
        &self,
        request_id: &str,
        generation: Uuid,
    ) -> Result<Option<Option<String>>> {
        let path = self.path(request_id, generation)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) if !metadata.is_file() => bail!("headless result is not a regular file"),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("inspect headless result"),
        }
        let file = File::open(path).context("open headless result")?;
        let mut bytes = Vec::new();
        file.take((MAX_RECORD_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .context("read headless result")?;
        if bytes.len() > MAX_RECORD_BYTES {
            bail!("headless result exceeds journal record size limit");
        }
        let value: Value = serde_json::from_slice(&bytes).context("decode headless result")?;
        if value["version"].as_u64() != Some(VERSION)
            || value["request_id"].as_str() != Some(request_id)
            || value["dispatch_generation"]
                .as_str()
                .and_then(|id| Uuid::parse_str(id).ok())
                != Some(generation)
            || value["complete"].as_bool() != Some(true)
        {
            bail!("headless result is incomplete or does not match the requested generation");
        }
        let answer = match value.get("answer") {
            Some(Value::Null) => None,
            Some(Value::String(answer)) => Some(answer.as_str()),
            _ => bail!("headless result has invalid answer"),
        };
        let answer_bytes = answer.map_or(0, str::len);
        if answer_bytes > MAX_ANSWER_BYTES
            || value["answer_bytes"].as_u64() != Some(answer_bytes as u64)
        {
            bail!("headless result answer length does not match its seal");
        }
        Ok(Some(answer.map(str::to_owned)))
    }

    pub(super) fn remove(&self, request_id: &str, generation: Uuid) -> Result<()> {
        match fs::remove_file(self.path(request_id, generation)?) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("remove committed headless result"),
        }
        sync_directory(&self.directory)
    }

    fn path(&self, request_id: &str, generation: Uuid) -> Result<PathBuf> {
        if request_id.is_empty()
            || request_id.len() > 128
            || !request_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err(anyhow!("invalid headless review request id"));
        }
        Ok(self
            .directory
            .join(format!("{request_id}.{generation}.json")))
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .with_context(|| format!("sync headless result directory {}", path.display()))
}

// Portable Rust cannot open a directory as a file on Windows. The result
// file itself is flushed before atomic publication on all platforms.
#[cfg(not(unix))]
fn sync_directory(_: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_survives_reopen_and_is_not_overwritten() {
        let root = tempfile::tempdir().unwrap();
        let generation = Uuid::new_v4();
        let stale = Uuid::new_v4();
        let id = "dispatch-fixture";
        let journal = Journal::open(root.path()).unwrap();
        assert_eq!(journal.read(id, generation).unwrap(), None);
        journal.write(id, generation, Some("검토 결과")).unwrap();
        journal.write(id, generation, Some("검토 결과")).unwrap();
        assert!(journal.write(id, generation, Some("different")).is_err());
        journal.write(id, stale, None).unwrap();
        let reopened = Journal::open(root.path()).unwrap();
        assert_eq!(
            reopened.read(id, generation).unwrap(),
            Some(Some("검토 결과".into()))
        );
        assert_eq!(reopened.read(id, stale).unwrap(), Some(None));
        reopened.remove(id, generation).unwrap();
        reopened.remove(id, generation).unwrap();
        assert_eq!(reopened.read(id, generation).unwrap(), None);
        assert_eq!(reopened.read(id, stale).unwrap(), Some(None));
    }

    #[test]
    fn rejects_unsafe_ids_and_incomplete_mismatched_or_oversized_records() {
        let root = tempfile::tempdir().unwrap();
        let journal = Journal::open(root.path()).unwrap();
        let generation = Uuid::new_v4();
        let id = "dispatch-fixture";
        assert!(journal.write("../escape", generation, None).is_err());
        assert!(journal
            .write(id, generation, Some(&"x".repeat(MAX_ANSWER_BYTES + 1)))
            .is_err());
        let path = journal.path(id, generation).unwrap();
        fs::write(&path, b"{\"complete\":true").unwrap();
        assert!(journal.read(id, generation).is_err());
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "version": VERSION,
                "request_id": id,
                "dispatch_generation": Uuid::new_v4().to_string(),
                "answer_bytes": 4,
                "answer": "test",
                "complete": true,
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(journal.read(id, generation).is_err());
        assert!(path.is_file());
        fs::write(&path, vec![b'x'; MAX_RECORD_BYTES + 1]).unwrap();
        assert!(journal.read(id, generation).is_err());
    }
}
