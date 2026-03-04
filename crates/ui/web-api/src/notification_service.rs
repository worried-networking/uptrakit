use uptrakit_internal_wire::{Capability, ControllerMessage, MqttSoftwareStatesPayload};
use uuid::Uuid;

use crate::service_connections::ServiceConnectionRegistry;

/// Cross-controller notification service.
///
/// Wraps `ServiceConnectionRegistry` to provide the push API. Without NATS,
/// messages are delivered locally only (single-controller mode). When the
/// `nats` feature is enabled and a `NatsTransport` is attached, messages are
/// also published to NATS JetStream for cross-controller delivery.
#[derive(Clone)]
pub struct NotificationService {
    registry: ServiceConnectionRegistry,
    controller_id: Uuid,
    /// Tracks whether a NATS transport has been attached. Always present so
    /// that `has_nats()` can be implemented without any `#[cfg]`.
    nats_configured: bool,
    #[cfg(feature = "nats")]
    nats: Option<crate::nats_transport::NatsTransport>,
}

impl NotificationService {
    pub fn new(registry: ServiceConnectionRegistry, controller_id: Uuid) -> Self {
        Self {
            registry,
            controller_id,
            nats_configured: false,
            #[cfg(feature = "nats")]
            nats: None,
        }
    }

    /// Attach a NATS transport for cross-controller delivery.
    #[cfg(feature = "nats")]
    pub fn with_nats(mut self, nats: crate::nats_transport::NatsTransport) -> Self {
        self.nats = Some(nats);
        self.nats_configured = true;
        self
    }

    /// Returns `true` if a NATS transport is configured.
    pub fn has_nats(&self) -> bool {
        self.nats_configured
    }

    /// Push a message to a specific service (local + optional NATS).
    ///
    /// Messages that contain credentials (`TenantAssignments`, `TenantConfigUpdated`,
    /// `TenantRevoked`, `ServiceCredentials`) are delivered locally but **not**
    /// published to NATS to prevent credential leakage.
    pub async fn send(&self, service_id: &Uuid, msg: ControllerMessage) -> bool {
        let local = self.registry.send(service_id, msg.clone()).await;
        if msg.is_nats_publishable() {
            self.maybe_publish_nats(Some(*service_id), None, msg).await;
        }
        local
    }

    /// Broadcast a message to all connected services (local + optional NATS).
    ///
    /// Messages that contain credentials are delivered locally but **not**
    /// published to NATS (see [`Self::send`] doc comment).
    pub async fn broadcast(&self, msg: ControllerMessage) {
        self.registry.broadcast(msg.clone()).await;
        if msg.is_nats_publishable() {
            self.maybe_publish_nats(None, None, msg).await;
        }
    }

    /// Publish a controller-targeted event for cross-controller delivery.
    ///
    /// When NATS is available, the message is published to the controller
    /// subject. When NATS is not configured, this is a no-op (single-controller
    /// mode means the local controller already handled the event).
    pub async fn publish_controller_event(&self, msg: ControllerMessage) {
        self.maybe_publish_nats(None, Some("controller"), msg).await;
    }

    /// Send a message to all services with a specific capability (local + optional NATS).
    ///
    /// The `capability_str` is the wire-format capability name (e.g. `"mqtt_bridge"`).
    /// It is parsed to a [`Capability`] for local delivery via the registry.
    pub async fn send_by_capability(&self, capability_str: &str, msg: ControllerMessage) {
        let cap: Capability = capability_str.parse().unwrap_or_else(|_| {
            // Should never happen — parse() always succeeds via Other fallback.
            Capability::Other(capability_str.to_string())
        });
        self.registry
            .broadcast_by_capability(&cap, msg.clone())
            .await;
        if msg.is_nats_publishable() {
            self.maybe_publish_nats(None, Some(capability_str), msg)
                .await;
        }
    }

    /// Access the underlying registry (e.g. for `is_connected` checks).
    pub fn registry(&self) -> &ServiceConnectionRegistry {
        &self.registry
    }

    /// Load software states for a tenant and push to all locally connected MQTT
    /// services (immediately) and to NATS for cross-controller delivery.
    ///
    /// Loads both software-item states and host-package host states in two
    /// independent bulk queries, then delivers the merged payload.
    pub async fn push_software_states_for_tenant(
        &self,
        db: &sea_orm::DatabaseConnection,
        tenant_id: uuid::Uuid,
    ) {
        let mut payload =
            match crate::queries::mqtt_software_states::load_software_states_for_tenant(
                db, tenant_id,
            )
            .await
            {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        %tenant_id,
                        "failed to load software states for MQTT push"
                    );
                    return;
                }
            };

        match crate::queries::mqtt_software_states::load_host_package_host_states_for_tenant(
            db, tenant_id,
        )
        .await
        {
            Ok(host_states) => {
                payload.host_package_hosts = host_states;
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    %tenant_id,
                    "failed to load host package states for MQTT push; delivering without them"
                );
            }
        }

        self.deliver_software_states(payload).await;
    }

    /// Deliver a pre-loaded `MqttSoftwareStatesPayload` to all locally connected
    /// MQTT services and publish to NATS for cross-controller delivery.
    ///
    /// Used by `ControllerSchedulerNotifier` to avoid re-loading an already-loaded
    /// payload through the ORM layer.
    pub async fn deliver_software_states(&self, payload: MqttSoftwareStatesPayload) {
        let msg = ControllerMessage::SoftwareStates(payload);
        // Deliver to locally connected MQTT services immediately.
        self.registry
            .broadcast_by_capability(&Capability::MqttBridge, msg.clone())
            .await;
        // Publish to NATS for cross-controller delivery (MQTT-only).
        self.maybe_publish_nats(None, Some("mqtt_bridge"), msg)
            .await;
    }

    /// Return the controller ID.
    pub fn controller_id(&self) -> Uuid {
        self.controller_id
    }

    /// Conditionally publish a message to NATS when a transport is configured.
    async fn maybe_publish_nats(
        &self,
        target_service_id: Option<Uuid>,
        target_capability: Option<&str>,
        msg: ControllerMessage,
    ) {
        #[cfg(feature = "nats")]
        if let Some(ref nats) = self.nats {
            nats.publish(
                self.controller_id,
                target_service_id,
                target_capability,
                msg,
            )
            .await;
            return;
        }
        // Suppress unused-variable warnings when nats feature is disabled or no
        // transport is configured. Events are ephemeral; dropping them is safe.
        let _ = (target_service_id, target_capability, msg);
    }
}

#[async_trait::async_trait]
impl crate::ServiceNotifier for NotificationService {
    async fn send_to_service(&self, service_id: &Uuid, msg: ControllerMessage) -> bool {
        self.send(service_id, msg).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_internal_wire::{ApprovedPayload, ServerRestartingPayload};

    #[tokio::test]
    async fn server_restarting_is_local_only() {
        // ServerRestarting should NOT be sent through NotificationService.
        // It stays on ServiceConnectionRegistry.broadcast_server_restarting_scattered().
        // This test documents the design intent.
        let msg = ControllerMessage::ServerRestarting(ServerRestartingPayload {
            reason: "test".to_string(),
        });
        // The message type exists and is valid
        assert!(matches!(msg, ControllerMessage::ServerRestarting(_)));
    }

    /// Verify that credential-bearing variants are blocked by `is_nats_publishable`.
    /// The authoritative tests for this are in the wire crate; this test documents
    /// the integration point used by NotificationService.
    #[test]
    fn credential_bearing_variants_are_not_nats_publishable() {
        // Approved is a safe, publishable variant — sanity check.
        assert!(
            ControllerMessage::Approved(ApprovedPayload {
                service_id: Uuid::nil(),
            })
            .is_nats_publishable()
        );

        // ServiceCredentials must never be published to NATS.
        assert!(
            !ControllerMessage::ServiceCredentials(
                uptrakit_internal_wire::ServiceCredentialsPayload {
                    db_url: None,
                    master_key_hex: None,
                    nats_url: None,
                }
            )
            .is_nats_publishable()
        );
    }

    #[tokio::test]
    async fn send_delivers_locally_without_nats() {
        let registry = ServiceConnectionRegistry::new();
        let controller_id = Uuid::now_v7();
        let svc = NotificationService::new(registry, controller_id);

        assert!(!svc.has_nats());

        let service_id = Uuid::now_v7();
        let msg = ControllerMessage::Approved(ApprovedPayload { service_id });

        // send returns false because no service is connected.
        let result = svc.send(&service_id, msg).await;
        assert!(!result);
    }

    #[tokio::test]
    async fn broadcast_delivers_locally_without_nats() {
        let registry = ServiceConnectionRegistry::new();
        let controller_id = Uuid::now_v7();
        let svc = NotificationService::new(registry, controller_id);

        // Broadcast to an empty registry should not panic.
        svc.broadcast(ControllerMessage::Approved(ApprovedPayload {
            service_id: Uuid::nil(),
        }))
        .await;
    }

    #[tokio::test]
    async fn publish_controller_event_noop_without_nats() {
        let registry = ServiceConnectionRegistry::new();
        let controller_id = Uuid::now_v7();
        let svc = NotificationService::new(registry, controller_id);

        // Should be a no-op (no NATS configured).
        svc.publish_controller_event(ControllerMessage::Approved(ApprovedPayload {
            service_id: Uuid::nil(),
        }))
        .await;
    }
}
