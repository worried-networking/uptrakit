use uuid::Uuid;

use crate::actions::MutationContext;
use crate::batch_progress_broadcaster::BatchProgressBroadcaster;
use crate::queries::{
    update_batches as batch_queries,
    update_dispatch::TriggerUpdateError,
    update_types::{ActorType, BatchType},
};
use crate::tenant_db::TenantDb;
use uptrakit_web_api_types::events::AdminEvent;
use uptrakit_web_api_types::update_batches::BatchUpdateResponse;

/// Trigger a host-wide batch update for all outdated software items on one host.
///
/// NOTE: `create_batch()` accepts `&dyn ServiceNotifier` and performs agent dispatch
/// internally as transactional query-layer logic. This ServiceNotifier call is
/// intentionally outside the action boundary rule.
pub(crate) async fn trigger_host_batch(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    batch_progress: &BatchProgressBroadcaster,
    host_id: Uuid,
    user_id: Uuid,
    category_filter: Option<&str>,
    exclude_item_ids: Option<&[Uuid]>,
) -> Result<BatchUpdateResponse, rootcause::Report<TriggerUpdateError>> {
    let candidates = batch_queries::find_outdated_items_for_host(
        tenant_db.db(),
        tenant_db.tenant_id,
        host_id,
        category_filter,
        exclude_item_ids,
    )
    .await?;

    let resp = batch_queries::create_batch(
        tenant_db.db(),
        ctx.notification_service,
        &batch_queries::CreateBatchParams {
            tenant_id: tenant_db.tenant_id,
            batch_type: BatchType::HostUpdate,
            actor_type: ActorType::User.as_str(),
            actor_id: &user_id.to_string(),
        },
        candidates,
    )
    .await?;

    if let Some(batch_id) = resp.batch_id {
        batch_progress.create_channel(batch_id).await;
    }

    ctx.notification_service
        .push_software_states_for_tenant(tenant_db.db(), tenant_db.tenant_id)
        .await;

    for item in &resp.updates {
        ctx.event_broadcaster
            .send(
                tenant_db.tenant_id,
                AdminEvent::UpdateTriggered {
                    update_history_id: item.update_history_id,
                    host_id: item.host_id,
                    software_item_id: item.software_item_id,
                },
            )
            .await;
    }

    Ok(resp)
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use sea_orm::{ActiveModelTrait, Set};
    use tokio::sync::broadcast::error::TryRecvError;
    use uptrakit_shared_db::entity::{host, software_item};
    use uuid::Uuid;

    use super::*;
    use crate::actions::MutationContext;
    use crate::batch_progress_broadcaster::BatchProgressBroadcaster;
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

    /// Insert a bare host row for testing (no software items installed → zero candidates).
    async fn insert_bare_host(db: &sea_orm::DatabaseConnection, tenant_id: Uuid) -> Uuid {
        let id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        host::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant_id),
            machine_id: Set(format!("machine-{}", &id.to_string()[..8])),
            hostname: Set(format!("host-{}", &id.to_string()[..8])),
            friendly_name: Set(format!("Host {}", &id.to_string()[..8])),
            os_type: Set(None),
            os_version: Set(None),
            architecture: Set(None),
            ip_address: Set(None),
            host_features: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert host")
        .id
    }

    /// Insert a bare software_item row (no host assignments → zero candidates).
    async fn insert_bare_software_item(db: &sea_orm::DatabaseConnection, tenant_id: Uuid) -> Uuid {
        let id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        software_item::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant_id),
            name: Set(format!("item-{}", &id.to_string()[..8])),
            featured: Set(false),
            icon_url: Set(None),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert software_item")
        .id
    }

    // ── trigger_host_batch with known host but no outdated items ─────────

    #[tokio::test]
    async fn trigger_host_batch_no_outdated_items_returns_empty() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let host_id = insert_bare_host(&db, tenant_id).await;

        let (notification_svc, dispatcher, broadcaster, _dispatcher_rx) = build_ctx_parts();
        let mut rx = broadcaster.subscribe(tenant_id).await;
        let ctx = MutationContext {
            notification_service: &notification_svc,
            notification_dispatcher: &dispatcher,
            event_broadcaster: &broadcaster,
        };
        let batch_progress = BatchProgressBroadcaster::new();
        let tenant_db = TenantDb::new_for_test(db, tenant_id);
        let user_id = Uuid::now_v7();

        let resp = trigger_host_batch(
            &tenant_db,
            &ctx,
            &batch_progress,
            host_id,
            user_id,
            None,
            None,
        )
        .await
        .expect("trigger_host_batch should not error");

        assert!(resp.updates.is_empty(), "no outdated items for bare host");
        assert_eq!(
            rx.try_recv().unwrap_err(),
            TryRecvError::Empty,
            "no broadcast event expected when updates list is empty"
        );
    }

    // ── trigger_item_batch with known item but no host assignments ───────

    #[tokio::test]
    async fn trigger_item_batch_no_host_assignments_returns_empty() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let item_id = insert_bare_software_item(&db, tenant_id).await;

        let (notification_svc, dispatcher, broadcaster, _dispatcher_rx) = build_ctx_parts();
        let mut rx = broadcaster.subscribe(tenant_id).await;
        let ctx = MutationContext {
            notification_service: &notification_svc,
            notification_dispatcher: &dispatcher,
            event_broadcaster: &broadcaster,
        };
        let batch_progress = BatchProgressBroadcaster::new();
        let tenant_db = TenantDb::new_for_test(db, tenant_id);
        let user_id = Uuid::now_v7();

        let resp = trigger_item_batch(
            &tenant_db,
            &ctx,
            &batch_progress,
            item_id,
            user_id,
            "1.2.3".to_string(),
            None,
        )
        .await
        .expect("trigger_item_batch should not error");

        assert!(resp.updates.is_empty(), "no outdated hosts for bare item");
        assert_eq!(
            rx.try_recv().unwrap_err(),
            TryRecvError::Empty,
            "no broadcast event expected when updates list is empty"
        );
    }
}

/// Trigger an item-wide rollout, updating one software item across multiple hosts.
///
/// NOTE: `create_batch()` accepts `&dyn ServiceNotifier` and performs agent dispatch
/// internally as transactional query-layer logic. This ServiceNotifier call is
/// intentionally outside the action boundary rule.
pub(crate) async fn trigger_item_batch(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    batch_progress: &BatchProgressBroadcaster,
    item_id: Uuid,
    user_id: Uuid,
    to_version: String,
    host_ids: Option<&[Uuid]>,
) -> Result<BatchUpdateResponse, rootcause::Report<TriggerUpdateError>> {
    let mut candidates = batch_queries::find_outdated_hosts_for_item(
        tenant_db.db(),
        tenant_db.tenant_id,
        item_id,
        host_ids,
    )
    .await?;

    for candidate in &mut candidates {
        candidate.latest_version = to_version.clone();
    }

    let resp = batch_queries::create_batch(
        tenant_db.db(),
        ctx.notification_service,
        &batch_queries::CreateBatchParams {
            tenant_id: tenant_db.tenant_id,
            batch_type: BatchType::ItemRollout,
            actor_type: ActorType::User.as_str(),
            actor_id: &user_id.to_string(),
        },
        candidates,
    )
    .await?;

    if let Some(batch_id) = resp.batch_id {
        batch_progress.create_channel(batch_id).await;
    }

    ctx.notification_service
        .push_software_states_for_tenant(tenant_db.db(), tenant_db.tenant_id)
        .await;

    for item in &resp.updates {
        ctx.event_broadcaster
            .send(
                tenant_db.tenant_id,
                AdminEvent::UpdateTriggered {
                    update_history_id: item.update_history_id,
                    host_id: item.host_id,
                    software_item_id: item.software_item_id,
                },
            )
            .await;
    }

    Ok(resp)
}
