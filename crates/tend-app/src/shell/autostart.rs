//! Start with Windows: a value under
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` holding
//! [`identity::run_command`].

use std::path::Path;

use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
use winreg::RegKey;

use crate::identity;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Whether the Run value starts this executable.
#[must_use]
pub fn enabled(exe: &Path) -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_KEY, KEY_READ)
        .and_then(|key| key.get_value::<String, _>(identity::run_value()))
        .is_ok_and(|value| value == identity::run_command(exe))
}

pub fn set(exe: &Path, on: bool) -> std::io::Result<()> {
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE)?;
    if on {
        key.set_value(identity::run_value(), &identity::run_command(exe))
    } else {
        match key.delete_value(identity::run_value()) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }
}
