//! Opt-in headless reviews use the normal SQLite mailbox, without a pane.
//! The dispatch marker is durable before any provider process may start.
//! A missing outcome after that marker is uncertain and is never replayed.
use super::{
    CollaborationError, CollaborationReply, CollaborationRequest, CollaborationStore,
    LaunchPurpose, NewRequest, Participant, RequestKind, RequestStatus, WorkMode,
};
use crate::{AgentKind, AgentState};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::Ordering;
use time::OffsetDateTime;
use uuid::Uuid;

pub const HEADLESS_REVIEW_TARGET: &str = "headless:codex";
const RUNNER_SESSION: &str = "tend:headless-review:codex";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HeadlessPhase {
    RunnerAccepted,
    ProcessStarted,
    ProviderTurnStarted,
    Finished,
    Uncertain,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HeadlessAttempt {
    pub dispatch_generation: Uuid,
    pub phase: HeadlessPhase,
    #[serde(with = "time::serde::rfc3339")]
    pub accepted_at: OffsetDateTime,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub process_started_at: Option<OffsetDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_identity: Option<super::ProcessFence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_thread_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub provider_turn_started_at: Option<OffsetDateTime>,
}

impl Participant {
    pub fn is_headless_review_runner(&self) -> bool {
        self.agent_session_id == RUNNER_SESSION && self.pane == HEADLESS_REVIEW_TARGET
    }

    pub(crate) fn headless_review_recipient(&self) -> Self {
        Self {
            launch_purpose: LaunchPurpose::Worker,
            draining: false,
            agent_kind: AgentKind::Codex,
            agent_session_id: RUNNER_SESSION.into(),
            pane: HEADLESS_REVIEW_TARGET.into(),
            socket: self.socket.clone(),
            room: self.room.clone(),
            tmux_session_id: self.tmux_session_id.clone(),
            tmux_session_name: self.tmux_session_name.clone(),
            window_name: self.window_name.clone(),
            state: AgentState::Idle,
            cwd: self.cwd.clone(),
            alias: None,
            roles: Vec::new(),
            console: false,
        }
    }
}

impl CollaborationStore {
    /// Only the daemon owning a durable local mailbox enables this runner.
    pub fn set_headless_reviews_enabled(&self, enabled: bool) {
        self.headless_reviews_enabled
            .store(enabled && self.database.is_some(), Ordering::SeqCst);
    }

    pub fn headless_reviews_enabled(&self) -> bool {
        self.headless_reviews_enabled.load(Ordering::SeqCst)
    }

    pub(crate) fn validate_headless_review(
        &self,
        sender: &Participant,
        input: &NewRequest,
    ) -> Result<(), CollaborationError> {
        let refuse = |message: &str| CollaborationError::InvalidDispatch(message.into());
        if !self.headless_reviews_enabled() {
            return Err(refuse("headless reviews are disabled on this daemon"));
        }
        if sender.console
            || sender.is_headless_review_runner()
            || sender
                .cwd
                .as_deref()
                .is_none_or(|cwd| !Path::new(cwd).is_absolute())
            || input.source_node_id.is_some()
            || input.initiator.is_some()
        {
            return Err(refuse(
                "headless reviews require a local tracked sender with a working directory",
            ));
        }
        if input.kind != RequestKind::Review
            || input.work_mode != WorkMode::ReadOnly
            || !input.expects_reply
            || input.task_contract.is_some()
            || input.human_action.is_some()
            || input.supersedes_request_id.is_some()
            || input.parent_request_id.is_some()
            || input.delegation_parent.is_some()
            || input.user_authorized
            || !input.paths.is_empty()
            || !input.artifacts.is_empty()
            || !input.links.is_empty()
            || input.allow_path_overlap
        {
            return Err(refuse("pilot accepts standalone read-only reviews without contracts, paths, links or artifacts"));
        }
        if input.body.len() > 16 * 1024 {
            return Err(CollaborationError::MessageTooLarge(16 * 1024));
        }
        if input.dispatch_id.is_none() {
            return Err(refuse(
                "headless reviews require a stable --dispatch-id UUID",
            ));
        }
        Ok(())
    }

    pub async fn reserve_headless_review(
        &self,
    ) -> Result<Option<CollaborationRequest>, CollaborationError> {
        if !self.headless_reviews_enabled() {
            return Ok(None);
        }
        let _transaction = self.transaction_lock.lock().await;
        let request = {
            let requests = self.requests.read().await;
            // One process on this daemon, even if the executor is called twice.
            if requests
                .values()
                .any(|r| r.to.is_headless_review_runner() && r.status == RequestStatus::Claimed)
            {
                return Ok(None);
            }
            requests
                .values()
                .filter(|r| {
                    r.to.is_headless_review_runner()
                        && r.status == RequestStatus::Queued
                        && r.headless_attempt.is_none()
                })
                .min_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)))
                .cloned()
        };
        let Some(mut request) = request else {
            return Ok(None);
        };
        let now = OffsetDateTime::now_utc();
        request.status = RequestStatus::Claimed;
        request.claimed_at = Some(now);
        request.headless_attempt = Some(HeadlessAttempt {
            dispatch_generation: Uuid::new_v4(),
            phase: HeadlessPhase::RunnerAccepted,
            accepted_at: now,
            process_started_at: None,
            process_id: None,
            process_identity: None,
            provider_thread_id: None,
            provider_turn_started_at: None,
        });
        self.commit_request_rows(std::slice::from_ref(&request))
            .await?;
        self.publish_change();
        Ok(Some(request))
    }

    pub async fn observe_headless_review(
        &self,
        request_id: &str,
        generation: Uuid,
        evidence: crate::ask::HeadlessReviewEvent,
    ) -> Result<bool, CollaborationError> {
        self.update_headless_attempt(request_id, generation, |attempt, _| match evidence {
            crate::ask::HeadlessReviewEvent::ProcessStarted { at, pid } => {
                attempt.process_started_at = Some(at);
                attempt.process_id = Some(pid);
                attempt.process_identity = super::ProcessFence::capture(pid);
                attempt.phase = HeadlessPhase::ProcessStarted;
            }
            crate::ask::HeadlessReviewEvent::ThreadRegistered(id) => {
                attempt.provider_thread_id = Some(id);
            }
            crate::ask::HeadlessReviewEvent::ProviderTurnStarted(at) => {
                attempt.provider_turn_started_at = Some(at);
                attempt.phase = HeadlessPhase::ProviderTurnStarted;
            }
        })
        .await
    }

    pub async fn finish_headless_review(
        &self,
        request_id: &str,
        generation: Uuid,
        answer: Option<String>,
    ) -> Result<bool, CollaborationError> {
        self.update_headless_attempt(request_id, generation, |attempt, request| {
            finish(
                attempt,
                request,
                answer.filter(|answer| {
                    !answer.trim().is_empty() && answer.len() <= self.opts.max_message_bytes
                }),
            );
        })
        .await
    }

    async fn update_headless_attempt(
        &self,
        request_id: &str,
        generation: Uuid,
        update: impl FnOnce(&mut HeadlessAttempt, &mut CollaborationRequest),
    ) -> Result<bool, CollaborationError> {
        let _transaction = self.transaction_lock.lock().await;
        let Some(mut request) = self.requests.read().await.get(request_id).cloned() else {
            return Ok(false);
        };
        let Some(mut attempt) = request.headless_attempt.take() else {
            return Ok(false);
        };
        if !request.to.is_headless_review_runner()
            || request.status != RequestStatus::Claimed
            || attempt.dispatch_generation != generation
        {
            return Ok(false);
        }
        update(&mut attempt, &mut request);
        request.headless_attempt = Some(attempt);
        self.commit_request_rows(&[request]).await?;
        self.publish_change();
        Ok(true)
    }

    /// Restart recovery is deliberately conservative across the spawn gap.
    pub async fn recover_headless_reviews(&self) -> Result<(), CollaborationError> {
        let _transaction = self.transaction_lock.lock().await;
        let mut rows = Vec::new();
        for mut request in self
            .requests
            .read()
            .await
            .values()
            .filter(|r| r.to.is_headless_review_runner() && r.status == RequestStatus::Claimed)
            .cloned()
        {
            let Some(mut attempt) = request.headless_attempt.take() else {
                continue;
            };
            finish(&mut attempt, &mut request, None);
            request.headless_attempt = Some(attempt);
            rows.push(request);
        }
        if !rows.is_empty() {
            self.commit_request_rows(&rows).await?;
            self.publish_change();
        }
        Ok(())
    }
}

fn finish(
    attempt: &mut HeadlessAttempt,
    request: &mut CollaborationRequest,
    answer: Option<String>,
) {
    let (phase, status, body) = match answer {
        Some(answer) => (HeadlessPhase::Finished, RequestStatus::Completed, answer),
        None => (
            HeadlessPhase::Uncertain,
            RequestStatus::Blocked,
            "The headless attempt did not produce a verified result. It may have started. Tend will not start this request again automatically.".into(),
        ),
    };
    attempt.phase = phase;
    request.status = status;
    // The opt-in headless route never types into a terminal, including on reply.
    request.reply_wake = Some(super::ReplyWake::Released {
        reason: super::ReplyWakeRelease::Withdrawn,
        at: OffsetDateTime::now_utc(),
    });
    request.reply = Some(CollaborationReply {
        status,
        body,
        artifacts: Vec::new(),
        report: None,
        at: OffsetDateTime::now_utc(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collaboration::CollaborationOptions;

    fn sender(cwd: &Path) -> Participant {
        serde_json::from_value(serde_json::json!({
            "agent_kind": "codex",
            "agent_session_id": "pilot-sender",
            "pane": "rmux:%3",
            "room": {"host": "rmux", "window_id": "@1"},
            "state": "working",
            "cwd": cwd,
        }))
        .unwrap()
    }

    fn review() -> NewRequest {
        NewRequest {
            dispatch_id: Some(Uuid::new_v4().to_string()),
            kind: RequestKind::Review,
            body: "review the local fixture".into(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn durable_restart_matrix_never_replays_a_reserved_attempt() {
        for _ in 0..5 {
            for phase in [
                None,
                Some(HeadlessPhase::RunnerAccepted),
                Some(HeadlessPhase::ProcessStarted),
                Some(HeadlessPhase::Finished),
            ] {
                let root = tempfile::tempdir().unwrap();
                let options = CollaborationOptions {
                    path: Some(root.path().join("reviews.sqlite3")),
                    ..Default::default()
                };
                let store = CollaborationStore::load(options.clone()).await.unwrap();
                store.set_headless_reviews_enabled(true);
                let from = sender(root.path());
                let to = from.headless_review_recipient();
                let input = review();
                let request = store
                    .create(from.clone(), to.clone(), input.clone())
                    .await
                    .unwrap();
                assert_eq!(
                    store
                        .create(from.clone(), to.clone(), input.clone())
                        .await
                        .unwrap()
                        .id,
                    request.id
                );
                assert!(store.pending_unnotified().await.is_empty());
                if let Some(phase) = phase {
                    let reservation = store.reserve_headless_review().await.unwrap().unwrap();
                    let generation = reservation.headless_attempt.unwrap().dispatch_generation;
                    assert!(store.reserve_headless_review().await.unwrap().is_none());
                    assert!(!store
                        .finish_headless_review(&request.id, Uuid::new_v4(), Some("stale".into()))
                        .await
                        .unwrap());
                    if phase != HeadlessPhase::RunnerAccepted {
                        store
                            .observe_headless_review(
                                &request.id,
                                generation,
                                crate::ask::HeadlessReviewEvent::ProcessStarted {
                                    pid: 42,
                                    at: OffsetDateTime::now_utc(),
                                },
                            )
                            .await
                            .unwrap();
                        store
                            .observe_headless_review(
                                &request.id,
                                generation,
                                crate::ask::HeadlessReviewEvent::ProviderTurnStarted(
                                    OffsetDateTime::now_utc(),
                                ),
                            )
                            .await
                            .unwrap();
                    }
                    if phase == HeadlessPhase::Finished {
                        assert!(store
                            .finish_headless_review(
                                &request.id,
                                generation,
                                Some("verified result".into())
                            )
                            .await
                            .unwrap());
                        assert!(!store
                            .finish_headless_review(
                                &request.id,
                                generation,
                                Some("overwrite".into())
                            )
                            .await
                            .unwrap());
                    }
                }
                drop(store);
                let store = CollaborationStore::load(options).await.unwrap();
                store.recover_headless_reviews().await.unwrap();
                store.set_headless_reviews_enabled(true);
                let recovered = store.peek_for(&from, &request.id).await.unwrap();
                match phase {
                    None => {
                        assert_eq!(recovered.status, RequestStatus::Queued);
                        assert_eq!(
                            store.reserve_headless_review().await.unwrap().unwrap().id,
                            request.id
                        );
                    }
                    Some(HeadlessPhase::Finished) => {
                        assert_eq!(recovered.reply.unwrap().body, "verified result");
                        assert!(store.reserve_headless_review().await.unwrap().is_none());
                    }
                    _ => {
                        let attempt = recovered.headless_attempt.unwrap();
                        assert_eq!(attempt.phase, HeadlessPhase::Uncertain);
                        assert_eq!(recovered.status, RequestStatus::Blocked);
                        assert!(store.reserve_headless_review().await.unwrap().is_none());
                        assert!(!store
                            .finish_headless_review(
                                &request.id,
                                attempt.dispatch_generation,
                                Some("late reply".into())
                            )
                            .await
                            .unwrap());
                    }
                }
                assert_eq!(store.create(from, to, input).await.unwrap().id, request.id);
            }
        }
    }

    #[tokio::test]
    async fn rejects_explicit_execute_and_disabled_pilot_before_enqueueing() {
        let root = tempfile::tempdir().unwrap();
        let store = CollaborationStore::load(CollaborationOptions {
            path: Some(root.path().join("reviews.sqlite3")),
            ..Default::default()
        })
        .await
        .unwrap();
        let from = sender(root.path());
        let to = from.headless_review_recipient();
        assert!(store
            .create(from.clone(), to.clone(), review())
            .await
            .is_err());
        store.set_headless_reviews_enabled(true);
        let mut input = review();
        input.work_mode = WorkMode::Execute;
        assert!(store.create(from.clone(), to.clone(), input).await.is_err());
        let mut input = review();
        input.dispatch_id = None;
        assert!(store.create(from, to, input).await.is_err());
        assert!(store.reserve_headless_review().await.unwrap().is_none());
    }
}
