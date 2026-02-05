use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rand::Rng;
use tokio::sync::{RwLock, mpsc};
use uptrakit_internal_wire::MqttControllerMessage;
use uuid::Uuid;

/// Per-connection state for an MQTT service instance.
struct MqttServiceConnection {
    /// Channel for pushing messages to the connected instance.
    sender: mpsc::Sender<MqttControllerMessage>,
    /// Instance ID provided during registration.
    instance_id: String,
    /// Maximum tenants this instance is willing to handle (0 = unlimited).
    max_tenants: u32,
    /// Set of tenant IDs currently assigned to this instance.
    assigned_tenants: HashSet<Uuid>,
    /// Timestamp of last heartbeat received.
    last_heartbeat: Instant,
}

/// Registry of connected MQTT service instances.
///
/// Tracks all connected MQTT services, their assigned tenants, and provides
/// push channels for real-time configuration updates.
#[derive(Clone)]
pub struct MqttServiceConnectionRegistry {
    inner: Arc<RwLock<HashMap<Uuid, MqttServiceConnection>>>,
}

impl Default for MqttServiceConnectionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl MqttServiceConnectionRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a connected MQTT service and return a receiver for push messages.
    ///
    /// The service_id is the database ID of the MQTT service. The instance_id and
    /// max_tenants come from the Register message sent by the service after connecting.
    pub async fn register(
        &self,
        service_id: Uuid,
        instance_id: String,
        max_tenants: u32,
    ) -> mpsc::Receiver<MqttControllerMessage> {
        let (tx, rx) = mpsc::channel(16);
        let conn = MqttServiceConnection {
            sender: tx,
            instance_id,
            max_tenants,
            assigned_tenants: HashSet::new(),
            last_heartbeat: Instant::now(),
        };
        self.inner.write().await.insert(service_id, conn);
        rx
    }

    /// Remove an MQTT service from the registry on disconnect.
    ///
    /// Returns the set of tenant IDs that were assigned to this service,
    /// so the lease coordinator can release them.
    pub async fn unregister(&self, service_id: &Uuid) -> HashSet<Uuid> {
        self.inner
            .write()
            .await
            .remove(service_id)
            .map(|c| c.assigned_tenants)
            .unwrap_or_default()
    }

    /// Push a message to a connected MQTT service. Returns `true` if sent.
    pub async fn send(&self, service_id: &Uuid, msg: MqttControllerMessage) -> bool {
        let guard = self.inner.read().await;
        if let Some(conn) = guard.get(service_id) {
            conn.sender.send(msg).await.is_ok()
        } else {
            false
        }
    }

    /// Check whether an MQTT service is currently connected.
    pub async fn is_connected(&self, service_id: &Uuid) -> bool {
        self.inner.read().await.contains_key(service_id)
    }

    /// Record a heartbeat from an MQTT service.
    pub async fn record_heartbeat(&self, service_id: &Uuid) {
        let mut guard = self.inner.write().await;
        if let Some(conn) = guard.get_mut(service_id) {
            conn.last_heartbeat = Instant::now();
        }
    }

    /// Assign a tenant to an MQTT service instance.
    ///
    /// Returns true if the assignment was recorded, false if the service
    /// is not connected.
    pub async fn assign_tenant(&self, service_id: &Uuid, tenant_id: Uuid) -> bool {
        let mut guard = self.inner.write().await;
        if let Some(conn) = guard.get_mut(service_id) {
            conn.assigned_tenants.insert(tenant_id);
            true
        } else {
            false
        }
    }

    /// Release a tenant from an MQTT service instance.
    ///
    /// Returns true if the tenant was previously assigned, false otherwise.
    pub async fn release_tenant(&self, service_id: &Uuid, tenant_id: &Uuid) -> bool {
        let mut guard = self.inner.write().await;
        if let Some(conn) = guard.get_mut(service_id) {
            conn.assigned_tenants.remove(tenant_id)
        } else {
            false
        }
    }

    /// Find which MQTT service instance holds a specific tenant.
    ///
    /// Returns the service_id of the instance holding this tenant, if any.
    pub async fn get_instance_for_tenant(&self, tenant_id: &Uuid) -> Option<Uuid> {
        let guard = self.inner.read().await;
        for (service_id, conn) in guard.iter() {
            if conn.assigned_tenants.contains(tenant_id) {
                return Some(*service_id);
            }
        }
        None
    }

    /// Get the current number of tenants assigned to a service.
    pub async fn assigned_tenant_count(&self, service_id: &Uuid) -> usize {
        self.inner
            .read()
            .await
            .get(service_id)
            .map(|c| c.assigned_tenants.len())
            .unwrap_or(0)
    }

    /// Get the maximum tenants limit for a service.
    ///
    /// Returns None if the service is not connected, Some(0) means unlimited.
    pub async fn get_max_tenants(&self, service_id: &Uuid) -> Option<u32> {
        self.inner
            .read()
            .await
            .get(service_id)
            .map(|c| c.max_tenants)
    }

    /// Get available capacity for a service (max_tenants - current assignments).
    ///
    /// Returns None if the service is not connected.
    /// Returns Some(u32::MAX) if max_tenants is 0 (unlimited).
    pub async fn get_available_capacity(&self, service_id: &Uuid) -> Option<u32> {
        let guard = self.inner.read().await;
        guard.get(service_id).map(|c| {
            if c.max_tenants == 0 {
                u32::MAX
            } else {
                c.max_tenants
                    .saturating_sub(c.assigned_tenants.len() as u32)
            }
        })
    }

    /// Broadcast a message to all connected MQTT services.
    pub async fn broadcast(&self, msg: MqttControllerMessage) {
        let guard = self.inner.read().await;
        for conn in guard.values() {
            let _ = conn.sender.send(msg.clone()).await;
        }
    }

    /// Broadcast a CA bundle update to all connected MQTT services.
    pub async fn broadcast_ca_bundle_updated(
        &self,
        payload: uptrakit_internal_wire::CaBundleUpdatedPayload,
    ) {
        let msg = MqttControllerMessage::CaBundleUpdated(payload);
        self.broadcast(msg).await;
    }

    /// Broadcast a certificate renewal request to all connected MQTT services.
    pub async fn broadcast_request_cert_renewal(
        &self,
        payload: uptrakit_internal_wire::RequestCertRenewalPayload,
    ) {
        let msg = MqttControllerMessage::RequestCertRenewal(payload);
        self.broadcast(msg).await;
    }

    /// Get the current number of connected MQTT services.
    pub async fn connection_count(&self) -> usize {
        self.inner.read().await.len()
    }

    /// Get all service IDs with their instance IDs.
    pub async fn list_connections(&self) -> Vec<(Uuid, String)> {
        self.inner
            .read()
            .await
            .iter()
            .map(|(id, conn)| (*id, conn.instance_id.clone()))
            .collect()
    }

    /// Get services that haven't sent a heartbeat within the given timeout.
    ///
    /// Returns a list of (service_id, last_heartbeat_age) for stale connections.
    pub async fn get_stale_services(&self, timeout: Duration) -> Vec<(Uuid, Duration)> {
        let guard = self.inner.read().await;
        let now = Instant::now();
        guard
            .iter()
            .filter_map(|(service_id, conn)| {
                let age = now.duration_since(conn.last_heartbeat);
                if age > timeout {
                    Some((*service_id, age))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Broadcast server restarting notification to all MQTT services, scattered over time.
    ///
    /// This avoids a thundering herd when services reconnect by spreading out the
    /// notifications randomly over the specified duration.
    pub async fn broadcast_server_restarting_scattered(
        &self,
        payload: uptrakit_internal_wire::ServerRestartingPayload,
        scatter_duration: Duration,
    ) {
        let guard = self.inner.read().await;
        let service_ids: Vec<Uuid> = guard.keys().copied().collect();
        let count = service_ids.len();
        drop(guard);

        if count == 0 {
            return;
        }

        let msg = MqttControllerMessage::ServerRestarting(payload);
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
                if let Some(conn) = guard.get(&service_id) {
                    let _ = conn.sender.send(msg_clone).await;
                }
            });
        }
    }
}
