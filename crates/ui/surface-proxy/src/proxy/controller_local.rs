#![expect(
    unreachable_pub,
    reason = "items re-exported as `pub` for downstream `local_executor.rs` wiring; the host crate gates visibility"
)]

use super::SurfaceProxyError;
use async_trait::async_trait;
use rootcause::report;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set, TransactionTrait,
};
use time::format_description::well_known::Rfc3339;
use uptrakit_plugin_infrastructure_registry::{
    DockerSurfaceStore, EmailSmtpSettingsStore, NotificationActionTokenRecord,
    NotificationChannelListItem, NotificationChannelListPage, NotificationChannelListRequest,
    NotificationChannelStore, PluginError, PluginOps, ProxmoxApproveMatchRequest,
    ProxmoxGlobalDefaultsSaveRequest, ProxmoxHostInfoRequest, ProxmoxHostMappingsRequest,
    ProxmoxItemOverridePreloadRequest, ProxmoxItemOverrideSaveRequest, ProxmoxManualMatchRequest,
    ProxmoxMappingRequest, ProxmoxPluginConfigRequest, ProxmoxScopeSelectionRequest,
    ProxmoxSurfaceStore, ProxmoxUnmatchedGuestsRequest, SurfaceActionController,
    SurfaceActionError, TelegramGlobalSettingsStore, execute_proxmox_controller_approve_match,
    execute_proxmox_controller_discover_hosts, execute_proxmox_controller_get_host_info,
    execute_proxmox_controller_list_all_unmatched, execute_proxmox_controller_list_host_mappings,
    execute_proxmox_controller_load_backup_target_options, execute_proxmox_controller_manual_match,
    execute_proxmox_controller_preload_global_defaults,
    execute_proxmox_controller_preload_item_overrides,
    execute_proxmox_controller_save_global_defaults,
    execute_proxmox_controller_save_item_overrides, execute_proxmox_controller_test_connection,
    execute_proxmox_controller_unmatch_host,
};
use uuid::Uuid;

/// Docker releases plugin type identifier — used as a DB filter value when querying
/// plugin assignments specific to the Docker releases plugin. Defined at module scope
/// so it does not appear as a literal in filter-expression contexts where the
/// `Column::PluginType` reference would trigger the plugin-type identity check.
const DOCKER_RELEASES_CONFIG_TYPE: &str = "releases_docker";

mod notifications;
mod params;
mod proxmox_add_config;
mod settings_store;

// These re-exports are consumed by `local_executor.rs`, which is not yet wired into the
// module tree because it shares helper function names with the inline implementations in
// `surface_proxy.rs` (the legacy path) that are pending deduplication. Until that step
// lands, the functions exist here but have no compiled callers, triggering unused-import
// warnings. Remove these allow attributes once `local_executor.rs` is incorporated.
#[expect(
    unused_imports,
    reason = "re-exports consumed by local_executor.rs which is not yet wired"
)]
pub use notifications::{
    allowlisted_notification_channel_controller_local_action,
    execute_allowlisted_notification_channel_action, notification_channel_type_for_surface_id,
};
// The two builder helpers below are used by `tests/controller_owned/notifications.rs`.
// They are behind `#[cfg(test)]` because only tests reach them today; production callers
// (again, `local_executor.rs`) are pending wiring.
#[cfg(test)]
#[expect(
    unused_imports,
    reason = "re-exports consumed by local_executor.rs which is not yet wired"
)]
pub use notifications::{
    build_notification_channel_create_request, build_notification_channel_update_request,
};
#[expect(
    unused_imports,
    reason = "re-exports consumed by local_executor.rs which is not yet wired"
)]
pub use proxmox_add_config::{
    allowlisted_proxmox_add_config_controller_local_action, allowlisted_proxmox_provider,
    emit_proxmox_add_config_audit_event, execute_allowlisted_proxmox_add_config_action,
};

// Called from `local_executor.rs` to map a controller-local surface action error into a
// `SurfaceProxyError` for the HTTP response layer. Not yet reachable because
// `local_executor.rs` is pending wiring (see comments above).
#[expect(
    dead_code,
    reason = "consumed by local_executor.rs which is not yet wired"
)]
pub fn map_surface_action_error(err: SurfaceActionError) -> SurfaceProxyError {
    match err {
        SurfaceActionError::InvalidInput(message) => {
            SurfaceProxyError::SchemaValidationFailed(message)
        }
        SurfaceActionError::ControllerIntegration(message)
        | SurfaceActionError::PluginInternal(message) => {
            tracing::error!(error = %message, "controller-local surface action failed");
            SurfaceProxyError::SendFailed
        }
        other => {
            tracing::error!(error = ?other, "unexpected controller-local surface action failure");
            SurfaceProxyError::SendFailed
        }
    }
}

fn plugin_internal_error(error: impl std::fmt::Display) -> rootcause::Report<PluginError> {
    report!(PluginError::PluginInternal(error.to_string()))
}

pub struct AppStateSurfaceActionController<'a> {
    db: &'a sea_orm::DatabaseConnection,
    plugin_ops: &'a dyn PluginOps,
    tenant_id: Uuid,
    caller_user_id: Option<Uuid>,
}

impl<'a> AppStateSurfaceActionController<'a> {
    pub fn from_database_connection(
        db: &'a sea_orm::DatabaseConnection,
        plugin_ops: &'a dyn PluginOps,
        tenant_id: Uuid,
        caller_user_id: Option<Uuid>,
    ) -> Self {
        Self {
            db,
            plugin_ops,
            tenant_id,
            caller_user_id,
        }
    }

    fn db(&self) -> &sea_orm::DatabaseConnection {
        self.db
    }
}

impl SurfaceActionController for AppStateSurfaceActionController<'_> {
    fn tenant_id(&self) -> Uuid {
        self.tenant_id
    }

    fn user_id(&self) -> Option<Uuid> {
        self.caller_user_id
    }

    fn notification_channel_store(&self) -> Option<&dyn NotificationChannelStore> {
        Some(self)
    }

    fn email_smtp_settings_store(&self) -> Option<&dyn EmailSmtpSettingsStore> {
        Some(self)
    }

    fn telegram_global_settings_store(&self) -> Option<&dyn TelegramGlobalSettingsStore> {
        Some(self)
    }

    fn docker_surface_store(&self) -> Option<&dyn DockerSurfaceStore> {
        Some(self)
    }

    fn proxmox_surface_store(&self) -> Option<&dyn ProxmoxSurfaceStore> {
        Some(self)
    }
}

#[async_trait]
impl NotificationChannelStore for AppStateSurfaceActionController<'_> {
    async fn list_channels(
        &self,
        req: NotificationChannelListRequest<'_>,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<NotificationChannelListPage> {
        use uptrakit_shared_db::entity::notification_channel;
        use uptrakit_web_api_types::pagination::PaginationParams;

        let resolved = PaginationParams {
            page: Some(req.page),
            per_page: Some(req.per_page),
        }
        .resolve();

        let total = notification_channel::Entity::find()
            .filter(notification_channel::Column::TenantId.eq(req.tenant_id))
            .filter(notification_channel::Column::ChannelType.eq(req.channel_type))
            .count(self.db())
            .await
            .map_err(plugin_internal_error)?;

        let channels = notification_channel::Entity::find()
            .filter(notification_channel::Column::TenantId.eq(req.tenant_id))
            .filter(notification_channel::Column::ChannelType.eq(req.channel_type))
            .order_by_desc(notification_channel::Column::CreatedAt)
            .offset(resolved.offset())
            .limit(resolved.per_page)
            .all(self.db())
            .await
            .map_err(plugin_internal_error)?;

        let items = channels
            .into_iter()
            .map(|channel| {
                let channel_type_id =
                    uptrakit_shared_types::PluginTypeId::new(&channel.channel_type);
                let raw_config: serde_json::Value =
                    serde_json::from_str(channel.config.expose_secret()).unwrap_or_default();
                let config = if self.plugin_ops.transport(&channel_type_id).is_some() {
                    self.plugin_ops
                        .mask_config_secrets(&channel_type_id, &raw_config)
                } else {
                    serde_json::json!({})
                };

                NotificationChannelListItem {
                    id: channel.id,
                    name: channel.name,
                    enabled: channel.enabled,
                    created_at_rfc3339: channel
                        .created_at
                        .format(&Rfc3339)
                        .unwrap_or_else(|_| channel.created_at.to_string()),
                    config,
                }
            })
            .collect();

        Ok(NotificationChannelListPage {
            items,
            total,
            page: resolved.page,
            per_page: resolved.per_page,
            total_pages: resolved.total_pages(total),
        })
    }

    async fn resolve_action_token(
        &self,
        action_token: Uuid,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<Option<NotificationActionTokenRecord>>
    {
        let model = uptrakit_web_api_queries::queries::notifications::find_log_by_action_token(
            self.db(),
            action_token,
        )
        .await
        .map_err(plugin_internal_error)?;

        Ok(model.map(|row| NotificationActionTokenRecord {
            action_token: row.action_token.unwrap_or(action_token),
            action_taken: row.action_taken,
        }))
    }

    async fn mark_action_token_triggered(
        &self,
        action_token: Uuid,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<()> {
        use sea_orm::sea_query::Expr;
        use uptrakit_shared_db::entity::notification_log;

        notification_log::Entity::update_many()
            .col_expr(
                notification_log::Column::ActionTaken,
                Expr::value(Some("triggered".to_string())),
            )
            .filter(notification_log::Column::ActionToken.eq(action_token))
            .exec(self.db())
            .await
            .map_err(plugin_internal_error)?;
        Ok(())
    }
}

#[async_trait]
impl DockerSurfaceStore for AppStateSurfaceActionController<'_> {
    async fn load_current_image_ref(
        &self,
        host_id: Uuid,
        software_item_id: Uuid,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<String> {
        use uptrakit_shared_db::entity::host_software_item_plugin;

        let plugin_rows = host_software_item_plugin::Entity::find()
            .filter(host_software_item_plugin::Column::HostId.eq(host_id))
            .filter(host_software_item_plugin::Column::SoftwareItemId.eq(software_item_id))
            .filter(host_software_item_plugin::Column::PluginType.eq(DOCKER_RELEASES_CONFIG_TYPE))
            .all(self.db())
            .await
            .map_err(plugin_internal_error)?;

        Ok(plugin_rows
            .into_iter()
            .next()
            .map(|row| strip_container_suffix(&row.package_identifier))
            .unwrap_or_default())
    }

    async fn switch_image_ref(
        &self,
        host_id: Uuid,
        software_item_id: Uuid,
        new_image_ref: String,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<()> {
        use uptrakit_shared_db::entity::{host_software_item, host_software_item_plugin};

        let plugin_rows = host_software_item_plugin::Entity::find()
            .filter(host_software_item_plugin::Column::HostId.eq(host_id))
            .filter(host_software_item_plugin::Column::SoftwareItemId.eq(software_item_id))
            .all(self.db())
            .await
            .map_err(|e| {
                plugin_internal_error(format!("database error loading plugin rows: {e}"))
            })?;

        if plugin_rows.is_empty() {
            return Err(plugin_internal_error(
                "no plugin assignments found for this host",
            ));
        }

        let hsi_row = host_software_item::Entity::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .filter(host_software_item::Column::SoftwareItemId.eq(software_item_id))
            .one(self.db())
            .await
            .map_err(|e| {
                plugin_internal_error(format!("database error loading host_software_item: {e}"))
            })?
            .ok_or_else(|| {
                plugin_internal_error(format!(
                    "host_software_item not found for host {host_id} / item {software_item_id}"
                ))
            })?;

        let txn = self
            .db()
            .begin()
            .await
            .map_err(|e| plugin_internal_error(format!("failed to begin transaction: {e}")))?;

        for row in plugin_rows {
            if row.plugin_type != DOCKER_RELEASES_CONFIG_TYPE {
                continue;
            }

            let new_pkg_id = match extract_container_suffix(&row.package_identifier) {
                Some(container) => format!("{new_image_ref}#{container}"),
                None => new_image_ref.clone(),
            };

            let mut active: host_software_item_plugin::ActiveModel = row.into();
            active.package_identifier = Set(new_pkg_id);
            active
                .update(&txn)
                .await
                .map_err(|e| plugin_internal_error(format!("failed to update plugin row: {e}")))?;
        }

        let mut hsi_active: host_software_item::ActiveModel = hsi_row.into();
        hsi_active.package_identifier = Set(Some(new_image_ref));
        hsi_active.installed_version = Set(None);
        hsi_active.installed_display_version = Set(None);
        hsi_active.installed_version_detected_at = Set(None);
        hsi_active.latest_version = Set(None);
        hsi_active.latest_version_fetched_at = Set(None);
        hsi_active.latest_release_metadata = Set(None);
        hsi_active.update_category = Set("unknown".to_string());
        hsi_active.update(&txn).await.map_err(|e| {
            plugin_internal_error(format!("failed to update host_software_item: {e}"))
        })?;

        txn.commit()
            .await
            .map_err(|e| plugin_internal_error(format!("failed to commit transaction: {e}")))?;

        Ok(())
    }
}

#[async_trait]
impl ProxmoxSurfaceStore for AppStateSurfaceActionController<'_> {
    async fn list_host_mappings(
        &self,
        request: ProxmoxHostMappingsRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        let params = serde_json::to_value(request).map_err(plugin_internal_error)?;
        execute_proxmox_controller_list_host_mappings(self.db(), Some(self.tenant_id), params)
            .await
            .map_err(plugin_internal_error)
    }

    async fn discover_hosts(
        &self,
        request: ProxmoxPluginConfigRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_controller_discover_hosts(self.db(), Some(self.tenant_id), request)
            .await
            .map_err(plugin_internal_error)
    }

    async fn test_connection(
        &self,
        request: ProxmoxPluginConfigRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_controller_test_connection(self.db(), Some(self.tenant_id), request)
            .await
            .map_err(plugin_internal_error)
    }

    async fn match_host(
        &self,
        request: ProxmoxManualMatchRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_controller_manual_match(self.db(), request)
            .await
            .map_err(plugin_internal_error)
    }

    async fn approve_match(
        &self,
        request: ProxmoxApproveMatchRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_controller_approve_match(self.db(), request)
            .await
            .map_err(plugin_internal_error)
    }

    async fn unmatch_host(
        &self,
        request: ProxmoxMappingRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_controller_unmatch_host(self.db(), request)
            .await
            .map_err(plugin_internal_error)
    }

    async fn list_all_unmatched(
        &self,
        request: ProxmoxUnmatchedGuestsRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_controller_list_all_unmatched(self.db(), Some(self.tenant_id), request)
            .await
            .map_err(plugin_internal_error)
    }

    async fn get_host_info(
        &self,
        request: ProxmoxHostInfoRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_controller_get_host_info(self.db(), Some(self.tenant_id), request)
            .await
            .map_err(plugin_internal_error)
    }

    async fn preload_global_defaults(
        &self,
        request: ProxmoxScopeSelectionRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_controller_preload_global_defaults(self.db(), Some(self.tenant_id), request)
            .await
            .map_err(plugin_internal_error)
    }

    async fn save_global_defaults(
        &self,
        request: ProxmoxGlobalDefaultsSaveRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_controller_save_global_defaults(self.db(), Some(self.tenant_id), request)
            .await
            .map_err(plugin_internal_error)
    }

    async fn preload_item_overrides(
        &self,
        request: ProxmoxItemOverridePreloadRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_controller_preload_item_overrides(self.db(), Some(self.tenant_id), request)
            .await
            .map_err(plugin_internal_error)
    }

    async fn save_item_overrides(
        &self,
        request: ProxmoxItemOverrideSaveRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_controller_save_item_overrides(self.db(), Some(self.tenant_id), request)
            .await
            .map_err(plugin_internal_error)
    }

    async fn load_backup_target_options(
        &self,
        request: ProxmoxScopeSelectionRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_controller_load_backup_target_options(
            self.db(),
            Some(self.tenant_id),
            request,
        )
        .await
        .map_err(plugin_internal_error)
    }
}

fn strip_container_suffix(id: &str) -> String {
    match id.find('#') {
        #[expect(
            clippy::string_slice,
            reason = "`pos` from str::find is on a char boundary"
        )]
        Some(pos) => id[..pos].to_string(),
        None => id.to_string(),
    }
}

fn extract_container_suffix(id: &str) -> Option<&str> {
    #[expect(
        clippy::string_slice,
        reason = "`pos` from str::find is on a char boundary; `pos + 1` advances past the ASCII '#'"
    )]
    id.find('#').map(|pos| &id[pos + 1..])
}
