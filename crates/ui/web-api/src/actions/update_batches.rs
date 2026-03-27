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
