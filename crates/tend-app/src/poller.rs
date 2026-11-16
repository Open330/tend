//! The background read: the decision queue (and whether anything is
//! working, for the tray mark) every [`INTERVAL`] while the dashboard
//! answers, backing off to [`BACKOFF`] while it does not. The tray never
//! shows an unreachable daemon as an empty queue.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::client::{Client, ClientError};
use crate::mark::Rooms;
use crate::queue::{self, Changes, Seen};
use crate::settings;

pub const INTERVAL: Duration = Duration::from_secs(15);
pub const BACKOFF: Duration = Duration::from_secs(60);
const SLICE: Duration = Duration::from_millis(250);

/// One read, for the tray.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Items that need the person, all hosts.
    pub waiting: usize,
    pub working: bool,
    /// Why the queue could not be read; `None` when it was.
    pub problem: Option<String>,
    /// Hosts the queue could not cover.
    pub unavailable: Vec<String>,
    pub changes: Changes,
}

impl Report {
    #[must_use]
    pub fn rooms(&self) -> Rooms {
        Rooms {
            waiting: self.waiting > 0,
            error: self.problem.is_some(),
            working: self.working,
            done: false,
        }
    }

    /// The tray's tooltip (Windows keeps it under 128 characters).
    #[must_use]
    pub fn tooltip(&self) -> String {
        let text = if let Some(problem) = &self.problem {
            format!("Tend: {problem}")
        } else {
            let mut text = match self.waiting {
                0 => "Tend: nothing needs you".to_owned(),
                1 => "Tend: 1 thing needs you".to_owned(),
                n => format!("Tend: {n} things need you"),
            };
            if !self.unavailable.is_empty() {
                text.push_str(" (");
                text.push_str(&self.unavailable.join(", "));
                text.push_str(" unreachable)");
            }
            text
        };
        text.chars().take(127).collect()
    }
}

/// Whether any agent on any host is working, from `/api/fleet`.
#[must_use]
pub fn any_working(fleet: &serde_json::Value) -> bool {
    fleet
        .get("hosts")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|host| {
            host.pointer("/remote/agents")
                .and_then(serde_json::Value::as_array)
        })
        .flatten()
        .any(|agent| agent.get("state").and_then(serde_json::Value::as_str) == Some("working"))
}

/// The keys already told about, from the state file.
#[must_use]
pub fn load_seen(path: &Path) -> Seen {
    let keys: Vec<String> = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| value.get("seen").cloned())
        .and_then(|seen| serde_json::from_value(seen).ok())
        .unwrap_or_default();
    Seen::from_keys(keys)
}

pub fn save_seen(path: &Path, seen: &Seen) {
    let keys: Vec<&String> = seen.keys().collect();
    let body = serde_json::json!({ "version": 1, "seen": keys });
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let temp = path.with_extension("json.tmp");
    if std::fs::write(&temp, body.to_string()).is_ok() {
        let _ = std::fs::rename(&temp, path);
    }
}

/// Reads the queue until `report` returns false. `wake` is checked every
/// quarter second (an "open Home" request from a second instance, say);
/// a jump in wall-clock time (the machine slept) reads at once.
pub struct Poller {
    pub seen_path: PathBuf,
}

impl Poller {
    pub fn run(&self, mut report: impl FnMut(Report) -> bool, mut wake: impl FnMut()) {
        let mut seen = load_seen(&self.seen_path);
        let mut first = true;
        let mut client: Option<Client> = None;
        loop {
            let (next, delay) = Self::read_once(&mut client, &mut seen, first);
            if next.problem.is_none() {
                first = false;
                save_seen(&self.seen_path, &seen);
            }
            if !report(next) {
                return;
            }
            let started = Instant::now();
            let wall = SystemTime::now();
            while started.elapsed() < delay {
                std::thread::sleep(SLICE);
                wake();
                let slept = SystemTime::now().duration_since(wall).unwrap_or_default();
                if slept > started.elapsed() + Duration::from_secs(5) {
                    break; // resumed from sleep
                }
            }
        }
    }

    fn read_once(client: &mut Option<Client>, seen: &mut Seen, first: bool) -> (Report, Duration) {
        let failed = |problem: String| {
            (
                Report {
                    waiting: 0,
                    working: false,
                    problem: Some(problem),
                    unavailable: Vec::new(),
                    changes: Changes::default(),
                },
                BACKOFF,
            )
        };
        if client.is_none() {
            match settings::load() {
                Ok(dashboard) => *client = Some(Client::new(dashboard)),
                Err(error) => return failed(error.to_string()),
            }
        }
        let current = client.as_ref().expect("set above");
        match current.get_json("/api/decisions") {
            Ok(decisions) => {
                let snapshot = queue::snapshot(&decisions);
                let working = current
                    .get_json("/api/fleet")
                    .is_ok_and(|fleet| any_working(&fleet));
                let changes = seen.update(&snapshot, first);
                (
                    Report {
                        waiting: snapshot.items.len(),
                        working,
                        problem: None,
                        unavailable: snapshot.unavailable,
                        changes,
                    },
                    INTERVAL,
                )
            }
            Err(error) => {
                // The token may have changed, or the daemon moved: read the
                // settings again next time.
                *client = None;
                match error {
                    ClientError::Unreachable(_) => {
                        failed("tendd is not running (the dashboard does not answer)".into())
                    }
                    other => failed(other.to_string()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tooltip_says_what_needs_the_person_or_what_is_wrong() {
        let mut report = Report {
            waiting: 2,
            working: true,
            problem: None,
            unavailable: vec!["mini: host is offline".into()],
            changes: Changes::default(),
        };
        assert_eq!(
            report.tooltip(),
            "Tend: 2 things need you (mini: host is offline unreachable)"
        );
        assert_eq!(
            report.rooms(),
            Rooms {
                waiting: true,
                error: false,
                working: true,
                done: false
            }
        );
        report.problem = Some("tendd is not running".into());
        assert_eq!(report.tooltip(), "Tend: tendd is not running");
        assert!(report.rooms().error);
        report.problem = Some("x".repeat(300));
        assert_eq!(report.tooltip().chars().count(), 127);
    }

    #[test]
    fn working_is_read_across_hosts() {
        let fleet = serde_json::json!({"hosts": [
            {"alias": "a", "remote": {"agents": [{"state": "idle"}]}},
            {"alias": "b", "remote": {"agents": [{"state": "working"}]}}
        ]});
        assert!(any_working(&fleet));
        assert!(!any_working(&serde_json::json!({"hosts": []})));
    }

    #[test]
    fn seen_keys_survive_a_restart() {
        let dir =
            std::env::temp_dir().join(format!("tend-app-seen-{}", uuid::Uuid::new_v4().simple()));
        let path = dir.join("app-state.json");
        let snapshot = crate::queue::snapshot(
            &serde_json::json!({"hosts": [{"alias": "h", "queue": {"items": [
                {"kind": "human_request", "request_id": "r1", "body": "?"}
            ]}}]}),
        );
        let mut seen = Seen::default();
        seen.update(&snapshot, true);
        save_seen(&path, &seen);
        let mut loaded = load_seen(&path);
        assert_eq!(loaded.update(&snapshot, true), Changes::default());
        assert_eq!(load_seen(&dir.join("missing.json")), Seen::default());
        let _ = std::fs::remove_dir_all(dir);
    }
}
