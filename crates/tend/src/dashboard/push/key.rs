//! The APNs signing key (`.p8`, PKCS#8 PEM, P-256).
//!
//! Opened without following a final symlink and without blocking (a FIFO
//! or a device at the path cannot hold the open), then checked on the open
//! descriptor: a regular file, owned by this user, with no group or other
//! permission bits, at most [`MAX_KEY_BYTES`]. It is read from that same
//! descriptor. An error names the path and the failed check, never the
//! contents.
use p256::ecdsa::SigningKey;
use std::path::Path;

pub(super) const MAX_KEY_BYTES: u64 = 4096;

/// Load the key at `path`, which must be owned by `uid` (the daemon's own).
#[cfg(unix)]
pub(super) fn load_as(path: &Path, uid: u32) -> Result<SigningKey, String> {
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let shown = path.display();
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| {
            if error.raw_os_error() == Some(libc::ELOOP) {
                format!("{shown}: is a symlink")
            } else {
                format!("{shown}: cannot be opened ({})", error.kind())
            }
        })?;
    // fstat: the descriptor just opened, whatever is at the path now.
    let meta = file
        .metadata()
        .map_err(|error| format!("{shown}: cannot be checked ({})", error.kind()))?;
    if !meta.file_type().is_file() {
        return Err(format!("{shown}: is not a regular file"));
    }
    if meta.uid() != uid {
        return Err(format!("{shown}: is not owned by the daemon's user"));
    }
    if meta.mode() & 0o077 != 0 {
        return Err(format!(
            "{shown}: has group or other permissions (mode {:o}); use 0600 or 0400",
            meta.mode() & 0o777
        ));
    }
    if meta.len() > MAX_KEY_BYTES {
        return Err(format!("{shown}: is larger than {MAX_KEY_BYTES} bytes"));
    }
    let mut pem = String::new();
    file.take(MAX_KEY_BYTES + 1)
        .read_to_string(&mut pem)
        .map_err(|_| format!("{shown}: cannot be read as text"))?;
    if pem.len() as u64 > MAX_KEY_BYTES {
        return Err(format!("{shown}: is larger than {MAX_KEY_BYTES} bytes"));
    }
    parse(&pem).map_err(|()| format!("{shown}: is not a PKCS#8 P-256 private key"))
}

/// Load the key at `path`, owned by the daemon's user.
#[cfg(unix)]
pub(super) fn load(path: &Path) -> Result<SigningKey, String> {
    load_as(path, nix::unistd::getuid().as_raw())
}

#[cfg(not(unix))]
pub(super) fn load(_path: &Path) -> Result<SigningKey, String> {
    Err("push is not supported on this platform".into())
}

/// A PKCS#8 PEM private key on P-256. The error carries nothing.
fn parse(pem: &str) -> Result<SigningKey, ()> {
    use p256::pkcs8::DecodePrivateKey;
    SigningKey::from_pkcs8_pem(pem.trim()).map_err(drop)
}
