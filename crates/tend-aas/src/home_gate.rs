//! One account home at a time for Tend's own bursts of activity on it.
//!
//! Several targets of a swap run at once, often onto one account. What Tend
//! itself does to that account's home — carrying Codex hook trust, quitting
//! an agent (its exit writes session state), starting the relaunch until it
//! first draws its prompt (its boot reads and writes config) — goes through
//! that home's gate, one at a time. It does not make the home single-writer:
//! long-running sessions on it keep writing as they always have. It only
//! keeps a swap from adding a burst of simultaneous boots and exits there.
//!
//! A home is its directory, canonicalized (aliases and symlinks are one
//! home). One that cannot be told shares a single per-provider bucket with
//! every other unknown home, so two names for one directory are never two
//! gates. A permit is held by value and released when dropped, on every
//! path; nothing holds two at once.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError};
use std::time::Instant;

use tend::AgentKind;

/// Which home: `<provider>:<canonical dir>`, or `<provider>:?` when unknown.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct HomeKey(String);

impl HomeKey {
    fn known(provider: &str, dir: &Path) -> Self {
        let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        Self(format!("{provider}:{}", dir.display()))
    }

    /// The shared bucket of every home of `provider` that cannot be told.
    pub(crate) fn unknown(provider: &str) -> Self {
        Self(format!("{provider}:?"))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// The environment variable that names a provider's home, and its default
/// under `$HOME`.
fn home_var(kind: AgentKind) -> Option<(&'static str, &'static str, &'static str)> {
    match kind {
        AgentKind::ClaudeCode => Some(("claude", "CLAUDE_CONFIG_DIR", ".claude")),
        AgentKind::Codex => Some(("codex", "CODEX_HOME", ".codex")),
        _ => None,
    }
}

fn default_home(default: &str) -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(default))
}

/// The home the running agent `pid` uses: its own environment's variable,
/// the provider's default when the variable is unset, and the unknown
/// bucket when the environment cannot be read.
pub(crate) fn source_home(kind: AgentKind, pid: u32) -> HomeKey {
    let Some((provider, var, default)) = home_var(kind) else {
        return HomeKey::unknown("other");
    };
    let environ = tend::accounts::process_environ(pid);
    if environ.is_empty() {
        return HomeKey::unknown(provider);
    }
    environ
        .iter()
        .find(|(name, value)| name == var && !value.is_empty())
        .map(|(_, value)| PathBuf::from(value))
        .filter(|dir| dir.is_absolute())
        .or_else(|| default_home(default))
        .map_or_else(
            || HomeKey::unknown(provider),
            |dir| HomeKey::known(provider, &dir),
        )
}

/// The home a relaunch onto `account` runs under: a configured profile's
/// variable, else what `aas export <account>` sets, else the unknown bucket.
pub(crate) fn destination_home(
    kind: AgentKind,
    profiles: &[tend::accounts::AccountProfile],
    account: &str,
    aas: &str,
) -> HomeKey {
    let Some((provider, var, _)) = home_var(kind) else {
        return HomeKey::unknown("other");
    };
    crate::codex_hook_trust::account_var(profiles, provider, account, var, aas)
        .filter(|dir| dir.is_absolute())
        .map_or_else(
            || HomeKey::unknown(provider),
            |dir| HomeKey::known(provider, &dir),
        )
}

#[derive(Default)]
struct Gate {
    busy: Mutex<bool>,
    free: Condvar,
}

fn gates() -> &'static Mutex<HashMap<HomeKey, Arc<Gate>>> {
    static GATES: OnceLock<Mutex<HashMap<HomeKey, Arc<Gate>>>> = OnceLock::new();
    GATES.get_or_init(Mutex::default)
}

/// A held home gate; dropping it lets the next one through.
#[must_use = "the gate is released as soon as the permit is dropped"]
pub(crate) struct Permit {
    gate: Arc<Gate>,
    key: HomeKey,
    pane: String,
    why: &'static str,
    acquired: Instant,
    released_because: Option<&'static str>,
}

impl Permit {
    /// Release now, saying what made it safe (for the daemon log).
    pub(crate) fn release(mut self, because: &'static str) {
        self.released_because = Some(because);
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        *self
            .gate
            .busy
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = false;
        self.gate.free.notify_one();
        tracing::info!(
            home = self.key.as_str(),
            pane = %self.pane,
            why = self.why,
            held_ms = u64::try_from(self.acquired.elapsed().as_millis()).unwrap_or(u64::MAX),
            released = self.released_because.unwrap_or("ended"),
            "swap home gate released"
        );
    }
}

/// Wait for `key`'s gate and hold it, for `why` in `pane`.
pub(crate) fn acquire(key: &HomeKey, pane: &str, why: &'static str) -> Permit {
    let gate = {
        let mut gates = gates().lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(gates.entry(key.clone()).or_default())
    };
    let asked = Instant::now();
    {
        let mut busy = gate.busy.lock().unwrap_or_else(PoisonError::into_inner);
        while *busy {
            busy = gate.free.wait(busy).unwrap_or_else(PoisonError::into_inner);
        }
        *busy = true;
    }
    tracing::info!(
        home = key.as_str(),
        pane,
        why,
        waited_ms = u64::try_from(asked.elapsed().as_millis()).unwrap_or(u64::MAX),
        "swap home gate acquired"
    );
    Permit {
        gate,
        key: key.clone(),
        pane: pane.to_owned(),
        why,
        acquired: Instant::now(),
        released_because: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn one_holder_per_home_and_none_between_homes() {
        let a = HomeKey("test:/one-home".into());
        let b = HomeKey("test:/other-home".into());
        let inside = Arc::new(AtomicUsize::new(0));
        let most = Arc::new(AtomicUsize::new(0));
        std::thread::scope(|scope| {
            for _ in 0..6 {
                let (a, inside, most) = (a.clone(), Arc::clone(&inside), Arc::clone(&most));
                scope.spawn(move || {
                    let permit = acquire(&a, "%1", "test");
                    let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                    most.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    inside.fetch_sub(1, Ordering::SeqCst);
                    permit.release("test");
                });
            }
        });
        assert_eq!(
            most.load(Ordering::SeqCst),
            1,
            "one holder of one home at a time"
        );
        // Another home is not held up by a held one.
        let held = acquire(&a, "%1", "test");
        let other = acquire(&b, "%2", "test");
        drop(other);
        drop(held);
    }

    #[test]
    fn aliases_of_one_directory_are_one_home_and_unknowns_share_a_bucket() {
        let dir = std::env::temp_dir().join(format!("tend-home-gate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let link = dir.with_extension("link");
        let _ = std::fs::remove_file(&link);
        #[cfg(unix)]
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        #[cfg(unix)]
        assert_eq!(
            HomeKey::known("claude", &dir),
            HomeKey::known("claude", &link)
        );
        assert_ne!(
            HomeKey::known("claude", &dir),
            HomeKey::known("codex", &dir)
        );
        assert_eq!(HomeKey::unknown("claude"), HomeKey::unknown("claude"));
        let _ = std::fs::remove_file(&link);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
