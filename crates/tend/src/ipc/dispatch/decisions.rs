//! Review items and the decision queue.

#![allow(clippy::wildcard_imports)] // the arms name the IPC module's items as `dispatch.rs` does

use super::*;

/// The ledger entry an operator's decision call leaves: a read of every
/// room's human requests goes in like a widened listing; closing tend's own
/// items in the operator's queue is its own operation.
fn decision_audit(body: &RequestBody) -> Option<CollaborationAuditContext> {
    match body {
        RequestBody::DecisionQueue { origin, limit }
        | RequestBody::DecisionHistory { origin, limit, .. } => origin
            .as_ref()
            .filter(|origin| origin.console)
            .map(|origin| {
                let mut context = CollaborationAuditContext::new(
                    CollaborationAuditOperation::List,
                    origin.clone(),
                );
                context.scope = Some(MailboxScope::All);
                context.read_view = Some(collaboration::CollaborationReadView::Full);
                context.query_limit = Some(*limit);
                context
            }),
        RequestBody::DecisionKeep {
            origin,
            request_id,
            via_fleet,
            ..
        } => origin
            .as_ref()
            .filter(|origin| origin.console)
            .map(|origin| {
                let mut context = CollaborationAuditContext::new(
                    CollaborationAuditOperation::DecisionKeep,
                    origin.clone(),
                );
                context.request_id = Some(request_id.clone());
                context.via_fleet.clone_from(via_fleet);
                context
            }),
        RequestBody::DecisionCloseUnanswerable {
            origin, via_fleet, ..
        } => origin
            .as_ref()
            .filter(|origin| origin.console)
            .map(|origin| {
                let mut context = CollaborationAuditContext::new(
                    CollaborationAuditOperation::DecisionCloseUnanswerable,
                    origin.clone(),
                );
                context.scope = Some(MailboxScope::All);
                context.via_fleet.clone_from(via_fleet);
                context
            }),
        RequestBody::SystemAskCleanup {
            apply: true,
            origin,
        } => origin
            .as_ref()
            .filter(|origin| origin.console)
            .map(|origin| {
                let mut context = CollaborationAuditContext::new(
                    CollaborationAuditOperation::SystemAskCleanup,
                    origin.clone(),
                );
                context.scope = Some(MailboxScope::All);
                context
            }),
        _ => None,
    }
}

pub(super) async fn handle(
    ctx: &ServerCtx,
    conn: &mut Conn,
    turn: &mut Turn,
    body: RequestBody,
) -> Result<Flow, RuntimeError> {
    let store = &ctx.store;
    let collaboration = &ctx.collaboration;
    let reviews = &ctx.reviews;
    let collaboration_actor = &mut conn.collaboration_actor;
    let kind = &mut turn.kind;
    let pending_audit = &mut turn.pending_audit;
    let response = match body {
        // Every way a result enters the store: an agent's result on a Work
        // with declared checks is verified by tend first, and a failure
        // keeps it from the person.
        RequestBody::ReviewCreate { mut review } => {
            let caller_is_agent = caller_is_agent(ctx, conn).await;
            *kind = "review_create";
            match Box::pin(verify_result(ctx, &mut review, caller_is_agent)).await {
                Ok(verification) => match reviews.create_verified(*review, verification).await {
                    Ok(item) => Response::with_review_item(item),
                    Err(error) => Response::err(error.to_string()),
                },
                Err(refusal) => Response::err(refusal),
            }
        }
        RequestBody::ReviewRecord {
            mut review,
            disposition,
            note,
        } => {
            // Normally the person recording what they decided; from an
            // agent's pane it is a result like any other.
            let caller_is_agent = caller_is_agent(ctx, conn).await;
            // A decided record keeps no verification: the person decided.
            match Box::pin(verify_result(ctx, &mut review, caller_is_agent)).await {
                Ok(_) => {
                    let (name, response) = Box::pin(review_response(
                        reviews,
                        RequestBody::ReviewRecord {
                            review,
                            disposition,
                            note,
                        },
                    ))
                    .await;
                    *kind = name;
                    response
                }
                Err(refusal) => {
                    *kind = "review_record";
                    Response::err(refusal)
                }
            }
        }
        body @ (RequestBody::ReviewGet { .. }
        | RequestBody::ReviewList { .. }
        | RequestBody::ReviewSummary
        | RequestBody::ReviewDispose { .. }) => {
            // One boxed arm keeps this dispatch frame from growing.
            let (name, response) = Box::pin(review_response(reviews, body)).await;
            *kind = name;
            response
        }
        body @ (RequestBody::DecisionQueue { .. }
        | RequestBody::DecisionHistory { .. }
        | RequestBody::SystemAskCleanup { .. }
        | RequestBody::DecisionKeep { .. }
        | RequestBody::DecisionCloseUnanswerable { .. }) => {
            let audit = decision_audit(&body);
            let (name, response) = Box::pin(super::review::decision_response(
                reviews,
                collaboration,
                store,
                body,
            ))
            .await;
            *kind = name;
            if let Some(context) = audit {
                *pending_audit = Some(collaboration_audit_entry(
                    collaboration_actor,
                    context,
                    &response,
                ));
            }
            response
        }
        body @ (RequestBody::DecisionKeepOnHost { .. }
        | RequestBody::DecisionCloseOnHost { .. }) => {
            return Ok(Flow::Respond(
                Box::pin(on_host(ctx, conn, turn, body)).await,
            ));
        }
        other => unreachable!("dispatch routed {other:?} to decisions"),
    };
    Ok(Flow::Respond(response))
}

/// `decision_keep_on_host` and `decision_close_on_host`: a person's Keep
/// or Close of another Fleet host's questions, sent from here
/// (`crate::fleet_decisions`). Only the person: the console origin, and a
/// caller that is not an agent, neither by the pane it runs in nor by any
/// agent process above it. The holding host applies it under its own rules
/// and records it with `via_fleet`; this host records the send.
async fn on_host(ctx: &ServerCtx, conn: &mut Conn, turn: &mut Turn, body: RequestBody) -> Response {
    let (operation, name, host, origin, request_id) = match &body {
        RequestBody::DecisionKeepOnHost {
            host,
            request_id,
            origin,
            ..
        } => (
            CollaborationAuditOperation::DecisionKeep,
            "decision_keep_on_host",
            host.clone(),
            origin.clone(),
            Some(request_id.clone()),
        ),
        RequestBody::DecisionCloseOnHost {
            host,
            request_id,
            origin,
            ..
        } => (
            CollaborationAuditOperation::DecisionCloseUnanswerable,
            "decision_close_on_host",
            host.clone(),
            origin.clone(),
            Some(request_id.clone()),
        ),
        _ => unreachable!("on_host takes only the on-host decisions"),
    };
    turn.kind = name;
    let Some(origin) = origin.filter(|origin| origin.console) else {
        return Response::err(collaboration::CollaborationError::ScopeDenied.to_string());
    };
    if let Some(refused) = Box::pin(refuse_agent_caller(ctx, conn)).await {
        return Response::err(refused);
    }
    let response = Box::pin(send_on_host(ctx, &host, body)).await;
    let mut context = CollaborationAuditContext::new(operation, origin);
    context.target = Some(format!("fleet:{host}"));
    context.request_id = request_id;
    turn.pending_audit = Some(collaboration_audit_entry(
        &conn.collaboration_actor,
        context,
        &response,
    ));
    response
}

/// Why this caller may not decide for the person over Fleet, if it may not.
async fn refuse_agent_caller(ctx: &ServerCtx, conn: &mut Conn) -> Option<String> {
    const REFUSED: &str = "only the person keeps or closes questions on another host: \
         run this from your own terminal, not from an agent, or use the dashboard";
    if caller_is_agent(ctx, conn).await {
        return Some(REFUSED.into());
    }
    let Some(pid) = conn.collaboration_actor.caller_pid else {
        return Some("could not tell who is asking; use the dashboard".into());
    };
    match tokio::task::spawn_blocking(move || {
        crate::adapters::proc_ancestry::agent_in_ancestry(pid)
    })
    .await
    {
        Ok(Some(false)) => None,
        Ok(Some(true)) => Some(REFUSED.into()),
        _ => Some("could not tell whether an agent runs this; use the dashboard".into()),
    }
}

async fn send_on_host(ctx: &ServerCtx, host: &str, body: RequestBody) -> Response {
    let Some(fleet) = &ctx.fleet else {
        return Response::err("fleet is not enabled in tendd");
    };
    let hosts = fleet.store.snapshot_selected(None).await.hosts;
    let Some(alias) = crate::fleet_decisions::remote_alias(Some(host), &hosts) else {
        return Response::err(format!(
            "{host} is this host: run the command without --host"
        ));
    };
    let user = std::env::var("USER").unwrap_or_default();
    let actor = if user.is_empty() {
        "cli".to_owned()
    } else {
        format!("cli:{user}")
    };
    let by = crate::fleet_decisions::Decider {
        actor: &actor,
        surface: "cli",
    };
    let (close, request_id, episode) = match body {
        RequestBody::DecisionKeepOnHost {
            request_id,
            episode,
            ..
        } => (false, request_id, episode),
        RequestBody::DecisionCloseOnHost {
            request_id,
            episode,
            ..
        } => (true, request_id, episode),
        _ => unreachable!("send_on_host takes only the on-host decisions"),
    };
    // The host's own refusal first, before its queue is read.
    if let Err(refusal) = crate::fleet_decisions::unanswerable_host(&hosts, alias, close) {
        return Response::err(refusal.to_string());
    }
    let episode = match episode {
        Some(episode) => episode,
        None => match crate::fleet_decisions::remote_episode(fleet, alias, &request_id).await {
            Ok(episode) => episode,
            Err(refusal) => return Response::err(refusal.to_string()),
        },
    };
    if close {
        let items = [collaboration::subject_check::CloseItem {
            request_id,
            episode,
        }];
        match crate::fleet_decisions::close_remote(fleet, &hosts, alias, &items, &by).await {
            Ok(outcome) => Response {
                decision_close_unanswerable: Some(Box::new(outcome)),
                ..Response::ok()
            },
            Err(refusal) => Response::err(refusal.to_string()),
        }
    } else {
        match crate::fleet_decisions::keep_remote(fleet, &hosts, alias, &request_id, &episode, &by)
            .await
        {
            Ok(kept) => Response {
                decision_keep: Some(Box::new(kept)),
                ..Response::ok()
            },
            Err(refusal) => Response::err(refusal.to_string()),
        }
    }
}

/// Run the declared checks of the Work an agent's result belongs to, and
/// put their outcome on the record. Any client claim that tend ran its
/// evidence is cleared, verified or not.
async fn verify_result(
    ctx: &ServerCtx,
    review: &mut crate::review::NewReview,
    caller_is_agent: bool,
) -> Result<Option<crate::work_verify::Verification>, String> {
    crate::work_verify::strip_claims(&mut review.decision);
    if !crate::work_verify::result_needs_checks(caller_is_agent, review.requested_by.kind) {
        return Ok(None);
    }
    let identity = match &review.target {
        crate::review::ReviewTarget::Work {
            workspace, work, ..
        } => Some(crate::work::WorkIdentity::new(
            workspace.clone(),
            work.clone(),
        )),
        crate::review::ReviewTarget::Request { request_id, .. } => ctx
            .collaboration
            .request_work(request_id)
            .await
            .map(|(workspace, work)| crate::work::WorkIdentity::new(workspace, work)),
        crate::review::ReviewTarget::Session { .. } => None,
    };
    let Some(identity) = identity else {
        return Ok(None);
    };
    match crate::work_verify::verify(&ctx.work_store, &identity).await {
        crate::work_verify::Gate::NoChecks => Ok(None),
        crate::work_verify::Gate::Passed(outcomes) => {
            // Also as evidence, for clients that render only the decision.
            crate::work_verify::attach(&mut review.decision, &outcomes);
            Ok(Some(crate::work_verify::Verification::new(outcomes)))
        }
        crate::work_verify::Gate::Failed(outcomes) => Err(crate::work_verify::refusal(&outcomes)),
    }
}

/// Whether tendd observes the caller running in a tracked agent's pane.
async fn caller_is_agent(ctx: &ServerCtx, conn: &mut Conn) -> bool {
    matches!(
        Box::pin(super::work_ops::declarer(ctx, conn)).await,
        crate::work_verify::Declarer::Agent { .. }
    )
}
