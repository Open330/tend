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

/// `dir` as the filesystem resolves it: walked one component at a time,
/// each existing prefix canonicalized before the next is applied (so a
/// `..` after a symlink leaves the symlink's target, as the kernel does),
/// the missing tail appended as written. `None` for a relative path, or a
/// `..` within the missing tail (where the answer would be a guess).
fn canonical(dir: &Path) -> Option<PathBuf> {
    if !dir.is_absolute() {
        return None;
    }
    let mut at = PathBuf::new();
    let mut missing = false;
    for part in dir.components() {
        match part {
            Component::Prefix(_) | Component::RootDir => at.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                if missing {
                    return None;
                }
                at.pop();
            }
            Component::Normal(name) => {
                at.push(name);
                if !missing {
                    match std::fs::canonicalize(&at) {
                        Ok(real) => at = real,
                        Err(_) => missing = true,
                    }
                }
            }
        }
    }
    Some(at)
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

/// Who holds a home: a target's own token, its pane, since when.
struct Holder {
    token: u64,
    pane: String,
    since: Instant,
}

/// Which homes are held, and by whom.
type Gates = HashMap<HomeKey, Vec<Holder>>;

fn next_token() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// How many targets may hold one Claude home at once. Claude itself writes
/// its config all through a session; with real claude 2.1.328 on a
/// disposable home, eight simultaneous boots and exits lost no update in
/// ten trials, sixteen lost some of its own counters in four. Four stays
/// well inside what was measured exact.
pub(crate) const CLAUDE_HOME_HOLDERS: usize = 4;

/// How many targets may hold `key` at once: [`CLAUDE_HOME_HOLDERS`] for a
/// Claude home (known or not), one for any other — Codex, whose hook
/// trust Tend itself prepares in the home, and whose own behaviour on a
/// shared home has not been measured.
fn capacity(key: &HomeKey) -> usize {
    if key.0.starts_with("claude:") {
        CLAUDE_HOME_HOLDERS
    } else {
        1
    }
}

/// Whether `pane` may take `keys` (distinct) now.
fn room(held: &Gates, keys: &[HomeKey]) -> bool {
    keys.iter()
        .all(|key| held.get(key).map_or(0, Vec::len) < capacity(key))
}

fn take(held: &mut Gates, keys: &[HomeKey], token: u64, pane: &str) {
    let since = Instant::now();
    for key in keys {
        held.entry(key.clone()).or_default().push(Holder {
            token,
            pane: pane.to_owned(),
            since,
        });
    }
}

fn distinct(source: &HomeKey, destination: &HomeKey) -> Vec<HomeKey> {
    let mut keys = vec![source.clone(), destination.clone()];
    keys.dedup();
    keys
}

fn gates() -> &'static (Mutex<Gates>, std::sync::Condvar) {
    static GATES: OnceLock<(Mutex<Gates>, std::sync::Condvar)> = OnceLock::new();
    GATES.get_or_init(|| (Mutex::default(), std::sync::Condvar::new()))
}

fn release(key: &HomeKey, token: u64, because: &str) {
    let (held, freed) = gates();
    let mut held = held.lock().unwrap_or_else(PoisonError::into_inner);
    let taken = held.get_mut(key).and_then(|holders| {
        let at = holders.iter().position(|holder| holder.token == token)?;
        Some(holders.remove(at))
    });
    if held.get(key).is_some_and(Vec::is_empty) {
        held.remove(key);
    }
    if let Some(Holder { pane, since, .. }) = taken {
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
    token: u64,
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
            release(&key, self.token, "the target ended");
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
                release(key, held.token, because);
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

fn holding(source: &HomeKey, destination: &HomeKey, token: u64, pane: &str) -> HeldHomes {
    let keys = distinct(source, destination);
    for key in &keys {
        tracing::info!(home = key.as_str(), pane, "swap home gate acquired");
    }
    HeldHomes(Some(Arc::new(Held {
        token,
        source: source.clone(),
        destination: destination.clone(),
        keys: Mutex::new(keys),
    })))
}

/// Both homes for `pane` if each has room ([`capacity`]), else `None` with
/// nothing held.
pub fn try_acquire_all(source: &HomeKey, destination: &HomeKey, pane: &str) -> Option<HeldHomes> {
    let keys = distinct(source, destination);
    let token = next_token();
    {
        let mut held = gates().0.lock().unwrap_or_else(PoisonError::into_inner);
        if !room(&held, &keys) {
            return None;
        }
        take(&mut held, &keys, token, pane);
    }
    Some(holding(source, destination, token, pane))
}

/// Both homes for `pane`, waiting until each has room; taken together, so
/// two waiters can never hold one each.
pub(crate) fn acquire_all(source: &HomeKey, destination: &HomeKey, pane: &str) -> HeldHomes {
    let keys = distinct(source, destination);
    let token = next_token();
    {
        let (held, freed) = gates();
        let mut held = held.lock().unwrap_or_else(PoisonError::into_inner);
        while !room(&held, &keys) {
            held = freed.wait(held).unwrap_or_else(PoisonError::into_inner);
        }
        take(&mut held, &keys, token, pane);
    }
    holding(source, destination, token, pane)
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
    fn a_claude_home_takes_up_to_four_holders_and_any_other_one() {
        let claude = HomeKey(format!("claude:/tmp/gate-test-{}", std::process::id()));
        let codex = HomeKey(format!("codex:/tmp/gate-test-{}", std::process::id()));
        let held: Vec<HeldHomes> = (0..CLAUDE_HOME_HOLDERS)
            .map(|i| try_acquire_all(&claude, &claude, &format!("%{i}")).expect("room"))
            .collect();
        assert!(try_acquire_all(&claude, &claude, "%9").is_none(), "full");
        // One pane name on two servers is two holders.
        let mut held = held;
        drop(held.remove(0));
        let again = try_acquire_all(&claude, &claude, "%0").expect("a holder left");
        assert!(try_acquire_all(&claude, &claude, "%0").is_none());
        drop(again);
        drop(held);
        let one = try_acquire_all(&codex, &codex, "%1").expect("free");
        assert!(
            try_acquire_all(&codex, &codex, "%2").is_none(),
            "codex: one at a time"
        );
        drop(one);
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
                HomeKey::known("claude", &alias.join("./new-home")),
                HomeKey::known("claude", &real.join("new-home"))
            );
        }
        #[cfg(unix)]
        {
            // `..` after a symlink leaves the symlink's target.
            let deep = base.join("real/sub");
            std::fs::create_dir_all(&deep).unwrap();
            std::fs::create_dir_all(base.join("real/home")).unwrap();
            let link = base.join("link");
            let _ = std::fs::remove_file(&link);
            std::os::unix::fs::symlink(&deep, &link).unwrap();
            assert_eq!(
                HomeKey::known("claude", &link.join("../home")),
                HomeKey::known(
                    "claude",
                    &std::fs::canonicalize(base.join("real/home")).unwrap()
                ),
                "as the kernel resolves it"
            );
            // `..` inside a missing tail is not guessed at.
            assert_eq!(
                HomeKey::known("claude", &real.join("nope/../x")),
                HomeKey::unknown("claude")
            );
            let _ = std::fs::remove_file(&link);
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
