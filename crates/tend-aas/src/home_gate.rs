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
//! A target takes every home it touches (the stopped agent's and the
//! relaunch's) before it starts, all at once or none ([`try_acquire_all`]
//! for the daemon, which puts a target back in its queue rather than wait;
//! [`acquire_all`], in key order, for a swap run in process before it has
//! looked at anything). So no wait ever comes between a target's checks and
//! its first effect, and no two targets can wait on each other. It lets the
//! source home go once the old agent has exited, and the rest once the
//! relaunch has drawn its own prompt ([`HeldHomes`]).
//!
//! A home is its directory: the deepest existing ancestor canonicalized
//! (aliases and symlinks are one home) and the rest appended. One that
//! cannot be told — an unreadable environment, a relative path — shares a
//! single per-provider bucket with every other unknown home, so two names
//! for one directory are never two gates.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Instant;

use tend::AgentKind;

/// Which home: `<provider>:<canonical dir>`, or `<provider>:?` when unknown.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HomeKey(String);

impl HomeKey {
    /// `dir` of `provider`; the unknown bucket unless it is absolute.
    pub(crate) fn known(provider: &str, dir: &Path) -> Self {
        canonical(dir).map_or_else(
            || Self::unknown(provider),
            |dir| Self(format!("{provider}:{}", dir.display())),
        )
    }

    /// The shared bucket of every home of `provider` that cannot be told.
    pub(crate) fn unknown(provider: &str) -> Self {
        Self(format!("{provider}:?"))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// `dir` with its deepest existing ancestor canonicalized and the missing
/// rest appended, `.` and `..` resolved by name. `None` for a relative path.
fn canonical(dir: &Path) -> Option<PathBuf> {
    if !dir.is_absolute() {
        return None;
    }
    let mut lexical = PathBuf::new();
    for part in dir.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                lexical.pop();
            }
            other => lexical.push(other),
        }
    }
    let mut missing = Vec::new();
    let mut existing = lexical.as_path();
    loop {
        if let Ok(real) = std::fs::canonicalize(existing) {
            let mut full = real;
            for part in missing.iter().rev() {
                full.push(part);
            }
            return Some(full);
        }
        missing.push(existing.file_name()?.to_owned());
        existing = existing.parent()?;
    }
}

/// The environment variable that names a provider's home, and its default
/// under the process's `HOME`.
fn home_var(kind: AgentKind) -> Option<(&'static str, &'static str, &'static str)> {
    match kind {
        AgentKind::ClaudeCode => Some(("claude", "CLAUDE_CONFIG_DIR", ".claude")),
        AgentKind::Codex => Some(("codex", "CODEX_HOME", ".codex")),
        _ => None,
    }
}

/// The home the running agent `pid` uses, read from its own environment:
/// the provider's variable, else that process's `HOME` and the provider's
/// default; the unknown bucket when the environment cannot be read, or what
/// it names is not absolute.
pub(crate) fn source_home(kind: AgentKind, pid: u32) -> HomeKey {
    let Some((provider, var, default)) = home_var(kind) else {
        return HomeKey::unknown("other");
    };
    source_home_from(
        provider,
        var,
        default,
        &tend::accounts::process_environ(pid),
    )
}

fn source_home_from(
    provider: &str,
    var: &str,
    default: &str,
    environ: &[(String, String)],
) -> HomeKey {
    let get = |name: &str| {
        environ
            .iter()
            .find(|(key, value)| key == name && !value.is_empty())
            .map(|(_, value)| PathBuf::from(value))
    };
    if environ.is_empty() {
        return HomeKey::unknown(provider);
    }
    match get(var) {
        Some(dir) => HomeKey::known(provider, &dir),
        None => get("HOME").map_or_else(
            || HomeKey::unknown(provider),
            |home| HomeKey::known(provider, &home.join(default)),
        ),
    }
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
    crate::codex_hook_trust::account_var(profiles, provider, account, var, aas).map_or_else(
        || HomeKey::unknown(provider),
        |dir| HomeKey::known(provider, &dir),
    )
}

/// Which homes are held, by whom (pane), since when.
type Gates = HashMap<HomeKey, (String, Instant)>;

fn gates() -> &'static (Mutex<Gates>, std::sync::Condvar) {
    static GATES: OnceLock<(Mutex<Gates>, std::sync::Condvar)> = OnceLock::new();
    GATES.get_or_init(|| (Mutex::default(), std::sync::Condvar::new()))
}

fn release(key: &HomeKey, because: &str) {
    let (held, freed) = gates();
    let mut held = held.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((pane, since)) = held.remove(key) {
        tracing::info!(
            home = key.as_str(),
            pane = %pane,
            held_ms = u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX),
            released = because,
            "swap home gate released"
        );
    }
    freed.notify_all();
}

/// The homes one target holds. Cloned into the target's run; the homes go
/// when [`Self::stopped`] / [`Self::booted`] say so, or when the last clone
/// is dropped.
#[derive(Clone, Default)]
pub struct HeldHomes(Option<Arc<Held>>);

struct Held {
    source: HomeKey,
    destination: HomeKey,
    /// Still held, of `source` and `destination`.
    keys: Mutex<Vec<HomeKey>>,
}

impl Drop for Held {
    fn drop(&mut self) {
        for key in self
            .keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
        {
            release(&key, "the target ended");
        }
    }
}

impl HeldHomes {
    /// Nothing held: a target with no home to gate (tests, a dry run).
    pub fn none() -> Self {
        Self(None)
    }

    fn let_go(&self, key: &HomeKey, because: &str) {
        if let Some(held) = &self.0 {
            let mut keys = held.keys.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(at) = keys.iter().position(|k| k == key) {
                keys.remove(at);
                drop(keys);
                release(key, because);
            }
        }
    }

    /// The old agent has exited: its home goes, unless the relaunch boots
    /// on the same one.
    pub(crate) fn stopped(&self) {
        if let Some(held) = &self.0 {
            if held.source != held.destination {
                self.let_go(&held.source, "the agent exited");
            }
        }
    }

    /// The relaunch has drawn its own prompt (or will not): everything goes.
    pub(crate) fn booted(&self, because: &str) {
        if let Some(held) = &self.0 {
            let keys: Vec<HomeKey> = held
                .keys
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            for key in keys {
                self.let_go(&key, because);
            }
        }
    }

    /// Whether any home is still held (tests).
    #[cfg(test)]
    pub(crate) fn holding(&self) -> bool {
        self.0.as_ref().is_some_and(|held| {
            !held
                .keys
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_empty()
        })
    }
}

fn holding(source: &HomeKey, destination: &HomeKey, pane: &str) -> HeldHomes {
    let mut keys = vec![source.clone(), destination.clone()];
    keys.dedup();
    let held = gates().0.lock().unwrap_or_else(PoisonError::into_inner);
    for key in &keys {
        tracing::info!(home = key.as_str(), pane, "swap home gate acquired");
    }
    drop(held);
    HeldHomes(Some(Arc::new(Held {
        source: source.clone(),
        destination: destination.clone(),
        keys: Mutex::new(keys),
    })))
}

/// Both homes for `pane` if neither is held, else `None` with nothing held.
pub fn try_acquire_all(source: &HomeKey, destination: &HomeKey, pane: &str) -> Option<HeldHomes> {
    {
        let mut held = gates().0.lock().unwrap_or_else(PoisonError::into_inner);
        if held.contains_key(source) || held.contains_key(destination) {
            return None;
        }
        let now = Instant::now();
        held.insert(source.clone(), (pane.to_owned(), now));
        held.insert(destination.clone(), (pane.to_owned(), now));
    }
    Some(holding(source, destination, pane))
}

/// Both homes for `pane`, waiting until neither is held; taken together, so
/// two waiters can never hold one each.
pub(crate) fn acquire_all(source: &HomeKey, destination: &HomeKey, pane: &str) -> HeldHomes {
    {
        let (held, freed) = gates();
        let mut held = held.lock().unwrap_or_else(PoisonError::into_inner);
        while held.contains_key(source) || held.contains_key(destination) {
            held = freed.wait(held).unwrap_or_else(PoisonError::into_inner);
        }
        let now = Instant::now();
        held.insert(source.clone(), (pane.to_owned(), now));
        held.insert(destination.clone(), (pane.to_owned(), now));
    }
    holding(source, destination, pane)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn key(name: &str) -> HomeKey {
        HomeKey(format!("test:{name}-{}", std::process::id()))
    }

    #[test]
    fn one_holder_per_home_and_both_homes_or_neither() {
        let (a, b, c) = (key("a1"), key("b1"), key("c1"));
        let held = try_acquire_all(&a, &b, "%1").expect("free");
        assert!(try_acquire_all(&b, &c, "%2").is_none(), "b is held");
        assert!(
            try_acquire_all(&c, &c, "%3").is_some(),
            "c was not taken by the failed try"
        );
        held.stopped();
        assert!(
            try_acquire_all(&a, &a, "%4").is_some(),
            "the source went on stop"
        );
        assert!(
            try_acquire_all(&b, &b, "%5").is_none(),
            "the destination stays until boot"
        );
        held.booted("test");
        assert!(!held.holding());
        assert!(try_acquire_all(&b, &b, "%6").is_some());
    }

    #[test]
    fn a_shared_home_stays_held_through_the_stop_until_boot() {
        let a = key("same");
        let held = try_acquire_all(&a, &a, "%1").expect("free");
        held.stopped();
        assert!(try_acquire_all(&a, &a, "%2").is_none());
        drop(held);
        assert!(try_acquire_all(&a, &a, "%3").is_some(), "dropped, it goes");
    }

    #[test]
    fn waiters_hold_one_home_at_a_time_each() {
        let (a, b) = (key("wa"), key("wb"));
        let inside = Arc::new(AtomicUsize::new(0));
        let most = Arc::new(AtomicUsize::new(0));
        std::thread::scope(|scope| {
            for i in 0..6 {
                let (a, b, inside, most) =
                    (a.clone(), b.clone(), Arc::clone(&inside), Arc::clone(&most));
                scope.spawn(move || {
                    // Half take (a, b), half (b, a): no deadlock either way.
                    let held = if i % 2 == 0 {
                        acquire_all(&a, &b, "%1")
                    } else {
                        acquire_all(&b, &a, "%2")
                    };
                    let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                    most.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    inside.fetch_sub(1, Ordering::SeqCst);
                    held.booted("test");
                });
            }
        });
        assert_eq!(most.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn aliases_missing_tails_and_relative_paths() {
        let base = std::env::temp_dir().join(format!("tend-home-gate-{}", std::process::id()));
        let real = base.join("real");
        std::fs::create_dir_all(&real).unwrap();
        let alias = base.join("alias");
        let _ = std::fs::remove_file(&alias);
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        #[cfg(unix)]
        {
            // One directory, not yet created, through two names.
            assert_eq!(
                HomeKey::known("claude", &real.join("new-home")),
                HomeKey::known("claude", &alias.join("new-home"))
            );
            assert_eq!(
                HomeKey::known("claude", &alias.join("x/../new-home")),
                HomeKey::known("claude", &real.join("new-home"))
            );
        }
        assert_ne!(
            HomeKey::known("claude", &real),
            HomeKey::known("codex", &real)
        );
        assert_eq!(
            HomeKey::known("claude", Path::new("rel/home")),
            HomeKey::unknown("claude")
        );
        let _ = std::fs::remove_file(&alias);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn an_unset_variable_means_that_processes_own_home() {
        let env = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect()
        };
        let theirs = source_home_from(
            "claude",
            "CLAUDE_CONFIG_DIR",
            ".claude",
            &env(&[("HOME", "/h/them")]),
        );
        assert_eq!(
            theirs,
            HomeKey::known("claude", Path::new("/h/them/.claude"))
        );
        let set = source_home_from(
            "claude",
            "CLAUDE_CONFIG_DIR",
            ".claude",
            &env(&[("HOME", "/h/them"), ("CLAUDE_CONFIG_DIR", "/acct/work")]),
        );
        assert_eq!(set, HomeKey::known("claude", Path::new("/acct/work")));
        assert_eq!(
            source_home_from(
                "claude",
                "CLAUDE_CONFIG_DIR",
                ".claude",
                &env(&[("PATH", "/bin")])
            ),
            HomeKey::unknown("claude"),
            "no HOME, no variable"
        );
        assert_eq!(
            source_home_from("claude", "CLAUDE_CONFIG_DIR", ".claude", &[]),
            HomeKey::unknown("claude"),
            "unreadable"
        );
        assert_eq!(
            source_home_from(
                "claude",
                "CLAUDE_CONFIG_DIR",
                ".claude",
                &env(&[("CLAUDE_CONFIG_DIR", "rel")])
            ),
            HomeKey::unknown("claude"),
            "relative"
        );
    }
}
