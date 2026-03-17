use std::sync::Arc;

use uptrakit_internal_wire::{
    Capability, ControllerMessage, HostConnectivityUpdate, HostConnectivityUpdatedPayload,
    SoftwareStatesPayload,
};
use uuid::Uuid;

use crate::service_connections::ServiceConnectionRegistry;
use crate::workload_claims::WorkloadClaimRegistry;

/// Cross-controller notification service.
///
/// Wraps `ServiceConnectionRegistry` to provide the push API. Without NATS,
/// messages are delivered locally only (single-controller mode). When the
/// `nats` feature is enabled and a `NatsTransport` is attached, messages are
/// also published to NATS JetStream for cross-controller delivery.
///
/// When a `WorkloadClaimRegistry` is set, tenant-scoped messages
/// (`SoftwareStates`, `HostConnectivityUpdated`) are routed only to services
/// that hold at least one claimed config key for the target tenant, rather
/// than broadcast to all update-tracking services.
#[derive(Clone)]
pub struct NotificationService {
    registry: ServiceConnectionRegistry,
    controller_id: Uuid,
    /// Tracks whether a NATS transport has been attached. Always present so
    /// that `has_nats()` can be implemented without any `#[cfg]`.
    nats_configured: bool,
    /// Workload claim registry for tenant-scoped routing.
    claim_registry: Option<Arc<WorkloadClaimRegistry>>,
    #[cfg(feature = "nats")]
    nats: Option<crate::nats_transport::NatsTransport>,
}

impl NotificationService {
    pub fn new(registry: ServiceConnectionRegistry, controller_id: Uuid) -> Self {
        Self {
            registry,
            controller_id,
            nats_configured: false,
            claim_registry: None,
            #[cfg(feature = "nats")]
            nats: None,
        }
    }

    /// Attach a workload claim registry for tenant-scoped delivery routing.
    pub fn with_claim_registry(mut self, registry: Arc<WorkloadClaimRegistry>) -> Self {
        self.claim_registry = Some(registry);
        self
    }

    /// Access the workload claim registry, if configured.
    pub fn claim_registry(&self) -> Option<&Arc<WorkloadClaimRegistry>> {
        self.claim_registry.as_ref()
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
    #[tracing::instrument(skip_all, fields(%service_id))]
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
    #[tracing::instrument(skip_all)]
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
    #[tracing::instrument(skip_all)]
    pub async fn publish_controller_event(&self, msg: ControllerMessage) {
        self.maybe_publish_nats(None, Some("controller"), msg).await;
    }

    /// Send a message to all services with a specific capability (local + optional NATS).
    ///
    /// The `capability_str` is the wire-format capability name (e.g. `"update_tracking"`).
    /// It is parsed to a [`Capability`] for local delivery via the registry.
    #[tracing::instrument(skip_all, fields(capability_str))]
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

    /// Load software states for a tenant and push to all locally connected
    /// update-tracking services (immediately) and to NATS for cross-controller delivery.
    ///
    /// Loads both software-item states and host-package host states in two
    /// independent bulk queries, then delivers the merged payload.
    #[tracing::instrument(skip_all, fields(%tenant_id))]
    pub async fn push_software_states_for_tenant(
        &self,
        db: &sea_orm::DatabaseConnection,
        tenant_id: uuid::Uuid,
    ) {
        let tenant_db = uptrakit_shared_db::TenantDb::new(db.clone(), tenant_id);
        let payload = match crate::queries::update_tracking_states::load_software_states_for_tenant(
            &tenant_db,
        )
        .await
        {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    %tenant_id,
                    "failed to load software states for push"
                );
                return;
            }
        };

        self.deliver_software_states(payload).await;
    }

    /// Load software states for a tenant using paginated host delivery and push
    /// each page to all locally connected update-tracking services and to NATS.
    ///
    /// Pages are delivered in order (page 0 first). The receiver accumulates
    /// pages until `page_index + 1 == total_pages` before applying state.
    #[tracing::instrument(skip_all, fields(%tenant_id))]
    pub async fn push_software_states_paginated_for_tenant(
        &self,
        db: &sea_orm::DatabaseConnection,
        tenant_id: uuid::Uuid,
    ) {
        let tenant_db = uptrakit_shared_db::TenantDb::new(db.clone(), tenant_id);
        let page_size = uptrakit_internal_wire::limits::STATES_HOST_PAGE_SIZE;
        let mut host_page: u64 = 0;
        loop {
            let payload =
                match crate::queries::update_tracking_states::load_software_states_page_for_tenant(
                    &tenant_db, host_page, page_size,
                )
                .await
                {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            %tenant_id,
                            host_page,
                            "failed to load software states page for push"
                        );
                        return;
                    }
                };
            let total_pages = u64::from(payload.page.total_pages);
            self.deliver_software_states(payload).await;
            host_page += 1;
            if host_page >= total_pages {
                break;
            }
        }
    }

    /// Deliver a pre-loaded `SoftwareStatesPayload` to all locally connected
    /// update-tracking services and publish to NATS for cross-controller delivery.
    ///
    /// When a `WorkloadClaimRegistry` is set, delivery is scoped to services that
    /// hold at least one claimed config key for the payload's tenant. Falls back
    /// to capability-based broadcast when no claim registry is configured or no
    /// services have claims for the tenant.
    ///
    /// Used by `ControllerSchedulerNotifier` to avoid re-loading an already-loaded
    /// payload through the ORM layer.
    #[tracing::instrument(skip_all, fields(tenant_id = %payload.tenant_id))]
    pub async fn deliver_software_states(&self, payload: SoftwareStatesPayload) {
        let tenant_id = payload.tenant_id;
        let msg = ControllerMessage::SoftwareStates(payload);
        // Deliver to locally connected services — scoped by claim when available.
        self.deliver_tenant_scoped(tenant_id, msg.clone()).await;
        // Publish to NATS for cross-controller delivery.
        self.maybe_publish_nats(None, Some("update_tracking"), msg)
            .await;
    }

    /// Publish a `HostConnectivityUpdated` event to all locally connected
    /// update-tracking services and to NATS for cross-controller delivery.
    ///
    /// This is the authoritative mechanism for notifying update-tracking services
    /// that an agent has connected or disconnected. Unlike `SoftwareStates`,
    /// connectivity state is sourced from the live WebSocket session on the
    /// controller that owns the agent connection — not from a DB query — so it
    /// must be delivered as an event rather than included in the
    /// `SoftwareStates` payload.
    ///
    /// Multi-controller safety: `HostConnectivityUpdated` is NATS-publishable
    /// (contains no credentials) so all update-tracking services across the
    /// cluster receive it regardless of which controller holds the agent
    /// WebSocket.
    #[tracing::instrument(skip_all, fields(%tenant_id))]
    pub async fn send_connectivity_update(
        &self,
        tenant_id: Uuid,
        updates: Vec<HostConnectivityUpdate>,
    ) {
        let payload = HostConnectivityUpdatedPayload::new(tenant_id, updates);
        let msg = ControllerMessage::HostConnectivityUpdated(payload);
        // Deliver to locally connected services — scoped by claim when available.
        self.deliver_tenant_scoped(tenant_id, msg.clone()).await;
        // Publish to NATS for cross-controller delivery.
        self.maybe_publish_nats(None, Some("update_tracking"), msg)
            .await;
    }

    /// Push `HostConnectivityUpdated` "online" events for all agents that are
    /// currently connected and serve hosts in `tenant_id`.
    ///
    /// Called when an update-tracking service (re)connects to ensure the service
    /// receives retained connectivity state for hosts whose agents were already
    /// online before the service connected.
    ///
    /// Only services that are currently registered in the connection registry
    /// receive an "online" event.  Services not in the registry are skipped —
    /// they will emit their own `HostConnectivityUpdated` events once they
    /// reconnect and send `ReportHosts`.
    #[tracing::instrument(skip_all, fields(%tenant_id))]
    pub async fn push_connected_agent_states_for_tenant(
        &self,
        db: &sea_orm::DatabaseConnection,
        tenant_id: uuid::Uuid,
    ) {
        let agent_services =
            match crate::queries::update_tracking_states::load_agent_connectivity_for_tenant(
                db, tenant_id,
            )
            .await
            {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        %tenant_id,
                        "failed to load agent connectivity for push"
                    );
                    return;
                }
            };

        if agent_services.is_empty() {
            return;
        }

        let service_ids: Vec<uuid::Uuid> = agent_services.iter().map(|s| s.service_id).collect();
        let connected = self.registry.filter_connected(&service_ids).await;

        if connected.is_empty() {
            return;
        }

        let now = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();

        let mut updates: Vec<HostConnectivityUpdate> = Vec::new();
        for s in &agent_services {
            if !connected.contains(&s.service_id) {
                continue;
            }
            let last_seen = s
                .last_seen_at
                .and_then(|t| {
                    t.format(&time::format_description::well_known::Rfc3339)
                        .ok()
                })
                .unwrap_or_else(|| now.clone());
            for &host_id in &s.host_ids {
                updates.push(HostConnectivityUpdate::online(
                    host_id,
                    Some(last_seen.clone()),
                    s.client_version.clone(),
                ));
            }
        }

        if !updates.is_empty() {
            self.send_connectivity_update(tenant_id, updates).await;
        }
    }

    /// Return the controller ID.
    pub fn controller_id(&self) -> Uuid {
        self.controller_id
    }

    /// Route a tenant-scoped message to local services.
    ///
    /// When a claim registry is configured, delivers only to services that hold
    /// at least one claimed config key for `tenant_id`. Falls back to
    /// capability-based broadcast when no claims exist or no registry is set.
    async fn deliver_tenant_scoped(&self, tenant_id: Uuid, msg: ControllerMessage) {
        if let Some(ref cr) = self.claim_registry {
            let service_ids = cr.services_for_tenant(tenant_id);
            if !service_ids.is_empty() {
                for svc_id in &service_ids {
                    self.registry.send(svc_id, msg.clone()).await;
                }
                return;
            }
            // No claims for this tenant — fall through to broadcast for graceful
            // degradation (e.g. during rolling deployment or before claims arrive).
        }
        self.registry
            .broadcast_by_capability(&Capability::UpdateTracking, msg)
            .await;
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
