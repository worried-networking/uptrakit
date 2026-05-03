use uuid::Uuid;

use crate::actions::MutationContext;
use crate::queries::host_tags as tag_queries;
use crate::tenant_db::TenantDb;
use uptrakit_web_api_types::events::AdminEvent;
use uptrakit_web_api_types::host_tags::{
    CreateHostTagRequest, HostTagResponse, HostTagSummary, UpdateHostTagRequest,
};

pub(crate) async fn create(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    body: &CreateHostTagRequest,
) -> Result<HostTagResponse, sea_orm::DbErr> {
    let resp = tag_queries::create_host_tag(tenant_db, body).await?;
    ctx.event_broadcaster
        .send(
            tenant_db.tenant_id(),
            AdminEvent::HostTagCreated { id: resp.id },
        )
        .await;
    Ok(resp)
}

/// Returns `Ok(Some(resp))` when found and updated; `Ok(None)` when not found.
/// No broadcast is sent when `Ok(None)`.
pub(crate) async fn update(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    tag_id: Uuid,
    body: &UpdateHostTagRequest,
) -> Result<Option<HostTagResponse>, sea_orm::DbErr> {
    let resp = tag_queries::update_host_tag(tenant_db, tag_id, body).await?;
    if resp.is_some() {
        ctx.event_broadcaster
            .send(
                tenant_db.tenant_id(),
                AdminEvent::HostTagUpdated { id: tag_id },
            )
            .await;
    }
    Ok(resp)
}

/// Returns `Ok(true)` when found and deleted; `Ok(false)` when not found.
/// No broadcast is sent when `Ok(false)`.
pub(crate) async fn delete(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    tag_id: Uuid,
) -> Result<bool, sea_orm::DbErr> {
    let found = tag_queries::delete_host_tag(tenant_db, tag_id).await?;
    if found {
        ctx.event_broadcaster
            .send(
                tenant_db.tenant_id(),
                AdminEvent::HostTagDeleted { id: tag_id },
            )
            .await;
    }
    Ok(found)
}

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
    use uptrakit_web_api_types::host_tags::UpdateHostTagRequest;
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
        let body = UpdateHostTagRequest {
            name: None,
            color: None,
            description: None,
        };

        let result = update(&tenant_db, &ctx, nonexistent_id, &body)
            .await
            .expect("update");

        assert!(result.is_none(), "should return Ok(None) for unknown tag");
        assert_eq!(
            rx.try_recv().unwrap_err(),
            TryRecvError::Empty,
            "no broadcast event expected"
        );
    }

    // ── delete not found ─────────────────────────────────────────────────

    #[tokio::test]
    async fn delete_not_found_returns_false_no_broadcast() {
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

        let found = delete(&tenant_db, &ctx, nonexistent_id)
            .await
            .expect("delete");

        assert!(!found, "should return Ok(false) for unknown tag");
        assert_eq!(
            rx.try_recv().unwrap_err(),
            TryRecvError::Empty,
            "no broadcast event expected"
        );
    }

    // ── batch_delete empty ids ───────────────────────────────────────────

    #[tokio::test]
    async fn batch_delete_empty_ids_succeeds() {
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
