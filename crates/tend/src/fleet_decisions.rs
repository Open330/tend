//! A person's Keep or Close on another Fleet host's unanswerable questions,
//! sent from this host (`docs/proposals/FLEET_UNANSWERABLE.md`). The
//! dashboard's routes and the CLI (`tend review keep|close --host`) both go
//! through here, so they refuse the same hosts with the same words and
//! believe the holding host's answer only when it accounts for exactly what
//! was asked. The holding host applies the decision under its own rules and
//! records it with `via_fleet`.

use crate::collaboration::subject_check::{CloseItem, CloseOutcome, KeepOutcome};
use crate::fleet::{FleetHostSnapshot, FleetRuntime, HostAccessMode};
use std::collections::BTreeSet;
use std::time::Duration;

/// How long a Keep or Close waits for the holding host.
pub const FLEET_WRITE_TIMEOUT: Duration = Duration::from_secs(20);

/// Why a Keep or Close was not done, by who should act on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// No such host, or Fleet is off here.
    NotFound(String),
    /// The holding host's own choice: observe-only, or Close all turned off.
    Forbidden(String),
    /// The holding host's tend is too old for it.
    Conflict(String),
    /// What was asked cannot be sent as it is.
    BadRequest(String),
    /// The holding host failed, or answered with something else.
    BadGateway(String),
}

impl Refusal {
    #[must_use]
    pub fn message(&self) -> &str {
        match self {
            Self::NotFound(m)
            | Self::Forbidden(m)
            | Self::Conflict(m)
            | Self::BadRequest(m)
            | Self::BadGateway(m) => m,
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

/// What can be done to a host's folded questions from here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnanswerableActions {
    /// Keep: control mode and a relay that applies it.
    pub keep: bool,
    /// Close: that, and the host's own `[review] close_unanswerable`.
    pub close: bool,
    /// Why not, when either is missing: shown under the folded group.
    pub note: Option<String>,
}

/// [`UnanswerableActions`] for `host`, from its mode and capabilities.
#[must_use]
pub fn unanswerable_actions(host: &FleetHostSnapshot) -> UnanswerableActions {
    use crate::fleet::{
        FLEET_CLOSE_UNANSWERABLE_CAPABILITY, FLEET_DECISION_UNANSWERABLE_CAPABILITY,
    };
    let has = |capability: &str| host.capabilities.iter().any(|c| c == capability);
    let alias = &host.alias;
    if host.mode != HostAccessMode::Control {
        return UnanswerableActions {
            keep: false,
            close: false,
            note: Some(format!(
                "{alias} is observe-only; keep or close these there, or set mode = \"control\""
            )),
        };
    }
    if !has(FLEET_DECISION_UNANSWERABLE_CAPABILITY) {
        return UnanswerableActions {
            keep: false,
            close: false,
            note: Some(format!(
                "{alias} cannot keep or close these over Fleet; upgrade tend there"
            )),
        };
    }
    if !has(FLEET_CLOSE_UNANSWERABLE_CAPABILITY) {
        return UnanswerableActions {
            keep: true,
            close: false,
            note: Some(format!(
                "Close all is turned off on {alias} ([review] close_unanswerable)"
            )),
        };
    }
    UnanswerableActions {
        keep: true,
        close: true,
        note: None,
    }
}

/// This host's Fleet alias (always `local` when the Fleet manager knows
/// itself), or "this host".
#[must_use]
pub fn local_alias(hosts: &[FleetHostSnapshot]) -> String {
    hosts
        .iter()
        .find(|host| host.local)
        .map_or_else(|| "this host".into(), |host| host.alias.clone())
}

/// `host` when it names another Fleet host: not absent, this host's alias
/// or "this host".
#[must_use]
pub fn remote_alias<'a>(host: Option<&'a str>, hosts: &[FleetHostSnapshot]) -> Option<&'a str> {
    host.filter(|alias| *alias != local_alias(hosts) && *alias != "this host")
}

/// What the holding host records as the controller: this machine's name.
/// Its own Fleet alias is always `local`, which names no host there.
#[must_use]
pub fn controller_name(hosts: &[FleetHostSnapshot]) -> String {
    crate::fleet::local_hostname().unwrap_or_else(|| local_alias(hosts))
}

/// The remote host for a Keep or Close, checked before anything is sent:
/// it exists, and can take `close` (Close) or Keep.
pub fn unanswerable_host(
    hosts: &[FleetHostSnapshot],
    alias: &str,
    close: bool,
) -> Result<(), Refusal> {
    let Some(host) = hosts.iter().find(|host| host.alias == alias && !host.local) else {
        return Err(Refusal::NotFound(format!("no Fleet host {alias}")));
    };
    let actions = unanswerable_actions(host);
    if if close { actions.close } else { actions.keep } {
        return Ok(());
    }
    let note = actions.note.unwrap_or_default();
    // Observe-only and Close all switched off are the host's own choices;
    // an older relay needs an upgrade.
    if host.mode != HostAccessMode::Control || actions.keep {
        Err(Refusal::Forbidden(note))
    } else {
        Err(Refusal::Conflict(note))
    }
}

/// The episode `id` is folded in on `alias`, read from that host's queue:
/// what a Keep or Close of it must name.
pub async fn remote_episode(
    fleet: &FleetRuntime,
    alias: &str,
    id: &str,
) -> Result<String, Refusal> {
    use crate::decision_queue::PendingItem;
    use crate::fleet::{FleetOperation, FLEET_DECISION_QUEUE_CAPABILITY};
    let reads = fleet
        .store
        .snapshot_selected(None)
        .await
        .hosts
        .iter()
        .any(|host| {
            host.alias == alias
                && host
                    .capabilities
                    .iter()
                    .any(|c| c == FLEET_DECISION_QUEUE_CAPABILITY)
        });
    if !reads {
        return Err(Refusal::Conflict(format!(
            "{alias} cannot list its questions over Fleet; upgrade tend there"
        )));
    }
    let result = fleet
        .execute(
            alias.to_owned(),
            FleetOperation::DecisionQueue {
                limit: crate::decision_queue::MAX_QUEUE_LIMIT,
            },
            FLEET_WRITE_TIMEOUT,
        )
        .await
        .map_err(|error| Refusal::BadGateway(format!("{alias}: {error}")))?;
    let queue = result
        .decision_queue
        .filter(|_| result.accepted)
        .ok_or_else(|| Refusal::BadGateway(format!("{alias} did not send its questions")))?;
    queue
        .items
        .iter()
        .find_map(|item| match item {
            PendingItem::HumanRequest(entry) if entry.request_id == id => entry
                .unanswerable
                .as_ref()
                .map(|unanswerable| unanswerable.episode.clone()),
            _ => None,
        })
        .ok_or_else(|| {
            Refusal::NotFound(format!(
                "{id} is not listed on {alias} as a question that can't be answered"
            ))
        })
}

/// Who decided, from where: as the holding host records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decider<'a> {
    /// The person, as this host established them (`web:<email>`,
    /// `device:<id>`, `cli:<user>`).
    pub actor: &'a str,
    /// One of `subject_check::SURFACES`.
    pub surface: &'a str,
}

/// Keep `id` through `episode` on `alias`.
pub async fn keep_remote(
    fleet: &FleetRuntime,
    hosts: &[FleetHostSnapshot],
    alias: &str,
    id: &str,
    episode: &str,
    by: &Decider<'_>,
) -> Result<KeepOutcome, Refusal> {
    use crate::fleet::{FleetDecisionKeep, FleetOperation};
    unanswerable_host(hosts, alias, false)?;
    let operation = FleetOperation::DecisionKeep {
        keep: FleetDecisionKeep {
            id: id.to_owned(),
            episode: episode.to_owned(),
            actor: by.actor.to_owned(),
            surface: by.surface.to_owned(),
            controller: controller_name(hosts),
        },
    };
    let result = fleet
        .execute(alias.to_owned(), operation, FLEET_WRITE_TIMEOUT)
        .await
        .map_err(|error| Refusal::BadGateway(format!("{alias}: {error}")))?;
    // A refusal is never taken as a Keep, whatever else it carries.
    let kept = result
        .decision_keep
        .filter(|_| result.accepted)
        .ok_or_else(|| Refusal::BadGateway(format!("{alias} did not say what it kept")))?;
    if kept.request_id != id || kept.episode != episode {
        return Err(Refusal::BadGateway(format!(
            "{alias} kept {} instead of {id}",
            kept.request_id
        )));
    }
    Ok(*kept)
}

/// Close exactly `items` on `alias`.
pub async fn close_remote(
    fleet: &FleetRuntime,
    hosts: &[FleetHostSnapshot],
    alias: &str,
    items: &[CloseItem],
    by: &Decider<'_>,
) -> Result<CloseOutcome, Refusal> {
    use crate::fleet::{FleetCloseUnanswerable, FleetOperation};
    unanswerable_host(hosts, alias, true)?;
    let asked: BTreeSet<&str> = items.iter().map(|item| item.request_id.as_str()).collect();
    if asked.len() != items.len() {
        return Err(Refusal::BadRequest(
            "each question may be listed once".into(),
        ));
    }
    let operation = FleetOperation::DecisionCloseUnanswerable {
        close: FleetCloseUnanswerable {
            items: items.to_vec(),
            actor: by.actor.to_owned(),
            surface: by.surface.to_owned(),
            controller: controller_name(hosts),
        },
    };
    let result = fleet
        .execute(alias.to_owned(), operation, FLEET_WRITE_TIMEOUT)
        .await
        .map_err(|error| Refusal::BadGateway(format!("{alias}: {error}")))?;
    // A refusal is never taken as a close, whatever else it carries.
    let outcome = result
        .decision_close_unanswerable
        .filter(|_| result.accepted)
        .ok_or_else(|| Refusal::BadGateway(format!("{alias} did not say what it closed")))?;
    // Every listed question exactly once, closed or skipped: nothing left
    // out, nothing twice, nothing that was not asked about.
    let reported: Vec<&str> = outcome
        .closed
        .iter()
        .map(String::as_str)
        .chain(outcome.skipped.iter().map(|skip| skip.request_id.as_str()))
        .collect();
    let unique: BTreeSet<&str> = reported.iter().copied().collect();
    if unique.len() != reported.len() || unique != asked {
        return Err(Refusal::BadGateway(format!(
            "{alias} did not account for exactly the listed questions"
        )));
    }
    Ok(*outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleet::{
        FLEET_CLOSE_UNANSWERABLE_CAPABILITY, FLEET_DECISION_UNANSWERABLE_CAPABILITY,
    };

    fn host(alias: &str, local: bool, mode: &str, capabilities: &[&str]) -> FleetHostSnapshot {
        serde_json::from_value(serde_json::json!({
            "alias": alias, "local": local, "ssh_target": "", "labels": {}, "annotations": {},
            "mode": mode, "state": "online", "capabilities": capabilities,
        }))
        .unwrap()
    }

    #[test]
    fn a_host_is_refused_for_what_it_cannot_take_with_its_reason() {
        let both = [
            FLEET_DECISION_UNANSWERABLE_CAPABILITY,
            FLEET_CLOSE_UNANSWERABLE_CAPABILITY,
        ];
        let hosts = vec![
            host("local", true, "control", &both),
            host("mini", false, "control", &both),
            host(
                "keeps",
                false,
                "control",
                &[FLEET_DECISION_UNANSWERABLE_CAPABILITY],
            ),
            host("old", false, "control", &[]),
            host("obs", false, "observe", &both),
        ];
        assert_eq!(unanswerable_host(&hosts, "mini", false), Ok(()));
        assert_eq!(unanswerable_host(&hosts, "mini", true), Ok(()));
        // Close all off there: Keep still goes, Close is the host's refusal.
        assert_eq!(unanswerable_host(&hosts, "keeps", false), Ok(()));
        assert_eq!(
            unanswerable_host(&hosts, "keeps", true),
            Err(Refusal::Forbidden(
                "Close all is turned off on keeps ([review] close_unanswerable)".into()
            ))
        );
        // An older relay needs an upgrade; observe-only is the host's choice.
        assert!(matches!(
            unanswerable_host(&hosts, "old", false),
            Err(Refusal::Conflict(note)) if note.contains("upgrade tend there")
        ));
        assert!(matches!(
            unanswerable_host(&hosts, "obs", true),
            Err(Refusal::Forbidden(note)) if note.contains("observe-only")
        ));
        // Not a remote host at all.
        assert!(matches!(
            unanswerable_host(&hosts, "local", false),
            Err(Refusal::NotFound(_))
        ));
        assert!(matches!(
            unanswerable_host(&hosts, "nowhere", false),
            Err(Refusal::NotFound(_))
        ));
        assert_eq!(remote_alias(Some("mini"), &hosts), Some("mini"));
        assert_eq!(remote_alias(Some("local"), &hosts), None);
        assert_eq!(remote_alias(Some("this host"), &hosts), None);
    }
}
