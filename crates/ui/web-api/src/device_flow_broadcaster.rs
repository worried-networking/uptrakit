//! Per-device-flow broadcast channel for real-time SSE notifications.
//!
//! [`DeviceFlowBroadcaster`] maintains a registry of `tokio::sync::broadcast`
//! channels, one per pending device authorization flow. When a user approves a
//! device flow in the browser, the approve handler calls [`notify_status_changed`]
//! which wakes the SSE stream waiting on the same device code hash.
//!
//! Follows the same pattern as [`UpdateOutputBroadcaster`] and
//! [`BatchProgressBroadcaster`].
//!
//! [`UpdateOutputBroadcaster`]: crate::update_output_broadcaster::UpdateOutputBroadcaster
//! [`BatchProgressBroadcaster`]: crate::batch_progress_broadcaster::BatchProgressBroadcaster
//! [`notify_status_changed`]: DeviceFlowBroadcaster::notify_status_changed

#![expect(
    clippy::let_underscore_must_use,
    reason = "fire-and-forget channel sends intentionally drop the send result"
)]

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{RwLock, broadcast};

/// Broadcast channel capacity per device flow. Only a few events per flow
/// (status changed, expired), so a small capacity suffices.
const CHANNEL_CAPACITY: usize = 4;

/// Event types for a device flow broadcast channel.
#[derive(Clone, Debug)]
pub enum DeviceFlowEvent {
    /// The device flow was approved by a user.
    StatusChanged,
    /// The device flow expired before approval.
    Expired,
}

/// Registry of per-device-flow broadcast channels for real-time SSE delivery.
///
/// Thread-safe and cheaply cloneable (interior `Arc`).
#[derive(Clone)]
pub struct DeviceFlowBroadcaster {
    channels: Arc<RwLock<HashMap<String, broadcast::Sender<DeviceFlowEvent>>>>,
}

impl DeviceFlowBroadcaster {
    /// Create a new empty broadcaster.
    pub fn new() -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a broadcast channel for the given device code hash.
    ///
    /// Called from `device_auth_start` after creating the device flow in the DB.
    pub async fn create_channel(&self, device_code_hash: &str) {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        self.channels
            .write()
            .await
            .insert(device_code_hash.to_string(), tx);
    }

    /// Subscribe to the broadcast channel for the given device code hash.
    ///
    /// Returns `None` if no channel exists (flow not started or already consumed).
    pub async fn subscribe(
        &self,
        device_code_hash: &str,
    ) -> Option<broadcast::Receiver<DeviceFlowEvent>> {
        let channels = self.channels.read().await;
        channels.get(device_code_hash).map(|tx| tx.subscribe())
    }

    /// Notify subscribers that the device flow status changed (approved).
    ///
    /// Called from `device_auth_approve` after successfully updating the DB.
    pub async fn notify_status_changed(&self, device_code_hash: &str) {
        let channels = self.channels.read().await;
        if let Some(tx) = channels.get(device_code_hash) {
            let _ = tx.send(DeviceFlowEvent::StatusChanged);
        }
    }

    /// Notify subscribers that the device flow expired.
    pub async fn notify_expired(&self, device_code_hash: &str) {
        let channels = self.channels.read().await;
        if let Some(tx) = channels.get(device_code_hash) {
            let _ = tx.send(DeviceFlowEvent::Expired);
        }
    }

    /// Remove the broadcast channel for the given device code hash.
    ///
    /// Called after the flow is consumed or expires, to free memory.
    pub async fn remove_channel(&self, device_code_hash: &str) {
        self.channels.write().await.remove(device_code_hash);
    }
}

impl Default for DeviceFlowBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_and_subscribe() {
        let broadcaster = DeviceFlowBroadcaster::new();
        let hash = "abc123";

        broadcaster.create_channel(hash).await;
        let rx = broadcaster.subscribe(hash).await;
        assert!(rx.is_some(), "subscribe should succeed after create");
    }

    #[tokio::test]
    async fn subscribe_nonexistent_returns_none() {
        let broadcaster = DeviceFlowBroadcaster::new();
        let rx = broadcaster.subscribe("nonexistent").await;
        assert!(
            rx.is_none(),
            "subscribe should return None for unknown hash"
        );
    }

    #[tokio::test]
    async fn notify_delivers_to_subscriber() {
        let broadcaster = DeviceFlowBroadcaster::new();
        let hash = "abc123";

        broadcaster.create_channel(hash).await;
        let mut rx = broadcaster.subscribe(hash).await.unwrap();

        broadcaster.notify_status_changed(hash).await;

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, DeviceFlowEvent::StatusChanged));
    }

    #[tokio::test]
    async fn notify_expired_delivers_to_subscriber() {
        let broadcaster = DeviceFlowBroadcaster::new();
        let hash = "def456";

        broadcaster.create_channel(hash).await;
        let mut rx = broadcaster.subscribe(hash).await.unwrap();

        broadcaster.notify_expired(hash).await;

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, DeviceFlowEvent::Expired));
    }

    #[tokio::test]
    async fn remove_channel_cleans_up() {
        let broadcaster = DeviceFlowBroadcaster::new();
        let hash = "abc123";

        broadcaster.create_channel(hash).await;
        assert!(broadcaster.subscribe(hash).await.is_some());

        broadcaster.remove_channel(hash).await;
        assert!(
            broadcaster.subscribe(hash).await.is_none(),
            "channel should be removed"
        );
    }
}
