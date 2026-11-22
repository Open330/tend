use super::{Client, RuntimeError, PROTOCOL_VERSION};
use crate::review::{
    DispositionKind, NewReview, ReviewActor, ReviewItem, ReviewPage, ReviewQuery, ReviewSummary,
};
use std::time::Duration;

fn field<T: serde::de::DeserializeOwned>(
    response: &serde_json::Value,
    name: &str,
) -> Result<T, RuntimeError> {
    serde_json::from_value(response[name].clone()).map_err(RuntimeError::Json)
}

impl Client {
    /// Record a claim that a result needs review. Requires `review_items_v1`.
    pub async fn review_create(&self, review: &NewReview) -> Result<ReviewItem, RuntimeError> {
        let req = serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "kind": "review_create",
            "review": review,
        });
        // The daemon may run the Work's declared checks first.
        let response = self
            .call_with_timeout(&req, crate::work_verify::VERIFY_CALL_TIMEOUT)
            .await?;
        if !response["ok"].as_bool().unwrap_or(false) {
            return Err(super::response_error(&response));
        }
        field(&response, "review_item")
    }

    pub async fn review_get(&self, id: &str) -> Result<ReviewItem, RuntimeError> {
        let req = serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "kind": "review_get",
            "id": id,
        });
        field(&self.call_checked(&req).await?, "review_item")
    }

    pub async fn review_list(&self, query: &ReviewQuery) -> Result<ReviewPage, RuntimeError> {
        let req = serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "kind": "review_list",
            "open_only": query.open_only,
            "oldest_first": query.oldest_first,
            "limit": query.limit,
            "offset": query.offset,
        });
        let response = self.call_checked(&req).await?;
        Ok(ReviewPage {
            items: field(&response, "review_items")?,
            has_more: response["review_has_more"].as_bool().unwrap_or(false),
        })
    }

    /// Bounded aggregate under `deadline`, for status surfaces that must
    /// never stall.
    pub async fn review_summary_with_timeout(
        &self,
        deadline: Duration,
    ) -> Result<ReviewSummary, RuntimeError> {
        let req = serde_json::json!({ "protocol": PROTOCOL_VERSION, "kind": "review_summary" });
        let response = self.call_with_timeout(&req, deadline).await?;
        if !response["ok"].as_bool().unwrap_or(false) {
            return Err(super::response_error(&response));
        }
        field(&response, "review_summary")
    }

    /// Record a claim and its disposition together. Requires
    /// `review_record_v1` and an idempotency key on `review`.
    pub async fn review_record(
        &self,
        review: &NewReview,
        disposition: DispositionKind,
        note: Option<&str>,
    ) -> Result<ReviewItem, RuntimeError> {
        let req = serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "kind": "review_record",
            "review": review,
            "disposition": disposition,
            "note": note,
        });
        // Like review_create, the daemon may run the Work's checks first.
        let response = self
            .call_with_timeout(&req, crate::work_verify::VERIFY_CALL_TIMEOUT)
            .await?;
        if !response["ok"].as_bool().unwrap_or(false) {
            return Err(super::response_error(&response));
        }
        field(&response, "review_item")
    }

    pub async fn review_dispose(
        &self,
        id: &str,
        disposition: DispositionKind,
        by: &ReviewActor,
        note: Option<&str>,
    ) -> Result<ReviewItem, RuntimeError> {
        let req = serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "kind": "review_dispose",
            "id": id,
            "disposition": disposition,
            "by": by,
            "note": note,
        });
        field(&self.call_checked(&req).await?, "review_item")
    }
}

impl Client {
    /// Everything pending on the person, in one ordered list. Requires
    /// `decision_queue_v1`.
    /// Human requests come back only for a console `origin`.
    pub async fn decision_queue(
        &self,
        origin: &crate::collaboration::CollaborationOrigin,
        limit: usize,
    ) -> Result<crate::decision_queue::PendingQueue, RuntimeError> {
        let req = serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "kind": "decision_queue",
            "limit": limit,
            "origin": origin,
        });
        field(&self.call_checked(&req).await?, "decision_queue")
    }

    /// Decided review items and answered human requests, newest first.
    /// Requires `decision_queue_v1`.
    pub async fn decision_history(
        &self,
        origin: &crate::collaboration::CollaborationOrigin,
        limit: usize,
    ) -> Result<crate::decision_queue::DecisionHistory, RuntimeError> {
        self.decision_history_with(origin, limit, false).await
    }

    /// [`Self::decision_history`], also listing tend's own closures when
    /// `include_closures` (needs `decision_history_closures_v1`; an older
    /// daemon ignores the flag and sends the plain history).
    pub async fn decision_history_with(
        &self,
        origin: &crate::collaboration::CollaborationOrigin,
        limit: usize,
        include_closures: bool,
    ) -> Result<crate::decision_queue::DecisionHistory, RuntimeError> {
        let req = serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "kind": "decision_history",
            "limit": limit,
            "origin": origin,
            "include_closures": include_closures,
        });
        field(&self.call_checked(&req).await?, "decision_history")
    }
}

impl Client {
    /// Close tendd's own human items that can no longer matter, as the
    /// operator, or with `apply` false only report them. Requires
    /// `system_ask_cleanup_v1`.
    pub async fn system_ask_cleanup(
        &self,
        apply: bool,
    ) -> Result<crate::collaboration::system_asks::SystemAskCleanup, RuntimeError> {
        let req = serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "kind": "system_ask_cleanup",
            "apply": apply,
            "origin": crate::decision_queue::operator_origin(),
        });
        field(&self.call_checked(&req).await?, "system_ask_cleanup")
    }
}

impl Client {
    /// Keep one unanswerable question in the person's turn through its
    /// current episode (`subject_check_v1`).
    pub async fn decision_keep(
        &self,
        request_id: &str,
        episode: &str,
    ) -> Result<crate::collaboration::subject_check::KeepOutcome, RuntimeError> {
        let req = serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "kind": "decision_keep",
            "request_id": request_id,
            "episode": episode,
            "origin": crate::decision_queue::operator_origin(),
        });
        field(&self.call_checked(&req).await?, "decision_keep")
    }

    /// Close exactly these `(request id, episode)` pairs as the person, from
    /// `surface` (`close_unanswerable_v1`). Pairs that no longer qualify are
    /// skipped and reported.
    pub async fn decision_close_unanswerable(
        &self,
        items: &[crate::collaboration::subject_check::CloseItem],
        surface: &str,
    ) -> Result<crate::collaboration::subject_check::CloseOutcome, RuntimeError> {
        let req = serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "kind": "decision_close_unanswerable",
            "items": items,
            "origin": crate::decision_queue::operator_origin(),
            "by_label": surface,
        });
        field(
            &self.call_checked(&req).await?,
            "decision_close_unanswerable",
        )
    }
}

impl Client {
    /// Keep `request_id` on Fleet host `host`, as the person
    /// (`decision_on_host_v1`). Without `episode`, the one it is folded in
    /// there now.
    pub async fn decision_keep_on_host(
        &self,
        host: &str,
        request_id: &str,
        episode: Option<&str>,
    ) -> Result<crate::collaboration::subject_check::KeepOutcome, RuntimeError> {
        let req = serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "kind": "decision_keep_on_host",
            "host": host,
            "request_id": request_id,
            "episode": episode,
            "origin": crate::decision_queue::operator_origin(),
        });
        field(&self.call_checked(&req).await?, "decision_keep")
    }

    /// Close `request_id` on Fleet host `host`, as the person, exactly as
    /// Close all would there (`decision_on_host_v1`).
    pub async fn decision_close_on_host(
        &self,
        host: &str,
        request_id: &str,
        episode: Option<&str>,
    ) -> Result<crate::collaboration::subject_check::CloseOutcome, RuntimeError> {
        let req = serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "kind": "decision_close_on_host",
            "host": host,
            "request_id": request_id,
            "episode": episode,
            "origin": crate::decision_queue::operator_origin(),
        });
        field(
            &self.call_checked(&req).await?,
            "decision_close_unanswerable",
        )
    }
}

impl Client {
    /// Apply a Fleet Keep on this host, as the operator, recording where it
    /// came from in the collaboration audit.
    pub async fn decision_keep_from_fleet(
        &self,
        keep: &crate::fleet::FleetDecisionKeep,
    ) -> Result<crate::collaboration::subject_check::KeepOutcome, RuntimeError> {
        let req = serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "kind": "fleet_decision_keep",
            "request_id": keep.id,
            "episode": keep.episode,
            "origin": crate::decision_queue::operator_origin(),
            "via_fleet": crate::collaboration_audit::FleetVia {
                controller: keep.controller.clone(),
                actor: keep.actor.clone(),
                surface: keep.surface.clone(),
            },
        });
        field(&self.call_checked(&req).await?, "decision_keep")
    }

    /// Apply a Fleet Close all on this host, as the operator: each closure
    /// records the person's own surface and `actor`, and the audit where it
    /// came from.
    pub async fn decision_close_unanswerable_from_fleet(
        &self,
        close: &crate::fleet::FleetCloseUnanswerable,
    ) -> Result<crate::collaboration::subject_check::CloseOutcome, RuntimeError> {
        let req = serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "kind": "fleet_decision_close_unanswerable",
            "items": close.items,
            "origin": crate::decision_queue::operator_origin(),
            "by_label": close.surface,
            "actor": close.actor,
            "via_fleet": crate::collaboration_audit::FleetVia {
                controller: close.controller.clone(),
                actor: close.actor.clone(),
                surface: close.surface.clone(),
            },
        });
        field(
            &self.call_checked(&req).await?,
            "decision_close_unanswerable",
        )
    }
}

impl Client {
    /// Apply a Fleet review decision on this host, as the operator, and
    /// say exactly how it ended: the option must be one the item offers,
    /// and the item must end up holding this disposition and note (a
    /// repeat of it is a replay; anything else already recorded is a
    /// conflict). The relay's side of `fleet_review_dispose_v1`.
    ///
    /// Works against an older daemon too: its `review_dispose` keeps an
    /// existing same-kind decision whatever the note, so the outcome is
    /// judged from the item it returns, not from the call succeeding.
    pub async fn review_dispose_exactly(
        &self,
        request: &crate::fleet::FleetReviewDispose,
    ) -> Result<crate::fleet::FleetReviewOutcome, RuntimeError> {
        use crate::fleet::FleetReviewOutcome as Outcome;
        let not_found = |error: &RuntimeError| matches!(error, RuntimeError::Daemon(message) if message.contains("no review item"));
        let before = match self.review_get(&request.id).await {
            Ok(item) => item,
            Err(error) if not_found(&error) => {
                return Ok(Outcome::NotFound {
                    message: format!("no review item {}", request.id),
                });
            }
            Err(error) => return Err(error),
        };
        if let Some(option) = &request.option {
            if !before.offers(option) {
                return Ok(Outcome::Invalid {
                    message: "that option is not one this item offers".into(),
                });
            }
        }
        let note = crate::review::decision_note(request.option.as_deref(), request.note.as_deref());
        if before.decided_as(request.disposition, note.as_deref()) {
            return Ok(Outcome::Recorded {
                item: Box::new(before),
                replay: true,
            });
        }
        if let Some(existing) = &before.disposition {
            return Ok(Outcome::Conflict {
                message: format!(
                    "review {} is already {}",
                    request.id,
                    existing.kind.as_str()
                ),
            });
        }
        let by = ReviewActor {
            kind: crate::review::ActorKind::User,
            label: Some(request.by.clone()),
        };
        let result = self
            .review_dispose(&request.id, request.disposition, &by, note.as_deref())
            .await;
        // Read back what the store holds: another decision may have landed
        // first, and an older daemon returns that one as if it were ours.
        let after = match result {
            Ok(item) => item,
            Err(RuntimeError::Daemon(message)) if message.contains("invalid review request") => {
                return Ok(Outcome::Invalid { message });
            }
            Err(error) => match self.review_get(&request.id).await {
                Ok(item) if item.disposition.is_some() => item,
                _ => return Err(error),
            },
        };
        if after.id != request.id {
            return Err(RuntimeError::Daemon(format!(
                "asked to decide review {} and got {}",
                request.id, after.id
            )));
        }
        Ok(if after.decided_as(request.disposition, note.as_deref()) {
            // The same decision by someone else won the race: a replay of
            // theirs, not a new one in this person's name.
            let replay = after
                .disposition
                .as_ref()
                .is_some_and(|existing| existing.by != by);
            Outcome::Recorded {
                item: Box::new(after),
                replay,
            }
        } else {
            Outcome::Conflict {
                message: match &after.disposition {
                    Some(existing) => {
                        format!(
                            "review {} is already {}",
                            request.id,
                            existing.kind.as_str()
                        )
                    }
                    None => format!("review {} was not decided", request.id),
                },
            }
        })
    }
}
