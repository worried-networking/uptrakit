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
            tenant_db.tenant_id,
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
                tenant_db.tenant_id,
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
                tenant_db.tenant_id,
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
            .send(tenant_db.tenant_id, AdminEvent::HostTagDeleted { id: *id })
            .await;
    }
    Ok((succeeded_ids, failed))
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
        .send(tenant_db.tenant_id, AdminEvent::HostTagsChanged { host_id })
        .await;

    ctx.notification_service
        .push_software_states_for_tenant(tenant_db.db(), tenant_db.tenant_id)
        .await;

    Ok(resp)
}
