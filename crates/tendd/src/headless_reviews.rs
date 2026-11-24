use crate::headless_result_journal::{Journal, MAX_ANSWER_BYTES};
use std::fs::{File, OpenOptions};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tend::collaboration::{
    headless::{HeadlessAttempt, HeadlessPause, HeadlessPhase},
    CollaborationRequest, CollaborationStore,
};
use tokio::sync::broadcast;
use uuid::Uuid;

pub(super) async fn start(
    collaboration: Arc<CollaborationStore>,
    shutdown: &broadcast::Sender<()>,
) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    anyhow::ensure!(
        cfg!(target_os = "linux") && Path::new("/usr/bin/flock").is_file(),
        "headless reviews require a Linux provider lifetime lock"
    );
    let path = tend::paths::default_collaboration_file()
        .ok_or_else(|| anyhow::anyhow!("headless review mailbox directory is unavailable"))?;
    let root = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("headless mailbox parent is unavailable"))?;
    // The daemon owner and the provider process use independent locks.
    let owner = lock_file(&path.with_extension("headless-review.lock"))?;
    let gate_dir = root.to_owned();
    let process_lock = path.with_extension("headless-provider.lock");
    let mut shutdown = shutdown.subscribe();
    Ok(tokio::spawn(async move {
        let _owner = owner;
        collaboration.set_headless_reviews_enabled(true);
        tokio::select! {
            biased;
            _ = shutdown.recv() => {}
            () = async {
                let journal = retry(&collaboration, HeadlessPause::ResultJournal, || async {
                    Journal::open(&gate_dir)
                })
                .await;
                Runner { journal, process_lock, gate_dir }.serve(&collaboration).await;
            } => {}
        }
        collaboration.set_headless_reviews_enabled(false);
    }))
}

struct Runner {
    journal: Journal,
    process_lock: PathBuf,
    gate_dir: PathBuf,
}

fn lock_file(path: &Path) -> anyhow::Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)?;
    file.try_lock()?;
    Ok(file)
}

impl Runner {
    fn gate(&self, generation: Uuid) -> PathBuf {
        self.gate_dir
            .join(format!("headless-launch-{generation}.permit"))
    }

    async fn serve(&self, collaboration: &CollaborationStore) {
        let mut changes = collaboration.subscribe();
        self.recover(collaboration).await;
        loop {
            let request = retry(collaboration, HeadlessPause::ReservationStorage, || async {
                Ok(collaboration.reserve_headless_review().await?)
            })
            .await;
            if let Some(request) = request {
                self.run(collaboration, &request).await;
            } else if changes.changed().await.is_err() {
                return;
            }
        }
    }

    async fn acquire_process_lock(&self, collaboration: &CollaborationStore) -> File {
        retry(collaboration, HeadlessPause::PreviousProcess, || async {
            lock_file(&self.process_lock)
        })
        .await
    }

    async fn recover(&self, collaboration: &CollaborationStore) {
        // Hold this across permit revocation and recovery. A launcher from
        // the dead daemon either already holds it, or sees its permit gone.
        let _process = self.acquire_process_lock(collaboration).await;
        for (id, attempt) in collaboration.claimed_headless_attempts().await {
            wait_for_legacy_process(collaboration, &attempt).await;
            let generation = attempt.dispatch_generation;
            self.revoke_gate(collaboration, generation).await;
            let sealed = retry(collaboration, HeadlessPause::ResultJournal, || async {
                self.journal.read(&id, generation)
            })
            .await;
            let had_record = sealed.is_some();
            self.commit(collaboration, &id, generation, sealed.flatten(), had_record)
                .await;
        }
    }

    async fn revoke_gate(&self, collaboration: &CollaborationStore, generation: Uuid) {
        let path = self.gate(generation);
        retry(collaboration, HeadlessPause::ResultJournal, || async {
            match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.into()),
            }
        })
        .await;
    }

    async fn commit(
        &self,
        collaboration: &CollaborationStore,
        id: &str,
        generation: Uuid,
        answer: Option<String>,
        had_record: bool,
    ) {
        let committed = retry(collaboration, HeadlessPause::ResultStorage, || {
            let answer = answer.clone();
            async move {
                Ok(collaboration
                    .finish_headless_review(id, generation, answer)
                    .await?)
            }
        })
        .await;
        if !committed {
            // Preserve an older generation's sealed output for audit.
            tracing::warn!(request_id = id, %generation, "headless result no longer matches its reservation; retaining journal");
            return;
        }
        if had_record {
            if let Err(error) = self.journal.remove(id, generation) {
                tracing::warn!(request_id = id, %generation, %error, "committed headless result journal could not be removed");
            }
        }
    }

    async fn run(&self, collaboration: &CollaborationStore, request: &CollaborationRequest) {
        let attempt = request
            .headless_attempt
            .as_ref()
            .expect("durable reservation");
        debug_assert_eq!(attempt.phase, HeadlessPhase::RunnerAccepted);
        let generation = attempt.dispatch_generation;
        let gate = self.gate(generation);
        retry(collaboration, HeadlessPause::ResultJournal, || async {
            // A generation can be launched only after its SQLite reservation.
            OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&gate)?;
            Ok(())
        })
        .await;
        let (answer, trailing_events) = run_provider(
            collaboration,
            request,
            generation,
            &self.process_lock,
            &gate,
        )
        .await;
        let answer = answer.filter(|answer| answer.len() <= MAX_ANSWER_BYTES);
        // Seal before trying SQLite. A write failure here pauses this exact
        // attempt with its answer in memory; it never runs the provider again.
        retry(collaboration, HeadlessPause::ResultJournal, || async {
            self.journal
                .write(&request.id, generation, answer.as_deref())
        })
        .await;
        for event in trailing_events {
            record(collaboration, request, generation, event).await;
        }
        // A timeout can return before the child has actually released its lock.
        let _process = self.acquire_process_lock(collaboration).await;
        self.revoke_gate(collaboration, generation).await;
        self.commit(collaboration, &request.id, generation, answer, true)
            .await;
    }
}

async fn wait_for_legacy_process(collaboration: &CollaborationStore, attempt: &HeadlessAttempt) {
    if !attempt.launch_fenced {
        retry(collaboration, HeadlessPause::PreviousProcess, || async {
            anyhow::ensure!(
                attempt
                    .process_identity
                    .as_ref()
                    .is_some_and(|identity| identity.exit_confirmed()),
                "an older attempt has no proof that its provider exited"
            );
            Ok(())
        })
        .await;
    }
}

/// Keep trying only the durable operation; never retry provider invocation.
/// One bounded-delay loop reports a persistent failure and resumes when the
/// path works again. The supervisor can cancel it at daemon shutdown.
async fn retry<T, F: Future<Output = anyhow::Result<T>>>(
    collaboration: &CollaborationStore,
    reason: HeadlessPause,
    mut operation: impl FnMut() -> F,
) -> T {
    let mut delay = Duration::from_millis(100);
    loop {
        match operation().await {
            Ok(value) => {
                collaboration.set_headless_pause(None);
                return value;
            }
            Err(error) => {
                if collaboration.headless_pause() != Some(reason) {
                    tracing::warn!(reason = reason.description(), %error, "headless review queue paused");
                    collaboration.set_headless_pause(Some(reason));
                }
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(5));
            }
        }
    }
}

async fn run_provider(
    collaboration: &CollaborationStore,
    request: &CollaborationRequest,
    generation: Uuid,
    process_lock: &Path,
    gate: &Path,
) -> (Option<String>, Vec<tend::ask::HeadlessReviewEvent>) {
    let scratch = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(error) => {
            tracing::error!(request_id = request.id, %error, "could not create isolated review directory");
            return (None, Vec::new());
        }
    };
    let document = tend::collaboration::instruction::inbox_instruction(request);
    let prompt = format!(
        "Review only the evidence included in the request. This is an isolated text-only review: no repository files or tools are available. If the evidence is insufficient, say what is missing instead of inventing findings. Return your review as the final response.\n\n{document}"
    );
    let (events, mut observed) = tokio::sync::mpsc::unbounded_channel();
    let runner =
        tend::ask::run_headless_review(&prompt, scratch.path(), process_lock, gate, events);
    tokio::pin!(runner);
    let answer = loop {
        tokio::select! {
            Some(event) = observed.recv() => record(collaboration, request, generation, event).await,
            result = &mut runner => break result,
        }
    };
    let mut trailing_events = Vec::new();
    while let Ok(event) = observed.try_recv() {
        trailing_events.push(event);
    }
    (answer.ok().map(|answer| answer.text), trailing_events)
}

async fn record(
    collaboration: &CollaborationStore,
    request: &CollaborationRequest,
    generation: Uuid,
    event: tend::ask::HeadlessReviewEvent,
) {
    if let Err(error) = collaboration
        .observe_headless_review(&request.id, generation, event)
        .await
    {
        tracing::warn!(request_id = request.id, %error, "headless review evidence was not committed");
    }
}
