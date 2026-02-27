//! Shared delivery routing logic for cross-controller events.
//!
//! This module extracts the delivery logic formerly embedded in `EventPoller`
//! so that both the NATS consumer and any future transport can reuse the same
//! routing decisions.

use sea_orm::DatabaseConnection;
use uptrakit_internal_wire::{
    Capability, ControllerMessage, MqttClientCreatedPayload, MqttTenantRevokedPayload,
};
use uuid::Uuid;

use crate::mqtt_lease_coordinator::{LeaseCoordinatorError, MqttLeaseCoordinator};
use crate::service_connections::ServiceConnectionRegistry;

/// Parse a capability string back to a typed [`Capability`] variant.
///
/// Returns `None` for unrecognised strings so the caller can fall back to
/// broadcast-to-all semantics.
pub fn parse_capability_str(s: &str) -> Option<Capability> {
    match s {
        "software_discovery" => Some(Capability::SoftwareDiscovery),
        "update_hooks" => Some(Capability::UpdateHooks),
        "graceful_shutdown" => Some(Capability::GracefulShutdown),
        "mqtt_bridge" => Some(Capability::MqttBridge),
        "ssh_remote" => Some(Capability::SshRemote),
        _ => None,
    }
}

/// Deliver a single event to the appropriate local service(s).
///
/// Returns `true` if the message was delivered (or the target is not on
/// this controller), `false` if delivery failed (channel full / send error).
pub async fn deliver_event(
    registry: &ServiceConnectionRegistry,
    db: &DatabaseConnection,
    target_service_id: Option<Uuid>,
    target_capability: Option<&str>,
    msg: ControllerMessage,
) -> bool {
    // Controller-targeted events are handled locally (not forwarded to services)
    if target_service_id.is_none() && target_capability == Some("controller") {
        return deliver_controller_event(db, registry, msg).await;
    }

    match (target_service_id, target_capability) {
        // Targeted to a specific service
        (Some(id), _) => {
            if registry.is_connected(&id).await {
                registry.send(&id, msg).await
            } else {
                // Service not on this controller — not our responsibility.
                true
            }
        }
        // Targeted to services with a specific capability
        (None, Some(cap_str)) => match cap_str {
            "mqtt_bridge" => deliver_mqtt_event(registry, msg).await,
            _ => {
                // For any known capability, broadcast to services with that capability
                if let Some(capability) = parse_capability_str(cap_str) {
                    registry.broadcast_by_capability(&capability, msg).await;
                } else {
                    registry.broadcast(msg).await;
                }
                true
            }
        },
        // No filter — broadcast to all
        (None, None) => {
            registry.broadcast(msg).await;
            true
        }
    }
}

/// Deliver an MQTT-targeted event with special routing for tenant messages.
///
/// Returns `true` if delivery succeeded or the target is not on this
/// controller.
pub async fn deliver_mqtt_event(registry: &ServiceConnectionRegistry, msg: ControllerMessage) -> bool {
    match &msg {
        ControllerMessage::TenantConfigUpdated(payload) => {
            // Route to the specific instance holding this MQTT client
            let mqtt_client_id = payload.tenant.mqtt_client_id;
            if let Some(service_id) = registry
                .get_instance_for_mqtt_client(&mqtt_client_id)
                .await
            {
                registry.send(&service_id, msg).await
            } else {
                // Not on this controller.
                true
            }
        }
        ControllerMessage::TenantRevoked(MqttTenantRevokedPayload {
            mqtt_client_id, ..
        }) => {
            // Route to the specific instance holding this MQTT client
            let mqtt_client_id = *mqtt_client_id;
            if let Some(service_id) = registry
                .get_instance_for_mqtt_client(&mqtt_client_id)
                .await
            {
                registry
                    .release_mqtt_client(&service_id, &mqtt_client_id)
                    .await;
                registry.send(&service_id, msg).await
            } else {
                // Not on this controller.
                true
            }
        }
        _ => {
            // Other MQTT messages: broadcast to all local MQTT services
            registry
                .broadcast_by_capability(&Capability::MqttBridge, msg)
                .await;
            true
        }
    }
}

/// Handle a controller-targeted event (e.g. `MqttClientCreated`).
///
/// Returns `true` on success, `false` on transient failure.
pub async fn deliver_controller_event(
    db: &DatabaseConnection,
    registry: &ServiceConnectionRegistry,
    msg: ControllerMessage,
) -> bool {
    match msg {
        ControllerMessage::MqttClientCreated(MqttClientCreatedPayload { mqtt_client_id }) => {
            let coordinator = MqttLeaseCoordinator::new(db.clone(), registry.clone());
            match coordinator.lease_client_by_id(mqtt_client_id).await {
                Ok(_) => true,
                Err(e) => {
                    if matches!(
                        e.current_context(),
                        LeaseCoordinatorError::MqttClientNotFound(_)
                    ) {
                        return true;
                    }
                    tracing::warn!(
                        error = %e,
                        %mqtt_client_id,
                        "failed to lease MQTT client from cross-controller event"
                    );
                    false
                }
            }
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_capability_str_known_values() {
        assert_eq!(
            parse_capability_str("software_discovery"),
            Some(Capability::SoftwareDiscovery)
        );
        assert_eq!(
            parse_capability_str("update_hooks"),
            Some(Capability::UpdateHooks)
        );
        assert_eq!(
            parse_capability_str("graceful_shutdown"),
            Some(Capability::GracefulShutdown)
        );
        assert_eq!(
            parse_capability_str("mqtt_bridge"),
            Some(Capability::MqttBridge)
        );
        assert_eq!(
            parse_capability_str("ssh_remote"),
            Some(Capability::SshRemote)
        );
    }

    #[test]
    fn parse_capability_str_unknown_returns_none() {
        assert_eq!(parse_capability_str("unknown_capability"), None);
        assert_eq!(parse_capability_str(""), None);
    }

    #[tokio::test]
    async fn deliver_event_broadcast_when_no_filter() {
        let registry = ServiceConnectionRegistry::new();
        let db = sea_orm::Database::connect("sqlite::memory:")
            .await
            .unwrap();

        let msg = ControllerMessage::CaBundleUpdated(
            uptrakit_internal_wire::CaBundleUpdatedPayload {
                ca_bundle_pem: "pem".to_string(),
            },
        );
        // With no connected services, broadcast succeeds.
        let result = deliver_event(&registry, &db, None, None, msg).await;
        assert!(result);
    }

    #[tokio::test]
    async fn deliver_event_service_targeted_not_connected() {
        let registry = ServiceConnectionRegistry::new();
        let db = sea_orm::Database::connect("sqlite::memory:")
            .await
            .unwrap();

        let service_id = Uuid::now_v7();
        let msg = ControllerMessage::CaBundleUpdated(
            uptrakit_internal_wire::CaBundleUpdatedPayload {
                ca_bundle_pem: "pem".to_string(),
            },
        );
        // Service not on this controller — returns true (not our responsibility).
        let result = deliver_event(&registry, &db, Some(service_id), None, msg).await;
        assert!(result);
    }

    #[tokio::test]
    async fn deliver_event_capability_targeted() {
        let registry = ServiceConnectionRegistry::new();
        let db = sea_orm::Database::connect("sqlite::memory:")
            .await
            .unwrap();

        let msg = ControllerMessage::CaBundleUpdated(
            uptrakit_internal_wire::CaBundleUpdatedPayload {
                ca_bundle_pem: "pem".to_string(),
            },
        );
        let result = deliver_event(&registry, &db, None, Some("software_discovery"), msg).await;
        assert!(result);
    }
}
