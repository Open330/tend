use std::sync::Arc;
use tend::collaboration::{headless::HeadlessPhase, CollaborationRequest, CollaborationStore};
use tokio::sync::broadcast;

pub(super) async fn start(
    collaboration: Arc<CollaborationStore>,
    shutdown: &broadcast::Sender<()>,
) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    // Lock the default durable mailbox's runner before admitting requests.
    let path = tend::paths::default_collaboration_file()
        .ok_or_else(|| anyhow::anyhow!("headless review mailbox directory is unavailable"))?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_extension("headless-review.lock"))?;
    lock.try_lock()?;
    collaboration.recover_headless_reviews().await?;
    collaboration.set_headless_reviews_enabled(true);
    let mut changes = collaboration.subscribe();
    let mut shutdown = shutdown.subscribe();
    Ok(tokio::spawn(async move {
        let _lock = lock;
        loop {
            match collaboration.reserve_headless_review().await {
                Ok(Some(request)) => {
                    tokio::select! {
                        // Leave the reservation intact. Startup recovery marks
                        // an interrupted invocation uncertain and never replays it.
                        biased;
                        _ = shutdown.recv() => break,
                        () = run(&collaboration, &request) => {}
                    }
                }
                Ok(None) => {
                    tokio::select! {
                        _ = shutdown.recv() => break,
                        change = changes.changed() => if change.is_err() { break; },
                    }
                }
                Err(error) => {
                    tracing::error!(%error, "cannot reserve headless review; executor stopped");
                    break;
                }
            }
        }
        collaboration.set_headless_reviews_enabled(false);
    }))
}

async fn run(collaboration: &CollaborationStore, request: &CollaborationRequest) {
    let attempt = request
        .headless_attempt
        .as_ref()
        .expect("durable reservation");
    debug_assert_eq!(attempt.phase, HeadlessPhase::RunnerAccepted);
    let generation = attempt.dispatch_generation;
    let scratch = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(error) => {
            tracing::error!(request_id = request.id, %error, "could not create isolated review directory");
            let _ = collaboration
                .finish_headless_review(&request.id, generation, None)
                .await;
            return;
        }
    };
    let document = tend::collaboration::instruction::inbox_instruction(request);
    let prompt = format!(
        "Review only the evidence included in the request. This is an isolated text-only review: no repository files or tools are available. If the evidence is insufficient, say what is missing instead of inventing findings. Return your review as the final response.\n\n{document}"
    );
    let (events, mut observed) = tokio::sync::mpsc::unbounded_channel();
    let runner = tend::ask::run_headless_review(&prompt, scratch.path(), events);
    tokio::pin!(runner);
    let answer = loop {
        tokio::select! {
            Some(event) = observed.recv() => record(collaboration, request, generation, event).await,
            result = &mut runner => break result,
        }
    };
    while let Ok(event) = observed.try_recv() {
        record(collaboration, request, generation, event).await;
    }
    if let Err(error) = collaboration
        .finish_headless_review(
            &request.id,
            generation,
            answer.ok().map(|answer| answer.text),
        )
        .await
    {
        tracing::error!(request_id = request.id, %error, "headless review result was not committed");
    }
}

async fn record(
    collaboration: &CollaborationStore,
    request: &CollaborationRequest,
    generation: uuid::Uuid,
    event: tend::ask::HeadlessReviewEvent,
) {
    if let Err(error) = collaboration
        .observe_headless_review(&request.id, generation, event)
        .await
    {
        tracing::warn!(request_id = request.id, %error, "headless review evidence was not committed");
    }
}
