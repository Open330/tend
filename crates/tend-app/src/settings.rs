//! Where the local dashboard is and its token, read the way tendd reads
//! them: the config file (`TEND_CONFIG`, else `$XDG_CONFIG_HOME/tend/
//! config.toml`, else the platform config directory, as
//! `tend::paths::default_config_file`), then the `TEND_DASHBOARD_*`
//! environment variables, which win.
//!
//! The app only ever talks to a loopback dashboard in token mode: it refuses
//! anything else and says how to fix it.

use std::ffi::OsString;
use std::fmt;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

const DEFAULT_BIND: &str = "127.0.0.1:7878";

/// The dashboard the app may talk to.
#[derive(Clone, PartialEq, Eq)]
pub struct Dashboard {
    pub addr: SocketAddr,
    pub token: String,
}

impl fmt::Debug for Dashboard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print the token.
        f.debug_struct("Dashboard")
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

/// Why the app cannot use the dashboard, with the fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsError {
    Unreadable { path: PathBuf, error: String },
    Disabled,
    AuthOff,
    NoToken,
    BadBind(String),
    NotLoopback(String),
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { path, error } => {
                write!(f, "cannot read {}: {error}", path.display())
            }
            Self::Disabled => f.write_str(
                "the dashboard is off; run `tend init --component dashboard`, then restart tendd",
            ),
            Self::AuthOff => f.write_str(
                "the dashboard runs without authentication (auth = \"none\"); set auth = \"token\" with a token",
            ),
            Self::NoToken => f.write_str(
                "the dashboard has no token; run `tend init --component dashboard` or set [dashboard] token",
            ),
            Self::BadBind(bind) => write!(f, "[dashboard] bind {bind:?} is not an address"),
            Self::NotLoopback(bind) => write!(
                f,
                "[dashboard] bind {bind} is not a loopback address; the app only talks to a local dashboard"
            ),
        }
    }
}

/// The config file tendd reads by default.
pub fn config_path(env: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let absolute = |key: &str| env(key).map(PathBuf::from).filter(|p| p.is_absolute());
    if let Some(path) = absolute("TEND_CONFIG") {
        return Some(path);
    }
    absolute("XDG_CONFIG_HOME")
        .or_else(dirs::config_dir)
        .map(|dir| dir.join("tend").join("config.toml"))
}

/// Read the dashboard settings from the process environment.
pub fn load() -> Result<Dashboard, SettingsError> {
    let env = |key: &str| std::env::var_os(key);
    let text = match config_path(&env) {
        Some(path) => read(&path)?,
        None => None,
    };
    resolve(text.as_deref(), &env)
}

fn read(path: &Path) -> Result<Option<String>, SettingsError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(SettingsError::Unreadable {
            path: path.to_path_buf(),
            error: error.to_string(),
        }),
    }
}

/// The settings from a config file's text (if any) and the environment.
pub fn resolve(
    config: Option<&str>,
    env: &impl Fn(&str) -> Option<OsString>,
) -> Result<Dashboard, SettingsError> {
    let table = config
        .and_then(|text| text.parse::<toml::Table>().ok())
        .and_then(|table| {
            table
                .get("dashboard")
                .and_then(toml::Value::as_table)
                .cloned()
        })
        .unwrap_or_default();
    let file_str = |key: &str| {
        table
            .get(key)
            .and_then(toml::Value::as_str)
            .map(str::to_owned)
    };
    let env_str = |key: &str| {
        env(key)
            .and_then(|value| value.into_string().ok())
            .filter(|value| !value.trim().is_empty())
    };

    let enabled = match env_str("TEND_DASHBOARD_ENABLED") {
        Some(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        None => table
            .get("enabled")
            .and_then(toml::Value::as_bool)
            .unwrap_or(false),
    };
    if !enabled {
        return Err(SettingsError::Disabled);
    }
    let auth = env_str("TEND_DASHBOARD_AUTH")
        .or_else(|| file_str("auth"))
        .unwrap_or_else(|| "token".into());
    if auth.trim().eq_ignore_ascii_case("none") {
        return Err(SettingsError::AuthOff);
    }
    let token = env_str("TEND_DASHBOARD_TOKEN")
        .or_else(|| file_str("token"))
        .map(|token| token.trim().to_owned())
        .filter(|token| !token.is_empty())
        .ok_or(SettingsError::NoToken)?;
    let bind = env_str("TEND_DASHBOARD_BIND")
        .or_else(|| file_str("bind"))
        .unwrap_or_else(|| DEFAULT_BIND.into());
    let addr: SocketAddr = bind
        .trim()
        .parse()
        .map_err(|_| SettingsError::BadBind(bind.clone()))?;
    if !addr.ip().is_loopback() {
        return Err(SettingsError::NotLoopback(bind));
    }
    Ok(Dashboard { addr, token })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: HashMap<String, OsString> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), OsString::from(*v)))
            .collect();
        move |key| map.get(key).cloned()
    }

    const FILE: &str = "[dashboard]\nenabled = true\nbind = \"127.0.0.1:17878\"\ntoken = \"t0k\"\n";

    #[test]
    fn the_config_file_gives_the_address_and_token() {
        let got = resolve(Some(FILE), &env(&[])).unwrap();
        assert_eq!(got.addr, "127.0.0.1:17878".parse().unwrap());
        assert_eq!(got.token, "t0k");
        assert!(!format!("{got:?}").contains("t0k"), "Debug hides the token");
    }

    #[test]
    fn the_environment_wins_like_it_does_for_tendd() {
        let got = resolve(
            Some(FILE),
            &env(&[
                ("TEND_DASHBOARD_TOKEN", "envtok"),
                ("TEND_DASHBOARD_BIND", "[::1]:9000"),
            ]),
        )
        .unwrap();
        assert_eq!(got.token, "envtok");
        assert_eq!(got.addr, "[::1]:9000".parse().unwrap());
        let on = resolve(
            None,
            &env(&[
                ("TEND_DASHBOARD_ENABLED", "true"),
                ("TEND_DASHBOARD_TOKEN", "x"),
            ]),
        )
        .unwrap();
        assert_eq!(on.addr, DEFAULT_BIND.parse().unwrap());
    }

    #[test]
    fn anything_but_a_loopback_token_dashboard_is_refused() {
        assert_eq!(resolve(None, &env(&[])), Err(SettingsError::Disabled));
        assert_eq!(
            resolve(
                Some("[dashboard]\nenabled = false\ntoken = \"x\"\n"),
                &env(&[])
            ),
            Err(SettingsError::Disabled)
        );
        assert_eq!(
            resolve(
                Some("[dashboard]\nenabled = true\nauth = \"none\"\ntoken = \"x\"\n"),
                &env(&[])
            ),
            Err(SettingsError::AuthOff)
        );
        assert_eq!(
            resolve(
                Some("[dashboard]\nenabled = true\ntoken = \"  \"\n"),
                &env(&[])
            ),
            Err(SettingsError::NoToken)
        );
        assert_eq!(
            resolve(
                Some("[dashboard]\nenabled = true\ntoken = \"x\"\nbind = \"0.0.0.0:7878\"\n"),
                &env(&[])
            ),
            Err(SettingsError::NotLoopback("0.0.0.0:7878".into()))
        );
        assert!(matches!(
            resolve(
                Some("[dashboard]\nenabled = true\ntoken = \"x\"\nbind = \"localhost:1\"\n"),
                &env(&[])
            ),
            Err(SettingsError::BadBind(_))
        ));
    }

    #[test]
    fn the_config_path_follows_tend_config_then_xdg() {
        let tend_config = if cfg!(windows) {
            r"C:\t\c.toml"
        } else {
            "/t/c.toml"
        };
        let xdg = if cfg!(windows) { r"C:\x" } else { "/x" };
        assert_eq!(
            config_path(&env(&[
                ("TEND_CONFIG", tend_config),
                ("XDG_CONFIG_HOME", xdg)
            ])),
            Some(PathBuf::from(tend_config))
        );
        assert_eq!(
            config_path(&env(&[
                ("TEND_CONFIG", "relative.toml"),
                ("XDG_CONFIG_HOME", xdg)
            ])),
            Some(PathBuf::from(xdg).join("tend").join("config.toml"))
        );
    }
}
