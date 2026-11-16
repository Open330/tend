//! Where the board's inputs come from in tendd: the daemon stores the IPC
//! handlers read (`pipeline_runs`, `automation_list`, `swap_health`,
//! `work_backlog`, `review_list`), gathered once per board read.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use super::{reviewed_keys, BoardHost, BoardInput};

/// How many of the newest review items are scanned for Home's reviewed
/// marks (the Mac reads at most 10 pages of 500 back to its oldest finished
/// unit; a board read keeps the same bound).
const REVIEWED_SCAN: usize = 5000;
const SWAP_HEALTH_TIMEOUT: Duration = Duration::from_millis(500);

/// The daemon's stores the board reads besides agents and decisions.
#[derive(Clone, Default)]
pub struct HomeSources {
    pub pipeline_runs: Option<Arc<crate::pipeline_run::PipelineRunStore>>,
    pub automation: Option<Arc<crate::automation::AutomationStore>>,
    pub swap: Option<Arc<dyn crate::swap_api::SwapJobs>>,
    pub work_store: Option<crate::dashboard::work_store::SharedWorkStore>,
}

impl std::fmt::Debug for HomeSources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HomeSources")
            .field("pipeline_runs", &self.pipeline_runs.is_some())
            .field("automation", &self.automation.is_some())
            .field("swap", &self.swap.is_some())
            .field("work_store", &self.work_store.is_some())
            .finish()
    }
}

impl HomeSources {
    /// Everything the board needs, read now.
    pub async fn gather(
        &self,
        hosts: Vec<BoardHost>,
        decisions: Vec<Value>,
        reviews: &crate::review::ReviewStore,
    ) -> BoardInput {
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
        let swap_ready = match self.swap.clone() {
            Some(swap) => {
                let check = tokio::task::spawn_blocking(move || swap.health());
                match tokio::time::timeout(SWAP_HEALTH_TIMEOUT, check).await {
                    Ok(Ok(health)) => serde_json::to_value(&health).ok().and_then(|v| {
                        v.get("readiness")
                            .and_then(Value::as_str)
                            .map(|r| r == "ready")
                    }),
                    _ => None,
                }
            }
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
            swap_ready,
            reviewed: read_reviewed(reviews).await,
            backlog,
            now: Some(now),
        }
    }
}

async fn read_reviewed(reviews: &crate::review::ReviewStore) -> Option<HashSet<String>> {
    let page = reviews
        .list(crate::review::ReviewQuery {
            open_only: false,
            oldest_first: false,
            limit: REVIEWED_SCAN,
            offset: 0,
        })
        .await
        .ok()?;
    let items: Vec<Value> = page
        .items
        .iter()
        .filter_map(|item| serde_json::to_value(item).ok())
        .collect();
    Some(reviewed_keys(&items))
}
