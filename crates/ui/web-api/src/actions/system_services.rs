use sea_orm::DatabaseConnection;
use uuid::Uuid;

use crate::actions::MutationContext;
use crate::app_state::CertState;
use crate::queries::system_services::{self as ss_queries, SystemServiceQueryError};
use crate::service_connections::ServiceConnectionRegistry;
use uptrakit_internal_wire::{
    ApprovedPayload, ControllerMessage, RejectedPayload, RequestCrlRenewalPayload,
};
use uptrakit_web_api_types::events::AdminEvent;
use uptrakit_web_api_types::system_services::SystemServiceResponse;

pub(crate) async fn approve(
    db: &DatabaseConnection,
    ctx: &MutationContext<'_>,
    service_id: Uuid,
) -> Result<SystemServiceResponse, rootcause::Report<SystemServiceQueryError>> {
    let resp = ss_queries::approve_system_service(db, service_id).await?;

    let _ = ctx
        .notification_service
        .send(
            &service_id,
            ControllerMessage::Approved(ApprovedPayload { service_id }),
        )
        .await;

    ctx.event_broadcaster
        .send_global(AdminEvent::SystemServiceStatusChanged {
            id: service_id,
            status: "approved".to_string(),
        })
        .await;

    Ok(resp)
}

pub(crate) async fn reject(
    db: &DatabaseConnection,
    ctx: &MutationContext<'_>,
    service_id: Uuid,
    service_connections: &ServiceConnectionRegistry,
) -> Result<SystemServiceResponse, rootcause::Report<SystemServiceQueryError>> {
    let resp = ss_queries::reject_system_service(db, service_id).await?;

    let _ = ctx
        .notification_service
        .send(
            &service_id,
            ControllerMessage::Rejected(RejectedPayload { service_id }),
        )
        .await;

    service_connections.force_disconnect(&service_id).await;

    ctx.event_broadcaster
        .send_global(AdminEvent::SystemServiceStatusChanged {
            id: service_id,
            status: "rejected".to_string(),
        })
        .await;

    Ok(resp)
}

/// Returns `Ok(true)` when the service was found and deactivated; `Ok(false)` when not found.
/// No side effects are run when `Ok(false)`.
pub(crate) async fn deactivate(
    db: &DatabaseConnection,
    ctx: &MutationContext<'_>,
    service_id: Uuid,
    cert: &CertState,
    service_connections: &ServiceConnectionRegistry,
) -> Result<bool, rootcause::Report<SystemServiceQueryError>> {
    let found = ss_queries::deactivate_system_service(db, service_id).await?;
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
        .send_global(AdminEvent::SystemServiceStatusChanged {
            id: service_id,
            status: "deactivated".to_string(),
        })
        .await;

    Ok(true)
}

pub(crate) async fn batch_approve(
    db: &DatabaseConnection,
    ctx: &MutationContext<'_>,
    ids: &[Uuid],
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>), rootcause::Report<SystemServiceQueryError>> {
    let (succeeded_ids, failed) = ss_queries::batch_approve_system_services(db, ids).await?;

    for id in &succeeded_ids {
        let _ = ctx
            .notification_service
            .send(
                id,
                ControllerMessage::Approved(ApprovedPayload { service_id: *id }),
            )
            .await;
        ctx.event_broadcaster
            .send_global(AdminEvent::SystemServiceStatusChanged {
                id: *id,
                status: "approved".to_string(),
            })
            .await;
    }

    Ok((succeeded_ids, failed))
}

pub(crate) async fn batch_reject(
    db: &DatabaseConnection,
    ctx: &MutationContext<'_>,
    ids: &[Uuid],
    service_connections: &ServiceConnectionRegistry,
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>), rootcause::Report<SystemServiceQueryError>> {
    let (succeeded_ids, failed) = ss_queries::batch_reject_system_services(db, ids).await?;

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
            .send_global(AdminEvent::SystemServiceStatusChanged {
                id: *id,
                status: "rejected".to_string(),
            })
            .await;
    }

    Ok((succeeded_ids, failed))
}

pub(crate) async fn batch_deactivate(
    db: &DatabaseConnection,
    ctx: &MutationContext<'_>,
    ids: &[Uuid],
    cert: &CertState,
    service_connections: &ServiceConnectionRegistry,
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>), rootcause::Report<SystemServiceQueryError>> {
    let (succeeded_ids, failed) = ss_queries::batch_deactivate_system_services(db, ids).await?;

    for id in &succeeded_ids {
        cert.revocation_notify.notify_one();
        ctx.notification_service
            .publish_controller_event(ControllerMessage::RequestCrlRenewal(
                RequestCrlRenewalPayload::default(),
            ))
            .await;
        service_connections.force_disconnect(id).await;
        ctx.event_broadcaster
            .send_global(AdminEvent::SystemServiceStatusChanged {
                id: *id,
                status: "deactivated".to_string(),
            })
            .await;
    }

    Ok((succeeded_ids, failed))
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use std::sync::Arc;

    use sea_orm::{ActiveModelTrait, Set};
    use tokio::sync::broadcast::error::TryRecvError;
    use uptrakit_shared_db::entity::system_service;
    use uptrakit_web_api_types::events::AdminEvent;
    use uuid::Uuid;

    use super::*;
    use crate::actions::MutationContext;
    use crate::app_state::CertState;
    use crate::event_broadcaster::EventBroadcaster;
    use crate::notification_service::NotificationService;
    use crate::notifications::dispatcher::NotificationDispatcher;
    use crate::service_connections::ServiceConnectionRegistry;
    use crate::test_harness::setup_migrated_db;

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

    /// Insert a minimal system service row with `Pending` status and return its id.
    async fn insert_system_service(db: &sea_orm::DatabaseConnection) -> system_service::Model {
        let id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        system_service::ActiveModel {
            id: Set(id),
            capabilities: Set("[]".to_string()),
            hostname: Set(format!("sys-host-{}", &id.to_string()[..8])),
            friendly_name: Set(format!("SysSvc {}", &id.to_string()[..8])),
            ip_address: Set(None),
            status: Set(system_service::SystemServiceStatus::Pending),
            enrollment_secret_hash: Set(format!("sys-secret-{id}")),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            cert_lifetime_hours: Set(None),
            system_enrollment_token_id: Set(None),
            service_app_name: Set(None),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        }
        .insert(db)
        .await
        .expect("insert system_service")
    }

    // ── approve ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn approve_sends_notification_and_broadcasts() {
        let db = setup_migrated_db().await;
        let svc = insert_system_service(&db).await;
        let service_id = svc.id;

        let (notification_svc, dispatcher, broadcaster, _dispatcher_rx) = build_ctx_parts();
        // send_global iterates all registered channels — subscribe with any tenant uuid.
        let any_tenant = Uuid::now_v7();
        let mut rx = broadcaster.subscribe(any_tenant).await;
        let ctx = MutationContext {
            notification_service: &notification_svc,
            notification_dispatcher: &dispatcher,
            event_broadcaster: &broadcaster,
        };

        approve(&db, &ctx, service_id).await.expect("approve");

        let broadcast = rx.try_recv().expect("broadcaster should have one event");
        assert!(
            matches!(
                broadcast,
                AdminEvent::SystemServiceStatusChanged { id, ref status }
                    if id == service_id && status == "approved"
            ),
            "unexpected broadcast: {broadcast:?}"
        );
    }

    // ── reject ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn reject_broadcasts_status_changed() {
        let db = setup_migrated_db().await;
        let svc = insert_system_service(&db).await;
        let service_id = svc.id;

        let (notification_svc, dispatcher, broadcaster, _dispatcher_rx) = build_ctx_parts();
        let any_tenant = Uuid::now_v7();
        let mut rx = broadcaster.subscribe(any_tenant).await;
        let ctx = MutationContext {
            notification_service: &notification_svc,
            notification_dispatcher: &dispatcher,
            event_broadcaster: &broadcaster,
        };
        let connections = ServiceConnectionRegistry::default();

        reject(&db, &ctx, service_id, &connections)
            .await
            .expect("reject");

        let broadcast = rx.try_recv().expect("broadcaster should have one event");
        assert!(
            matches!(
                broadcast,
                AdminEvent::SystemServiceStatusChanged { id, ref status }
                    if id == service_id && status == "rejected"
            ),
            "unexpected broadcast: {broadcast:?}"
        );
    }

    // ── deactivate not found ─────────────────────────────────────────────

    #[tokio::test]
    async fn deactivate_not_found_returns_false_no_side_effects() {
        let db = setup_migrated_db().await;
        let nonexistent_id = Uuid::now_v7();

        let (notification_svc, dispatcher, broadcaster, mut dispatcher_rx) = build_ctx_parts();
        let any_tenant = Uuid::now_v7();
        let mut rx = broadcaster.subscribe(any_tenant).await;
        let ctx = MutationContext {
            notification_service: &notification_svc,
            notification_dispatcher: &dispatcher,
            event_broadcaster: &broadcaster,
        };
        let cert = make_cert_state();
        let connections = ServiceConnectionRegistry::default();

        let found = deactivate(&db, &ctx, nonexistent_id, &cert, &connections)
            .await
            .expect("deactivate");

        assert!(!found, "should return Ok(false) for unknown system service");
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
}
