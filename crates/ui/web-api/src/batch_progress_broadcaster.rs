//! Per-batch broadcast channel for real-time progress streaming.
//!
//! [`BatchProgressBroadcaster`] maintains a registry of `tokio::sync::broadcast`
//! channels, one per in-progress batch. As individual updates within a batch
//! start, complete, or fail, the corresponding events are sent to all SSE
//! subscribers watching that batch.
//!
//! ## Multi-instance support (NATS)
//!
//! When the `nats` feature is enabled and a NATS client is attached via
//! [`BatchProgressBroadcaster::with_nats`], every event is also published to the
//! ephemeral core-NATS subject `uptrakit.batch_progress.<batch_id>`.  SSE clients
//! connected to a controller instance that did not originate the batch can
//! subscribe to that subject as a fallback when no local broadcast channel exists.
//!
//! Core NATS publish/subscribe (not JetStream) is used intentionally — batch
//! progress events are transient and must not be persisted.

#![expect(
    clippy::let_underscore_must_use,
    reason = "fire-and-forget channel sends intentionally drop the send result"
)]
#![expect(
    clippy::allow_attributes,
    reason = "feature-conditional #[allow] for unreachable_code; #[expect] would be unfulfilled when nats feature is enabled"
)]

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
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
    BatchCompleted { status: String },
}

/// Registry of per-batch broadcast channels for real-time progress streaming.
///
/// Thread-safe and cheaply cloneable (interior `Arc`).
#[derive(Clone)]
pub struct BatchProgressBroadcaster {
    channels: Arc<RwLock<HashMap<Uuid, broadcast::Sender<BatchProgressEvent>>>>,
    /// Optional NATS client for cross-instance event fan-out.
    ///
    /// When present, every call to [`send`](Self::send) and
    /// [`send_batch_completed`](Self::send_batch_completed) also publishes the
    /// serialised event to the ephemeral NATS subject
    /// `uptrakit.batch_progress.<batch_id>` so that SSE clients connected to
    /// other controller instances can receive the event.
    #[cfg(feature = "nats")]
    nats_client: Option<async_nats::Client>,
}

impl BatchProgressBroadcaster {
    /// Create a new empty broadcaster (no NATS, single-instance mode).
    pub fn new() -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            #[cfg(feature = "nats")]
            nats_client: None,
        }
    }

    /// Attach a NATS client for cross-instance event fan-out.
    ///
    /// Returns a new `BatchProgressBroadcaster` that publishes every progress
    /// event to core NATS in addition to the local broadcast channel.  Events
    /// are published to `uptrakit.batch_progress.<batch_id>` (not JetStream).
    #[cfg(feature = "nats")]
    #[must_use]
    pub fn with_nats(mut self, client: async_nats::Client) -> Self {
        self.nats_client = Some(client);
        self
    }

    /// Returns `true` when a NATS client has been attached.
    #[allow(
        unreachable_code,
        reason = "fallback false is unreachable when nats feature is enabled"
    )]
    pub fn has_nats(&self) -> bool {
        #[cfg(feature = "nats")]
        {
            return self.nats_client.is_some();
        }
        // Without the nats feature there is no NATS client field; always false.
        false
    }

    /// Create a broadcast channel for the given batch.
    pub async fn create_channel(&self, batch_id: Uuid) {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        self.channels.write().insert(batch_id, tx);
    }

    /// Send a progress event to all local subscribers of the given batch.
    ///
    /// When a NATS client is configured the event is also published to the
    /// ephemeral core-NATS subject `uptrakit.batch_progress.<batch_id>` so
    /// that SSE clients on other controller instances receive it.
    pub async fn send(&self, batch_id: Uuid, event: BatchProgressEvent) {
        // Local broadcast.
        {
            let channels = self.channels.read();
            if let Some(tx) = channels.get(&batch_id) {
                let _ = tx.send(event.clone());
            }
        }

        // Cross-instance NATS publish (best-effort; failures are logged only).
        #[cfg(feature = "nats")]
        if let Some(ref client) = self.nats_client {
            self.publish_nats(client, batch_id, &event).await;
        }
    }

    /// Send a batch completion event, remove the local channel, and publish to
    /// NATS when configured.
    pub async fn send_batch_completed(&self, batch_id: Uuid, status: String) {
        let event = BatchProgressEvent::BatchCompleted { status };

        // Local broadcast (removes channel atomically).
        {
            let mut channels = self.channels.write();
            if let Some(tx) = channels.remove(&batch_id) {
                let _ = tx.send(event.clone());
            }
        }

        // Cross-instance NATS publish.
        #[cfg(feature = "nats")]
        if let Some(ref client) = self.nats_client {
            self.publish_nats(client, batch_id, &event).await;
        }
    }

    /// Publish `event` to the ephemeral core-NATS subject for `batch_id`.
    ///
    /// Failures are logged at `WARN` level and never propagate to callers —
    /// NATS delivery is best-effort for progress events.
    #[cfg(feature = "nats")]
    async fn publish_nats(
        &self,
        client: &async_nats::Client,
        batch_id: Uuid,
        event: &BatchProgressEvent,
    ) {
        let subject = uptrakit_nats::subjects::batch_progress(&batch_id);
        match serde_json::to_vec(event) {
            Ok(payload) => {
                if let Err(e) = client.publish(subject.clone(), payload.into()).await {
                    tracing::warn!(
                        batch_id = %batch_id,
                        subject,
                        error = %e,
                        "failed to publish batch progress event to NATS"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    batch_id = %batch_id,
                    error = %e,
                    "failed to serialize batch progress event for NATS"
                );
            }
        }
    }

    /// Subscribe to the local broadcast channel for the given batch.
    ///
    /// Returns `None` if no local channel exists (batch running on another
    /// instance).  In that case callers with NATS access should fall back to
    /// [`subscribe_nats`](Self::subscribe_nats).
    pub async fn subscribe(
        &self,
        batch_id: Uuid,
    ) -> Option<broadcast::Receiver<BatchProgressEvent>> {
        let channels = self.channels.read();
        channels.get(&batch_id).map(|tx| tx.subscribe())
    }

    /// Subscribe to the NATS subject for the given batch.
    ///
    /// Returns `Some(Subscriber)` when a NATS client is configured, or `None`
    /// when running in single-instance mode (no NATS).  The subscriber receives
    /// serialised [`BatchProgressEvent`] JSON published by the origin instance.
    ///
    /// Errors creating the subscription are logged and returned as `None`.
    #[cfg(feature = "nats")]
    pub async fn subscribe_nats(&self, batch_id: Uuid) -> Option<async_nats::Subscriber> {
        let client = self.nats_client.as_ref()?;
        let subject = uptrakit_nats::subjects::batch_progress(&batch_id);
        match client.subscribe(subject.clone()).await {
            Ok(sub) => Some(sub),
            Err(e) => {
                tracing::warn!(
                    batch_id = %batch_id,
                    subject,
                    error = %e,
                    "failed to subscribe to NATS batch progress subject"
                );
                None
            }
        }
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
