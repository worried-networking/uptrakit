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
            .send(tenant_db.tenant_id, AdminEvent::HostUpdated { id: host_id })
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
            .send(tenant_db.tenant_id, AdminEvent::HostDeleted { id: host_id })
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
            .send(tenant_db.tenant_id, AdminEvent::HostDeleted { id: *id })
            .await;
    }
    Ok((succeeded_ids, failed))
}
