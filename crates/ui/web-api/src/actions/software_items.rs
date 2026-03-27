use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::actions::MutationContext;
use crate::queries::software_items::{self as item_queries, SoftwareItemQueryError};
use crate::queries::update_dispatch::TriggerUpdateError;
use crate::queries::update_triggers::{self, TriggerUpdateParams, TriggerUpdateResult};
use crate::tenant_db::TenantDb;
use uptrakit_shared_db::entity::software_item;
use uptrakit_web_api_types::events::AdminEvent;
use uptrakit_web_api_types::software_items::{SoftwareItemResponse, UpdateSoftwareItemRequest};

pub(crate) async fn update(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    item_id: Uuid,
    req: UpdateSoftwareItemRequest,
) -> Result<SoftwareItemResponse, rootcause::Report<SoftwareItemQueryError>> {
    let resp = item_queries::update_software_item(tenant_db, item_id, req).await?;

    ctx.event_broadcaster
        .send(
            tenant_db.tenant_id,
            AdminEvent::SoftwareItemUpdated { id: item_id },
        )
        .await;

    Ok(resp)
}

/// Trigger an update for a single host/item pair.
///
/// NOTE: `trigger_update_for_host()` accepts `&dyn ServiceNotifier` and performs agent
/// dispatch internally as transactional query-layer logic. This ServiceNotifier call is
/// intentionally outside the action boundary rule.
pub(crate) async fn trigger_update(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    params: TriggerUpdateParams<'_>,
) -> Result<TriggerUpdateResult, rootcause::Report<TriggerUpdateError>> {
    // Copy fields needed after params is consumed by value.
    let tenant_id = params.tenant_id;
    let host_id = params.host_id;
    let item_id = params.item_id;

    let result =
        update_triggers::trigger_update_for_host(tenant_db.db(), ctx.notification_service, params)
            .await?;

    ctx.notification_service
        .push_software_states_for_tenant(tenant_db.db(), tenant_id)
        .await;

    ctx.event_broadcaster
        .send(
            tenant_id,
            AdminEvent::UpdateTriggered {
                update_history_id: result.update_history_id,
                host_id,
                software_item_id: item_id,
            },
        )
        .await;

    Ok(result)
}

/// Batch "feature" (approve): sets `featured = true` for matching items.
/// Already-featured items are treated as idempotent success.
pub(crate) async fn batch_feature(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    ids: &[Uuid],
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>), sea_orm::DbErr> {
    let now = OffsetDateTime::now_utc();
    let mut succeeded = Vec::new();
    let mut failures: Vec<(Uuid, String)> = Vec::new();

    for &id in ids {
        match software_item::Entity::find_by_id(id)
            .filter(software_item::Column::TenantId.eq(tenant_db.tenant_id))
            .filter(software_item::Column::DeactivatedAt.is_null())
            .one(tenant_db.db())
            .await
        {
            Ok(Some(item)) => {
                if item.featured {
                    // Already featured — still counts as success (idempotent).
                    succeeded.push(id);
                    continue;
                }
                let mut active: software_item::ActiveModel = item.into();
                active.featured = Set(true);
                active.updated_at = Set(now);
                match active.update(tenant_db.db()).await {
                    Ok(_) => succeeded.push(id),
                    Err(e) => failures.push((id, e.to_string())),
                }
            }
            Ok(None) => failures.push((id, "not found".to_string())),
            Err(e) => failures.push((id, e.to_string())),
        }
    }

    for id in &succeeded {
        ctx.event_broadcaster
            .send(
                tenant_db.tenant_id,
                AdminEvent::SoftwareItemUpdated { id: *id },
            )
            .await;
    }

    Ok((succeeded, failures))
}

/// Batch soft-delete software items.
///
/// NOTE: Preserved asymmetry — broadcasts `SoftwareItemUpdated` (not `SoftwareItemDeleted`)
/// per item, matching the existing handler behavior.
pub(crate) async fn batch_delete(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    ids: &[Uuid],
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>), rootcause::Report<SoftwareItemQueryError>> {
    let (succeeded_ids, failed) = item_queries::batch_delete_software_items(tenant_db, ids).await?;

    for id in &succeeded_ids {
        ctx.event_broadcaster
            .send(
                tenant_db.tenant_id,
                AdminEvent::SoftwareItemUpdated { id: *id },
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

    // ── batch_feature empty ──────────────────────────────────────────────

    #[tokio::test]
    async fn batch_feature_empty_succeeds() {
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

        let (succeeded, failed) = batch_feature(&tenant_db, &ctx, &[])
            .await
            .expect("batch_feature");

        assert!(succeeded.is_empty(), "no items featured");
        assert!(failed.is_empty(), "no failures");
        assert_eq!(
            rx.try_recv().unwrap_err(),
            TryRecvError::Empty,
            "no broadcast event expected for empty input"
        );
    }

    // ── batch_delete empty ───────────────────────────────────────────────

    #[tokio::test]
    async fn batch_delete_empty_no_broadcast() {
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

        assert!(succeeded.is_empty(), "no items deleted");
        assert!(failed.is_empty(), "no failures");
        assert_eq!(
            rx.try_recv().unwrap_err(),
            TryRecvError::Empty,
            "no broadcast event expected for empty input"
        );
    }
}
