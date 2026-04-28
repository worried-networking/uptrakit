//! Shared delivery routing logic for cross-controller events.
//!
//! This module extracts the delivery logic formerly embedded in `EventPoller`
//! so that both the NATS consumer and any future transport can reuse the same
//! routing decisions.

use std::sync::Arc;

use sea_orm::DatabaseConnection;
use tokio::sync::Notify;
use uptrakit_web_api_types::events::AdminEvent;
use uptrakit_wire::{
    BroadcastAdminEventPayload, Capability, ControllerMessage, SoftwareStatesChangedPayload,
    TokenRevokedPayload, WorkloadClaimResultPayload,
};
use uuid::Uuid;

use crate::service_connections::ServiceConnectionRegistry;
use crate::workload_claims::WorkloadClaimRegistry;

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
    /// Workload claim registry for tenant-scoped routing of remote events.
    pub claim_registry: Option<&'a Arc<WorkloadClaimRegistry>>,
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
        "workload_claims" => Some(Capability::WorkloadClaims),
        "system_service" => Some(Capability::SystemService),
        "ui_surfaces" => Some(Capability::UiSurfaces),
        _ => None,
    }
}

/// Extract `tenant_id` from a [`ControllerMessage`] when it carries tenant-scoped data.
///
/// Returns `Some(tenant_id)` for `SoftwareStates` and `HostConnectivityUpdated`
/// messages, which should be routed via the workload claim registry rather than
/// broadcast to all services with a matching capability.
pub fn extract_tenant_id(msg: &ControllerMessage) -> Option<Uuid> {
    match msg {
        ControllerMessage::SoftwareStates(p) => Some(p.tenant_id),
        ControllerMessage::HostConnectivityUpdated(p) => Some(p.tenant_id),
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
        return deliver_controller_event(db, resources, msg).await;
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
        (None, Some(cap_str)) => {
            if let Some(capability) = parse_capability_str(cap_str) {
                // Attempt tenant-scoped delivery via claim registry
                if let Some(tenant_id) = extract_tenant_id(&msg)
                    && let Some(cr) = resources.claim_registry
                {
                    let service_ids = cr.services_for_tenant(tenant_id);
                    if !service_ids.is_empty() {
                        for svc_id in &service_ids {
                            registry.send(svc_id, msg.clone()).await;
                        }
                        return true;
                    }
                    // No local claimants — skip (served by other controllers)
                    return true;
                }
                // No tenant scope or no claim registry — broadcast by capability
                registry.broadcast_by_capability(&capability, msg).await;
            } else {
                registry.broadcast(msg).await;
            }
            true
        }
        // No filter — broadcast to all
        (None, None) => {
            registry.broadcast(msg).await;
            true
        }
    }
}

/// Handle a controller-targeted event (e.g. `RequestCaRotation`, `TokenRevoked`).
///
/// Returns `true` on success, `false` on transient failure.
#[tracing::instrument(skip_all)]
pub async fn deliver_controller_event(
    db: &DatabaseConnection,
    resources: &ControllerResources<'_>,
    msg: ControllerMessage,
) -> bool {
    match msg {
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
        ControllerMessage::WorkloadClaimAnnouncement(ref payload) => {
            if let Some(cr) = resources.claim_registry {
                let claimed_at = time::OffsetDateTime::parse(
                    &payload.claimed_at,
                    &time::format_description::well_known::Rfc3339,
                )
                .unwrap_or_else(|_| time::OffsetDateTime::now_utc());

                let revocations = cr.apply_remote_announcement(
                    payload.service_id,
                    payload.controller_id,
                    &payload.claimed,
                    &payload.released,
                    claimed_at,
                );
                let registry = resources.notification_service.map(|ns| ns.registry());
                // Send revocation results to affected local services.
                for rev in &revocations {
                    if let Some(reg) = registry {
                        let result = WorkloadClaimResultPayload::new(
                            std::collections::BTreeSet::new(),
                            rev.revoked_keys.clone(),
                        );
                        reg.send(
                            &rev.service_id,
                            ControllerMessage::WorkloadClaimResult(result),
                        )
                        .await;
                    }
                }
                // Proactive re-grant for released keys that local services wanted.
                if !payload.released.is_empty() {
                    let re_grantable = cr.find_pending_desires_for_keys(&payload.released);
                    if let (Some(ns), false) =
                        (resources.notification_service, re_grantable.is_empty())
                    {
                        let controller_id = ns.controller_id();
                        for (svc_id, desired) in re_grantable {
                            let result = cr.try_claim(svc_id, controller_id, desired);
                            if !result.granted.is_empty() {
                                let new_tenants = result.new_tenants();
                                let claim_result = WorkloadClaimResultPayload::new(
                                    result.granted.clone(),
                                    result.rejected,
                                );
                                ns.registry()
                                    .send(
                                        &svc_id,
                                        ControllerMessage::WorkloadClaimResult(claim_result),
                                    )
                                    .await;
                                // Announce newly granted claims via NATS.
                                let claimed_at_str = time::OffsetDateTime::now_utc()
                                    .format(&time::format_description::well_known::Rfc3339)
                                    .unwrap_or_default();
                                let announcement =
                                    uptrakit_wire::WorkloadClaimAnnouncementPayload::new(
                                        svc_id,
                                        controller_id,
                                        result
                                            .granted
                                            .iter()
                                            .filter_map(|k| {
                                                cr.tenant_for_key(k).map(|tid| (k.clone(), tid))
                                            })
                                            .collect(),
                                        std::collections::BTreeSet::new(),
                                        claimed_at_str,
                                    );
                                ns.publish_controller_event(
                                    ControllerMessage::WorkloadClaimAnnouncement(announcement),
                                )
                                .await;
                                // Push initial state for newly served tenants.
                                for tid in &new_tenants {
                                    ns.push_software_states_paginated_for_tenant(db, *tid).await;
                                    ns.push_connected_agent_states_for_tenant(db, *tid).await;
                                }
                            }
                        }
                    }
                }
            } else {
                tracing::debug!(
                    "received WorkloadClaimAnnouncement but no claim_registry configured"
                );
            }
            true
        }
        ControllerMessage::WorkloadClaimSyncRequest(ref payload) => {
            // Another controller is requesting our local claim state.
            if let (Some(cr), Some(ns)) = (resources.claim_registry, resources.notification_service)
            {
                let local = cr.local_claims(ns.controller_id());
                let response = build_sync_response(ns.controller_id(), &local);
                ns.publish_controller_event(ControllerMessage::WorkloadClaimSyncResponse(response))
                    .await;
                tracing::info!(
                    requester = %payload.controller_id,
                    local_claims = local.len(),
                    "responded to WorkloadClaimSyncRequest"
                );
            }
            true
        }
        ControllerMessage::WorkloadClaimSyncResponse(ref payload) => {
            // Merge remote claims into our global registry.
            if let Some(cr) = resources.claim_registry {
                let claims = parse_sync_claims(&payload.claims);
                cr.apply_sync_response(payload.controller_id, &claims);
                tracing::info!(
                    responder = %payload.controller_id,
                    claims = payload.claims.len(),
                    "merged WorkloadClaimSyncResponse into global registry"
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

/// Build a [`WorkloadClaimSyncResponsePayload`] from the local claims map.
fn build_sync_response(
    controller_id: Uuid,
    local: &std::collections::BTreeMap<String, (Uuid, Uuid, time::OffsetDateTime)>,
) -> uptrakit_wire::WorkloadClaimSyncResponsePayload {
    use std::collections::BTreeMap;
    use uptrakit_wire::{WorkloadClaimSyncEntry, WorkloadClaimSyncResponsePayload};

    let claims: BTreeMap<String, WorkloadClaimSyncEntry> = local
        .iter()
        .map(|(key, (service_id, tenant_id, claimed_at))| {
            let ts = claimed_at
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_default();
            (
                key.clone(),
                WorkloadClaimSyncEntry::new(*service_id, *tenant_id, ts),
            )
        })
        .collect();
    WorkloadClaimSyncResponsePayload::new(controller_id, claims)
}

/// Parse sync response claims from wire format into the internal tuple format.
fn parse_sync_claims(
    wire_claims: &std::collections::BTreeMap<String, uptrakit_wire::WorkloadClaimSyncEntry>,
) -> std::collections::BTreeMap<String, (Uuid, Uuid, time::OffsetDateTime)> {
    wire_claims
        .iter()
        .map(|(key, entry)| {
            let claimed_at = time::OffsetDateTime::parse(
                &entry.claimed_at,
                &time::format_description::well_known::Rfc3339,
            )
            .unwrap_or_else(|_| time::OffsetDateTime::now_utc());
            (key.clone(), (entry.service_id, entry.tenant_id, claimed_at))
        })
        .collect()
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
        assert_eq!(
            parse_capability_str("workload_claims"),
            Some(Capability::WorkloadClaims)
        );
        assert_eq!(
            parse_capability_str("system_service"),
            Some(Capability::SystemService)
        );
        assert_eq!(
            parse_capability_str("ui_surfaces"),
            Some(Capability::UiSurfaces)
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

        let msg = ControllerMessage::CaBundleUpdated(uptrakit_wire::CaBundleUpdatedPayload {
            ca_bundle_pem: "pem".to_string(),
        });
        // With no connected services, broadcast succeeds.
        let resources = ControllerResources {
            notification_service: None,
            ca_rotation_trigger: None,
            revocation_notify: None,
            token_denylist: None,
            event_broadcaster: None,
            claim_registry: None,
        };
        let result = deliver_event(&registry, &db, &resources, None, None, msg).await;
        assert!(result);
    }

    #[tokio::test]
    async fn deliver_event_service_targeted_not_connected() {
        let registry = ServiceConnectionRegistry::new();
        let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();

        let service_id = Uuid::now_v7();
        let msg = ControllerMessage::CaBundleUpdated(uptrakit_wire::CaBundleUpdatedPayload {
            ca_bundle_pem: "pem".to_string(),
        });
        // Service not on this controller — returns true (not our responsibility).
        let resources = ControllerResources {
            notification_service: None,
            ca_rotation_trigger: None,
            revocation_notify: None,
            token_denylist: None,
            event_broadcaster: None,
            claim_registry: None,
        };
        let result = deliver_event(&registry, &db, &resources, Some(service_id), None, msg).await;
        assert!(result);
    }

    #[tokio::test]
    async fn deliver_event_capability_targeted() {
        let registry = ServiceConnectionRegistry::new();
        let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();

        let msg = ControllerMessage::CaBundleUpdated(uptrakit_wire::CaBundleUpdatedPayload {
            ca_bundle_pem: "pem".to_string(),
        });
        let resources = ControllerResources {
            notification_service: None,
            ca_rotation_trigger: None,
            revocation_notify: None,
            token_denylist: None,
            event_broadcaster: None,
            claim_registry: None,
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
