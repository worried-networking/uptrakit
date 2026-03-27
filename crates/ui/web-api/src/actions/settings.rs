use crate::actions::MutationContext;
use crate::service_connections::ServiceConnectionRegistry;
use crate::tenant_db::TenantDb;
use uptrakit_internal_wire::{Capability, ControllerMessage};
use uptrakit_web_api_queries::queries::reset_data::ResetDataQueryError;
use uptrakit_web_api_types::events::AdminEvent;
use uptrakit_web_api_types::settings_reset::ResetDeletedCounts;

pub(crate) async fn reset_data(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    service_connections: &ServiceConnectionRegistry,
) -> Result<ResetDeletedCounts, rootcause::Report<ResetDataQueryError>> {
    let counts =
        uptrakit_web_api_queries::queries::reset_data::reset_tenant_data(tenant_db).await?;

    service_connections
        .broadcast_by_capability(&Capability::ResetData, ControllerMessage::ResetData)
        .await;

    ctx.event_broadcaster
        .send(tenant_db.tenant_id, AdminEvent::DataReset)
        .await;

    tracing::info!(
        hosts = counts.hosts,
        software_items = counts.software_items,
        plugin_configs = counts.plugin_configs,
        host_tags = counts.host_tags,
        update_history = counts.update_history,
        update_batches = counts.update_batches,
        "tenant data reset completed"
    );

    Ok(counts)
}
