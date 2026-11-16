use super::{Client, RuntimeError, PROTOCOL_VERSION};
use crate::work_queue::{QueueAddRequest, QueueEditRequest, QueueItem, QueueSnapshot};

/// `work_queue_v1` requests. Every change returns the item as it is now.
impl Client {
    /// The queue in order; finished items only with `include_finished`.
    pub async fn work_queue_list(
        &self,
        include_finished: bool,
    ) -> Result<QueueSnapshot, RuntimeError> {
        let response = self
            .call_checked(&serde_json::json!({
                "protocol": PROTOCOL_VERSION,
                "kind": "work_queue_list",
                "include_finished": include_finished,
            }))
            .await?;
        serde_json::from_value(response["work_queue"].clone()).map_err(RuntimeError::Json)
    }

    pub async fn work_queue_add(&self, item: &QueueAddRequest) -> Result<QueueItem, RuntimeError> {
        self.work_queue_item(serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "kind": "work_queue_add",
            "item": item,
        }))
        .await
    }

    pub async fn work_queue_edit(
        &self,
        id: &str,
        expected_revision: Option<u64>,
        edit: &QueueEditRequest,
    ) -> Result<QueueItem, RuntimeError> {
        self.work_queue_item(serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "kind": "work_queue_edit",
            "id": id,
            "expected_revision": expected_revision,
            "edit": edit,
        }))
        .await
    }

    /// Move a waiting item to `to`, a 0-based place among unfinished items.
    pub async fn work_queue_move(
        &self,
        id: &str,
        expected_revision: Option<u64>,
        to: usize,
    ) -> Result<QueueItem, RuntimeError> {
        self.work_queue_item(serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "kind": "work_queue_move",
            "id": id,
            "expected_revision": expected_revision,
            "to": to,
        }))
        .await
    }

    pub async fn work_queue_cancel(
        &self,
        id: &str,
        expected_revision: Option<u64>,
    ) -> Result<QueueItem, RuntimeError> {
        self.work_queue_item(serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "kind": "work_queue_cancel",
            "id": id,
            "expected_revision": expected_revision,
        }))
        .await
    }

    /// Forget an item. `ack_duplicate_risk` is needed when its attempt
    /// may have reached the agent: it releases that agent's fence.
    pub async fn work_queue_remove(
        &self,
        id: &str,
        ack_duplicate_risk: bool,
    ) -> Result<QueueItem, RuntimeError> {
        self.work_queue_item(serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "kind": "work_queue_remove",
            "id": id,
            "ack_duplicate_risk": ack_duplicate_risk,
        }))
        .await
    }

    /// Dispatch now, past the queue order and throttle but not past a
    /// safety gate. `allow_duplicate` is needed for an uncertain item.
    pub async fn work_queue_run_now(
        &self,
        id: &str,
        expected_revision: Option<u64>,
        allow_duplicate: bool,
    ) -> Result<QueueItem, RuntimeError> {
        self.work_queue_item(serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "kind": "work_queue_run_now",
            "id": id,
            "expected_revision": expected_revision,
            "allow_duplicate": allow_duplicate,
        }))
        .await
    }

    /// Settle an uncertain item: it was sent, or it should wait again
    /// (`ack_duplicate_risk` required: it may arrive twice).
    pub async fn work_queue_resolve(
        &self,
        id: &str,
        sent: bool,
        ack_duplicate_risk: bool,
    ) -> Result<QueueItem, RuntimeError> {
        self.work_queue_item(serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "kind": "work_queue_resolve",
            "id": id,
            "sent": sent,
            "ack_duplicate_risk": ack_duplicate_risk,
        }))
        .await
    }

    /// Forget finished items; returns how many.
    pub async fn work_queue_clear(&self) -> Result<usize, RuntimeError> {
        let response = self
            .call_checked(&serde_json::json!({
                "protocol": PROTOCOL_VERSION,
                "kind": "work_queue_clear",
            }))
            .await?;
        serde_json::from_value(response["work_queue_cleared"].clone()).map_err(RuntimeError::Json)
    }

    async fn work_queue_item(&self, request: serde_json::Value) -> Result<QueueItem, RuntimeError> {
        let response = self.call_checked(&request).await?;
        serde_json::from_value(response["work_queue_item"].clone()).map_err(RuntimeError::Json)
    }
}
