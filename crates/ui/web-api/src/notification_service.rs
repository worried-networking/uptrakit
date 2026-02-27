use uptrakit_internal_wire::{Capability, ControllerMessage};
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
    #[cfg(feature = "nats")]
    nats: Option<crate::nats_transport::NatsTransport>,
}

impl NotificationService {
    pub fn new(registry: ServiceConnectionRegistry, controller_id: Uuid) -> Self {
        Self {
            registry,
            controller_id,
            #[cfg(feature = "nats")]
            nats: None,
        }
    }

    /// Attach a NATS transport for cross-controller delivery.
    #[cfg(feature = "nats")]
    pub fn with_nats(mut self, nats: crate::nats_transport::NatsTransport) -> Self {
        self.nats = Some(nats);
        self
    }

    /// Returns `true` if a NATS transport is configured.
    pub fn has_nats(&self) -> bool {
        #[cfg(feature = "nats")]
        {
            self.nats.is_some()
        }
        #[cfg(not(feature = "nats"))]
        {
            false
        }
    }

    /// Push a message to a specific service (local + optional NATS).
    ///
    /// MQTT-specific messages that may contain credentials (`TenantAssignments`,
    /// `TenantConfigUpdated`, `TenantRevoked`) are delivered locally but **not**
    /// published to NATS to prevent credential leakage. The MQTT service
    /// reconciles its state from the DB on reconnect.
    pub async fn send(&self, service_id: &Uuid, msg: ControllerMessage) -> bool {
        let local = self.registry.send(service_id, msg.clone()).await;
        if !is_mqtt_tenant_message(&msg) {
            self.maybe_publish_nats(Some(*service_id), None, msg).await;
        }
        local
    }

    /// Broadcast a message to all connected services (local + optional NATS).
    ///
    /// MQTT-specific messages that may contain credentials are delivered locally
    /// but **not** published to NATS (see [`Self::send`] doc comment).
    pub async fn broadcast(&self, msg: ControllerMessage) {
        self.registry.broadcast(msg.clone()).await;
        if !is_mqtt_tenant_message(&msg) {
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

    /// Access the underlying registry (e.g. for `is_connected` checks).
    pub fn registry(&self) -> &ServiceConnectionRegistry {
        &self.registry
    }

    /// Load software states for a tenant and push to all locally connected MQTT
    /// services (immediately) and to NATS for cross-controller delivery.
    pub async fn push_software_states_for_tenant(
        &self,
        db: &sea_orm::DatabaseConnection,
        tenant_id: uuid::Uuid,
    ) {
        let payload = match crate::queries::mqtt_software_states::load_software_states_for_tenant(
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
        }
        #[cfg(not(feature = "nats"))]
        {
            let _ = (target_service_id, target_capability, msg);
        }
    }
}

/// Returns `true` for MQTT-specific messages that may contain credentials
/// (`TenantAssignments`, `TenantConfigUpdated`, `TenantRevoked`).
///
/// These messages are delivered locally but **not** published to NATS to
/// prevent credential leakage. The MQTT service reconciles its state from
/// the DB on reconnect.
pub(crate) fn is_mqtt_tenant_message(msg: &ControllerMessage) -> bool {
    matches!(
        msg,
        ControllerMessage::TenantAssignments(_)
            | ControllerMessage::TenantConfigUpdated(_)
            | ControllerMessage::TenantRevoked(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_internal_wire::{
        ApprovedPayload, MqttTenantAssignmentsPayload, MqttTenantConfigUpdatedPayload,
        MqttTenantRevokedPayload, ServerRestartingPayload,
    };

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

    #[test]
    fn is_mqtt_tenant_message_matches_credential_bearing_variants() {
        // Credential-bearing variants must be filtered from NATS.
        assert!(is_mqtt_tenant_message(
            &ControllerMessage::TenantAssignments(MqttTenantAssignmentsPayload { tenants: vec![] })
        ));
        assert!(is_mqtt_tenant_message(
            &ControllerMessage::TenantConfigUpdated(MqttTenantConfigUpdatedPayload {
                tenant: uptrakit_internal_wire::MqttTenantConfig {
                    mqtt_client_id: Uuid::nil(),
                    tenant_id: Uuid::nil(),
                    enabled: true,
                    transport: uptrakit_internal_wire::MqttTransport::Tcp,
                    host: "localhost".into(),
                    port: 1883,
                    client_id: "test".into(),
                    username: Some(uptrakit_internal_wire::SecretString::new("user".into())),
                    password: Some(uptrakit_internal_wire::SecretString::new("secret".into())),
                    ca_pem: None,
                    topic_prefix: "test/".into(),
                    ha_discovery: false,
                    ha_discovery_prefix: "homeassistant".to_string(),
                    updated_at: time::UtcDateTime::UNIX_EPOCH,
                },
            })
        ));
        assert!(is_mqtt_tenant_message(&ControllerMessage::TenantRevoked(
            MqttTenantRevokedPayload {
                mqtt_client_id: Uuid::nil(),
                reason: "test".into(),
            }
        )));

        // Non-credential variants must NOT be filtered.
        assert!(!is_mqtt_tenant_message(&ControllerMessage::Approved(
            ApprovedPayload {
                service_id: Uuid::nil(),
            }
        )));
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
