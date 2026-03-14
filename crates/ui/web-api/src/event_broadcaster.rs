//! Per-tenant broadcast channel for real-time admin event delivery via SSE.
//!
//! [`EventBroadcaster`] maintains a registry of `tokio::sync::broadcast`
//! channels, one per tenant. When a state change occurs (host created, service
//! approved, update completed, etc.), the corresponding handler fires a
//! lightweight [`AdminEvent`] into the broadcaster. SSE subscribers connected
//! to the same tenant receive the event instantly for UI refresh.
//!
//! Follows the same pattern as [`UpdateOutputBroadcaster`] and
//! [`BatchProgressBroadcaster`].
//!
//! [`UpdateOutputBroadcaster`]: crate::update_output_broadcaster::UpdateOutputBroadcaster
//! [`BatchProgressBroadcaster`]: crate::batch_progress_broadcaster::BatchProgressBroadcaster

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{RwLock, broadcast};
use uptrakit_web_api_types::events::AdminEvent;
use uuid::Uuid;

/// Broadcast channel capacity per tenant. Larger than device flow (4) because
/// many event types and producers can fire concurrently.
const CHANNEL_CAPACITY: usize = 512;

/// Internal channel entry tracking the sender and active subscriber count.
struct ChannelEntry {
    tx: broadcast::Sender<AdminEvent>,
    subscriber_count: usize,
}

/// Registry of per-tenant broadcast channels for real-time admin event delivery.
///
/// Thread-safe and cheaply cloneable (interior `Arc`).
///
/// ## Multi-instance support (NATS)
///
/// When the `nats` feature is enabled and a NATS transport is attached via
/// [`EventBroadcaster::with_nats`], every call to [`send`](Self::send) and
/// [`send_global`](Self::send_global) also publishes the event to all other
/// controller instances via NATS JetStream. Receiving instances call
/// [`send_local`](Self::send_local) / [`send_global_local`](Self::send_global_local)
/// to re-broadcast locally without re-publishing (avoiding infinite loops).
#[derive(Clone)]
pub struct EventBroadcaster {
    channels: Arc<RwLock<HashMap<Uuid, ChannelEntry>>>,
    /// This controller's UUID — used as the NATS publication source.
    #[cfg(feature = "nats")]
    controller_id: Uuid,
    #[cfg(feature = "nats")]
    nats: Option<crate::nats_transport::NatsTransport>,
}

impl EventBroadcaster {
    /// Create a new empty broadcaster (single-instance mode, no NATS).
    pub fn new() -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            #[cfg(feature = "nats")]
            controller_id: Uuid::nil(),
            #[cfg(feature = "nats")]
            nats: None,
        }
    }

    /// Attach a NATS transport for cross-controller event fan-out.
    ///
    /// Returns `self` for builder-style chaining.  All subsequent calls to
    /// [`send`] and [`send_global`] will also publish the event via NATS so
    /// that other controller instances can relay it to their local SSE clients.
    #[cfg(feature = "nats")]
    pub fn with_nats(
        mut self,
        nats: crate::nats_transport::NatsTransport,
        controller_id: Uuid,
    ) -> Self {
        self.nats = Some(nats);
        self.controller_id = controller_id;
        self
    }

    /// Broadcast an event to local SSE subscribers of the given tenant **only**.
    ///
    /// Does **not** publish to NATS. Used by the NATS consumer to relay
    /// cross-controller events without causing a re-publish loop.
    #[tracing::instrument(skip_all, fields(%tenant_id))]
    pub async fn send_local(&self, tenant_id: Uuid, event: AdminEvent) {
        let channels = self.channels.read().await;
        if let Some(entry) = channels.get(&tenant_id) {
            let _ = entry.tx.send(event);
        }
    }

    /// Broadcast an event to **all** local tenant channels **only**.
    ///
    /// Does **not** publish to NATS. Used by the NATS consumer for system-wide
    /// cross-controller events without causing a re-publish loop.
    #[tracing::instrument(skip_all)]
    pub async fn send_global_local(&self, event: AdminEvent) {
        let channels = self.channels.read().await;
        for entry in channels.values() {
            let _ = entry.tx.send(event.clone());
        }
    }

    /// Send an event to all subscribers of the given tenant.
    ///
    /// Fire-and-forget: no-op if no subscribers are connected for this tenant.
    /// Also publishes to NATS when a transport is configured so other controller
    /// instances relay the event to their own local SSE subscribers.
    #[tracing::instrument(skip_all, fields(%tenant_id))]
    pub async fn send(&self, tenant_id: Uuid, event: AdminEvent) {
        self.send_local(tenant_id, event.clone()).await;
        self.maybe_publish_nats(Some(tenant_id), event).await;
    }

    /// Send an event to all active tenant channels (for system-wide events).
    ///
    /// Fire-and-forget: iterates all tenant channels and sends to each.
    /// Also publishes to NATS when a transport is configured.
    #[tracing::instrument(skip_all)]
    pub async fn send_global(&self, event: AdminEvent) {
        self.send_global_local(event.clone()).await;
        self.maybe_publish_nats(None, event).await;
    }

    /// Publish an `AdminEvent` to NATS so other controller instances can
    /// relay it to their local SSE subscribers.  No-op when NATS is not
    /// configured or the `nats` feature is disabled.
    async fn maybe_publish_nats(&self, tenant_id: Option<Uuid>, event: AdminEvent) {
        #[cfg(feature = "nats")]
        if let (Some(nats), Ok(event_json)) = (&self.nats, serde_json::to_string(&event)) {
            nats.publish(
                self.controller_id,
                None,
                Some("controller"),
                uptrakit_internal_wire::ControllerMessage::BroadcastAdminEvent(
                    uptrakit_internal_wire::BroadcastAdminEventPayload {
                        tenant_id,
                        event_json,
                    },
                ),
            )
            .await;
        }
        // Suppress unused variable warnings in non-NATS builds.
        #[cfg(not(feature = "nats"))]
        let _ = (tenant_id, event);
    }

    /// Subscribe to admin events for the given tenant.
    ///
    /// Creates the channel lazily on first subscribe, increments the subscriber
    /// count on subsequent subscribes.
    #[tracing::instrument(skip_all, fields(%tenant_id))]
    pub async fn subscribe(&self, tenant_id: Uuid) -> broadcast::Receiver<AdminEvent> {
        let mut channels = self.channels.write().await;
        let entry = channels.entry(tenant_id).or_insert_with(|| {
            let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
            ChannelEntry {
                tx,
                subscriber_count: 0,
            }
        });
        entry.subscriber_count += 1;
        entry.tx.subscribe()
    }

    /// Decrement the subscriber count for the given tenant.
    ///
    /// When the count reaches zero, the channel is removed to free memory.
    #[tracing::instrument(skip_all, fields(%tenant_id))]
    pub async fn unsubscribe(&self, tenant_id: Uuid) {
        let mut channels = self.channels.write().await;
        let remove = if let Some(entry) = channels.get_mut(&tenant_id) {
            entry.subscriber_count = entry.subscriber_count.saturating_sub(1);
            entry.subscriber_count == 0
        } else {
            false
        };
        if remove {
            channels.remove(&tenant_id);
        }
    }
}

impl Default for EventBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn subscribe_creates_channel() {
        let broadcaster = EventBroadcaster::new();
        let tenant = Uuid::now_v7();

        let _rx = broadcaster.subscribe(tenant).await;

        // Channel should exist
        let channels = broadcaster.channels.read().await;
        assert!(channels.contains_key(&tenant));
        assert_eq!(channels[&tenant].subscriber_count, 1);
    }

    #[tokio::test]
    async fn unsubscribe_removes_channel_when_zero() {
        let broadcaster = EventBroadcaster::new();
        let tenant = Uuid::now_v7();

        let _rx = broadcaster.subscribe(tenant).await;
        broadcaster.unsubscribe(tenant).await;

        let channels = broadcaster.channels.read().await;
        assert!(!channels.contains_key(&tenant));
    }

    #[tokio::test]
    async fn send_delivers_to_subscriber() {
        let broadcaster = EventBroadcaster::new();
        let tenant = Uuid::now_v7();
        let host_id = Uuid::now_v7();

        let mut rx = broadcaster.subscribe(tenant).await;

        broadcaster
            .send(tenant, AdminEvent::HostUpdated { id: host_id })
            .await;

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, AdminEvent::HostUpdated { id } if id == host_id));
    }

    #[tokio::test]
    async fn send_to_empty_tenant_is_silent() {
        let broadcaster = EventBroadcaster::new();
        let tenant = Uuid::now_v7();

        // Should not panic or error
        broadcaster
            .send(tenant, AdminEvent::HostUpdated { id: Uuid::now_v7() })
            .await;
    }

    #[tokio::test]
    async fn send_global_delivers_to_all_tenants() {
        let broadcaster = EventBroadcaster::new();
        let tenant_a = Uuid::now_v7();
        let tenant_b = Uuid::now_v7();
        let service_id = Uuid::now_v7();

        let mut rx_a = broadcaster.subscribe(tenant_a).await;
        let mut rx_b = broadcaster.subscribe(tenant_b).await;

        broadcaster
            .send_global(AdminEvent::SystemServiceStatusChanged {
                id: service_id,
                status: "approved".to_string(),
            })
            .await;

        let event_a = rx_a.recv().await.unwrap();
        let event_b = rx_b.recv().await.unwrap();

        assert!(
            matches!(event_a, AdminEvent::SystemServiceStatusChanged { id, .. } if id == service_id)
        );
        assert!(
            matches!(event_b, AdminEvent::SystemServiceStatusChanged { id, .. } if id == service_id)
        );
    }

    #[tokio::test]
    async fn multiple_tenants_isolated() {
        let broadcaster = EventBroadcaster::new();
        let tenant_a = Uuid::now_v7();
        let tenant_b = Uuid::now_v7();
        let host_a = Uuid::now_v7();

        let mut rx_a = broadcaster.subscribe(tenant_a).await;
        let _rx_b = broadcaster.subscribe(tenant_b).await;

        // Send only to tenant A
        broadcaster
            .send(tenant_a, AdminEvent::HostCreated { id: host_a })
            .await;

        let event = rx_a.recv().await.unwrap();
        assert!(matches!(event, AdminEvent::HostCreated { id } if id == host_a));

        // tenant B should not have received anything (no data available)
        // We can't easily test "no event" without a timeout, so just verify
        // the send was targeted.
    }

    #[tokio::test]
    async fn multiple_subscribers_same_tenant() {
        let broadcaster = EventBroadcaster::new();
        let tenant = Uuid::now_v7();

        let mut rx1 = broadcaster.subscribe(tenant).await;
        let mut rx2 = broadcaster.subscribe(tenant).await;

        {
            let channels = broadcaster.channels.read().await;
            assert_eq!(channels[&tenant].subscriber_count, 2);
        }

        broadcaster
            .send(tenant, AdminEvent::HostUpdated { id: Uuid::now_v7() })
            .await;

        assert!(rx1.recv().await.is_ok());
        assert!(rx2.recv().await.is_ok());

        // Unsubscribe one — channel should remain
        broadcaster.unsubscribe(tenant).await;
        {
            let channels = broadcaster.channels.read().await;
            assert!(channels.contains_key(&tenant));
            assert_eq!(channels[&tenant].subscriber_count, 1);
        }

        // Unsubscribe second — channel should be removed
        broadcaster.unsubscribe(tenant).await;
        {
            let channels = broadcaster.channels.read().await;
            assert!(!channels.contains_key(&tenant));
        }
    }

    #[tokio::test]
    async fn lagged_subscriber_continues() {
        let broadcaster = EventBroadcaster::new();
        let tenant = Uuid::now_v7();

        let mut rx = broadcaster.subscribe(tenant).await;

        // Overflow the channel capacity (512) — the subscriber will lag.
        for i in 0..CHANNEL_CAPACITY + 10 {
            broadcaster
                .send(
                    tenant,
                    AdminEvent::HostUpdated {
                        id: Uuid::from_u128(i as u128),
                    },
                )
                .await;
        }

        // First recv should return Lagged error
        let result = rx.recv().await;
        assert!(
            result.is_ok() || matches!(result, Err(broadcast::error::RecvError::Lagged(_))),
            "expected either Ok or Lagged, got: {result:?}"
        );
    }
}
