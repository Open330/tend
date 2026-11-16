//! Where the board's inputs come from in tendd: the daemon stores the IPC
//! handlers read (`pipeline_runs`, `automation_list`, `swap_health`,
//! `work_backlog`, review records), gathered once per board read.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::Mutex;

use super::{BoardHost, BoardInput};

const SWAP_HEALTH_TIMEOUT: Duration = Duration::from_millis(500);
/// Swap readiness is asked at most once a minute (the Mac's cadence), so a
/// slow health check never piles up blocking jobs.
const SWAP_READY_REUSE: Duration = Duration::from_secs(60);
/// How long one built board is served to every reader: several tabs (and
/// the apps) polling at once build it once.
pub const BOARD_REUSE: Duration = Duration::from_secs(2);

type Cached<T> = Arc<Mutex<Option<(Instant, T)>>>;

/// The daemon's stores the board reads besides agents and decisions.
#[derive(Clone, Default)]
pub struct HomeSources {
    pub pipeline_runs: Option<Arc<crate::pipeline_run::PipelineRunStore>>,
    pub automation: Option<Arc<crate::automation::AutomationStore>>,
    pub swap: Option<Arc<dyn crate::swap_api::SwapJobs>>,
    pub work_store: Option<crate::dashboard::work_store::SharedWorkStore>,
    swap_ready: Cached<Option<bool>>,
    /// The last board built and when.
    pub last_board: Cached<Value>,
}

impl HomeSources {
    #[must_use]
    pub fn new(
        pipeline_runs: Option<Arc<crate::pipeline_run::PipelineRunStore>>,
        automation: Option<Arc<crate::automation::AutomationStore>>,
        swap: Option<Arc<dyn crate::swap_api::SwapJobs>>,
        work_store: Option<crate::dashboard::work_store::SharedWorkStore>,
    ) -> Self {
        Self {
            pipeline_runs,
            automation,
            swap,
            work_store,
            ..Self::default()
        }
    }
}

impl std::fmt::Debug for HomeSources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HomeSources")
            .field("pipeline_runs", &self.pipeline_runs.is_some())
            .field("automation", &self.automation.is_some())
            .field("swap", &self.swap.is_some())
            .field("work_store", &self.work_store.is_some())
            .finish_non_exhaustive()
    }
}

impl HomeSources {
    /// Everything the board needs besides the reviewed marks, read now.
    pub async fn gather(&self, hosts: Vec<BoardHost>, decisions: Vec<Value>) -> BoardInput {
        let now = time::OffsetDateTime::now_utc();
        let runs = match &self.pipeline_runs {
            Some(store) => store
                .list()
                .await
                .iter()
                .filter_map(|run| serde_json::to_value(run).ok())
                .collect(),
            None => Vec::new(),
        };
        let automation = match &self.automation {
            Some(store) => serde_json::to_value(store.views(now).await).ok(),
            None => None,
        };
        let backlog = match &self.work_store {
            Some(store) => {
                let mut records = store.lock().await.records();
                crate::work::sort_backlog(&mut records);
                records
                    .iter()
                    .filter_map(|record| serde_json::to_value(record).ok())
                    .collect()
            }
            None => Vec::new(),
        };
        BoardInput {
            hosts,
            runs,
            decisions,
            automation,
            swap_ready: self.swap_ready().await,
            reviewed: None,
            backlog,
            now: Some(now),
        }
    }

    async fn swap_ready(&self) -> Option<bool> {
        let swap = self.swap.clone()?;
        let mut cached = self.swap_ready.lock().await;
        if let Some((at, ready)) = *cached {
            if at.elapsed() < SWAP_READY_REUSE {
                return ready;
            }
        }
        let check = tokio::task::spawn_blocking(move || swap.health());
        let ready = match tokio::time::timeout(SWAP_HEALTH_TIMEOUT, check).await {
            Ok(Ok(health)) => serde_json::to_value(&health).ok().and_then(|v| {
                v.get("readiness")
                    .and_then(Value::as_str)
                    .map(|r| r == "ready")
            }),
            _ => None,
        };
        *cached = Some((Instant::now(), ready));
        ready
    }
}

/// Home's reviewed marks for exactly these keys (the board's finished
/// units), however many other review records exist; `None` when the store
/// cannot be read.
pub async fn accepted(
    reviews: &crate::review::ReviewStore,
    keys: Vec<String>,
) -> Option<HashSet<String>> {
    reviews.accepted_keys(keys).await.ok()
}
