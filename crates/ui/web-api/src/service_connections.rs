use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rand::Rng;
use std::cmp::Ordering;
use time::OffsetDateTime;
use tokio::sync::{RwLock, mpsc};
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
    /// Instance ID provided during registration (MQTT only).
    instance_id: Option<String>,
    /// Maximum tenants this instance is willing to handle, 0 = unlimited (MQTT only).
    max_tenants: Option<u32>,
    /// Set of MQTT client IDs currently assigned to this instance (MQTT only).
    assigned_mqtt_clients: HashSet<Uuid>,
    /// Timestamp of last heartbeat received (MQTT only).
    last_heartbeat: Option<Instant>,
    /// Timestamp when the connection was registered.
    connected_at: OffsetDateTime,
}

/// Interior state protected by the `RwLock`.
struct RegistryInner {
    /// Primary map: service_id -> connection state.
    connections: HashMap<Uuid, ServiceConnection>,
    /// Reverse index: mqtt_client_id -> service_id for O(1) lookup.
    mqtt_client_index: HashMap<Uuid, Uuid>,
}

impl RegistryInner {
    fn new() -> Self {
        Self {
            connections: HashMap::new(),
            mqtt_client_index: HashMap::new(),
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

/// Snapshot of MQTT service load used for lease selection.
#[derive(Clone, Debug)]
pub struct MqttServiceLoad {
    pub service_id: Uuid,
    pub instance_id: String,
    pub assigned_count: usize,
    pub max_tenants: u32,
    pub available_capacity: u32,
    pub utilization_numerator: u32,
    pub utilization_denominator: u32,
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
    /// The `capabilities` set describes what the service can do.  For MQTT
    /// bridge services (those with `Capability::MqttBridge`), `instance_id`
    /// and `max_tenants` should be provided; for all other services they
    /// can be `None`.
    pub async fn register(
        &self,
        service_id: Uuid,
        capabilities: BTreeSet<Capability>,
        instance_id: Option<String>,
        max_tenants: Option<u32>,
    ) -> (mpsc::Receiver<ControllerMessage>, CancellationToken) {
        let (tx, rx) = mpsc::channel(PUSH_CHANNEL_CAPACITY);
        let cancel_token = CancellationToken::new();

        let is_mqtt = capabilities.contains(&Capability::MqttBridge);

        let conn = ServiceConnection {
            sender: tx,
            cancel_token: cancel_token.clone(),
            capabilities,
            instance_id,
            max_tenants,
            assigned_mqtt_clients: HashSet::new(),
            last_heartbeat: if is_mqtt { Some(Instant::now()) } else { None },
            connected_at: OffsetDateTime::now_utc(),
        };

        let mut guard = self.inner.write().await;
        if let Some(old) = guard.connections.remove(&service_id) {
            old.cancel_token.cancel();
            // Clean up reverse index for superseded connection.
            for client_id in &old.assigned_mqtt_clients {
                guard.mqtt_client_index.remove(client_id);
            }
            tracing::info!(%service_id, "cancelled superseded connection");
        }
        guard.connections.insert(service_id, conn);
        (rx, cancel_token)
    }

    /// Remove a service from the registry on disconnect.
    ///
    /// Returns the set of MQTT client IDs that were assigned to this service
    /// (empty for non-MQTT services), so the lease coordinator can release them.
    pub async fn unregister(&self, service_id: &Uuid) -> Option<HashSet<Uuid>> {
        let mut guard = self.inner.write().await;
        guard.connections.remove(service_id).map(|c| {
            // Clean up reverse index entries for all MQTT clients assigned to this service.
            for client_id in &c.assigned_mqtt_clients {
                guard.mqtt_client_index.remove(client_id);
            }
            c.assigned_mqtt_clients
        })
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
            let guard = self.inner.read().await;
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
        self.inner.read().await.connections.contains_key(service_id)
    }

    /// Get the connection time for a service, if connected.
    pub async fn connected_at(&self, service_id: &Uuid) -> Option<OffsetDateTime> {
        self.inner
            .read()
            .await
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
            let guard = self.inner.read().await;
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
            let guard = self.inner.read().await;
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
        self.inner.read().await.connections.len()
    }

    /// Broadcast server restarting notification to all services, scattered over time.
    ///
    /// This avoids a thundering herd when services reconnect by spreading out the
    /// notifications randomly over the specified duration. Each service receives
    /// the message at a random time within the scatter window.
    pub async fn broadcast_server_restarting_scattered(
        &self,
        payload: uptrakit_internal_wire::ServerRestartingPayload,
        scatter_duration: Duration,
    ) {
        let guard = self.inner.read().await;
        let service_ids: Vec<Uuid> = guard.connections.keys().copied().collect();
        let count = service_ids.len();
        drop(guard);

        if count == 0 {
            return;
        }

        let msg = ControllerMessage::ServerRestarting(payload);
        let scatter_ms = scatter_duration.as_millis() as u64;

        for service_id in service_ids {
            // Random delay within scatter window
            let delay_ms = if scatter_ms > 0 {
                rand::rng().random_range(0..scatter_ms)
            } else {
                0
            };
            let delay = Duration::from_millis(delay_ms);

            let msg_clone = msg.clone();
            let inner = Arc::clone(&self.inner);

            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                let guard = inner.read().await;
                if let Some(conn) = guard.connections.get(&service_id) {
                    let _ = conn.sender.send(msg_clone).await;
                }
            });
        }
    }

    // ---------------------------------------------------------------
    // MQTT-specific methods
    // ---------------------------------------------------------------

    /// Record a heartbeat from an MQTT service.
    pub async fn record_heartbeat(&self, service_id: &Uuid) {
        let mut guard = self.inner.write().await;
        if let Some(conn) = guard.connections.get_mut(service_id) {
            conn.last_heartbeat = Some(Instant::now());
        }
    }

    /// Get the instance ID for a connected MQTT service.
    pub async fn get_instance_id(&self, service_id: &Uuid) -> Option<String> {
        self.inner
            .read()
            .await
            .connections
            .get(service_id)
            .and_then(|c| c.instance_id.clone())
    }

    /// Assign an MQTT client to an MQTT service instance.
    ///
    /// Returns `true` if the assignment was recorded, `false` if the service
    /// is not connected.
    pub async fn assign_mqtt_client(&self, service_id: &Uuid, mqtt_client_id: Uuid) -> bool {
        let mut guard = self.inner.write().await;
        if let Some(conn) = guard.connections.get_mut(service_id) {
            conn.assigned_mqtt_clients.insert(mqtt_client_id);
            guard.mqtt_client_index.insert(mqtt_client_id, *service_id);
            true
        } else {
            false
        }
    }

    /// Release an MQTT client from an MQTT service instance.
    ///
    /// Returns `true` if the MQTT client was previously assigned, `false` otherwise.
    pub async fn release_mqtt_client(&self, service_id: &Uuid, mqtt_client_id: &Uuid) -> bool {
        let mut guard = self.inner.write().await;
        if let Some(conn) = guard.connections.get_mut(service_id) {
            let removed = conn.assigned_mqtt_clients.remove(mqtt_client_id);
            if removed {
                guard.mqtt_client_index.remove(mqtt_client_id);
            }
            removed
        } else {
            false
        }
    }

    /// Find which MQTT service instance holds a specific MQTT client.
    ///
    /// Returns the `service_id` of the instance holding this MQTT client, if any.
    /// Uses a reverse index for O(1) lookup.
    pub async fn get_instance_for_mqtt_client(&self, mqtt_client_id: &Uuid) -> Option<Uuid> {
        self.inner
            .read()
            .await
            .mqtt_client_index
            .get(mqtt_client_id)
            .copied()
    }

    /// Get the current number of MQTT clients assigned to a service.
    pub async fn assigned_mqtt_client_count(&self, service_id: &Uuid) -> usize {
        self.inner
            .read()
            .await
            .connections
            .get(service_id)
            .map(|c| c.assigned_mqtt_clients.len())
            .unwrap_or(0)
    }

    /// Get the maximum tenants limit for a service.
    ///
    /// Returns `None` if the service is not connected or has no max_tenants set.
    /// `Some(0)` means unlimited.
    pub async fn get_max_tenants(&self, service_id: &Uuid) -> Option<u32> {
        self.inner
            .read()
            .await
            .connections
            .get(service_id)
            .and_then(|c| c.max_tenants)
    }

    /// Get available capacity for a service (max_tenants - current assignments).
    ///
    /// Returns `None` if the service is not connected or has no capacity info.
    /// Returns `Some(u32::MAX)` if max_tenants is 0 (unlimited).
    pub async fn get_available_capacity(&self, service_id: &Uuid) -> Option<u32> {
        let guard = self.inner.read().await;
        guard.connections.get(service_id).and_then(|c| {
            c.max_tenants.map(|max| {
                if max == 0 {
                    u32::MAX
                } else {
                    max.saturating_sub(c.assigned_mqtt_clients.len() as u32)
                }
            })
        })
    }

    /// Get all MQTT service IDs with their instance IDs.
    pub async fn list_connections(&self) -> Vec<(Uuid, String)> {
        self.inner
            .read()
            .await
            .connections
            .iter()
            .filter_map(|(id, conn)| conn.instance_id.as_ref().map(|iid| (*id, iid.clone())))
            .collect()
    }

    /// Check whether any connected service advertises the given capability.
    pub async fn has_capability_connected(&self, capability: &Capability) -> bool {
        self.inner
            .read()
            .await
            .connections
            .values()
            .any(|c| c.capabilities.contains(capability))
    }

    /// Get MQTT services that haven't sent a heartbeat within the given timeout.
    ///
    /// Returns a list of `(service_id, last_heartbeat_age)` for stale connections.
    pub async fn get_stale_services(&self, timeout: Duration) -> Vec<(Uuid, Duration)> {
        let guard = self.inner.read().await;
        let now = Instant::now();
        guard
            .connections
            .iter()
            .filter_map(|(service_id, conn)| {
                conn.last_heartbeat.and_then(|hb| {
                    let age = now.duration_since(hb);
                    if age > timeout {
                        Some((*service_id, age))
                    } else {
                        None
                    }
                })
            })
            .collect()
    }

    /// List MQTT service load information, sorted from least busy to most busy.
    pub async fn list_mqtt_service_loads(&self) -> Vec<MqttServiceLoad> {
        let guard = self.inner.read().await;
        let mut loads = Vec::new();

        for (service_id, conn) in guard.connections.iter() {
            if !conn.capabilities.contains(&Capability::MqttBridge) {
                continue;
            }

            let Some(instance_id) = conn.instance_id.clone() else {
                continue;
            };
            let max_tenants = conn.max_tenants.unwrap_or(0);
            let assigned_count = conn.assigned_mqtt_clients.len();

            if max_tenants > 0 && assigned_count >= max_tenants as usize {
                continue;
            }

            let (utilization_numerator, utilization_denominator, available_capacity) =
                if max_tenants == 0 {
                    (0, 1, u32::MAX)
                } else {
                    (
                        assigned_count as u32,
                        max_tenants,
                        max_tenants.saturating_sub(assigned_count as u32),
                    )
                };

            loads.push(MqttServiceLoad {
                service_id: *service_id,
                instance_id,
                assigned_count,
                max_tenants,
                available_capacity,
                utilization_numerator,
                utilization_denominator,
            });
        }

        loads.sort_by(compare_mqtt_service_load);
        loads
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

fn compare_mqtt_service_load(a: &MqttServiceLoad, b: &MqttServiceLoad) -> Ordering {
    let left = u128::from(a.utilization_numerator) * u128::from(b.utilization_denominator);
    let right = u128::from(b.utilization_numerator) * u128::from(a.utilization_denominator);

    match left.cmp(&right) {
        Ordering::Equal => match a.assigned_count.cmp(&b.assigned_count) {
            Ordering::Equal => a.service_id.as_bytes().cmp(b.service_id.as_bytes()),
            other => other,
        },
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a capability set for an MQTT bridge service.
    fn mqtt_caps() -> BTreeSet<Capability> {
        BTreeSet::from([Capability::MqttBridge, Capability::GracefulShutdown])
    }

    #[tokio::test]
    async fn broadcast_delivers_to_all_services() {
        let registry = ServiceConnectionRegistry::new();
        let svc_a = Uuid::now_v7();
        let svc_b = Uuid::now_v7();

        let caps = BTreeSet::from([Capability::GracefulShutdown]);
        let (mut rx_a, _) = registry.register(svc_a, caps.clone(), None, None).await;
        let (mut rx_b, _) = registry.register(svc_b, caps, None, None).await;

        let msg = ControllerMessage::ServerRestarting(
            uptrakit_internal_wire::ServerRestartingPayload {
                reason: "test".to_string(),
            },
        );
        registry.broadcast(msg).await;

        assert!(rx_a.recv().await.is_some(), "service A should receive msg");
        assert!(rx_b.recv().await.is_some(), "service B should receive msg");
    }

    #[tokio::test]
    async fn broadcast_by_capability_filters_correctly() {
        let registry = ServiceConnectionRegistry::new();
        let svc_mqtt = Uuid::now_v7();
        let svc_other = Uuid::now_v7();

        let (mut rx_mqtt, _) = registry
            .register(svc_mqtt, mqtt_caps(), Some("m".to_string()), Some(10))
            .await;
        let (mut rx_other, _) = registry
            .register(
                svc_other,
                BTreeSet::from([Capability::GracefulShutdown]),
                None,
                None,
            )
            .await;

        let msg = ControllerMessage::ServerRestarting(
            uptrakit_internal_wire::ServerRestartingPayload {
                reason: "test".to_string(),
            },
        );
        registry
            .broadcast_by_capability(&Capability::MqttBridge, msg)
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

    #[tokio::test]
    async fn broadcast_does_not_block_on_full_channel() {
        let registry = ServiceConnectionRegistry::new();
        let svc = Uuid::now_v7();

        let caps = BTreeSet::from([Capability::GracefulShutdown]);
        let (rx, _) = registry.register(svc, caps, None, None).await;

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
    async fn least_busy_prefers_lower_utilization_ratio() {
        let registry = ServiceConnectionRegistry::new();
        let svc_a = Uuid::now_v7();
        let svc_b = Uuid::now_v7();

        let _ = registry
            .register(svc_a, mqtt_caps(), Some("a".to_string()), Some(4))
            .await;
        let _ = registry
            .register(svc_b, mqtt_caps(), Some("b".to_string()), Some(2))
            .await;

        let _ = registry.assign_mqtt_client(&svc_a, Uuid::now_v7()).await;
        let _ = registry.assign_mqtt_client(&svc_b, Uuid::now_v7()).await;

        let loads = registry.list_mqtt_service_loads().await;
        let selected = loads.first().map(|l| l.service_id);
        assert_eq!(selected, Some(svc_a));
    }

    #[tokio::test]
    async fn least_busy_tiebreaks_by_assigned_count() {
        let registry = ServiceConnectionRegistry::new();
        let svc_unlimited = Uuid::now_v7();
        let svc_idle = Uuid::now_v7();

        let _ = registry
            .register(
                svc_unlimited,
                mqtt_caps(),
                Some("unlimited".to_string()),
                Some(0),
            )
            .await;
        let _ = registry
            .register(svc_idle, mqtt_caps(), Some("idle".to_string()), Some(10))
            .await;

        for _ in 0..3 {
            let _ = registry
                .assign_mqtt_client(&svc_unlimited, Uuid::now_v7())
                .await;
        }

        let loads = registry.list_mqtt_service_loads().await;
        let selected = loads.first().map(|l| l.service_id);
        assert_eq!(selected, Some(svc_idle));
    }
}
