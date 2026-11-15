//! The decision queue as the tray sees it: what needs the person, per
//! host, and what changed since the last read, for toasts.
//!
//! An item's key is host + kind + id (+ a review item's generation), so the
//! same request id on two hosts is two items, and a result filed again is
//! new. Toast rules: one toast per new item; at start, or when several
//! arrive at once, one summary toast instead; an item that left the queue
//! (decided here, on the phone, on another host) takes its toast with it.

use serde_json::Value;
use std::collections::BTreeSet;

/// At most this many separate toasts per read; more become one summary.
pub const TOAST_BURST: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Question,
    Review,
    Waiting,
}

/// One thing that needs the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub key: String,
    pub host: String,
    /// The id Home links to (`#item=<id>`).
    pub id: String,
    pub kind: Kind,
    pub title: String,
}

impl Item {
    #[must_use]
    pub fn heading(&self) -> &'static str {
        match self.kind {
            Kind::Question => "An agent asks you",
            Kind::Review => "A result to review",
            Kind::Waiting => "An agent is waiting",
        }
    }
}

/// The queue read once: its items and the hosts that could not be read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub items: Vec<Item>,
    /// `host: why` for each host the queue could not cover.
    pub unavailable: Vec<String>,
    /// Hosts read in full: listed, nothing unavailable, not truncated. Only
    /// for these does a missing item mean it was decided.
    pub covered: BTreeSet<String>,
}

fn first_line(text: &str, limit: usize) -> String {
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.chars().count() <= limit {
        line.to_owned()
    } else {
        let mut cut: String = line.chars().take(limit.saturating_sub(1)).collect();
        cut.push('…');
        cut
    }
}

/// Read `/api/decisions` (the per-host shape; an older single-queue answer
/// reads as one local host).
#[must_use]
pub fn snapshot(decisions: &Value) -> Snapshot {
    let hosts: Vec<Value> = match decisions.get("hosts").and_then(Value::as_array) {
        Some(hosts) => hosts.clone(),
        None => vec![serde_json::json!({"alias": "local", "queue": decisions.get("queue")})],
    };
    let mut out = Snapshot::default();
    for host in &hosts {
        let alias = host
            .get("alias")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_owned();
        for why in host
            .get("unavailable")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            out.unavailable.push(format!("{alias}: {why}"));
        }
        let partial = host
            .get("unavailable")
            .and_then(Value::as_array)
            .is_some_and(|reasons| !reasons.is_empty())
            || host
                .pointer("/queue/truncated")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            || host.pointer("/queue/items").is_none();
        if !partial {
            out.covered.insert(alias.clone());
        }
        let items = host
            .pointer("/queue/items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten();
        for item in items {
            let text = |key: &str| item.get(key).and_then(Value::as_str).unwrap_or("");
            let (kind, id, title, generation) = match text("kind") {
                "human_request" => (Kind::Question, text("request_id"), text("body"), None),
                "review" => (
                    Kind::Review,
                    text("id"),
                    text("summary"),
                    item.get("generation").and_then(Value::as_u64),
                ),
                "agent_waiting" => {
                    let message = text("message");
                    let title = if message.is_empty() {
                        "waiting in its pane"
                    } else {
                        message
                    };
                    (Kind::Waiting, text("session_id"), title, None)
                }
                _ => continue,
            };
            if id.is_empty() {
                continue;
            }
            let kind_tag = match kind {
                Kind::Question => "q",
                Kind::Review => "r",
                Kind::Waiting => "w",
            };
            let key = match generation {
                Some(generation) => format!("{alias}|{kind_tag}|{id}|{generation}"),
                None => format!("{alias}|{kind_tag}|{id}"),
            };
            out.items.push(Item {
                key,
                host: alias.clone(),
                id: id.to_owned(),
                kind,
                title: first_line(title, 200),
            });
        }
    }
    out
}

/// What to tell the person after one read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Toast {
    One(Item),
    /// Several new items at once: one toast naming the count.
    Summary(usize),
}

/// The changes since the previous read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    pub toasts: Vec<Toast>,
    /// Keys of items that left the queue: their toasts are withdrawn.
    pub gone: Vec<String>,
}

/// The keys already told about, kept across restarts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Seen {
    keys: BTreeSet<String>,
}

impl Seen {
    #[must_use]
    pub fn from_keys(keys: impl IntoIterator<Item = String>) -> Self {
        Self {
            keys: keys.into_iter().collect(),
        }
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.keys.iter()
    }

    /// Compare with a fresh read and remember it. `first` is the first read
    /// since the app started: what is new then is summarised, never one
    /// toast per item.
    pub fn update(&mut self, current: &Snapshot, first: bool) -> Changes {
        let now: BTreeSet<String> = current.items.iter().map(|i| i.key.clone()).collect();
        let fresh: Vec<&Item> = current
            .items
            .iter()
            .filter(|i| !self.keys.contains(&i.key))
            .collect();
        // Gone only where the read was complete: a host that could not be
        // read, or a truncated queue, keeps what was known about it.
        let host_of = |key: &str| key.split('|').next().unwrap_or("").to_owned();
        let (gone, kept): (Vec<String>, Vec<String>) = self
            .keys
            .difference(&now)
            .cloned()
            .partition(|key| current.covered.contains(&host_of(key)));
        let toasts = match fresh.len() {
            0 => Vec::new(),
            1 if !first => vec![Toast::One(fresh[0].clone())],
            n if !first && n <= TOAST_BURST => {
                fresh.iter().map(|i| Toast::One((*i).clone())).collect()
            }
            1 => vec![Toast::One(fresh[0].clone())],
            n => vec![Toast::Summary(n)],
        };
        self.keys = now;
        self.keys.extend(kept);
        Changes { toasts, gone }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn decisions(items: Value) -> Value {
        json!({"hosts": [
            {"alias": "home", "local": true, "queue": {"items": items}, "unavailable": []},
            {"alias": "mini", "local": false, "queue": {"items": [
                {"kind": "human_request", "request_id": "rq_1", "body": "Merge #12?\nmore"}
            ]}, "unavailable": ["host is offline"]}
        ]})
    }

    #[test]
    fn the_queue_reads_every_hosts_items_with_host_scoped_keys() {
        let snap = snapshot(&decisions(json!([
            {"kind": "human_request", "request_id": "rq_1", "body": "Ship it?"},
            {"kind": "review", "id": "rv_1", "summary": "Phase 1", "generation": 2},
            {"kind": "agent_waiting", "session_id": "s1", "message": ""},
            {"kind": "something_new", "id": "x"},
            {"kind": "review", "summary": "no id"}
        ])));
        let keys: Vec<&str> = snap.items.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(
            keys,
            ["home|q|rq_1", "home|r|rv_1|2", "home|w|s1", "mini|q|rq_1"]
        );
        assert_eq!(snap.items[2].title, "waiting in its pane");
        assert_eq!(snap.items[3].title, "Merge #12?");
        assert_eq!(snap.unavailable, ["mini: host is offline"]);
        let old =
            snapshot(&json!({"queue": {"items": [{"kind": "review", "id": "a", "summary": "s"}]}}));
        assert_eq!(old.items[0].key, "local|r|a");
    }

    #[test]
    fn long_titles_are_cut_on_a_character_boundary() {
        let long = "é".repeat(300);
        let snap = snapshot(&json!({"hosts": [{"alias": "h", "queue": {"items": [
            {"kind": "human_request", "request_id": "r", "body": long}
        ]}}]}));
        assert_eq!(snap.items[0].title.chars().count(), 200);
        assert!(snap.items[0].title.ends_with('…'));
    }

    #[test]
    fn toasts_follow_the_rules() {
        let item = |id: &str| json!({"kind": "human_request", "request_id": id, "body": id});
        let read = |ids: &[&str]| {
            snapshot(
                &json!({"hosts": [{"alias": "h", "queue": {"items": ids.iter().map(|id| item(id)).collect::<Vec<_>>()}}]}),
            )
        };
        let mut seen = Seen::default();
        // At start, several waiting items are one summary.
        let start = seen.update(&read(&["a", "b"]), true);
        assert_eq!(start.toasts, vec![Toast::Summary(2)]);
        // Nothing new: nothing said.
        assert_eq!(seen.update(&read(&["a", "b"]), false), Changes::default());
        // One new item: one toast; one decided: its toast goes.
        let next = seen.update(&read(&["b", "c"]), false);
        assert!(matches!(&next.toasts[..], [Toast::One(i)] if i.id == "c"));
        assert_eq!(next.gone, ["h|q|a"]);
        // A burst beyond the limit is summarised.
        let burst = seen.update(&read(&["b", "c", "d", "e", "f", "g"]), false);
        assert_eq!(burst.toasts, vec![Toast::Summary(4)]);
        // A host that could not be read keeps its items (and toasts).
        let partial = snapshot(
            &json!({"hosts": [{"alias": "h", "queue": {"items": []}, "unavailable": ["host is offline"]}]}),
        );
        let offline = seen.update(&partial, false);
        assert_eq!(
            offline,
            Changes::default(),
            "nothing withdrawn while h is unreadable"
        );
        let truncated = snapshot(
            &json!({"hosts": [{"alias": "h", "queue": {"items": [], "truncated": true}}]}),
        );
        assert!(seen.update(&truncated, false).gone.is_empty());
        // Kept across a restart: the same items are not told again.
        let mut again = Seen::from_keys(seen.keys().cloned());
        assert_eq!(
            again.update(&read(&["b", "c", "d", "e", "f", "g"]), true),
            Changes::default()
        );
        // A single new item at start is told as itself.
        let mut fresh = Seen::default();
        assert!(matches!(
            &fresh.update(&read(&["z"]), true).toasts[..],
            [Toast::One(_)]
        ));
    }
}
