use super::{RequestBody, Response};
use crate::review::ReviewStore;

/// Serve the `review_*` requests. Kept out of `handle` so the dispatch
/// table's frame does not grow with each review operation.
pub(super) async fn review_response(
    reviews: &ReviewStore,
    body: RequestBody,
) -> (&'static str, Response) {
    let result = match body {
        RequestBody::ReviewCreate { review } => (
            "review_create",
            reviews
                .create(*review)
                .await
                .map(Response::with_review_item),
        ),
        RequestBody::ReviewGet { id } => (
            "review_get",
            reviews.get(id).await.map(Response::with_review_item),
        ),
        RequestBody::ReviewList {
            open_only,
            oldest_first,
            limit,
            offset,
        } => (
            "review_list",
            reviews
                .list(crate::review::ReviewQuery {
                    open_only,
                    oldest_first,
                    limit,
                    offset,
                })
                .await
                .map(Response::with_review_page),
        ),
        RequestBody::ReviewSummary => (
            "review_summary",
            reviews.summary().await.map(Response::with_review_summary),
        ),
        RequestBody::ReviewDispose {
            id,
            disposition,
            by,
            note,
        } => (
            "review_dispose",
            reviews
                .dispose(id, disposition, by, note)
                .await
                .map(Response::with_review_item),
        ),
        RequestBody::ReviewRecord {
            review,
            disposition,
            note,
        } => (
            "review_record",
            reviews
                .record(*review, disposition, note)
                .await
                .map(Response::with_review_item),
        ),
        _ => return ("review", Response::err("not a review request")),
    };
    match result {
        (kind, Ok(response)) => (kind, response),
        (kind, Err(error)) => (kind, Response::err(error.to_string())),
    }
}

/// Human requests from every room are an operator-console read, the same
/// rule `collaboration_list` applies past the caller's own mailbox.
fn operator_only(
    origin: Option<&crate::collaboration::CollaborationOrigin>,
) -> Option<crate::collaboration::CollaborationError> {
    (!origin.is_some_and(|origin| origin.console))
        .then_some(crate::collaboration::CollaborationError::ScopeDenied)
}

/// Serve `decision_queue` and `decision_history`. A source that cannot be
/// read is named in `unavailable` rather than failing the whole read, so
/// the person still sees what the other sources hold.
pub(super) async fn decision_response(
    reviews: &ReviewStore,
    collaboration: &crate::collaboration::CollaborationStore,
    store: &super::SharedStore,
    body: RequestBody,
) -> (&'static str, Response) {
    match body {
        RequestBody::DecisionQueue { limit, origin } => {
            let operator = operator_only(origin.as_ref()).is_none();
            let agents = store.decision_agents().await;
            let queue = crate::decision_queue::read_pending(
                reviews,
                collaboration,
                &agents,
                limit,
                operator,
            )
            .await;
            (
                "decision_queue",
                Response {
                    decision_queue: Some(Box::new(queue)),
                    ..Response::ok()
                },
            )
        }
        RequestBody::DecisionHistory {
            limit,
            origin,
            include_closures,
        } => {
            let operator = operator_only(origin.as_ref()).is_none();
            let history = crate::decision_queue::read_history(
                reviews,
                collaboration,
                limit,
                operator,
                include_closures,
            )
            .await;
            (
                "decision_history",
                Response {
                    decision_history: Some(Box::new(history)),
                    ..Response::ok()
                },
            )
        }
        RequestBody::SystemAskCleanup { apply, origin } => {
            // Declared, not authenticated (see the request's docs): the
            // safety is in what the rules may close, and the ledger names
            // the observed caller.
            if let Some(refused) = operator_only(origin.as_ref()) {
                return ("system_ask_cleanup", Response::err(refused.to_string()));
            }
            let agents = store.snapshot().await;
            let response = match collaboration.cleanup_system_asks(&agents, !apply).await {
                Ok(cleanup) => Response {
                    system_ask_cleanup: Some(Box::new(cleanup)),
                    ..Response::ok()
                },
                Err(error) => Response::err(error.to_string()),
            };
            ("system_ask_cleanup", response)
        }
        body @ (RequestBody::DecisionKeep { .. }
        | RequestBody::DecisionCloseUnanswerable { .. }) => {
            Box::pin(subject_response(collaboration, store, body)).await
        }
        _ => ("decision", Response::err("not a decision request")),
    }
}

/// Serve `decision_keep` and `decision_close_unanswerable`
/// (`subject_check.rs`): operator-console actions, like the cleanup.
async fn subject_response(
    collaboration: &crate::collaboration::CollaborationStore,
    store: &super::SharedStore,
    body: RequestBody,
) -> (&'static str, Response) {
    match body {
        RequestBody::DecisionKeep {
            request_id,
            episode,
            origin,
        } => {
            if let Some(refused) = operator_only(origin.as_ref()) {
                return ("decision_keep", Response::err(refused.to_string()));
            }
            let response = match collaboration.keep_unanswerable(&request_id, &episode).await {
                Ok(kept) => Response {
                    decision_keep: Some(Box::new(kept)),
                    ..Response::ok()
                },
                Err(error) => Response::err(error.to_string()),
            };
            ("decision_keep", response)
        }
        RequestBody::DecisionCloseUnanswerable {
            items,
            origin,
            by_label,
        } => {
            if let Some(refused) = operator_only(origin.as_ref()) {
                return (
                    "decision_close_unanswerable",
                    Response::err(refused.to_string()),
                );
            }
            let agents = store.snapshot().await;
            let surface = by_label.as_deref().unwrap_or("cli");
            let response = match collaboration
                .close_unanswerable(&items, &agents, surface, None)
                .await
            {
                Ok(outcome) => Response {
                    decision_close_unanswerable: Some(Box::new(outcome)),
                    ..Response::ok()
                },
                Err(error) => Response::err(error.to_string()),
            };
            ("decision_close_unanswerable", response)
        }
        _ => ("decision", Response::err("not a decision request")),
    }
}
