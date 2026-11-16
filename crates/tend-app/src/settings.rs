//! Where the local dashboard is and its token, read the way tendd reads
//! them: the config file (`TEND_CONFIG`, else `$XDG_CONFIG_HOME/tend/
//! config.toml`, else the platform config directory, as
//! `tend::paths::default_config_file`), then the `TEND_DASHBOARD_*`
//! environment variables, which win.
//!
//! The app only ever talks to a loopback dashboard in token mode, with a
//! token long enough to have been generated (`tend init` makes 64 hex
//! digits): it refuses anything else and says how to fix it. It also reads
//! this host's Fleet node id (`$XDG_DATA_HOME/tend/host-id`), which the
//! dashboard's proof is bound to.

use std::ffi::OsString;
use std::fmt;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

const DEFAULT_BIND: &str = "127.0.0.1:7878";
/// The shortest token the app accepts: a (nonce, proof) pair is an offline
/// check for a guessed token, harmless only when the token is random.
pub const MIN_TOKEN_LEN: usize = 32;

/// The dashboard the app may talk to.
#[derive(Clone, PartialEq, Eq)]
pub struct Dashboard {
    pub addr: SocketAddr,
    pub token: String,
    /// This host's Fleet node id, which the server proof covers.
    pub node: String,
}

impl fmt::Debug for Dashboard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print the token.
        f.debug_struct("Dashboard")
            .field("addr", &self.addr)
            .field("node", &self.node)
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
    WeakToken,
    NoNode(PathBuf),
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
            Self::WeakToken => write!(
                f,
                "the dashboard token is shorter than {MIN_TOKEN_LEN} characters; let `tend init --component dashboard` generate one"
            ),
            Self::NoNode(path) => write!(
                f,
                "no node id at {} yet; start tendd once (`tend daemon start`)",
                path.display()
            ),
            Self::BadBind(bind) => write!(f, "[dashboard] bind {bind:?} is not an address"),
            Self::NotLoopback(bind) => write!(
                f,
                "[dashboard] bind {bind} is not a loopback address; the app only talks to a local dashboard"
            ),
        }
    }
}

fn absolute(env: &impl Fn(&str) -> Option<OsString>, key: &str) -> Option<PathBuf> {
    env(key).map(PathBuf::from).filter(|p| p.is_absolute())
}

/// The config file tendd reads by default.
pub fn config_path(env: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    if let Some(path) = absolute(env, "TEND_CONFIG") {
        return Some(path);
    }
    absolute(env, "XDG_CONFIG_HOME")
        .or_else(dirs::config_dir)
        .map(|dir| dir.join("tend").join("config.toml"))
}

/// This host's node id file, where tendd keeps it.
pub fn node_path(env: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    absolute(env, "XDG_DATA_HOME")
        .or_else(dirs::data_dir)
        .map(|dir| dir.join("tend").join("host-id"))
}

/// Read the dashboard settings from the process environment.
pub fn load() -> Result<Dashboard, SettingsError> {
    let env = |key: &str| std::env::var_os(key);
    let text = match config_path(&env) {
        Some(path) => read(&path)?,
        None => None,
    };
    let node_file = node_path(&env).unwrap_or_else(|| PathBuf::from("host-id"));
    let node = read(&node_file)?
        .map(|text| text.trim().to_owned())
        .filter(|node| !node.is_empty())
        .ok_or(SettingsError::NoNode(node_file))?;
    resolve(text.as_deref(), &node, &env)
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

/// The settings from a config file's text (if any), this host's node id
/// and the environment.
pub fn resolve(
    config: Option<&str>,
    node: &str,
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
    if token.chars().count() < MIN_TOKEN_LEN {
        return Err(SettingsError::WeakToken);
    }
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
    Ok(Dashboard {
        addr,
        token,
        node: node.to_owned(),
    })
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

    const TOKEN: &str = "t0k-0123456789abcdef0123456789abcdef";
    const FILE: &str = "[dashboard]\nenabled = true\nbind = \"127.0.0.1:17878\"\ntoken = \"t0k-0123456789abcdef0123456789abcdef\"\n";

    fn file(body: &str) -> String {
        format!("[dashboard]\nenabled = true\n{body}")
    }

    #[test]
    fn the_config_file_gives_the_address_and_token() {
        let got = resolve(Some(FILE), "n1", &env(&[])).unwrap();
        assert_eq!(got.addr, "127.0.0.1:17878".parse().unwrap());
        assert_eq!(got.token, TOKEN);
        assert_eq!(got.node, "n1");
        assert!(
            !format!("{got:?}").contains("t0k-"),
            "Debug hides the token"
        );
    }

    #[test]
    fn the_environment_wins_like_it_does_for_tendd() {
        let other = "envtok-0123456789abcdef0123456789abcdef";
        let got = resolve(
            Some(FILE),
            "n1",
            &env(&[
                ("TEND_DASHBOARD_TOKEN", other),
                ("TEND_DASHBOARD_BIND", "[::1]:9000"),
            ]),
        )
        .unwrap();
        assert_eq!(got.token, other);
        assert_eq!(got.addr, "[::1]:9000".parse().unwrap());
        let on = resolve(
            None,
            "n1",
            &env(&[
                ("TEND_DASHBOARD_ENABLED", "true"),
                ("TEND_DASHBOARD_TOKEN", TOKEN),
            ]),
        )
        .unwrap();
        assert_eq!(on.addr, DEFAULT_BIND.parse().unwrap());
    }

    #[test]
    fn anything_but_a_loopback_token_dashboard_is_refused() {
        let token = format!("token = \"{TOKEN}\"\n");
        assert_eq!(resolve(None, "n", &env(&[])), Err(SettingsError::Disabled));
        assert_eq!(
            resolve(
                Some(&format!("[dashboard]\nenabled = false\n{token}")),
                "n",
                &env(&[])
            ),
            Err(SettingsError::Disabled)
        );
        assert_eq!(
            resolve(
                Some(&file(&format!("auth = \"none\"\n{token}"))),
                "n",
                &env(&[])
            ),
            Err(SettingsError::AuthOff)
        );
        assert_eq!(
            resolve(Some(&file("token = \"  \"\n")), "n", &env(&[])),
            Err(SettingsError::NoToken)
        );
        assert_eq!(
            resolve(Some(&file("token = \"short\"\n")), "n", &env(&[])),
            Err(SettingsError::WeakToken)
        );
        assert_eq!(
            resolve(
                Some(&file(&format!("{token}bind = \"0.0.0.0:7878\"\n"))),
                "n",
                &env(&[])
            ),
            Err(SettingsError::NotLoopback("0.0.0.0:7878".into()))
        );
        assert!(matches!(
            resolve(
                Some(&file(&format!("{token}bind = \"localhost:1\"\n"))),
                "n",
                &env(&[])
            ),
            Err(SettingsError::BadBind(_))
        ));
    }

    #[test]
    fn the_paths_follow_tend_config_then_xdg() {
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
        assert_eq!(
            node_path(&env(&[("XDG_DATA_HOME", xdg)])),
            Some(PathBuf::from(xdg).join("tend").join("host-id"))
        );
    }
}
