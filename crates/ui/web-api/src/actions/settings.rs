use crate::actions::MutationContext;
use crate::service_connections::ServiceConnectionRegistry;
use crate::tenant_db::TenantDb;
use uptrakit_web_api_queries::queries::reset_data::ResetDataQueryError;
use uptrakit_web_api_types::events::AdminEvent;
use uptrakit_web_api_types::settings_reset::ResetDeletedCounts;
use uptrakit_wire::{Capability, ControllerMessage};

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

#[cfg(all(test, feature = "db-sqlite", feature = "reset-data"))]
mod tests {
    use uptrakit_web_api_types::events::AdminEvent;
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

    // ── reset_data ───────────────────────────────────────────────────────

    // Ignored: reset_tenant_data deletes from proxmox_host_mappings which is
    // created by the controller-side plugin migration (not run_migrations()).
    // Requires a full controller DB setup to execute.
    #[ignore]
    #[tokio::test]
    async fn reset_data_broadcasts_data_reset() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;

        let (notification_svc, dispatcher, broadcaster, _dispatcher_rx) = build_ctx_parts();
        let mut rx = broadcaster.subscribe(tenant_id).await;
        let ctx = MutationContext {
            notification_service: &notification_svc,
            notification_dispatcher: &dispatcher,
            event_broadcaster: &broadcaster,
        };
        let connections = ServiceConnectionRegistry::default();
        let tenant_db = TenantDb::new_for_test(db, tenant_id);

        reset_data(&tenant_db, &ctx, &connections)
            .await
            .expect("reset_data");

        let broadcast = rx.try_recv().expect("broadcaster should have one event");
        assert!(
            matches!(broadcast, AdminEvent::DataReset),
            "unexpected broadcast: {broadcast:?}"
        );
    }
}
