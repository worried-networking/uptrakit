//! Shared delivery routing logic for cross-controller events.
//!
//! This module extracts the delivery logic formerly embedded in `EventPoller`
//! so that both the NATS consumer and any future transport can reuse the same
//! routing decisions.

use std::sync::Arc;

use sea_orm::DatabaseConnection;
use tokio::sync::Notify;
use uptrakit_internal_wire::{
    BroadcastAdminEventPayload, Capability, ControllerMessage, MqttClientCreatedPayload,
    MqttTenantRevokedPayload, SoftwareStatesChangedPayload, TokenRevokedPayload,
};
use uptrakit_web_api_types::events::AdminEvent;
use uuid::Uuid;

use crate::mqtt_lease_coordinator::{LeaseCoordinatorError, MqttLeaseCoordinator};
use crate::service_connections::ServiceConnectionRegistry;

/// Optional controller-local resources needed when processing controller-targeted events.
///
/// All fields may be `None` when the corresponding subsystem is not running on this instance.
pub struct ControllerResources<'a> {
    /// Notification service for loading and pushing software states on
    /// `SoftwareStatesChanged` signals from the external scheduler.
    pub notification_service: Option<&'a crate::notification_service::NotificationService>,
    pub ca_rotation_trigger: Option<&'a Arc<Notify>>,
    pub revocation_notify: Option<&'a Arc<Notify>>,
    pub token_denylist: Option<&'a Arc<crate::auth::token_denylist::TokenDenylist>>,
    /// Admin event broadcaster for relaying cross-controller SSE events.
    ///
    /// When set, `BroadcastAdminEvent` messages are decoded and re-broadcast
    /// to local SSE subscribers using `send_local` / `send_global_local`
    /// (without re-publishing to NATS to avoid loops).
    pub event_broadcaster: Option<&'a crate::event_broadcaster::EventBroadcaster>,
}

/// Parse a capability string back to a typed [`Capability`] variant.
///
/// Returns `None` for unrecognised strings so the caller can fall back to
/// broadcast-to-all semantics.
pub fn parse_capability_str(s: &str) -> Option<Capability> {
    match s {
        "software_discovery" => Some(Capability::SoftwareDiscovery),
        "update_hooks" => Some(Capability::UpdateHooks),
        "graceful_shutdown" => Some(Capability::GracefulShutdown),
        "update_tracking" => Some(Capability::UpdateTracking),
        "ssh_remote" => Some(Capability::SshRemote),
        "scheduler" => Some(Capability::Scheduler),
        "database_access" => Some(Capability::DatabaseAccess),
        "nats_access" => Some(Capability::NatsAccess),
        "master_key_access" => Some(Capability::MasterKeyAccess),
        "ca_management" => Some(Capability::CaManagement),
        _ => None,
    }
}

/// Deliver a single event to the appropriate local service(s).
///
/// Returns `true` if the message was delivered (or the target is not on
/// this controller), `false` if delivery failed (channel full / send error).
#[tracing::instrument(skip_all)]
pub async fn deliver_event(
    registry: &ServiceConnectionRegistry,
    db: &DatabaseConnection,
    resources: &ControllerResources<'_>,
    target_service_id: Option<Uuid>,
    target_capability: Option<&str>,
    msg: ControllerMessage,
) -> bool {
    // Controller-targeted events are handled locally (not forwarded to services)
    if target_service_id.is_none() && target_capability == Some("controller") {
        return deliver_controller_event(db, registry, resources, msg).await;
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
            "update_tracking" => deliver_mqtt_event(registry, msg).await,
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
#[tracing::instrument(skip_all)]
pub async fn deliver_mqtt_event(
    registry: &ServiceConnectionRegistry,
    msg: ControllerMessage,
) -> bool {
    match &msg {
        ControllerMessage::TenantConfigUpdated(payload) => {
            // Route to the specific instance holding this MQTT client
            let mqtt_client_id = payload.tenant.mqtt_client_id;
            if let Some(service_id) = registry.get_instance_for_mqtt_client(&mqtt_client_id).await {
                registry.send(&service_id, msg).await
            } else {
                // Not on this controller.
                true
            }
        }
        ControllerMessage::TenantRevoked(MqttTenantRevokedPayload { mqtt_client_id, .. }) => {
            // Route to the specific instance holding this MQTT client
            let mqtt_client_id = *mqtt_client_id;
            if let Some(service_id) = registry.get_instance_for_mqtt_client(&mqtt_client_id).await {
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
                .broadcast_by_capability(&Capability::UpdateTracking, msg)
                .await;
            true
        }
    }
}

/// Handle a controller-targeted event (e.g. `MqttClientCreated`, `RequestCaRotation`,
/// `TokenRevoked`).
///
/// Returns `true` on success, `false` on transient failure.
#[tracing::instrument(skip_all)]
pub async fn deliver_controller_event(
    db: &DatabaseConnection,
    registry: &ServiceConnectionRegistry,
    resources: &ControllerResources<'_>,
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
        ControllerMessage::RequestCaRotation(payload) => {
            if let Some(trigger) = resources.ca_rotation_trigger {
                tracing::info!(reason = %payload.reason, "CA rotation requested via cross-controller event");
                trigger.notify_one();
            } else {
                tracing::debug!("received RequestCaRotation but no CA rotation trigger configured");
            }
            true
        }
        ControllerMessage::RequestCrlRenewal(_) => {
            if let Some(notify) = resources.revocation_notify {
                tracing::debug!("CRL rebuild requested via cross-controller event");
                notify.notify_one();
            } else {
                tracing::debug!("received RequestCrlRenewal but no revocation notify configured");
            }
            true
        }
        ControllerMessage::TokenRevoked(TokenRevokedPayload {
            jti,
            exp,
            user_id,
            iat_cutoff,
            purge_after,
        }) => {
            if let Some(denylist) = resources.token_denylist {
                // JTI-level revocation from another controller instance.
                if let (Some(jti), Some(exp)) = (jti, exp) {
                    denylist.deny_token_remote(&jti, exp).await;
                }
                // User-level revocation from another controller instance.
                if let (Some(uid), Some(cutoff), Some(purge)) = (user_id, iat_cutoff, purge_after) {
                    denylist.deny_user_remote(uid, cutoff, purge).await;
                }
            } else {
                tracing::debug!("received TokenRevoked but no token denylist configured");
            }
            true
        }
        ControllerMessage::BroadcastAdminEvent(BroadcastAdminEventPayload {
            tenant_id,
            event_json,
        }) => {
            if let Some(broadcaster) = resources.event_broadcaster {
                match serde_json::from_str::<AdminEvent>(&event_json) {
                    Ok(event) => {
                        if let Some(tid) = tenant_id {
                            broadcaster.send_local(tid, event).await;
                        } else {
                            broadcaster.send_global_local(event).await;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "failed to deserialise BroadcastAdminEvent payload, skipping"
                        );
                    }
                }
            } else {
                tracing::debug!("received BroadcastAdminEvent but no event_broadcaster configured");
            }
            true
        }
        ControllerMessage::SoftwareStatesChanged(SoftwareStatesChangedPayload {
            tenant_id,
            ..
        }) => {
            if let Some(ns) = resources.notification_service {
                ns.push_software_states_for_tenant(db, tenant_id).await;
            } else {
                tracing::debug!(
                    "received SoftwareStatesChanged but no notification_service configured"
                );
            }
            true
        }
        _ => {
            tracing::warn!(
                msg_type = ?std::mem::discriminant(&msg),
                "unhandled controller-targeted event"
            );
            true
        }
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
            parse_capability_str("update_tracking"),
            Some(Capability::UpdateTracking)
        );
        assert_eq!(
            parse_capability_str("ssh_remote"),
            Some(Capability::SshRemote)
        );
        assert_eq!(
            parse_capability_str("scheduler"),
            Some(Capability::Scheduler)
        );
        assert_eq!(
            parse_capability_str("database_access"),
            Some(Capability::DatabaseAccess)
        );
        assert_eq!(
            parse_capability_str("nats_access"),
            Some(Capability::NatsAccess)
        );
        assert_eq!(
            parse_capability_str("master_key_access"),
            Some(Capability::MasterKeyAccess)
        );
        assert_eq!(
            parse_capability_str("ca_management"),
            Some(Capability::CaManagement)
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
        let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();

        let msg =
            ControllerMessage::CaBundleUpdated(uptrakit_internal_wire::CaBundleUpdatedPayload {
                ca_bundle_pem: "pem".to_string(),
            });
        // With no connected services, broadcast succeeds.
        let resources = ControllerResources {
            notification_service: None,
            ca_rotation_trigger: None,
            revocation_notify: None,
            token_denylist: None,
            event_broadcaster: None,
        };
        let result = deliver_event(&registry, &db, &resources, None, None, msg).await;
        assert!(result);
    }

    #[tokio::test]
    async fn deliver_event_service_targeted_not_connected() {
        let registry = ServiceConnectionRegistry::new();
        let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();

        let service_id = Uuid::now_v7();
        let msg =
            ControllerMessage::CaBundleUpdated(uptrakit_internal_wire::CaBundleUpdatedPayload {
                ca_bundle_pem: "pem".to_string(),
            });
        // Service not on this controller — returns true (not our responsibility).
        let resources = ControllerResources {
            notification_service: None,
            ca_rotation_trigger: None,
            revocation_notify: None,
            token_denylist: None,
            event_broadcaster: None,
        };
        let result = deliver_event(&registry, &db, &resources, Some(service_id), None, msg).await;
        assert!(result);
    }

    #[tokio::test]
    async fn deliver_event_capability_targeted() {
        let registry = ServiceConnectionRegistry::new();
        let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();

        let msg =
            ControllerMessage::CaBundleUpdated(uptrakit_internal_wire::CaBundleUpdatedPayload {
                ca_bundle_pem: "pem".to_string(),
            });
        let resources = ControllerResources {
            notification_service: None,
            ca_rotation_trigger: None,
            revocation_notify: None,
            token_denylist: None,
            event_broadcaster: None,
        };
        let result = deliver_event(
            &registry,
            &db,
            &resources,
            None,
            Some("software_discovery"),
            msg,
        )
        .await;
        assert!(result);
    }
}
