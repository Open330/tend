//! The app's identity on the machine: its `AppUserModelID` (toasts), its
//! start-with-Windows value, and where it keeps its state. Each can be
//! overridden through the environment so a test run never touches the
//! person's own entries.

use std::path::{Path, PathBuf};

/// The `AppUserModelID` toasts are sent under. Owner decision 3 in
/// `docs/proposals/WINDOWS_APP.md`; changing it later loses the person's
/// notification settings.
pub const APP_ID: &str = "dev.tend.windows";
/// The name toasts and the Run value show.
pub const DISPLAY_NAME: &str = "Tend";

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default.to_owned())
}

/// `TEND_APP_ID`, else [`APP_ID`].
#[must_use]
pub fn app_id() -> String {
    env_or("TEND_APP_ID", APP_ID)
}

/// `TEND_APP_RUN_VALUE`, else [`DISPLAY_NAME`]: the value under
/// `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
#[must_use]
pub fn run_value() -> String {
    env_or("TEND_APP_RUN_VALUE", DISPLAY_NAME)
}

/// `TEND_APP_STATE_DIR`, else `%LOCALAPPDATA%\tend\app`.
#[must_use]
pub fn state_dir() -> PathBuf {
    std::env::var_os("TEND_APP_STATE_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| dirs::data_local_dir().map(|dir| dir.join("tend").join("app")))
        .unwrap_or_else(|| PathBuf::from("tend-app"))
}

/// The command the Run value holds: the executable, quoted, started in the
/// tray.
#[must_use]
pub fn run_command(exe: &Path) -> String {
    format!("\"{}\" --background", exe.display())
}

/// A toast tag for an item key: short (older Windows allows 16 characters),
/// stable, and free of anything the agent wrote.
#[must_use]
pub fn toast_tag(key: &str) -> String {
    // FNV-1a, 64 bits.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in key.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_run_command_quotes_the_executable() {
        assert_eq!(
            run_command(Path::new(r"C:\Users\A B\.cargo\bin\tend-app.exe")),
            r#""C:\Users\A B\.cargo\bin\tend-app.exe" --background"#
        );
    }

    #[test]
    fn toast_tags_are_short_stable_and_distinct() {
        let a = toast_tag("home|q|rq_1");
        assert_eq!(a.len(), 16);
        assert_eq!(a, toast_tag("home|q|rq_1"));
        assert_ne!(a, toast_tag("mini|q|rq_1"));
        assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
    }
}
