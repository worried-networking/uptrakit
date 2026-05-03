use uuid::Uuid;

use crate::actions::MutationContext;
use crate::queries::hosts as host_queries;
use crate::tenant_db::TenantDb;
use uptrakit_web_api_types::events::AdminEvent;
use uptrakit_web_api_types::hosts::{HostResponse, UpdateHostRequest};

/// Returns `Ok(Some(resp))` when found and updated; `Ok(None)` when not found.
/// No broadcast is sent when `Ok(None)`.
pub(crate) async fn update(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    host_id: Uuid,
    body: &UpdateHostRequest,
) -> Result<Option<HostResponse>, sea_orm::DbErr> {
    let resp = host_queries::update_host(tenant_db, host_id, body).await?;
    if resp.is_some() {
        ctx.event_broadcaster
            .send(
                tenant_db.tenant_id(),
                AdminEvent::HostUpdated { id: host_id },
            )
            .await;
    }
    Ok(resp)
}

/// Returns `Ok(true)` when found and deactivated; `Ok(false)` when not found.
/// No broadcast is sent when `Ok(false)`.
pub(crate) async fn deactivate(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    host_id: Uuid,
) -> Result<bool, sea_orm::DbErr> {
    let found = host_queries::deactivate_host(tenant_db, host_id).await?;
    if found {
        ctx.event_broadcaster
            .send(
                tenant_db.tenant_id(),
                AdminEvent::HostDeleted { id: host_id },
            )
            .await;
    }
    Ok(found)
}

pub(crate) async fn batch_deactivate(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    ids: &[Uuid],
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>), sea_orm::DbErr> {
    let (succeeded_ids, failed) = host_queries::batch_deactivate_hosts(tenant_db, ids).await?;
    for id in &succeeded_ids {
        ctx.event_broadcaster
            .send(tenant_db.tenant_id(), AdminEvent::HostDeleted { id: *id })
            .await;
    }
    Ok((succeeded_ids, failed))
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use tokio::sync::broadcast::error::TryRecvError;
    use uptrakit_web_api_types::hosts::UpdateHostRequest;
    use uuid::Uuid;

    use super::*;
    use crate::actions::MutationContext;
    use crate::event_broadcaster::EventBroadcaster;
    use crate::notification_service::NotificationService;
    use crate::notifications::dispatcher::NotificationDispatcher;
    use crate::service_connections::ServiceConnectionRegistry;
    use crate::tenant_db::TenantDb;
    use crate::test_harness::{insert_default_tenant, setup_migrated_db};

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

    // ── update not found ─────────────────────────────────────────────────

    #[tokio::test]
    async fn update_not_found_no_broadcast() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let nonexistent_id = Uuid::now_v7();

        let (notification_svc, dispatcher, broadcaster, _dispatcher_rx) = build_ctx_parts();
        let mut rx = broadcaster.subscribe(tenant_id).await;
        let ctx = MutationContext {
            notification_service: &notification_svc,
            notification_dispatcher: &dispatcher,
            event_broadcaster: &broadcaster,
        };
        let tenant_db = TenantDb::new_for_test(db, tenant_id);
        let body = UpdateHostRequest {
            friendly_name: None,
        };

        let result = update(&tenant_db, &ctx, nonexistent_id, &body)
            .await
            .expect("update");

        assert!(result.is_none(), "should return Ok(None) for unknown host");
        assert_eq!(
            rx.try_recv().unwrap_err(),
            TryRecvError::Empty,
            "no broadcast event expected"
        );
    }

    // ── deactivate not found ─────────────────────────────────────────────

    #[tokio::test]
    async fn deactivate_not_found_returns_false_no_broadcast() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let nonexistent_id = Uuid::now_v7();

        let (notification_svc, dispatcher, broadcaster, _dispatcher_rx) = build_ctx_parts();
        let mut rx = broadcaster.subscribe(tenant_id).await;
        let ctx = MutationContext {
            notification_service: &notification_svc,
            notification_dispatcher: &dispatcher,
            event_broadcaster: &broadcaster,
        };
        let tenant_db = TenantDb::new_for_test(db, tenant_id);

        let found = deactivate(&tenant_db, &ctx, nonexistent_id)
            .await
            .expect("deactivate");

        assert!(!found, "should return Ok(false) for unknown host");
        assert_eq!(
            rx.try_recv().unwrap_err(),
            TryRecvError::Empty,
            "no broadcast event expected"
        );
    }

    // ── batch_deactivate empty ids ───────────────────────────────────────

    #[tokio::test]
    async fn batch_deactivate_empty_ids_no_broadcast() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;

        let (notification_svc, dispatcher, broadcaster, _dispatcher_rx) = build_ctx_parts();
        let mut rx = broadcaster.subscribe(tenant_id).await;
        let ctx = MutationContext {
            notification_service: &notification_svc,
            notification_dispatcher: &dispatcher,
            event_broadcaster: &broadcaster,
        };
        let tenant_db = TenantDb::new_for_test(db, tenant_id);

        let (succeeded, failed) = batch_deactivate(&tenant_db, &ctx, &[])
            .await
            .expect("batch_deactivate");

        assert!(succeeded.is_empty(), "no hosts deactivated");
        assert!(failed.is_empty(), "no failures");
        assert_eq!(
            rx.try_recv().unwrap_err(),
            TryRecvError::Empty,
            "no broadcast event expected"
        );
    }
}
