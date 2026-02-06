use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rand::Rng;
use tokio::sync::{RwLock, mpsc};
use uptrakit_internal_wire::ControllerMessage;
use uptrakit_shared_db::entity::service::ServiceType;
use uuid::Uuid;

/// Per-connection state for a connected service (agent or MQTT).
struct ServiceConnection {
    /// Channel for pushing messages to the connected service.
    sender: mpsc::Sender<ControllerMessage>,
    /// Type of service (Agent or Mqtt).
    service_type: ServiceType,
    /// Instance ID provided during registration (MQTT only).
    instance_id: Option<String>,
    /// Maximum tenants this instance is willing to handle, 0 = unlimited (MQTT only).
    max_tenants: Option<u32>,
    /// Set of tenant IDs currently assigned to this instance (MQTT only).
    assigned_tenants: HashSet<Uuid>,
    /// Timestamp of last heartbeat received (MQTT only).
    last_heartbeat: Option<Instant>,
}

/// Unified registry of connected services (agents and MQTT service instances).
///
/// The controller registers services when they connect via WebSocket and
/// unregisters them on disconnect. Admin actions (approve/reject) use
/// `send()` to push messages to connected services in real time.
#[derive(Clone)]
pub struct ServiceConnectionRegistry {
    inner: Arc<RwLock<HashMap<Uuid, ServiceConnection>>>,
}

impl Default for ServiceConnectionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceConnectionRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    // ---------------------------------------------------------------
    // Registration
    // ---------------------------------------------------------------

    /// Register a connected agent and return a receiver for push messages.
    pub async fn register_agent(&self, service_id: Uuid) -> mpsc::Receiver<ControllerMessage> {
        let (tx, rx) = mpsc::channel(16);
        let conn = ServiceConnection {
            sender: tx,
            service_type: ServiceType::Agent,
            instance_id: None,
            max_tenants: None,
            assigned_tenants: HashSet::new(),
            last_heartbeat: None,
        };
        self.inner.write().await.insert(service_id, conn);
        rx
    }

    /// Register a connected MQTT service and return a receiver for push messages.
    ///
    /// The `service_id` is the database ID of the MQTT service. The `instance_id`
    /// and `max_tenants` come from the Register message sent by the service after
    /// connecting.
    pub async fn register_mqtt(
        &self,
        service_id: Uuid,
        instance_id: String,
        max_tenants: u32,
    ) -> mpsc::Receiver<ControllerMessage> {
        let (tx, rx) = mpsc::channel(16);
        let conn = ServiceConnection {
            sender: tx,
            service_type: ServiceType::Mqtt,
            instance_id: Some(instance_id),
            max_tenants: Some(max_tenants),
            assigned_tenants: HashSet::new(),
            last_heartbeat: Some(Instant::now()),
        };
        self.inner.write().await.insert(service_id, conn);
        rx
    }

    /// Remove a service from the registry on disconnect.
    ///
    /// Returns the set of tenant IDs that were assigned to this service
    /// (empty for agents), so the lease coordinator can release them.
    pub async fn unregister(&self, service_id: &Uuid) -> Option<HashSet<Uuid>> {
        self.inner
            .write()
            .await
            .remove(service_id)
            .map(|c| c.assigned_tenants)
    }

    // ---------------------------------------------------------------
    // Messaging
    // ---------------------------------------------------------------

    /// Push a message to a connected service. Returns `true` if sent.
    pub async fn send(&self, service_id: &Uuid, msg: ControllerMessage) -> bool {
        let guard = self.inner.read().await;
        if let Some(conn) = guard.get(service_id) {
            conn.sender.send(msg).await.is_ok()
        } else {
            false
        }
    }

    /// Check whether a service is currently connected.
    pub async fn is_connected(&self, service_id: &Uuid) -> bool {
        self.inner.read().await.contains_key(service_id)
    }

    /// Broadcast a message to all connected services.
    pub async fn broadcast(&self, msg: ControllerMessage) {
        let guard = self.inner.read().await;
        for conn in guard.values() {
            let _ = conn.sender.send(msg.clone()).await;
        }
    }

    /// Broadcast a message to all connected services of a specific type.
    pub async fn broadcast_by_type(&self, service_type: ServiceType, msg: ControllerMessage) {
        let guard = self.inner.read().await;
        for conn in guard.values() {
            if conn.service_type == service_type {
                let _ = conn.sender.send(msg.clone()).await;
            }
        }
    }

    /// Broadcast a CA bundle update to all connected services.
    pub async fn broadcast_ca_bundle_updated(
        &self,
        payload: uptrakit_internal_wire::CaBundleUpdatedPayload,
    ) {
        let msg = ControllerMessage::CaBundleUpdated(payload);
        self.broadcast(msg).await;
    }

    /// Broadcast a certificate renewal request to all connected services.
    pub async fn broadcast_request_cert_renewal(
        &self,
        payload: uptrakit_internal_wire::RequestCertRenewalPayload,
    ) {
        let msg = ControllerMessage::RequestCertRenewal(payload);
        self.broadcast(msg).await;
    }

    /// Get the current number of connected services.
    pub async fn connection_count(&self) -> usize {
        self.inner.read().await.len()
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
        let service_ids: Vec<Uuid> = guard.keys().copied().collect();
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
                if let Some(conn) = guard.get(&service_id) {
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
        if let Some(conn) = guard.get_mut(service_id) {
            conn.last_heartbeat = Some(Instant::now());
        }
    }

    /// Get the instance ID for a connected MQTT service.
    pub async fn get_instance_id(&self, service_id: &Uuid) -> Option<String> {
        self.inner
            .read()
            .await
            .get(service_id)
            .and_then(|c| c.instance_id.clone())
    }

    /// Assign a tenant to an MQTT service instance.
    ///
    /// Returns `true` if the assignment was recorded, `false` if the service
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
    /// Returns `true` if the tenant was previously assigned, `false` otherwise.
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
    /// Returns the `service_id` of the instance holding this tenant, if any.
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
    /// Returns `None` if the service is not connected or has no max_tenants set.
    /// `Some(0)` means unlimited.
    pub async fn get_max_tenants(&self, service_id: &Uuid) -> Option<u32> {
        self.inner
            .read()
            .await
            .get(service_id)
            .and_then(|c| c.max_tenants)
    }

    /// Get available capacity for a service (max_tenants - current assignments).
    ///
    /// Returns `None` if the service is not connected or has no capacity info.
    /// Returns `Some(u32::MAX)` if max_tenants is 0 (unlimited).
    pub async fn get_available_capacity(&self, service_id: &Uuid) -> Option<u32> {
        let guard = self.inner.read().await;
        guard.get(service_id).and_then(|c| {
            c.max_tenants.map(|max| {
                if max == 0 {
                    u32::MAX
                } else {
                    max.saturating_sub(c.assigned_tenants.len() as u32)
                }
            })
        })
    }

    /// Get all MQTT service IDs with their instance IDs.
    pub async fn list_connections(&self) -> Vec<(Uuid, String)> {
        self.inner
            .read()
            .await
            .iter()
            .filter_map(|(id, conn)| conn.instance_id.as_ref().map(|iid| (*id, iid.clone())))
            .collect()
    }

    /// Get MQTT services that haven't sent a heartbeat within the given timeout.
    ///
    /// Returns a list of `(service_id, last_heartbeat_age)` for stale connections.
    pub async fn get_stale_services(&self, timeout: Duration) -> Vec<(Uuid, Duration)> {
        let guard = self.inner.read().await;
        let now = Instant::now();
        guard
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
}
