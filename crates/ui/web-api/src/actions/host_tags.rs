use uuid::Uuid;

use crate::actions::MutationContext;
use crate::queries::host_tags as tag_queries;
use crate::tenant_db::TenantDb;
use uptrakit_web_api_types::events::AdminEvent;
use uptrakit_web_api_types::host_tags::HostTagSummary;

pub(crate) async fn batch_delete(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    ids: &[Uuid],
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>), sea_orm::DbErr> {
    let (succeeded_ids, failed) = tag_queries::batch_delete_host_tags(tenant_db, ids).await?;
    for id in &succeeded_ids {
        ctx.event_broadcaster
            .send(
                tenant_db.tenant_id(),
                AdminEvent::HostTagDeleted { id: *id },
            )
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

    // ── batch_delete empty ids ───────────────────────────────────────────

    #[tokio::test]
    async fn batch_delete_empty_ids_succeeds() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;

        let (notification_svc, broadcaster) = build_ctx_parts();
        let mut rx = broadcaster.subscribe(tenant_id).await;
        let ctx = MutationContext {
            notification_service: &notification_svc,
            event_broadcaster: &broadcaster,
        };
        let tenant_db = TenantDb::new_for_test(db, tenant_id);

        let (succeeded, failed) = batch_delete(&tenant_db, &ctx, &[])
            .await
            .expect("batch_delete");

        assert!(succeeded.is_empty(), "no tags deleted");
        assert!(failed.is_empty(), "no failures");
        assert_eq!(
            rx.try_recv().unwrap_err(),
            TryRecvError::Empty,
            "no broadcast event expected"
        );
    }
}

/// Replaces all tags on a host. Broadcasts `HostTagsChanged` and pushes MQTT
/// software states so connected services refresh their tag-aware state.
pub(crate) async fn set(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    host_id: Uuid,
    tag_ids: &[Uuid],
) -> Result<Vec<HostTagSummary>, sea_orm::DbErr> {
    let resp = tag_queries::set_host_tags(tenant_db, host_id, tag_ids).await?;

    ctx.event_broadcaster
        .send(
            tenant_db.tenant_id(),
            AdminEvent::HostTagsChanged { host_id },
        )
        .await;

    ctx.notification_service
        .push_software_states_for_tenant(tenant_db.db(), tenant_db.tenant_id())
        .await;

    Ok(resp)
}
