//! Per-batch broadcast channel for real-time progress streaming.
//!
//! [`BatchProgressBroadcaster`] maintains a registry of `tokio::sync::broadcast`
//! channels, one per in-progress batch. As individual updates within a batch
//! start, complete, or fail, the corresponding events are sent to all SSE
//! subscribers watching that batch.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, broadcast};
use uuid::Uuid;

/// Broadcast channel capacity per batch.
const CHANNEL_CAPACITY: usize = 256;

/// A single event emitted on a per-batch broadcast channel.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum BatchProgressEvent {
    /// An individual update within the batch was dispatched to an agent.
    UpdateDispatched {
        update_history_id: Uuid,
        software_item_name: String,
        host_name: String,
    },
    /// An individual update within the batch has started executing.
    UpdateStarted {
        update_history_id: Uuid,
        software_item_name: String,
        host_name: String,
    },
    /// An individual update within the batch completed successfully.
    UpdateCompleted {
        update_history_id: Uuid,
        software_item_name: String,
        host_name: String,
    },
    /// An individual update within the batch failed.
    UpdateFailed {
        update_history_id: Uuid,
        software_item_name: String,
        host_name: String,
        error: Option<String>,
    },
    /// Overall batch progress summary.
    Progress {
        completed: i64,
        failed: i64,
        pending: i64,
        total: i32,
    },
    /// The batch has reached a terminal status.
    BatchCompleted {
        status: String,
    },
}

/// Registry of per-batch broadcast channels for real-time progress streaming.
///
/// Thread-safe and cheaply cloneable (interior `Arc`).
#[derive(Clone)]
pub struct BatchProgressBroadcaster {
    channels: Arc<RwLock<HashMap<Uuid, broadcast::Sender<BatchProgressEvent>>>>,
}

impl BatchProgressBroadcaster {
    /// Create a new empty broadcaster.
    pub fn new() -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a broadcast channel for the given batch.
    pub async fn create_channel(&self, batch_id: Uuid) {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        self.channels.write().await.insert(batch_id, tx);
    }

    /// Send a progress event to all subscribers of the given batch.
    pub async fn send(&self, batch_id: Uuid, event: BatchProgressEvent) {
        let channels = self.channels.read().await;
        if let Some(tx) = channels.get(&batch_id) {
            let _ = tx.send(event);
        }
    }

    /// Send a batch completion event and remove the channel.
    pub async fn send_batch_completed(&self, batch_id: Uuid, status: String) {
        let mut channels = self.channels.write().await;
        if let Some(tx) = channels.remove(&batch_id) {
            let _ = tx.send(BatchProgressEvent::BatchCompleted { status });
        }
    }

    /// Subscribe to the broadcast channel for the given batch.
    ///
    /// Returns `None` if no channel exists.
    pub async fn subscribe(
        &self,
        batch_id: Uuid,
    ) -> Option<broadcast::Receiver<BatchProgressEvent>> {
        let channels = self.channels.read().await;
        channels.get(&batch_id).map(|tx| tx.subscribe())
    }
}

impl Default for BatchProgressBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_and_subscribe() {
        let broadcaster = BatchProgressBroadcaster::new();
        let id = Uuid::now_v7();

        broadcaster.create_channel(id).await;
        let rx = broadcaster.subscribe(id).await;
        assert!(rx.is_some());
    }

    #[tokio::test]
    async fn subscribe_nonexistent_returns_none() {
        let broadcaster = BatchProgressBroadcaster::new();
        let rx = broadcaster.subscribe(Uuid::now_v7()).await;
        assert!(rx.is_none());
    }

    #[tokio::test]
    async fn send_delivers_to_subscriber() {
        let broadcaster = BatchProgressBroadcaster::new();
        let id = Uuid::now_v7();

        broadcaster.create_channel(id).await;
        let mut rx = broadcaster.subscribe(id).await.unwrap();

        broadcaster
            .send(
                id,
                BatchProgressEvent::Progress {
                    completed: 1,
                    failed: 0,
                    pending: 2,
                    total: 3,
                },
            )
            .await;

        let event = rx.recv().await.unwrap();
        match event {
            BatchProgressEvent::Progress {
                completed, total, ..
            } => {
                assert_eq!(completed, 1);
                assert_eq!(total, 3);
            }
            _ => panic!("expected Progress event"),
        }
    }

    #[tokio::test]
    async fn batch_completed_removes_channel() {
        let broadcaster = BatchProgressBroadcaster::new();
        let id = Uuid::now_v7();

        broadcaster.create_channel(id).await;
        let mut rx = broadcaster.subscribe(id).await.unwrap();

        broadcaster
            .send_batch_completed(id, "completed".to_string())
            .await;

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, BatchProgressEvent::BatchCompleted { .. }));

        let rx2 = broadcaster.subscribe(id).await;
        assert!(rx2.is_none());
    }
}
