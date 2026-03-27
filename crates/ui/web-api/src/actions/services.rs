use uuid::Uuid;

use crate::actions::MutationContext;
use crate::app_state::CertState;
use crate::notifications::events::{NotificationEvent, NotificationEventDetails};
use crate::queries::services::{self as svc_queries, ServiceQueryError};
use crate::service_connections::ServiceConnectionRegistry;
use crate::tenant_db::TenantDb;
use uptrakit_internal_wire::{
    ApprovedPayload, ControllerMessage, RejectedPayload, RequestCrlRenewalPayload,
};
use uptrakit_web_api_types::events::AdminEvent;
use uptrakit_web_api_types::services::ServiceResponse;

pub(crate) async fn approve(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    service_id: Uuid,
) -> Result<ServiceResponse, rootcause::Report<ServiceQueryError>> {
    let resp = svc_queries::approve_service(tenant_db, service_id).await?;

    let _ = ctx
        .notification_service
        .send(
            &service_id,
            ControllerMessage::Approved(ApprovedPayload { service_id }),
        )
        .await;

    let service_label = resp.service_label.clone();
    ctx.notification_dispatcher.dispatch(NotificationEvent {
        tenant_id: tenant_db.tenant_id,
        host_id: None,
        host_name: None,
        software_item_id: None,
        software_item_name: None,
        plugin_type: None,
        details: NotificationEventDetails::NewServiceEnrolled {
            service_id,
            service_label,
        },
    });

    ctx.event_broadcaster
        .send(
            tenant_db.tenant_id,
            AdminEvent::ServiceStatusChanged {
                id: service_id,
                status: "approved".to_string(),
            },
        )
        .await;

    Ok(resp)
}

/// NOTE: Does NOT dispatch a `NotificationEvent` (preserved asymmetry — individual approve only).
pub(crate) async fn reject(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    service_id: Uuid,
    service_connections: &ServiceConnectionRegistry,
) -> Result<ServiceResponse, rootcause::Report<ServiceQueryError>> {
    let resp = svc_queries::reject_service(tenant_db, service_id).await?;

    let _ = ctx
        .notification_service
        .send(
            &service_id,
            ControllerMessage::Rejected(RejectedPayload { service_id }),
        )
        .await;

    service_connections.force_disconnect(&service_id).await;

    ctx.event_broadcaster
        .send(
            tenant_db.tenant_id,
            AdminEvent::ServiceStatusChanged {
                id: service_id,
                status: "rejected".to_string(),
            },
        )
        .await;

    Ok(resp)
}

/// Returns `Ok(true)` when the service was found and deactivated; `Ok(false)` when not found.
/// No side effects are run when `Ok(false)`.
pub(crate) async fn deactivate(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    service_id: Uuid,
    default_tenant_id: Uuid,
    cert: &CertState,
    service_connections: &ServiceConnectionRegistry,
) -> Result<bool, rootcause::Report<ServiceQueryError>> {
    let found = svc_queries::deactivate_service(tenant_db, service_id, default_tenant_id).await?;
    if !found {
        return Ok(false);
    }

    cert.revocation_notify.notify_one();
    ctx.notification_service
        .publish_controller_event(ControllerMessage::RequestCrlRenewal(
            RequestCrlRenewalPayload::default(),
        ))
        .await;
    service_connections.force_disconnect(&service_id).await;

    ctx.event_broadcaster
        .send(
            tenant_db.tenant_id,
            AdminEvent::ServiceStatusChanged {
                id: service_id,
                status: "deactivated".to_string(),
            },
        )
        .await;

    Ok(true)
}

/// Parameters for a service merge operation.
pub(crate) struct MergeParams {
    pub target_id: Uuid,
    pub source_id: Uuid,
    pub target_connected: bool,
    pub default_tenant_id: Uuid,
}

/// NOTE: Does NOT broadcast `ServiceStatusChanged` (preserved asymmetry — merge has no status event).
pub(crate) async fn merge(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    params: MergeParams,
    cert: &CertState,
    service_connections: &ServiceConnectionRegistry,
) -> Result<ServiceResponse, rootcause::Report<ServiceQueryError>> {
    let resp = svc_queries::merge_service(
        tenant_db,
        params.target_id,
        params.source_id,
        params.target_connected,
        params.default_tenant_id,
    )
    .await?;

    cert.revocation_notify.notify_one();
    ctx.notification_service
        .publish_controller_event(ControllerMessage::RequestCrlRenewal(
            RequestCrlRenewalPayload::default(),
        ))
        .await;
    service_connections
        .force_disconnect(&params.source_id)
        .await;

    // Preserved asymmetry: merge does NOT broadcast ServiceStatusChanged.
    // See design doc "Preserved asymmetric behaviors" item 1.

    Ok(resp)
}

/// NOTE: Does NOT dispatch a `NotificationEvent` (preserved asymmetry — individual approve only).
pub(crate) async fn batch_approve(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    ids: &[Uuid],
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>), rootcause::Report<ServiceQueryError>> {
    let (succeeded_ids, failed) = svc_queries::batch_approve_services(tenant_db, ids).await?;

    for id in &succeeded_ids {
        let _ = ctx
            .notification_service
            .send(
                id,
                ControllerMessage::Approved(ApprovedPayload { service_id: *id }),
            )
            .await;

        // Preserved asymmetry: batch approve does NOT dispatch NotificationEvent::NewServiceEnrolled.
        // Only individual approve triggers the notification dispatcher.

        ctx.event_broadcaster
            .send(
                tenant_db.tenant_id,
                AdminEvent::ServiceStatusChanged {
                    id: *id,
                    status: "approved".to_string(),
                },
            )
            .await;
    }

    Ok((succeeded_ids, failed))
}

pub(crate) async fn batch_reject(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    ids: &[Uuid],
    service_connections: &ServiceConnectionRegistry,
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>), rootcause::Report<ServiceQueryError>> {
    let (succeeded_ids, failed) = svc_queries::batch_reject_services(tenant_db, ids).await?;

    for id in &succeeded_ids {
        let _ = ctx
            .notification_service
            .send(
                id,
                ControllerMessage::Rejected(RejectedPayload { service_id: *id }),
            )
            .await;
        service_connections.force_disconnect(id).await;
        ctx.event_broadcaster
            .send(
                tenant_db.tenant_id,
                AdminEvent::ServiceStatusChanged {
                    id: *id,
                    status: "rejected".to_string(),
                },
            )
            .await;
    }

    Ok((succeeded_ids, failed))
}

pub(crate) async fn batch_deactivate(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    ids: &[Uuid],
    default_tenant_id: Uuid,
    cert: &CertState,
    service_connections: &ServiceConnectionRegistry,
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>), rootcause::Report<ServiceQueryError>> {
    let (succeeded_ids, failed) =
        svc_queries::batch_deactivate_services(tenant_db, ids, default_tenant_id).await?;

    for id in &succeeded_ids {
        cert.revocation_notify.notify_one();
        ctx.notification_service
            .publish_controller_event(ControllerMessage::RequestCrlRenewal(
                RequestCrlRenewalPayload::default(),
            ))
            .await;
        service_connections.force_disconnect(id).await;
        ctx.event_broadcaster
            .send(
                tenant_db.tenant_id,
                AdminEvent::ServiceStatusChanged {
                    id: *id,
                    status: "deactivated".to_string(),
                },
            )
            .await;
    }

    Ok((succeeded_ids, failed))
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use std::sync::Arc;

    use tokio::sync::broadcast::error::TryRecvError;
    use uptrakit_shared_db::entity::service::ServiceStatus;
    use uptrakit_web_api_types::events::AdminEvent;
    use uuid::Uuid;

    use super::*;
    use crate::actions::MutationContext;
    use crate::event_broadcaster::EventBroadcaster;
    use crate::notification_service::NotificationService;
    use crate::notifications::dispatcher::NotificationDispatcher;
    use crate::notifications::events::NotificationEventDetails;
    use crate::service_connections::ServiceConnectionRegistry;
    use crate::tenant_db::TenantDb;
    use crate::test_harness::fixtures::insert_service;
    use crate::test_harness::{insert_default_tenant, setup_migrated_db};

    fn make_cert_state() -> CertState {
        let ca_pem = "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n";
        let snapshot_data = crate::ca_snapshot::CaPublicSnapshot {
            active_cert_pem: ca_pem.to_string(),
            active_fingerprint: "0".repeat(64),
            previous_cert_pem: None,
            previous_fingerprint: None,
            trusted_cas: vec![],
            trusted_ca_cns: vec![],
            bundle_pem: ca_pem.to_string(),
            bundle_hash: "0".repeat(64),
            managed: true,
            active_not_after: time::OffsetDateTime::now_utc() + time::Duration::days(365),
            pki_addr: None,
        };
        let (_tx, ca_snapshot) = tokio::sync::watch::channel(snapshot_data);
        CertState {
            ca_snapshot,
            ca_key_store: Arc::new(tokio::sync::RwLock::new(crate::ca_snapshot::CaKeyStore {
                active_key_pem: zeroize::Zeroizing::new(String::new()),
                previous_key_pem: None,
                trusted_ca_keys: vec![],
            })),
            revocation_notify: Arc::new(tokio::sync::Notify::const_new()),
            crl_pem_cache: Arc::new(tokio::sync::RwLock::new(String::new())),
            ca_rotation_trigger: Arc::new(tokio::sync::Notify::const_new()),
        }
    }

    fn build_ctx_parts() -> (
        NotificationService,
        NotificationDispatcher,
        EventBroadcaster,
        tokio::sync::mpsc::Receiver<crate::notifications::events::NotificationEvent>,
    ) {
        let (dispatcher, dispatcher_rx) = NotificationDispatcher::test_channel();
        let broadcaster = EventBroadcaster::new();
        let notification_svc =
            NotificationService::new(ServiceConnectionRegistry::default(), Uuid::nil());
        (notification_svc, dispatcher, broadcaster, dispatcher_rx)
    }

    // ── approve ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn approve_dispatches_notification_and_broadcasts() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let svc = insert_service(&db, tenant_id, ServiceStatus::Pending).await;
        let service_id = svc.id;

        let (notification_svc, dispatcher, broadcaster, mut dispatcher_rx) = build_ctx_parts();
        let mut rx = broadcaster.subscribe(tenant_id).await;
        let ctx = MutationContext {
            notification_service: &notification_svc,
            notification_dispatcher: &dispatcher,
            event_broadcaster: &broadcaster,
        };
        let tenant_db = TenantDb::new_for_test(db, tenant_id);

        approve(&tenant_db, &ctx, service_id)
            .await
            .expect("approve");

        // Dispatcher event fired.
        let event = dispatcher_rx
            .try_recv()
            .expect("dispatcher should have received one event");
        assert_eq!(event.tenant_id, tenant_id);
        assert!(event.host_id.is_none());
        assert!(event.host_name.is_none());
        assert!(event.software_item_id.is_none());
        assert!(event.software_item_name.is_none());
        assert!(event.plugin_type.is_none());
        assert!(
            matches!(
                event.details,
                NotificationEventDetails::NewServiceEnrolled {
                    service_id: sid,
                    ..
                } if sid == service_id
            ),
            "expected NewServiceEnrolled with correct service_id"
        );

        // Broadcaster event fired.
        let broadcast = rx.try_recv().expect("broadcaster should have one event");
        assert!(
            matches!(
                broadcast,
                AdminEvent::ServiceStatusChanged { id, ref status } if id == service_id && status == "approved"
            ),
            "unexpected broadcast: {broadcast:?}"
        );
    }

    // ── reject ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn reject_broadcasts_no_dispatcher_event() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let svc = insert_service(&db, tenant_id, ServiceStatus::Pending).await;
        let service_id = svc.id;

        let (notification_svc, dispatcher, broadcaster, mut dispatcher_rx) = build_ctx_parts();
        let mut rx = broadcaster.subscribe(tenant_id).await;
        let ctx = MutationContext {
            notification_service: &notification_svc,
            notification_dispatcher: &dispatcher,
            event_broadcaster: &broadcaster,
        };
        let connections = ServiceConnectionRegistry::default();
        let tenant_db = TenantDb::new_for_test(db, tenant_id);

        reject(&tenant_db, &ctx, service_id, &connections)
            .await
            .expect("reject");

        // No dispatcher event for reject (preserved asymmetry).
        assert!(
            dispatcher_rx.try_recv().is_err(),
            "reject must NOT dispatch a NotificationEvent"
        );

        // Broadcaster event fired with "rejected" status.
        let broadcast = rx.try_recv().expect("broadcaster should have one event");
        assert!(
            matches!(
                broadcast,
                AdminEvent::ServiceStatusChanged { id, ref status } if id == service_id && status == "rejected"
            ),
            "unexpected broadcast: {broadcast:?}"
        );
    }

    // ── deactivate not found ─────────────────────────────────────────────

    #[tokio::test]
    async fn deactivate_not_found_has_zero_side_effects() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let nonexistent_id = Uuid::now_v7();

        let (notification_svc, dispatcher, broadcaster, mut dispatcher_rx) = build_ctx_parts();
        let mut rx = broadcaster.subscribe(tenant_id).await;
        let ctx = MutationContext {
            notification_service: &notification_svc,
            notification_dispatcher: &dispatcher,
            event_broadcaster: &broadcaster,
        };
        let cert = make_cert_state();
        let connections = ServiceConnectionRegistry::default();
        let tenant_db = TenantDb::new_for_test(db, tenant_id);

        let found = deactivate(
            &tenant_db,
            &ctx,
            nonexistent_id,
            Uuid::nil(),
            &cert,
            &connections,
        )
        .await
        .expect("deactivate");

        assert!(!found, "should return Ok(false) for unknown service");
        assert!(
            dispatcher_rx.try_recv().is_err(),
            "no dispatcher event expected"
        );
        assert_eq!(
            rx.try_recv().unwrap_err(),
            TryRecvError::Empty,
            "no broadcast event expected"
        );
    }

    // ── batch_approve ────────────────────────────────────────────────────

    #[tokio::test]
    async fn batch_approve_no_dispatcher_dispatch() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let svc = insert_service(&db, tenant_id, ServiceStatus::Pending).await;

        let (notification_svc, dispatcher, broadcaster, mut dispatcher_rx) = build_ctx_parts();
        let mut rx = broadcaster.subscribe(tenant_id).await;
        let ctx = MutationContext {
            notification_service: &notification_svc,
            notification_dispatcher: &dispatcher,
            event_broadcaster: &broadcaster,
        };
        let tenant_db = TenantDb::new_for_test(db, tenant_id);

        let (succeeded, _failed) = batch_approve(&tenant_db, &ctx, &[svc.id])
            .await
            .expect("batch_approve");

        assert_eq!(succeeded, vec![svc.id]);

        // Preserved asymmetry: batch approve must NOT dispatch NotificationEvent.
        assert!(
            dispatcher_rx.try_recv().is_err(),
            "batch_approve must NOT dispatch a NotificationEvent"
        );

        // Broadcaster event fired with "approved" status.
        let broadcast = rx.try_recv().expect("broadcaster should have one event");
        assert!(
            matches!(
                broadcast,
                AdminEvent::ServiceStatusChanged { id, ref status } if id == svc.id && status == "approved"
            ),
            "unexpected broadcast: {broadcast:?}"
        );
    }

    // ── merge (no broadcast) ─────────────────────────────────────────────

    /// Insert a service with the `SoftwareDiscovery` capability, which is
    /// required by `merge_service` on both target and source.
    async fn insert_mergeable_service(
        db: &sea_orm::DatabaseConnection,
        tenant_id: uuid::Uuid,
        status: ServiceStatus,
    ) -> uptrakit_shared_db::entity::service::Model {
        use sea_orm::{ActiveModelTrait, Set};
        use uptrakit_shared_db::entity::service;

        let id = uuid::Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        service::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant_id),
            capabilities: Set(r#"["software_discovery"]"#.to_string()),
            hostname: Set(format!("host-{}", &id.to_string()[..8])),
            friendly_name: Set(format!("Service {}", &id.to_string()[..8])),
            ip_address: Set(Some("10.0.0.1".to_string())),
            status: Set(status),
            enrollment_secret_hash: Set(format!("secret-{id}")),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            enrollment_token_id: Set(None),
            cert_lifetime_hours: Set(None),
            service_app_name: Set(None),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        }
        .insert(db)
        .await
        .expect("insert mergeable service")
    }

    #[tokio::test]
    async fn merge_no_broadcast() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        // Target must be Approved with SoftwareDiscovery; source must be Pending with same.
        let target = insert_mergeable_service(&db, tenant_id, ServiceStatus::Approved).await;
        let source = insert_mergeable_service(&db, tenant_id, ServiceStatus::Pending).await;

        let (notification_svc, dispatcher, broadcaster, mut dispatcher_rx) = build_ctx_parts();
        let mut rx = broadcaster.subscribe(tenant_id).await;
        let ctx = MutationContext {
            notification_service: &notification_svc,
            notification_dispatcher: &dispatcher,
            event_broadcaster: &broadcaster,
        };
        let cert = make_cert_state();
        let connections = ServiceConnectionRegistry::default();
        let tenant_db = TenantDb::new_for_test(db, tenant_id);

        merge(
            &tenant_db,
            &ctx,
            MergeParams {
                target_id: target.id,
                source_id: source.id,
                target_connected: false,
                default_tenant_id: Uuid::nil(),
            },
            &cert,
            &connections,
        )
        .await
        .expect("merge");

        // Preserved asymmetry: merge must NOT dispatch NotificationEvent.
        assert!(
            dispatcher_rx.try_recv().is_err(),
            "merge must NOT dispatch a NotificationEvent"
        );

        // Preserved asymmetry: merge must NOT broadcast ServiceStatusChanged.
        assert_eq!(
            rx.try_recv().unwrap_err(),
            TryRecvError::Empty,
            "merge must NOT broadcast any event"
        );
    }
}
