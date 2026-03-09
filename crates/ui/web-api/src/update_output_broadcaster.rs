//! Per-update broadcast channel for real-time output streaming.
//!
//! [`UpdateOutputBroadcaster`] maintains a registry of `tokio::sync::broadcast`
//! channels, one per in-progress update. As the service WebSocket handler
//! receives `UpdateOutput` messages, it calls [`UpdateOutputBroadcaster::send_line`]
//! to fan out the line to all SSE subscribers.

use std::collections::HashMap;
use std::sync::Arc;

use time::OffsetDateTime;
use tokio::sync::{RwLock, broadcast};
use uptrakit_internal_wire::OutputStreamType;
use uuid::Uuid;

/// Broadcast channel capacity per update. Slow consumers that fall behind by
/// more than this many messages will experience lag (dropped oldest messages).
const CHANNEL_CAPACITY: usize = 256;

/// A single event emitted on a per-update broadcast channel.
#[derive(Clone, Debug)]
pub enum BroadcastEvent {
    /// A new output line from the update process.
    Line {
        id: Uuid,
        text: String,
        stream: OutputStreamType,
        timestamp: OffsetDateTime,
        seq: u64,
    },
    /// The update has completed (successfully or with an error).
    Completed {
        status: String,
        error: Option<String>,
    },
    /// The update process appears to be waiting for stdin input.
    StdinAttention { hint: Option<String> },
}

/// Internal state for a single update's broadcast channel.
struct ChannelEntry {
    tx: broadcast::Sender<BroadcastEvent>,
    next_seq: u64,
}

/// Registry of per-update broadcast channels for real-time output streaming.
///
/// Thread-safe and cheaply cloneable (interior `Arc`).
#[derive(Clone)]
pub struct UpdateOutputBroadcaster {
    channels: Arc<RwLock<HashMap<Uuid, ChannelEntry>>>,
}

impl UpdateOutputBroadcaster {
    /// Create a new empty broadcaster.
    pub fn new() -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a broadcast channel for the given update.
    ///
    /// Called from `handle_update_started` after clearing old output lines.
    pub async fn create_channel(&self, update_history_id: Uuid) {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        let entry = ChannelEntry { tx, next_seq: 0 };
        self.channels.write().await.insert(update_history_id, entry);
    }

    /// Send an output line to all subscribers of the given update.
    ///
    /// Increments the internal sequence counter. If no subscribers are
    /// listening, the send is silently ignored.
    pub async fn send_line(
        &self,
        update_history_id: Uuid,
        line_id: Uuid,
        text: String,
        stream: OutputStreamType,
        timestamp: OffsetDateTime,
    ) {
        let mut channels = self.channels.write().await;
        let Some(entry) = channels.get_mut(&update_history_id) else {
            return;
        };
        let seq = entry.next_seq;
        entry.next_seq += 1;
        let event = BroadcastEvent::Line {
            id: line_id,
            text,
            stream,
            timestamp,
            seq,
        };
        // Ignore send errors -- no subscribers is fine.
        let _ = entry.tx.send(event);
    }

    /// Send a completion event and remove the channel.
    ///
    /// Called from `handle_update_result` before deleting output lines.
    pub async fn send_completed(
        &self,
        update_history_id: Uuid,
        status: String,
        error: Option<String>,
    ) {
        let mut channels = self.channels.write().await;
        if let Some(entry) = channels.remove(&update_history_id) {
            let event = BroadcastEvent::Completed { status, error };
            let _ = entry.tx.send(event);
        }
    }

    /// Send a stdin attention event to all subscribers of the given update.
    ///
    /// Called when the agent reports that the update process appears to be
    /// waiting for stdin input.
    pub async fn send_stdin_attention(&self, update_history_id: Uuid, hint: Option<String>) {
        let channels = self.channels.read().await;
        if let Some(entry) = channels.get(&update_history_id) {
            let _ = entry.tx.send(BroadcastEvent::StdinAttention { hint });
        }
    }

    /// Subscribe to the broadcast channel for the given update.
    ///
    /// Returns `None` if no channel exists (update not in progress or already
    /// completed).
    pub async fn subscribe(
        &self,
        update_history_id: Uuid,
    ) -> Option<broadcast::Receiver<BroadcastEvent>> {
        let channels = self.channels.read().await;
        channels
            .get(&update_history_id)
            .map(|entry| entry.tx.subscribe())
    }
}

impl Default for UpdateOutputBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_and_subscribe() {
        let broadcaster = UpdateOutputBroadcaster::new();
        let id = Uuid::now_v7();

        broadcaster.create_channel(id).await;
        let rx = broadcaster.subscribe(id).await;
        assert!(rx.is_some(), "subscribe should succeed after create");
    }

    #[tokio::test]
    async fn subscribe_nonexistent_returns_none() {
        let broadcaster = UpdateOutputBroadcaster::new();
        let id = Uuid::now_v7();

        let rx = broadcaster.subscribe(id).await;
        assert!(rx.is_none(), "subscribe should return None for unknown ID");
    }

    #[tokio::test]
    async fn send_line_delivers_to_subscriber() {
        let broadcaster = UpdateOutputBroadcaster::new();
        let id = Uuid::now_v7();
        let line_id = Uuid::now_v7();

        broadcaster.create_channel(id).await;
        let mut rx = broadcaster.subscribe(id).await.unwrap();

        let now = OffsetDateTime::now_utc();
        broadcaster
            .send_line(
                id,
                line_id,
                "hello\n".to_string(),
                OutputStreamType::Stdout,
                now,
            )
            .await;

        let event = rx.recv().await.unwrap();
        match event {
            BroadcastEvent::Line {
                id: recv_id,
                text,
                stream,
                seq,
                ..
            } => {
                assert_eq!(recv_id, line_id);
                assert_eq!(text, "hello\n");
                assert!(matches!(stream, OutputStreamType::Stdout));
                assert_eq!(seq, 0);
            }
            _ => panic!("expected Line event"),
        }
    }

    #[tokio::test]
    async fn completed_removes_channel() {
        let broadcaster = UpdateOutputBroadcaster::new();
        let id = Uuid::now_v7();

        broadcaster.create_channel(id).await;
        let mut rx = broadcaster.subscribe(id).await.unwrap();

        broadcaster
            .send_completed(id, "completed".to_string(), None)
            .await;

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, BroadcastEvent::Completed { .. }));

        // Channel should be removed.
        let rx2 = broadcaster.subscribe(id).await;
        assert!(rx2.is_none(), "channel should be removed after completed");
    }

    #[tokio::test]
    async fn seq_increments_monotonically() {
        let broadcaster = UpdateOutputBroadcaster::new();
        let id = Uuid::now_v7();

        broadcaster.create_channel(id).await;
        let mut rx = broadcaster.subscribe(id).await.unwrap();

        let now = OffsetDateTime::now_utc();
        for _ in 0..5 {
            broadcaster
                .send_line(
                    id,
                    Uuid::now_v7(),
                    "line\n".to_string(),
                    OutputStreamType::Stdout,
                    now,
                )
                .await;
        }

        for expected_seq in 0..5u64 {
            let event = rx.recv().await.unwrap();
            match event {
                BroadcastEvent::Line { seq, .. } => {
                    assert_eq!(seq, expected_seq);
                }
                _ => panic!("expected Line event"),
            }
        }
    }
}
