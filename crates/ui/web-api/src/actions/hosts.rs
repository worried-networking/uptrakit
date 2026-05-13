use uuid::Uuid;

use crate::actions::MutationContext;
use crate::queries::hosts as host_queries;
use crate::tenant_db::TenantDb;
use uptrakit_web_api_types::events::AdminEvent;

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
    use uuid::Uuid;

    use super::*;
    use crate::actions::MutationContext;
    use crate::event_broadcaster::EventBroadcaster;
    use crate::notification_service::NotificationService;
    use crate::service_connections::ServiceConnectionRegistry;
    use crate::tenant_db::TenantDb;
    use crate::test_harness::{insert_default_tenant, setup_migrated_db};

    fn build_ctx_parts() -> (NotificationService, EventBroadcaster) {
        let broadcaster = EventBroadcaster::new();
        let notification_svc =
            NotificationService::new(ServiceConnectionRegistry::default(), Uuid::nil());
        (notification_svc, broadcaster)
    }

    // ── batch_deactivate empty ids ───────────────────────────────────────

    #[tokio::test]
    async fn batch_deactivate_empty_ids_no_broadcast() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;

        let (notification_svc, broadcaster) = build_ctx_parts();
        let mut rx = broadcaster.subscribe(tenant_id).await;
        let ctx = MutationContext {
            notification_service: &notification_svc,
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
