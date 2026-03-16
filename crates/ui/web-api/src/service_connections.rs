use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use rand::Rng;
use time::OffsetDateTime;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uptrakit_internal_wire::Capability;
use uptrakit_internal_wire::ControllerMessage;
use uuid::Uuid;

/// Channel buffer size for push messages to connected services.
const PUSH_CHANNEL_CAPACITY: usize = 32;

/// Per-connection state for a connected service.
struct ServiceConnection {
    /// Channel for pushing messages to the connected service.
    sender: mpsc::Sender<ControllerMessage>,
    /// Token cancelled when this connection is superseded by a new registration
    /// for the same `service_id`, signalling the old WebSocket handler to exit.
    cancel_token: CancellationToken,
    /// Set of capabilities advertised by this service.
    capabilities: BTreeSet<Capability>,
    /// The `service_app_name` from the service record, used to fan out
    /// `ServiceConfigUpdated` to all instances of the same app.
    service_app_name: Option<String>,
    /// Timestamp when the connection was registered.
    connected_at: OffsetDateTime,
}

/// Interior state protected by the `RwLock`.
struct RegistryInner {
    /// Primary map: service_id -> connection state.
    connections: HashMap<Uuid, ServiceConnection>,
}

impl RegistryInner {
    fn new() -> Self {
        Self {
            connections: HashMap::new(),
        }
    }
}

/// Unified registry of connected services (agents and MQTT service instances).
///
/// The controller registers services when they connect via WebSocket and
/// unregisters them on disconnect. Admin actions (approve/reject) use
/// `send()` to push messages to connected services in real time.
#[derive(Clone)]
pub struct ServiceConnectionRegistry {
    inner: Arc<RwLock<RegistryInner>>,
}

impl Default for ServiceConnectionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceConnectionRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(RegistryInner::new())),
        }
    }

    // ---------------------------------------------------------------
    // Registration
    // ---------------------------------------------------------------

    /// Register a connected service and return a receiver for push messages
    /// plus a cancellation token that is triggered if the same `service_id`
    /// registers again (connection deduplication).
    ///
    /// The `capabilities` set describes what the service can do.
    ///
    /// `service_app_name` is stored per-connection so that
    /// [`broadcast_to_app_except`](Self::broadcast_to_app_except) can fan out
    /// `ServiceConfigUpdated` to all instances of the same service app.
    pub async fn register(
        &self,
        service_id: Uuid,
        capabilities: BTreeSet<Capability>,
        instance_id: Option<String>,
        max_tenants: Option<u32>,
        service_app_name: Option<String>,
    ) -> (mpsc::Receiver<ControllerMessage>, CancellationToken) {
        // instance_id and max_tenants are kept for API compatibility but no
        // longer stored — MQTT lease management has been removed.
        let _ = (instance_id, max_tenants);

        let (tx, rx) = mpsc::channel(PUSH_CHANNEL_CAPACITY);
        let cancel_token = CancellationToken::new();

        let conn = ServiceConnection {
            sender: tx,
            cancel_token: cancel_token.clone(),
            capabilities,
            service_app_name,
            connected_at: OffsetDateTime::now_utc(),
        };

        let mut guard = self.inner.write();
        if let Some(old) = guard.connections.remove(&service_id) {
            old.cancel_token.cancel();
            tracing::info!(%service_id, "cancelled superseded connection");
        }
        guard.connections.insert(service_id, conn);
        (rx, cancel_token)
    }

    /// Remove a service from the registry on disconnect.
    pub async fn unregister(&self, service_id: &Uuid) {
        self.inner.write().connections.remove(service_id);
    }

    /// Force-disconnect a service by cancelling its connection token and
    /// removing it from the registry.
    ///
    /// The handler loop's `cancel_token.cancelled()` branch fires, closing
    /// the WebSocket.
    ///
    /// This is used when a certificate is revoked or a service is deactivated
    /// to ensure the existing WebSocket session is terminated immediately,
    /// rather than waiting for the next message round-trip to detect the
    /// disconnection.
    pub async fn force_disconnect(&self, service_id: &Uuid) {
        let mut guard = self.inner.write();
        if let Some(conn) = guard.connections.remove(service_id) {
            conn.cancel_token.cancel();
        }
    }

    // ---------------------------------------------------------------
    // Messaging
    // ---------------------------------------------------------------

    /// Push a message to a connected service. Returns `true` if sent.
    ///
    /// The lock is released before the async send to prevent holding the
    /// `RwLock` across an await point.
    pub async fn send(&self, service_id: &Uuid, msg: ControllerMessage) -> bool {
        let sender = {
            let guard = self.inner.read();
            guard.connections.get(service_id).map(|c| c.sender.clone())
        };
        if let Some(sender) = sender {
            sender.send(msg).await.is_ok()
        } else {
            false
        }
    }

    /// Check whether a service is currently connected.
    pub async fn is_connected(&self, service_id: &Uuid) -> bool {
        self.inner.read().connections.contains_key(service_id)
    }

    /// Get the connection time for a service, if connected.
    pub async fn connected_at(&self, service_id: &Uuid) -> Option<OffsetDateTime> {
        self.inner
            .read()
            .connections
            .get(service_id)
            .map(|c| c.connected_at)
    }

    /// Broadcast a message to all connected services.
    ///
    /// Snapshots the senders under the lock and releases it before sending,
    /// so a slow consumer cannot block connection management operations.
    /// Sends are dispatched in parallel with a per-send timeout to prevent
    /// a single slow consumer from blocking the entire broadcast.
    pub async fn broadcast(&self, msg: ControllerMessage) {
        let senders: Vec<mpsc::Sender<ControllerMessage>> = {
            let guard = self.inner.read();
            guard
                .connections
                .values()
                .map(|c| c.sender.clone())
                .collect()
        };
        send_parallel(&senders, msg).await;
    }

    /// Broadcast a message to all connected services that advertise the given
    /// capability.
    ///
    /// Snapshots the senders under the lock and releases it before sending.
    /// Sends are dispatched in parallel with a per-send timeout.
    pub async fn broadcast_by_capability(&self, capability: &Capability, msg: ControllerMessage) {
        let senders: Vec<mpsc::Sender<ControllerMessage>> = {
            let guard = self.inner.read();
            guard
                .connections
                .values()
                .filter(|c| c.capabilities.contains(capability))
                .map(|c| c.sender.clone())
                .collect()
        };
        send_parallel(&senders, msg).await;
    }

    /// Get the current number of connected services.
    pub async fn connection_count(&self) -> usize {
        self.inner.read().connections.len()
    }

    /// Broadcast server restarting notification to all services, scattered over time.
    ///
    /// This avoids a thundering herd when services reconnect by spreading out the
    /// notifications randomly over the specified duration. Each service receives
    /// the message at a random time within the scatter window.
    ///
    /// **This method returns immediately** after scheduling all notifications; it does
    /// not block until the scatter window elapses. Callers that need to wait for services
    /// to disconnect should use [`wait_for_service_drain`](super::tasks::wait_for_service_drain)
    /// after calling this method.
    pub async fn broadcast_server_restarting_scattered(
        &self,
        payload: uptrakit_internal_wire::ServerRestartingPayload,
        scatter_duration: Duration,
    ) {
        let guard = self.inner.read();
        let service_ids: Vec<Uuid> = guard.connections.keys().copied().collect();
        let count = service_ids.len();
        drop(guard);

        if count == 0 {
            return;
        }

        let msg = ControllerMessage::ServerRestarting(payload);
        let scatter_ms = scatter_duration.as_millis() as u64;

        tracing::debug!(
            count,
            scatter_window_ms = scatter_ms,
            "scheduling scattered ServerRestarting notifications"
        );

        for service_id in service_ids {
            // Assign a random delay within the scatter window so that reconnects
            // from all services are spread out over time rather than hitting the
            // controller simultaneously (thundering herd prevention).
            let delay_ms = if scatter_ms > 0 {
                rand::rng().random_range(0..scatter_ms)
            } else {
                0
            };
            let delay = Duration::from_millis(delay_ms);

            tracing::trace!(
                %service_id,
                delay_ms,
                "scheduled ServerRestarting notification"
            );

            let msg_clone = msg.clone();
            let inner = Arc::clone(&self.inner);

            // Spawn instead of join_all so this function returns immediately.
            // The drain loop in `wait_for_service_drain` is responsible for
            // waiting until all services have actually disconnected.
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                // Clone the sender under the lock then drop the guard before
                // any await point — parking_lot guards are not Send.
                let sender = {
                    let guard = inner.read();
                    guard.connections.get(&service_id).map(|c| c.sender.clone())
                };
                if let Some(sender) = sender {
                    tracing::trace!(%service_id, "sending ServerRestarting notification");
                    // Use the same send timeout as broadcast()/send_parallel() to
                    // prevent an unresponsive service from blocking indefinitely.
                    if tokio::time::timeout(BROADCAST_SEND_TIMEOUT, sender.send(msg_clone))
                        .await
                        .is_err()
                    {
                        tracing::warn!(
                            %service_id,
                            "ServerRestarting send timed out for service"
                        );
                    }
                } else {
                    tracing::trace!(
                        %service_id,
                        "service already disconnected before scatter send"
                    );
                }
            });
        }

        tracing::debug!(count, "ServerRestarting notifications scheduled");
    }

    /// Check whether any connected service advertises the given capability.
    pub async fn has_capability_connected(&self, capability: &Capability) -> bool {
        self.inner
            .read()
            .connections
            .values()
            .any(|c| c.capabilities.contains(capability))
    }

    /// Broadcast a message to all connected instances of `service_app_name`
    /// except the given `exclude_service_id`.
    ///
    /// Used by the service config store to push `ServiceConfigUpdated` to all
    /// instances of the same service app after a config write.
    pub async fn broadcast_to_app_except(
        &self,
        service_app_name: &str,
        exclude_service_id: Uuid,
        msg: ControllerMessage,
    ) {
        let senders: Vec<mpsc::Sender<ControllerMessage>> = {
            let inner = self.inner.read();
            inner
                .connections
                .iter()
                .filter(|(id, conn)| {
                    **id != exclude_service_id
                        && conn.service_app_name.as_deref() == Some(service_app_name)
                })
                .map(|(_, conn)| conn.sender.clone())
                .collect()
        };
        for sender in senders {
            let _ = sender.try_send(msg.clone());
        }
    }

    /// Returns the subset of the given service IDs that are currently connected.
    ///
    /// Acquires a single read lock to check all IDs efficiently.
    pub async fn filter_connected(
        &self,
        ids: &[uuid::Uuid],
    ) -> std::collections::HashSet<uuid::Uuid> {
        let guard = self.inner.read();
        ids.iter()
            .filter(|id| guard.connections.contains_key(*id))
            .copied()
            .collect()
    }
}

/// Timeout for individual send operations during parallel broadcast.
const BROADCAST_SEND_TIMEOUT: Duration = Duration::from_secs(5);

/// Send a message to multiple senders in parallel with a per-send timeout.
///
/// Each send is executed concurrently via `futures_util::future::join_all`.
/// If a single send exceeds [`BROADCAST_SEND_TIMEOUT`], it is abandoned and
/// a warning is logged so operators can identify slow consumers.
async fn send_parallel(senders: &[mpsc::Sender<ControllerMessage>], msg: ControllerMessage) {
    let futures: Vec<_> = senders
        .iter()
        .map(|sender| {
            let msg = msg.clone();
            let sender = sender.clone();
            async move {
                if tokio::time::timeout(BROADCAST_SEND_TIMEOUT, sender.send(msg))
                    .await
                    .is_err()
                {
                    tracing::warn!("broadcast send timed out for a consumer");
                }
            }
        })
        .collect();
    futures_util::future::join_all(futures).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn scattered_broadcast_is_non_blocking() {
        let registry = ServiceConnectionRegistry::new();
        let svc_a = Uuid::now_v7();
        let svc_b = Uuid::now_v7();

        let caps = BTreeSet::from([Capability::GracefulShutdown]);
        let (mut rx_a, _) = registry
            .register(svc_a, caps.clone(), None, None, None)
            .await;
        let (mut rx_b, _) = registry.register(svc_b, caps, None, None, None).await;

        let payload = uptrakit_internal_wire::ServerRestartingPayload {
            reason: "test".to_string(),
        };

        // Virtual time is paused: no sleep can complete without explicit `advance()`.
        // The old `join_all` implementation would block here indefinitely.
        let start = tokio::time::Instant::now();
        registry
            .broadcast_server_restarting_scattered(payload, Duration::from_secs(5))
            .await;
        assert!(
            start.elapsed() < Duration::from_millis(10),
            "broadcast_server_restarting_scattered must return immediately, not await scatter delays"
        );

        // Advance virtual time past the scatter window so the spawned tasks wake up and send.
        tokio::time::advance(Duration::from_secs(6)).await;

        // Both receivers should now have a message enqueued.
        assert!(
            rx_a.recv().await.is_some(),
            "service A should receive the ServerRestarting notification"
        );
        assert!(
            rx_b.recv().await.is_some(),
            "service B should receive the ServerRestarting notification"
        );
    }

    #[tokio::test]
    async fn broadcast_delivers_to_all_services() {
        let registry = ServiceConnectionRegistry::new();
        let svc_a = Uuid::now_v7();
        let svc_b = Uuid::now_v7();

        let caps = BTreeSet::from([Capability::GracefulShutdown]);
        let (mut rx_a, _) = registry
            .register(svc_a, caps.clone(), None, None, None)
            .await;
        let (mut rx_b, _) = registry.register(svc_b, caps, None, None, None).await;

        let msg =
            ControllerMessage::ServerRestarting(uptrakit_internal_wire::ServerRestartingPayload {
                reason: "test".to_string(),
            });
        registry.broadcast(msg).await;

        assert!(rx_a.recv().await.is_some(), "service A should receive msg");
        assert!(rx_b.recv().await.is_some(), "service B should receive msg");
    }

    #[tokio::test]
    async fn broadcast_by_capability_filters_correctly() {
        let registry = ServiceConnectionRegistry::new();
        let svc_mqtt = Uuid::now_v7();
        let svc_other = Uuid::now_v7();

        let mqtt_caps = BTreeSet::from([Capability::UpdateTracking, Capability::GracefulShutdown]);
        let (mut rx_mqtt, _) = registry
            .register(svc_mqtt, mqtt_caps, None, None, None)
            .await;
        let (mut rx_other, _) = registry
            .register(
                svc_other,
                BTreeSet::from([Capability::GracefulShutdown]),
                None,
                None,
                None,
            )
            .await;

        let msg =
            ControllerMessage::ServerRestarting(uptrakit_internal_wire::ServerRestartingPayload {
                reason: "test".to_string(),
            });
        registry
            .broadcast_by_capability(&Capability::UpdateTracking, msg)
            .await;

        assert!(
            rx_mqtt.recv().await.is_some(),
            "mqtt service should receive msg"
        );

        // The non-MQTT service should NOT have received anything.
        // Use try_recv to avoid blocking.
        assert!(
            rx_other.try_recv().is_err(),
            "non-mqtt service should not receive msg"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn broadcast_does_not_block_on_full_channel() {
        let registry = ServiceConnectionRegistry::new();
        let svc = Uuid::now_v7();

        let caps = BTreeSet::from([Capability::GracefulShutdown]);
        let (rx, _) = registry.register(svc, caps, None, None, None).await;

        // Fill the channel to capacity (PUSH_CHANNEL_CAPACITY = 32)
        // without consuming from rx so the channel is full.
        for i in 0..PUSH_CHANNEL_CAPACITY {
            let msg = ControllerMessage::ServerRestarting(
                uptrakit_internal_wire::ServerRestartingPayload {
                    reason: format!("fill-{i}"),
                },
            );
            let _ = registry.send(&svc, msg).await;
        }

        // Broadcast should complete within the timeout rather than blocking
        // indefinitely on the full channel.
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            let msg = ControllerMessage::ServerRestarting(
                uptrakit_internal_wire::ServerRestartingPayload {
                    reason: "overflow".to_string(),
                },
            );
            registry.broadcast(msg).await;
        })
        .await;

        assert!(
            result.is_ok(),
            "broadcast should not block indefinitely on a full channel"
        );

        // Keep rx alive to prevent channel closure before broadcast completes.
        drop(rx);
    }

    #[tokio::test]
    async fn force_disconnect_cancels_token_and_removes_connection() {
        let registry = ServiceConnectionRegistry::new();
        let svc = Uuid::now_v7();
        let caps = BTreeSet::from([Capability::GracefulShutdown]);
        let (_rx, cancel_token) = registry.register(svc, caps, None, None, None).await;

        assert!(registry.is_connected(&svc).await);
        assert!(!cancel_token.is_cancelled());

        registry.force_disconnect(&svc).await;
        assert!(cancel_token.is_cancelled());
        assert!(!registry.is_connected(&svc).await);
    }

    #[tokio::test]
    async fn force_disconnect_noop_for_unknown_service() {
        let registry = ServiceConnectionRegistry::new();
        // Should not panic
        registry.force_disconnect(&Uuid::now_v7()).await;
    }

    #[tokio::test]
    async fn filter_connected_returns_connected_subset() {
        let registry = ServiceConnectionRegistry::new();
        let id1 = uuid::Uuid::now_v7();
        let id2 = uuid::Uuid::now_v7();
        let id3 = uuid::Uuid::now_v7();

        // Register id1 and id2 only.
        let _ = registry
            .register(id1, BTreeSet::new(), None, None, None)
            .await;
        let _ = registry
            .register(id2, BTreeSet::new(), None, None, None)
            .await;

        let result = registry.filter_connected(&[id1, id2, id3]).await;
        assert!(result.contains(&id1));
        assert!(result.contains(&id2));
        assert!(!result.contains(&id3));
        assert_eq!(result.len(), 2);
    }

    #[tokio::test]
    async fn filter_connected_empty_input() {
        let registry = ServiceConnectionRegistry::new();
        let result = registry.filter_connected(&[]).await;
        assert!(result.is_empty());
    }
}
