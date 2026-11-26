//! When to notify whom: a pure state machine over the decision queue, on
//! an injected clock, so every rule is tested without time passing.
//!
//! - **Readable and complete.** A host's read is readable when it returned
//!   a queue, complete when also not truncated with no source unavailable.
//! - **Baseline per host.** The first readable read of a host records what
//!   is there and announces nothing.
//! - **New.** After that, a key in any readable read that is not known is
//!   new. A key is forgotten only after [`FORGET_AFTER`] absent from
//!   complete reads; time while the host is incomplete or unreadable does
//!   not count.
//! - **Pending per device and host**, at most [`MAX_PENDING`] keys (older
//!   ones dropped and only counted). At most one alert per device every
//!   [`ALERT_EVERY`], one per host per send; only Apple's `200` clears what
//!   an alert announced.
//! - **Badge.** The app icon shows Your turn's number, the one every
//!   client's badge shows (`counts.your_turn`, apps/shared/Design/
//!   COUNTS.md), once the sender has read it; until then, what is waiting
//!   in the queue. When it changes without an alert to carry it, a silent
//!   badge-only update, at most once every [`BADGE_EVERY`] and never while
//!   an alert is pending, until Apple has accepted the current number.
//! - **Back-off** per device on `429`/`5xx`, doubling from
//!   [`BACKOFF_FIRST`] to [`BACKOFF_MAX`], and for the whole sender on
//!   `TooManyProviderTokenUpdates`.
use super::payload::{canonical_line, Content, Kind};
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

pub(super) const FORGET_AFTER: Duration = Duration::from_secs(5 * 60);
pub(super) const MAX_PENDING: usize = 64;
pub(super) const ALERT_EVERY: Duration = Duration::from_secs(30);
pub(super) const BADGE_EVERY: Duration = Duration::from_secs(60);
pub(super) const BACKOFF_FIRST: Duration = Duration::from_secs(30);
pub(super) const BACKOFF_MAX: Duration = Duration::from_secs(10 * 60);

/// A queue entry, keyed by kind and id, with what content would show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Entry {
    pub kind: Kind,
    pub id: String,
    /// The question, the review's summary, or the waiting agent's message.
    pub text: String,
    /// The asking agent's label.
    pub label: String,
    /// When the wait began (unix seconds): orders keys new in one read.
    pub since: i64,
}

/// One host's section of a queue read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HostRead {
    pub host: String,
    pub readable: bool,
    pub complete: bool,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct Key {
    pub kind: Kind,
    pub id: String,
}

/// A registration as the tracker sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Target {
    pub device: String,
    /// The logical registration: the same token, environment and channel
    /// on this device (a fingerprint, never the token itself). A refresh
    /// of the same registration keeps what is pending, the rate and the
    /// back-off; a different one starts over.
    pub registration: String,
    pub generation: i64,
    /// The row's `content` (the host's `allow_content` is passed to
    /// [`Tracker::plan`]).
    pub content: bool,
}

/// One send the tracker wants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Send {
    Alert {
        device: String,
        generation: i64,
        host: String,
        /// What this alert announces: only these are cleared by a `200`.
        keys: Vec<Key>,
        overflow: usize,
        /// Pending keys on that host (those dropped past the cap included).
        count: usize,
        newest: Key,
        content: Option<Content>,
        badge: usize,
    },
    Badge {
        device: String,
        generation: i64,
        badge: usize,
    },
}

impl Send {
    /// The device and the generation this send was planned for.
    pub(super) fn target(&self) -> (&str, i64) {
        match self {
            Send::Alert {
                device, generation, ..
            }
            | Send::Badge {
                device, generation, ..
            } => (device, *generation),
        }
    }
}

fn content_of(hosts: &BTreeMap<String, HostState>, host: &str, newest: &Key) -> Option<Content> {
    hosts
        .get(host)
        .and_then(|h| h.known.get(newest))
        .map(|known| Content {
            kind: known.entry.kind,
            body: canonical_line(&known.entry.text),
            subtitle: known.entry.label.clone(),
        })
}

/// How a send ended, as the tracker needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Result {
    Accepted,
    /// Back off this device (`429`, `5xx`, a timeout, any other refusal).
    Retry,
    /// Back off the whole sender (`TooManyProviderTokenUpdates`).
    BackOffAll,
    /// The registration is gone (`410`, a bad token), or its environment was
    /// turned off: nothing to change here beyond what the next targets say.
    Dropped,
}

#[derive(Debug, Clone)]
struct Known {
    present: bool,
    /// Absent from complete reads this long.
    absent: Duration,
    entry: Entry,
}

#[derive(Debug, Default)]
struct HostState {
    /// The first readable read has been taken.
    baselined: bool,
    known: HashMap<Key, Known>,
    /// When the latest read was taken, if it was complete.
    last_complete: Option<Duration>,
    /// The latest read was complete: a key it lacks is gone.
    complete: bool,
    /// Entries in the latest readable read (kept while unreadable).
    present: usize,
}

#[derive(Debug, Default, Clone, Copy)]
struct Backoff {
    until: Option<Duration>,
    step: Duration,
}

impl Backoff {
    fn active(&self, now: Duration) -> bool {
        self.until.is_some_and(|until| now < until)
    }

    fn bump(&mut self, now: Duration) {
        self.step = if self.step.is_zero() {
            BACKOFF_FIRST
        } else {
            (self.step * 2).min(BACKOFF_MAX)
        };
        self.until = Some(now + self.step);
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

#[derive(Debug, Default)]
struct Pending {
    /// Oldest first.
    keys: Vec<Key>,
    /// Keys dropped past [`MAX_PENDING`], counted only.
    overflow: usize,
}

#[derive(Debug)]
struct DeviceState {
    registration: String,
    generation: i64,
    pending: BTreeMap<String, Pending>,
    last_alert: Option<Duration>,
    last_badge: Option<Duration>,
    /// The last badge Apple accepted. `None` until the first plan, which
    /// takes the current total: a phone registers when the app opens, and
    /// the app sets its badge from its own count then.
    accepted_badge: Option<usize>,
    backoff: Backoff,
}

impl DeviceState {
    fn new(registration: String, generation: i64) -> Self {
        Self {
            registration,
            generation,
            pending: BTreeMap::new(),
            last_alert: None,
            last_badge: None,
            accepted_badge: None,
            backoff: Backoff::default(),
        }
    }

    fn has_pending(&self) -> bool {
        self.pending.values().any(|p| !p.keys.is_empty())
    }
}

#[derive(Debug, Default)]
pub(super) struct Tracker {
    hosts: BTreeMap<String, HostState>,
    devices: BTreeMap<String, DeviceState>,
    backoff: Backoff,
    /// Your turn's number from the last board read (`counts.your_turn`).
    your_turn: Option<usize>,
}

impl Tracker {
    /// Follow the registrations. A device registered again with the same
    /// token, environment and channel (the app does at every launch) keeps
    /// what is pending, its rate and its back-off, and takes the new
    /// generation. A new device, a token that moved, a removal or another
    /// environment starts with nothing pending; a device no longer
    /// registered is dropped.
    fn sync(&mut self, targets: &[Target]) {
        self.devices
            .retain(|device, _| targets.iter().any(|t| &t.device == device));
        for target in targets {
            match self.devices.get_mut(&target.device) {
                Some(state) if state.registration == target.registration => {
                    state.generation = target.generation;
                }
                _ => {
                    self.devices.insert(
                        target.device.clone(),
                        DeviceState::new(target.registration.clone(), target.generation),
                    );
                }
            }
        }
    }

    /// Take one read of the queue at `now`.
    pub(super) fn observe(&mut self, now: Duration, reads: &[HostRead], targets: &[Target]) {
        self.sync(targets);
        for read in reads {
            let host = self.hosts.entry(read.host.clone()).or_default();
            if !read.readable {
                // Absence does not run while a host cannot be read.
                host.last_complete = None;
                host.complete = false;
                continue;
            }
            let baseline = !host.baselined;
            host.baselined = true;
            let mut new: Vec<&Entry> = Vec::new();
            for known in host.known.values_mut() {
                known.present = false;
            }
            for entry in &read.entries {
                let key = Key {
                    kind: entry.kind,
                    id: entry.id.clone(),
                };
                if let Some(known) = host.known.get_mut(&key) {
                    known.present = true;
                    known.absent = Duration::ZERO;
                    known.entry = entry.clone();
                } else {
                    host.known.insert(
                        key,
                        Known {
                            present: true,
                            absent: Duration::ZERO,
                            entry: entry.clone(),
                        },
                    );
                    if !baseline {
                        new.push(entry);
                    }
                }
            }
            // Absence counts only between two complete reads.
            let elapsed = match (read.complete, host.last_complete) {
                (true, Some(at)) => now.saturating_sub(at),
                _ => Duration::ZERO,
            };
            host.known.retain(|_, known| {
                if !known.present {
                    known.absent += elapsed;
                }
                known.present || known.absent < FORGET_AFTER
            });
            host.complete = read.complete;
            host.last_complete = read.complete.then_some(now);
            host.present = read.entries.len();
            // Newest last.
            new.sort_by_key(|entry| entry.since);
            for device in self.devices.values_mut() {
                let pending = device.pending.entry(read.host.clone()).or_default();
                for entry in &new {
                    let key = Key {
                        kind: entry.kind,
                        id: entry.id.clone(),
                    };
                    if !pending.keys.contains(&key) {
                        pending.keys.push(key);
                    }
                }
                if pending.keys.len() > MAX_PENDING {
                    let dropped = pending.keys.len() - MAX_PENDING;
                    pending.keys.drain(..dropped);
                    pending.overflow += dropped;
                }
            }
        }
    }

    /// Your turn's number, as every client's badge says it, from the board
    /// the sender read. A board read that failed keeps the last one.
    pub(super) fn set_your_turn(&mut self, your_turn: Option<usize>) {
        if your_turn.is_some() {
            self.your_turn = your_turn;
        }
    }

    /// The app icon's number: Your turn once known, else what is waiting.
    pub(super) fn badge(&self) -> usize {
        self.your_turn.unwrap_or_else(|| self.total())
    }

    /// What is waiting across hosts: each host's latest readable read.
    pub(super) fn total(&self) -> usize {
        self.hosts.values().map(|host| host.present).sum()
    }

    fn gone(&self, host: &str, key: &Key) -> bool {
        self.hosts.get(host).is_none_or(|state| {
            state
                .known
                .get(key)
                .is_none_or(|known| state.complete && !known.present)
        })
    }

    /// The sends due at `now`. Each pending set is rebuilt first: keys a
    /// complete read shows gone are dropped, the count recomputed, and
    /// consent (`allow_content` and each target's `content`) taken as it is
    /// now.
    pub(super) fn plan(
        &mut self,
        now: Duration,
        targets: &[Target],
        allow_content: bool,
    ) -> Vec<Send> {
        self.sync(targets);
        if self.backoff.active(now) {
            return Vec::new();
        }
        let total = self.badge();
        // Rebuild every pending set against what is known now.
        let gone: Vec<(String, String, Key)> = self
            .devices
            .iter()
            .flat_map(|(device, state)| {
                state.pending.iter().flat_map(move |(host, pending)| {
                    pending
                        .keys
                        .iter()
                        .map(move |key| (device.clone(), host.clone(), key.clone()))
                })
            })
            .filter(|(_, host, key)| self.gone(host, key))
            .collect();
        for (device, host, key) in gone {
            if let Some(pending) = self
                .devices
                .get_mut(&device)
                .and_then(|state| state.pending.get_mut(&host))
            {
                pending.keys.retain(|k| k != &key);
            }
        }
        let mut sends = Vec::new();
        for target in targets {
            let Some(state) = self.devices.get_mut(&target.device) else {
                continue;
            };
            for pending in state.pending.values_mut() {
                if pending.keys.is_empty() {
                    pending.overflow = 0;
                }
            }
            state.pending.retain(|_, pending| !pending.keys.is_empty());
            if state.accepted_badge.is_none() {
                state.accepted_badge = Some(total);
            }
            if state.backoff.active(now) {
                continue;
            }
            if state.has_pending() {
                if state
                    .last_alert
                    .is_some_and(|at| now.saturating_sub(at) < ALERT_EVERY)
                {
                    continue;
                }
                state.last_alert = Some(now);
                for (host, pending) in &state.pending {
                    let newest = pending.keys.last().expect("non-empty").clone();
                    let content = (allow_content && target.content)
                        .then(|| content_of(&self.hosts, host, &newest))
                        .flatten();
                    sends.push(Send::Alert {
                        device: target.device.clone(),
                        generation: state.generation,
                        host: host.clone(),
                        keys: pending.keys.clone(),
                        overflow: pending.overflow,
                        count: pending.keys.len() + pending.overflow,
                        newest,
                        content,
                        badge: total,
                    });
                }
            } else if state.accepted_badge != Some(total)
                && state
                    .last_badge
                    .is_none_or(|at| now.saturating_sub(at) >= BADGE_EVERY)
            {
                state.last_badge = Some(now);
                sends.push(Send::Badge {
                    device: target.device.clone(),
                    generation: state.generation,
                    badge: total,
                });
            }
        }
        sends
    }

    /// What content the alert for `newest` on `host` would show, as the
    /// queue is known now (the caller decides consent).
    pub(super) fn content(&self, host: &str, newest: &Key) -> Option<Content> {
        content_of(&self.hosts, host, newest)
    }

    /// Whether a planned send may still go out at `now`: its device still
    /// holds the generation it was planned for, and neither that device
    /// nor the whole sender has backed off since (an earlier send in the
    /// same tick may have).
    pub(super) fn still_due(&self, now: Duration, send: &Send) -> bool {
        let (device, generation) = send.target();
        !self.backoff.active(now)
            && self
                .devices
                .get(device)
                .is_some_and(|state| state.generation == generation && !state.backoff.active(now))
    }

    /// How a planned send ended.
    pub(super) fn record(&mut self, now: Duration, send: &Send, result: Result) {
        let (device, generation) = send.target();
        if result == Result::BackOffAll {
            self.backoff.bump(now);
        } else if result == Result::Accepted {
            self.backoff.reset();
        }
        let Some(state) = self
            .devices
            .get_mut(device)
            .filter(|state| state.generation == generation)
        else {
            return;
        };
        match result {
            Result::Accepted => {
                state.backoff.reset();
                match send {
                    Send::Alert {
                        host,
                        keys,
                        overflow,
                        badge,
                        ..
                    } => {
                        if let Some(pending) = state.pending.get_mut(host) {
                            pending.keys.retain(|key| !keys.contains(key));
                            pending.overflow = pending.overflow.saturating_sub(*overflow);
                        }
                        state.accepted_badge = Some(*badge);
                    }
                    Send::Badge { badge, .. } => state.accepted_badge = Some(*badge),
                }
            }
            Result::Retry => state.backoff.bump(now),
            Result::BackOffAll | Result::Dropped => {}
        }
    }

    /// Whether this device has an alert pending (tests).
    #[cfg(test)]
    pub(super) fn pending(&self, device: &str) -> Vec<(String, usize)> {
        self.devices.get(device).map_or_else(Vec::new, |state| {
            state
                .pending
                .iter()
                .filter(|(_, p)| !p.keys.is_empty())
                .map(|(host, p)| (host.clone(), p.keys.len() + p.overflow))
                .collect()
        })
    }

    /// Hosts and the keys known on each (tests).
    #[cfg(test)]
    pub(super) fn knows(&self, host: &str, kind: Kind, id: &str) -> bool {
        self.hosts.get(host).is_some_and(|state| {
            state.known.contains_key(&Key {
                kind,
                id: id.to_owned(),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(secs: u64) -> Duration {
        Duration::from_secs(secs)
    }

    fn entry(kind: Kind, id: &str) -> Entry {
        Entry {
            kind,
            id: id.into(),
            text: format!("text of {id}"),
            label: "claude".into(),
            since: 0,
        }
    }

    fn read(host: &str, ids: &[&str]) -> HostRead {
        HostRead {
            host: host.into(),
            readable: true,
            complete: true,
            entries: ids.iter().map(|id| entry(Kind::Request, id)).collect(),
        }
    }

    fn partial(host: &str, ids: &[&str]) -> HostRead {
        HostRead {
            complete: false,
            ..read(host, ids)
        }
    }

    fn unreadable(host: &str) -> HostRead {
        HostRead {
            host: host.into(),
            readable: false,
            complete: false,
            entries: Vec::new(),
        }
    }

    fn phone() -> Vec<Target> {
        vec![Target {
            device: "dev_a".into(),
            registration: "reg_a".into(),
            generation: 1,
            content: false,
        }]
    }

    fn alerts(sends: &[Send]) -> Vec<(String, usize, String)> {
        sends
            .iter()
            .filter_map(|send| match send {
                Send::Alert {
                    host,
                    count,
                    newest,
                    ..
                } => Some((host.clone(), *count, newest.id.clone())),
                Send::Badge { .. } => None,
            })
            .collect()
    }

    fn badges(sends: &[Send]) -> Vec<usize> {
        sends
            .iter()
            .filter_map(|send| match send {
                Send::Badge { badge, .. } => Some(*badge),
                Send::Alert { .. } => None,
            })
            .collect()
    }

    fn accept_all(tracker: &mut Tracker, now: Duration, sends: &[Send]) {
        for send in sends {
            tracker.record(now, send, Result::Accepted);
        }
    }

    #[test]
    fn the_baseline_is_per_host_including_a_host_unavailable_at_start() {
        let mut t = Tracker::default();
        let targets = phone();
        t.observe(s(0), &[read("mini", &["a"]), unreadable("mbp")], &targets);
        assert!(
            t.plan(s(0), &targets, false).is_empty(),
            "nothing announced"
        );
        assert_eq!(t.total(), 1);
        // mbp becomes readable: its first read is its baseline.
        t.observe(
            s(15),
            &[read("mini", &["a"]), read("mbp", &["x", "y"])],
            &targets,
        );
        let sends = t.plan(s(15), &targets, false);
        assert!(alerts(&sends).is_empty());
        // The badge still follows the totals.
        assert_eq!(badges(&sends), vec![3]);
        // Then a new one on mbp alerts.
        t.observe(
            s(30),
            &[read("mini", &["a"]), read("mbp", &["x", "y", "z"])],
            &targets,
        );
        assert_eq!(
            alerts(&t.plan(s(30), &targets, false)),
            vec![("mbp".into(), 1, "z".into())]
        );
    }

    #[test]
    fn new_items_alert_even_in_a_read_that_stays_truncated() {
        let mut t = Tracker::default();
        let targets = phone();
        t.observe(s(0), &[partial("mini", &["a"])], &targets);
        t.observe(s(15), &[partial("mini", &["a", "b"])], &targets);
        let sends = t.plan(s(15), &targets, false);
        assert_eq!(alerts(&sends), vec![("mini".into(), 1, "b".into())]);
        accept_all(&mut t, s(15), &sends);
        t.observe(s(60), &[partial("mini", &["a", "b", "c"])], &targets);
        assert_eq!(
            alerts(&t.plan(s(60), &targets, false)),
            vec![("mini".into(), 1, "c".into())]
        );
    }

    #[test]
    fn absence_counts_only_across_complete_reads_and_a_flapping_agent_is_announced_once() {
        let mut t = Tracker::default();
        let targets = phone();
        t.observe(s(0), &[read("mini", &[])], &targets);
        t.observe(s(15), &[read("mini", &["w"])], &targets);
        let sends = t.plan(s(15), &targets, false);
        assert_eq!(alerts(&sends).len(), 1);
        accept_all(&mut t, s(15), &sends);
        // Flapping: absent, back, absent, back, within 5 minutes.
        for (at, ids) in [
            (30, &[][..]),
            (45, &["w"][..]),
            (60, &[][..]),
            (75, &["w"][..]),
        ] {
            t.observe(s(at), &[read("mini", ids)], &targets);
            assert!(
                alerts(&t.plan(s(at), &targets, false)).is_empty(),
                "at {at}"
            );
        }
        // Absent from complete reads, but with an incomplete stretch of 10
        // minutes in between: that stretch does not count.
        t.observe(s(90), &[read("mini", &[])], &targets);
        t.observe(s(200), &[read("mini", &[])], &targets);
        t.observe(s(215), &[partial("mini", &[])], &targets);
        t.observe(s(815), &[unreadable("mini")], &targets);
        t.observe(s(830), &[read("mini", &[])], &targets);
        t.observe(s(900), &[read("mini", &[])], &targets);
        // 15 + 110 + 70 seconds absent across complete reads: still known.
        assert!(t.knows("mini", Kind::Request, "w"));
        t.observe(s(1020), &[read("mini", &[])], &targets);
        // 300 seconds now: forgotten, so its return is new.
        assert!(!t.knows("mini", Kind::Request, "w"));
        t.observe(s(1035), &[read("mini", &["w"])], &targets);
        assert_eq!(alerts(&t.plan(s(1035), &targets, false)).len(), 1);
    }

    #[test]
    fn one_alert_per_host_per_send_and_the_30_second_rate() {
        let mut t = Tracker::default();
        let targets = phone();
        t.observe(s(0), &[read("mini", &[]), read("mbp", &[])], &targets);
        t.observe(
            s(15),
            &[read("mini", &["a", "b"]), read("mbp", &["c"])],
            &targets,
        );
        let sends = t.plan(s(15), &targets, false);
        let mut got = alerts(&sends);
        got.sort();
        assert_eq!(
            got,
            vec![
                ("mbp".into(), 1, "c".into()),
                ("mini".into(), 2, "b".into())
            ]
        );
        accept_all(&mut t, s(15), &sends);
        t.observe(
            s(30),
            &[read("mini", &["a", "b", "d"]), read("mbp", &["c"])],
            &targets,
        );
        // 15 s since the last alert: rate-limited, and kept.
        assert!(t.plan(s(30), &targets, false).is_empty());
        assert_eq!(t.pending("dev_a"), vec![("mini".into(), 1)]);
        t.observe(
            s(45),
            &[read("mini", &["a", "b", "d"]), read("mbp", &["c"])],
            &targets,
        );
        assert_eq!(
            alerts(&t.plan(s(45), &targets, false)),
            vec![("mini".into(), 1, "d".into())]
        );
    }

    #[test]
    fn pending_survives_a_429_is_rebuilt_after_the_back_off_and_drops_what_was_decided() {
        let mut t = Tracker::default();
        let targets = phone();
        t.observe(s(0), &[read("mini", &[])], &targets);
        t.observe(s(15), &[read("mini", &["a", "b"])], &targets);
        let sends = t.plan(s(15), &targets, false);
        assert_eq!(alerts(&sends), vec![("mini".into(), 2, "b".into())]);
        t.record(s(15), &sends[0], Result::Retry);
        // Backing off 30 s.
        t.observe(s(30), &[read("mini", &["a", "b", "c"])], &targets);
        assert!(t.plan(s(30), &targets, false).is_empty());
        // `b` decided meanwhile: gone from a complete read.
        t.observe(s(45), &[read("mini", &["a", "c"])], &targets);
        let sends = t.plan(s(45), &targets, false);
        assert_eq!(alerts(&sends), vec![("mini".into(), 2, "c".into())]);
        // A second failure doubles the back-off to 60 s.
        t.record(s(45), &sends[0], Result::Retry);
        t.observe(s(90), &[read("mini", &["a", "c"])], &targets);
        assert!(t.plan(s(90), &targets, false).is_empty());
        t.observe(s(105), &[read("mini", &["a", "c"])], &targets);
        let sends = t.plan(s(105), &targets, false);
        assert_eq!(alerts(&sends).len(), 1);
        accept_all(&mut t, s(105), &sends);
        assert!(t.pending("dev_a").is_empty());
    }

    #[test]
    fn the_back_off_doubles_to_ten_minutes() {
        let mut backoff = Backoff::default();
        let mut steps = Vec::new();
        for _ in 0..7 {
            backoff.bump(s(0));
            steps.push(backoff.step.as_secs());
        }
        assert_eq!(steps, vec![30, 60, 120, 240, 480, 600, 600]);
    }

    #[test]
    fn an_incomplete_read_never_takes_an_item_as_decided() {
        let mut t = Tracker::default();
        let targets = phone();
        t.observe(s(0), &[read("mini", &[])], &targets);
        t.observe(s(15), &[read("mini", &["a"])], &targets);
        let sends = t.plan(s(15), &targets, false);
        t.record(s(15), &sends[0], Result::Retry);
        // Cut off by truncation: still pending after the back-off.
        t.observe(s(50), &[partial("mini", &[])], &targets);
        assert_eq!(
            alerts(&t.plan(s(50), &targets, false)),
            vec![("mini".into(), 1, "a".into())]
        );
    }

    /// The icon says Your turn's number, as every client's badge does: a
    /// question nobody can answer is in the queue but not in Your turn, a
    /// waiting Work unit is in Your turn. A silent badge-only update when
    /// it changes; a board read that failed keeps the last number.
    #[test]
    fn the_badge_is_your_turn_once_the_board_says_it() {
        let mut t = Tracker::default();
        let targets = phone();
        t.observe(s(0), &[read("mini", &["a", "b", "c"])], &targets);
        assert_eq!(t.badge(), 3, "before any board read: what is waiting");
        t.set_your_turn(Some(2));
        assert_eq!(t.badge(), 2);
        assert!(
            t.plan(s(0), &targets, false).is_empty(),
            "the phone set its own"
        );
        // Your turn moves with nothing new in the queue: a silent update.
        t.set_your_turn(Some(1));
        t.observe(s(15), &[read("mini", &["a", "b", "c"])], &targets);
        let sends = t.plan(s(15), &targets, false);
        assert_eq!(badges(&sends), vec![1]);
        assert!(alerts(&sends).is_empty());
        t.record(s(15), &sends[0], Result::Accepted);
        // A failed board read changes nothing.
        t.set_your_turn(None);
        t.observe(s(90), &[read("mini", &["a", "b", "c"])], &targets);
        assert!(t.plan(s(90), &targets, false).is_empty());
        assert_eq!(t.badge(), 1);
    }

    #[test]
    fn the_badge_goes_down_to_zero_waits_for_alerts_and_is_retried_until_accepted() {
        let mut t = Tracker::default();
        let targets = phone();
        t.observe(s(0), &[read("mini", &["a", "b"])], &targets);
        assert!(
            t.plan(s(0), &targets, false).is_empty(),
            "the phone set 2 itself"
        );
        t.observe(s(15), &[read("mini", &["b"])], &targets);
        let sends = t.plan(s(15), &targets, false);
        assert_eq!(badges(&sends), vec![1]);
        t.record(s(15), &sends[0], Result::Accepted);
        // A new item while the badge would go down: the alert carries it,
        // and no badge update is sent while it is pending.
        t.observe(s(80), &[read("mini", &["c"])], &targets);
        let sends = t.plan(s(80), &targets, false);
        assert_eq!(alerts(&sends), vec![("mini".into(), 1, "c".into())]);
        assert!(badges(&sends).is_empty());
        t.record(s(80), &sends[0], Result::Retry);
        t.observe(s(95), &[read("mini", &[])], &targets);
        // `c` decided: gone. Backing off until 110.
        assert!(t.plan(s(95), &targets, false).is_empty());
        t.observe(s(110), &[read("mini", &[])], &targets);
        let sends = t.plan(s(110), &targets, false);
        assert_eq!(badges(&sends), vec![0]);
        // Refused: retried at the next allowed tick (a minute on, once the
        // back-off has run), until accepted.
        t.record(s(110), &sends[0], Result::Retry);
        assert!(t.plan(s(125), &targets, false).is_empty());
        let sends = t.plan(s(170), &targets, false);
        assert_eq!(badges(&sends), vec![0]);
        t.record(s(170), &sends[0], Result::Accepted);
        assert!(t.plan(s(300), &targets, false).is_empty());
    }

    #[test]
    fn badge_updates_are_at_most_once_a_minute() {
        let mut t = Tracker::default();
        let targets = phone();
        t.observe(s(0), &[read("mini", &["a", "b", "c"])], &targets);
        t.plan(s(0), &targets, false);
        t.observe(s(15), &[read("mini", &["a", "b"])], &targets);
        let sends = t.plan(s(15), &targets, false);
        assert_eq!(badges(&sends), vec![2]);
        t.record(s(15), &sends[0], Result::Accepted);
        t.observe(s(30), &[read("mini", &["a"])], &targets);
        assert!(t.plan(s(30), &targets, false).is_empty());
        t.observe(s(75), &[read("mini", &["a"])], &targets);
        assert_eq!(badges(&t.plan(s(75), &targets, false)), vec![1]);
    }

    #[test]
    fn the_pending_set_is_capped_at_64_per_host_and_counts_the_rest() {
        let mut t = Tracker::default();
        let targets = phone();
        t.observe(s(0), &[read("mini", &[])], &targets);
        let ids: Vec<String> = (0..70).map(|i| format!("r{i:02}")).collect();
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let mut many = read("mini", &refs);
        for (i, entry) in many.entries.iter_mut().enumerate() {
            entry.since = i64::try_from(i).unwrap();
        }
        t.observe(s(15), &[many], &targets);
        let sends = t.plan(s(15), &targets, false);
        let Send::Alert {
            keys,
            count,
            newest,
            ..
        } = &sends[0]
        else {
            panic!("an alert")
        };
        assert_eq!(keys.len(), MAX_PENDING);
        assert_eq!(*count, 70);
        assert_eq!(newest.id, "r69");
    }

    #[test]
    fn content_needs_both_flags_read_again_at_send_time() {
        let mut t = Tracker::default();
        let mut targets = phone();
        t.observe(s(0), &[read("mini", &[])], &targets);
        let mut with_text = read("mini", &["a"]);
        with_text.entries[0].text = "## Ship *it*?\nmore".into();
        t.observe(s(15), &[with_text], &targets);
        let content = |sends: &[Send]| match &sends[0] {
            Send::Alert { content, .. } => content.clone(),
            Send::Badge { .. } => panic!("an alert"),
        };
        // The host allows it, the phone did not ask.
        let sends = t.plan(s(15), &targets, true);
        assert_eq!(content(&sends), None);
        t.record(s(15), &sends[0], Result::Retry);
        // The phone asks now (same generation here), the host does not.
        targets[0].content = true;
        let sends = t.plan(s(50), &targets, false);
        assert_eq!(content(&sends), None);
        t.record(s(50), &sends[0], Result::Retry);
        let sends = t.plan(s(200), &targets, true);
        assert_eq!(
            content(&sends),
            Some(Content {
                kind: Kind::Request,
                body: "Ship it?".into(),
                subtitle: "claude".into(),
            })
        );
    }

    #[test]
    fn a_new_registration_starts_with_nothing_pending() {
        let mut t = Tracker::default();
        let mut targets = phone();
        t.observe(s(0), &[read("mini", &[])], &targets);
        t.observe(s(15), &[read("mini", &["a"])], &targets);
        assert_eq!(t.pending("dev_a"), vec![("mini".into(), 1)]);
        // Another token, environment or channel: a new registration.
        targets[0].registration = "reg_a2".into();
        targets[0].generation = 2;
        t.observe(s(30), &[read("mini", &["a"])], &targets);
        assert!(t.plan(s(30), &targets, false).is_empty());
        // A late answer to the old generation changes nothing.
        let old = Send::Badge {
            device: "dev_a".into(),
            generation: 1,
            badge: 0,
        };
        t.record(s(30), &old, Result::Retry);
        t.observe(s(45), &[read("mini", &["a", "b"])], &targets);
        assert_eq!(alerts(&t.plan(s(45), &targets, false)).len(), 1);
    }

    #[test]
    fn a_refresh_of_the_same_registration_keeps_pending_rate_and_back_off() {
        let mut t = Tracker::default();
        let mut targets = phone();
        t.observe(s(0), &[read("mini", &[])], &targets);
        t.observe(s(15), &[read("mini", &["a"])], &targets);
        let sends = t.plan(s(15), &targets, false);
        assert_eq!(alerts(&sends).len(), 1);
        t.record(s(15), &sends[0], Result::Retry);
        // The app launches again: same token, environment and channel, a
        // new generation, and consent read again.
        targets[0].generation = 2;
        targets[0].content = true;
        t.observe(s(30), &[read("mini", &["a"])], &targets);
        assert_eq!(t.pending("dev_a"), vec![("mini".into(), 1)]);
        assert!(t.plan(s(30), &targets, true).is_empty(), "still backed off");
        assert!(!t.still_due(s(30), &sends[0]), "planned for generation 1");
        t.observe(s(45), &[read("mini", &["a"])], &targets);
        let sends = t.plan(s(45), &targets, true);
        match sends.as_slice() {
            [Send::Alert {
                generation,
                content: Some(_),
                ..
            }] => assert_eq!(*generation, 2),
            other => panic!("{other:?}"),
        }
        assert!(t.still_due(s(45), &sends[0]));
        // A late answer to generation 1 changes nothing now.
        let old = Send::Badge {
            device: "dev_a".into(),
            generation: 1,
            badge: 0,
        };
        t.record(s(45), &old, Result::Retry);
        assert!(t.still_due(s(45), &sends[0]));
    }

    #[test]
    fn too_many_token_updates_backs_off_every_device() {
        let mut t = Tracker::default();
        let targets = vec![
            Target {
                device: "dev_a".into(),
                registration: "reg_a".into(),
                generation: 1,
                content: false,
            },
            Target {
                device: "dev_b".into(),
                registration: "reg_b".into(),
                generation: 1,
                content: false,
            },
        ];
        t.observe(s(0), &[read("mini", &[])], &targets);
        t.observe(s(15), &[read("mini", &["a"])], &targets);
        let sends = t.plan(s(15), &targets, false);
        assert_eq!(sends.len(), 2);
        t.record(s(15), &sends[0], Result::BackOffAll);
        assert!(t.plan(s(40), &targets, false).is_empty());
        assert_eq!(t.plan(s(46), &targets, false).len(), 2);
    }
}
