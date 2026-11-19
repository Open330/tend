//! What tendd observes about agent sessions that a question's subject check
//! (`collaboration::subject_check`) may later count as the session being
//! gone.
//!
//! Two kinds of input, both recorded as they happen and never reconstructed
//! from a row read back later:
//!
//! - **Facts** ([`SubjectFact`]) written when a hook is ingested: the
//!   provider's own `SessionEnded` (E1), and a real provider `Started` of a
//!   *different* session in a pane, on the same server incarnation, written
//!   before the predecessor's row is flipped or evicted (E2). The reconcile
//!   sweeps (orphan, paneless, cross-host, pid-exit) write none. The
//!   collaboration store keeps them durably for [`FACT_RETENTION`].
//! - **The latest pane scan** ([`PaneScan`]): per backend, whether the scan
//!   was complete, which sockets answered and with what server incarnation,
//!   and for each pane its foreground command and whether the workload walk
//!   found a primary agent process in it. Only the newest is kept, in memory.
//!
//! A server incarnation is the multiplexer server's pid and start time
//! (`#{pid}`, `#{start_time}`), the same identity a launch receipt records
//! (`launch_identity.rs`). Pane ids are unique only within one incarnation.

use crate::backend::HostKind;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// How long an evidence fact is kept. Longer than any grace it feeds.
pub const FACT_RETENTION: time::Duration = time::Duration::days(30);

/// A multiplexer server's identity: its pid and start time. Two different
/// values for one socket mean the server was restarted in between, and a
/// pane id seen before then names nothing now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ServerIncarnation {
    pub pid: u32,
    pub start_time: u64,
}

impl ServerIncarnation {
    /// From the `#{pid}` and `#{start_time}` columns; `None` when either is
    /// missing or not a number (a backend that does not expand them).
    #[must_use]
    pub fn parse(pid: Option<&str>, start_time: Option<&str>) -> Option<Self> {
        Some(Self {
            pid: pid?.trim().parse().ok()?,
            start_time: start_time?.trim().parse().ok()?,
        })
    }
}

/// One socket a scan asked and got an answer from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerAnswer {
    /// The normalized endpoint, as pane rows carry it (`PaneInfo::socket`).
    pub endpoint: String,
    /// `false` when the socket answered that no server runs there: every
    /// pane it ever had is gone.
    pub running: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incarnation: Option<ServerIncarnation>,
}

/// One pane in a scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScannedPane {
    pub pane_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// The multiplexer's own foreground command for the pane.
    pub foreground: String,
    /// Whether the workload walk found a primary agent process in the pane;
    /// `None` when the walk did not cover it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_process: Option<bool>,
}

/// One backend's part of a scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostScan {
    pub kind: HostKind,
    /// Every socket of the backend answered. Partial backends (herdr, cmux,
    /// zellij) and failed scans are never complete, so they never show
    /// absence.
    pub complete: bool,
    pub servers: Vec<ServerAnswer>,
    pub panes: Vec<ScannedPane>,
}

/// The newest reconcile scan, as the subject check reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneScan {
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    pub hosts: Vec<HostScan>,
}

impl PaneScan {
    /// The complete scan of `kind`, if this pass had one.
    #[must_use]
    pub fn complete_host(&self, kind: HostKind) -> Option<&HostScan> {
        self.hosts
            .iter()
            .find(|host| host.kind == kind && host.complete)
    }

    /// The server incarnation a socket answered with in this scan.
    #[must_use]
    pub fn incarnation(&self, kind: HostKind, endpoint: &str) -> Option<ServerIncarnation> {
        self.hosts
            .iter()
            .filter(|host| host.kind == kind)
            .flat_map(|host| &host.servers)
            .find(|server| server.endpoint == endpoint)
            .and_then(|server| server.incarnation)
    }
}

/// Where a session was last seen bound, and on which server incarnation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubjectBinding {
    pub pane: String,
    pub endpoint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incarnation: Option<ServerIncarnation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactKind {
    /// E1: the provider's own `SessionEnded` hook for the session.
    Ended,
    /// E2: a real provider `Started` hook for another session in the
    /// session's pane.
    Replaced,
}

/// One evidence fact, keyed by host, backend, endpoint, pane, session and,
/// when known, the server incarnation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubjectFact {
    pub id: String,
    pub kind: FactKind,
    /// Always this host: facts are only ever recorded about sessions tendd
    /// itself observes.
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<HostKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incarnation: Option<ServerIncarnation>,
    /// For [`FactKind::Replaced`]: the predecessor's binding was on the
    /// server incarnation the newest scan reports for its socket. A
    /// replacement fact without it never counts.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub same_incarnation: bool,
    /// For [`FactKind::Replaced`]: the session that started in the pane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaced_by: Option<String>,
    /// When tendd ingested the hook.
    #[serde(with = "time::serde::rfc3339")]
    pub observed_at: OffsetDateTime,
    /// What recorded it: `session_ended_hook` or `started_hook`.
    pub source: String,
}

impl SubjectFact {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: FactKind,
        session_id: &str,
        pane: Option<&str>,
        endpoint: Option<&str>,
        incarnation: Option<ServerIncarnation>,
        observed_at: OffsetDateTime,
    ) -> Self {
        let backend = pane.and_then(crate::backend::pane_id_host_kind);
        let endpoint =
            endpoint.map(|endpoint| crate::backend::pane_endpoint_identity(pane, endpoint));
        Self {
            id: format!("sf_{}", uuid::Uuid::new_v4().simple()),
            kind,
            host: "local".into(),
            backend,
            endpoint,
            pane: pane.map(str::to_owned),
            session_id: session_id.to_owned(),
            incarnation,
            same_incarnation: false,
            replaced_by: None,
            observed_at,
            source: match kind {
                FactKind::Ended => "session_ended_hook",
                FactKind::Replaced => "started_hook",
            }
            .into(),
        }
    }
}

/// What the agent registry keeps for the subject check: the newest scan,
/// each live session's binding with its server incarnation, and the facts
/// recorded at ingestion that the collaboration store has not taken yet.
/// In memory only; the facts become durable once the collaboration store
/// takes them.
#[derive(Debug, Default)]
pub struct SubjectObservations {
    scan: std::sync::Mutex<Option<std::sync::Arc<PaneScan>>>,
    bindings: std::sync::Mutex<std::collections::HashMap<String, SubjectBinding>>,
    facts: std::sync::Mutex<Vec<SubjectFact>>,
    /// Replacement facts waiting for the first scan after their hook to
    /// establish whether the server is the incarnation the predecessor was
    /// bound on.
    unverified: std::sync::Mutex<Vec<SubjectFact>>,
}

/// A replacement no complete scan of its socket could settle within this
/// long is recorded as not on the same incarnation (it never counts).
const UNVERIFIED_FOR: time::Duration = time::Duration::minutes(10);

/// Facts waiting to be taken are bounded: a collaboration store that never
/// takes them (collaboration off) must not grow the registry.
const MAX_PENDING_FACTS: usize = 4096;

impl SubjectObservations {
    #[must_use]
    pub fn scan(&self) -> Option<std::sync::Arc<PaneScan>> {
        self.scan.lock().ok().and_then(|scan| scan.clone())
    }

    /// Keep `scan` as the newest, and settle the replacement facts whose
    /// hook came before it: the server answered with the incarnation the
    /// predecessor was bound on both before the hook (the binding) and after
    /// it (this scan), so no restart happened in between, or it did not.
    pub fn set_scan(&self, scan: PaneScan) {
        let settled = self.unverified.lock().map_or_else(
            |_| Vec::new(),
            |mut pending| {
                let mut settled = Vec::new();
                pending.retain_mut(|fact| {
                    if scan.at <= fact.observed_at {
                        return true;
                    }
                    let answer = fact.backend.and_then(|kind| {
                        let host = scan.complete_host(kind)?;
                        let endpoint = fact.endpoint.as_deref()?;
                        host.servers
                            .iter()
                            .find(|server| server.endpoint == endpoint)
                    });
                    match answer {
                        Some(server) => {
                            fact.same_incarnation = server.running
                                && fact.incarnation.is_some()
                                && server.incarnation == fact.incarnation;
                        }
                        None if scan.at - fact.observed_at < UNVERIFIED_FOR => return true,
                        None => fact.same_incarnation = false,
                    }
                    settled.push(fact.clone());
                    false
                });
                settled
            },
        );
        for fact in settled {
            self.record(fact);
        }
        if let Ok(mut slot) = self.scan.lock() {
            *slot = Some(std::sync::Arc::new(scan));
        }
    }

    /// A replacement fact, held until a scan after its hook settles it.
    pub fn record_unverified(&self, fact: SubjectFact) {
        if let Ok(mut pending) = self.unverified.lock() {
            if pending.len() >= MAX_PENDING_FACTS {
                pending.remove(0);
            }
            pending.push(fact);
        }
    }

    #[must_use]
    pub fn binding(&self, session: &str) -> Option<SubjectBinding> {
        self.bindings
            .lock()
            .ok()
            .and_then(|bindings| bindings.get(session).cloned())
    }

    #[must_use]
    pub fn bindings(&self) -> std::collections::HashMap<String, SubjectBinding> {
        self.bindings
            .lock()
            .map(|bindings| bindings.clone())
            .unwrap_or_default()
    }

    pub fn forget_binding(&self, session: &str) {
        if let Ok(mut bindings) = self.bindings.lock() {
            bindings.remove(session);
        }
    }

    /// Keep each live session's binding: a new pane or endpoint replaces it;
    /// the same one keeps the incarnation it was first seen on, so a server
    /// restart that reuses the pane id never re-stamps an old binding.
    /// Sessions with no row left (`rows`) are dropped; a row that is no
    /// longer live keeps the binding it had.
    pub fn stamp_bindings(
        &self,
        live: Vec<(String, SubjectBinding)>,
        rows: &std::collections::HashSet<String>,
    ) {
        let Ok(mut bindings) = self.bindings.lock() else {
            return;
        };
        bindings.retain(|session, _| rows.contains(session));
        let mut next = std::mem::take(&mut *bindings);
        for (session, binding) in live {
            let kept = next
                .remove(&session)
                .filter(|old| old.pane == binding.pane && old.endpoint == binding.endpoint)
                .map(|old| SubjectBinding {
                    incarnation: old.incarnation.or(binding.incarnation),
                    ..old
                });
            next.insert(session, kept.unwrap_or(binding));
        }
        *bindings = next;
    }

    pub fn record(&self, fact: SubjectFact) {
        tracing::info!(
            kind = ?fact.kind,
            session = fact.session_id,
            pane = fact.pane.as_deref().unwrap_or(""),
            same_incarnation = fact.same_incarnation,
            "recorded subject evidence",
        );
        if let Ok(mut facts) = self.facts.lock() {
            if facts.len() >= MAX_PENDING_FACTS {
                facts.remove(0);
            }
            facts.push(fact);
        }
    }

    /// The settled facts not yet taken, left in place.
    #[must_use]
    pub fn peek_facts(&self) -> Vec<SubjectFact> {
        self.facts.lock().map(|f| f.clone()).unwrap_or_default()
    }

    /// Put back facts a failed write could not keep, ahead of newer ones.
    pub fn requeue(&self, mut facts: Vec<SubjectFact>) {
        if let Ok(mut held) = self.facts.lock() {
            facts.append(&mut held);
            let excess = facts.len().saturating_sub(MAX_PENDING_FACTS);
            facts.drain(..excess);
            *held = facts;
        }
    }

    #[must_use]
    pub fn take_facts(&self) -> Vec<SubjectFact> {
        self.facts
            .lock()
            .map(|mut facts| std::mem::take(&mut *facts))
            .unwrap_or_default()
    }
}
