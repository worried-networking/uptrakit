use super::SurfaceProxyError;
use async_trait::async_trait;
use rootcause::report;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set, TransactionTrait,
};
use serde::Serialize;
use time::format_description::well_known::Rfc3339;
use uptrakit_plugin_infrastructure_registry::{
    DockerSurfaceStore, EmailSmtpSettings, EmailSmtpSettingsPatch, EmailSmtpSettingsStore,
    NotificationActionTokenRecord, NotificationChannelListItem, NotificationChannelListPage,
    NotificationChannelListRequest, NotificationChannelStore, PluginError, PluginOps,
    ProxmoxApproveMatchRequest, ProxmoxGlobalDefaultsSaveRequest, ProxmoxHostInfoRequest,
    ProxmoxHostMappingsRequest, ProxmoxItemOverridePreloadRequest, ProxmoxItemOverrideSaveRequest,
    ProxmoxManualMatchRequest, ProxmoxMappingRequest, ProxmoxPluginConfigRequest,
    ProxmoxScopeSelectionRequest, ProxmoxSurfaceStore, ProxmoxUnmatchedGuestsRequest,
    SurfaceActionController, SurfaceActionError, TelegramGlobalSettingsStore,
    execute_proxmox_controller_surface_action,
};
use uuid::Uuid;
const SMTP_PASSWORD_AAD: &str = "uptrakit:settings:smtp_password";
const GLOBAL_SMTP_PASSWORD_AAD: &str = "uptrakit:settings:global_smtp_password";

const SMTP_PREFIX: &str = "smtp.";
const GLOBAL_SMTP_PREFIX: &str = "global_smtp.";

const KEY_SMTP_HOST: &str = "smtp.host";
const KEY_SMTP_PORT: &str = "smtp.port";
const KEY_SMTP_USERNAME: &str = "smtp.username";
const KEY_SMTP_PASSWORD: &str = "smtp.password";
const KEY_SMTP_FROM_ADDRESS: &str = "smtp.from_address";
const KEY_SMTP_FROM_NAME: &str = "smtp.from_name";
const KEY_SMTP_TLS_MODE: &str = "smtp.tls_mode";
const KEY_SMTP_HELO_HOST: &str = "smtp.helo_host";

const KEY_GLOBAL_SMTP_HOST: &str = "global_smtp.host";
const KEY_GLOBAL_SMTP_PORT: &str = "global_smtp.port";
const KEY_GLOBAL_SMTP_USERNAME: &str = "global_smtp.username";
const KEY_GLOBAL_SMTP_PASSWORD: &str = "global_smtp.password";
const KEY_GLOBAL_SMTP_FROM_ADDRESS: &str = "global_smtp.from_address";
const KEY_GLOBAL_SMTP_FROM_NAME: &str = "global_smtp.from_name";
const KEY_GLOBAL_SMTP_TLS_MODE: &str = "global_smtp.tls_mode";
const KEY_GLOBAL_SMTP_HELO_HOST: &str = "global_smtp.helo_host";
const GLOBAL_TELEGRAM_PREFIX: &str = "global_telegram.";
const KEY_GLOBAL_TELEGRAM_BOT_TOKEN: &str = "global_telegram.bot_token";

pub(crate) fn map_surface_action_error(err: SurfaceActionError) -> SurfaceProxyError {
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

pub(crate) struct AppStateSurfaceActionController<'a> {
    db: &'a sea_orm::DatabaseConnection,
    plugin_ops: &'a dyn PluginOps,
    tenant_id: Uuid,
    caller_user_id: Option<Uuid>,
}

impl<'a> AppStateSurfaceActionController<'a> {
    pub(crate) fn from_app_state(
        state: &'a crate::AppState,
        tenant_id: Uuid,
        caller_user_id: Option<Uuid>,
    ) -> Self {
        Self::from_database_connection(
            state.db(),
            state.plugin_ops.as_ref(),
            tenant_id,
            caller_user_id,
        )
    }

    pub(crate) fn from_database_connection(
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
        let model =
            crate::queries::notifications::find_log_by_action_token(self.db(), action_token)
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
impl EmailSmtpSettingsStore for AppStateSurfaceActionController<'_> {
    async fn load_tenant_smtp_settings(
        &self,
        tenant_id: Uuid,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<EmailSmtpSettings> {
        let settings = uptrakit_web_api_auth::settings_store::load_typed_settings_by_prefix(
            self.db(),
            tenant_id,
            SMTP_PREFIX,
        )
        .await
        .map_err(plugin_internal_error)?;
        Ok(normalize_smtp_settings(
            settings,
            SMTP_PASSWORD_AAD,
            "tenant",
            Some(tenant_id),
        ))
    }

    async fn save_tenant_smtp_settings(
        &self,
        tenant_id: Uuid,
        patch: EmailSmtpSettingsPatch,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<EmailSmtpSettings> {
        apply_tenant_smtp_patch(self.db(), tenant_id, patch).await?;
        self.load_tenant_smtp_settings(tenant_id).await
    }

    async fn load_global_smtp_settings(
        &self,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<EmailSmtpSettings> {
        let settings = uptrakit_web_api_auth::settings_store::load_typed_global_settings_by_prefix(
            self.db(),
            GLOBAL_SMTP_PREFIX,
        )
        .await
        .map_err(plugin_internal_error)?;
        Ok(normalize_smtp_settings(
            settings,
            GLOBAL_SMTP_PASSWORD_AAD,
            "global",
            None,
        ))
    }

    async fn save_global_smtp_settings(
        &self,
        patch: EmailSmtpSettingsPatch,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<EmailSmtpSettings> {
        apply_global_smtp_patch(self.db(), patch).await?;
        self.load_global_smtp_settings().await
    }

    async fn load_user_email(
        &self,
        user_id: Uuid,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<Option<String>> {
        let model = uptrakit_shared_db::entity::prelude::User::find_by_id(user_id)
            .one(self.db())
            .await
            .map_err(plugin_internal_error)?;
        Ok(model.map(|user| user.email.expose_email().to_string()))
    }
}

#[async_trait]
impl TelegramGlobalSettingsStore for AppStateSurfaceActionController<'_> {
    async fn load_global_bot_token(
        &self,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<String> {
        let map = uptrakit_web_api_auth::settings_store::load_global_settings_by_prefix(
            self.db(),
            GLOBAL_TELEGRAM_PREFIX,
        )
        .await
        .map_err(plugin_internal_error)?;

        Ok(map
            .get(KEY_GLOBAL_TELEGRAM_BOT_TOKEN)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string())
    }

    async fn save_global_bot_token(
        &self,
        bot_token: String,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<String> {
        uptrakit_web_api_auth::settings_store::upsert_global_setting_raw(
            self.db(),
            KEY_GLOBAL_TELEGRAM_BOT_TOKEN,
            serde_json::json!(bot_token),
        )
        .await
        .map_err(plugin_internal_error)?;
        self.load_global_bot_token().await
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
            .filter(host_software_item_plugin::Column::PluginType.eq("releases_docker"))
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
            if row.plugin_type != "releases_docker" {
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
        execute_proxmox_surface_action(self, "proxmox.hosts", "list", &request).await
    }

    async fn discover_hosts(
        &self,
        request: ProxmoxPluginConfigRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_surface_action(self, "proxmox.hosts", "discover", &request).await
    }

    async fn test_connection(
        &self,
        request: ProxmoxPluginConfigRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_surface_action(self, "proxmox.hosts", "test-connection", &request).await
    }

    async fn match_host(
        &self,
        request: ProxmoxManualMatchRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_surface_action(self, "proxmox.hosts", "match", &request).await
    }

    async fn approve_match(
        &self,
        request: ProxmoxApproveMatchRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_surface_action(self, "proxmox.hosts", "approve-match", &request).await
    }

    async fn unmatch_host(
        &self,
        request: ProxmoxMappingRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_surface_action(self, "proxmox.hosts", "unmatch", &request).await
    }

    async fn list_all_unmatched(
        &self,
        request: ProxmoxUnmatchedGuestsRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_surface_action(self, "proxmox.hosts", "list-all-unmatched", &request).await
    }

    async fn get_host_info(
        &self,
        request: ProxmoxHostInfoRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_surface_action(self, "proxmox.host-info", "get-info", &request).await
    }

    async fn preload_global_defaults(
        &self,
        request: ProxmoxScopeSelectionRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_surface_action(
            self,
            "proxmox.settings.update-protection",
            "preload-global-defaults",
            &request,
        )
        .await
    }

    async fn save_global_defaults(
        &self,
        request: ProxmoxGlobalDefaultsSaveRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_surface_action(
            self,
            "proxmox.settings.update-protection",
            "save-global-defaults",
            &request,
        )
        .await
    }

    async fn preload_item_overrides(
        &self,
        request: ProxmoxItemOverridePreloadRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_surface_action(
            self,
            "proxmox.software-item.update-protection",
            "preload-item-overrides",
            &request,
        )
        .await
    }

    async fn save_item_overrides(
        &self,
        request: ProxmoxItemOverrideSaveRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_surface_action(
            self,
            "proxmox.software-item.update-protection",
            "save-item-overrides",
            &request,
        )
        .await
    }

    async fn load_backup_target_options(
        &self,
        request: ProxmoxScopeSelectionRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_surface_action(
            self,
            "proxmox.settings.update-protection",
            "load-backup-target-options",
            &request,
        )
        .await
    }
}

async fn execute_proxmox_surface_action<Request: Serialize>(
    controller: &AppStateSurfaceActionController<'_>,
    surface_id: &str,
    action_id: &str,
    request: &Request,
) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
    let params = serde_json::to_value(request).map_err(plugin_internal_error)?;
    execute_proxmox_controller_surface_action(
        controller.db(),
        Some(controller.tenant_id),
        surface_id,
        action_id,
        params,
    )
    .await
    .map_err(plugin_internal_error)
}

fn strip_container_suffix(id: &str) -> String {
    match id.find('#') {
        Some(pos) => id[..pos].to_string(),
        None => id.to_string(),
    }
}

fn extract_container_suffix(id: &str) -> Option<&str> {
    id.find('#').map(|pos| &id[pos + 1..])
}

fn normalize_smtp_settings(
    settings: EmailSmtpSettings,
    password_aad: &str,
    scope: &'static str,
    tenant_id: Option<Uuid>,
) -> EmailSmtpSettings {
    EmailSmtpSettings {
        host: normalize_non_empty_string(settings.host),
        port: settings.port,
        username: normalize_non_empty_string(settings.username),
        password: decode_smtp_password(settings.password, password_aad, scope, tenant_id),
        from_address: normalize_non_empty_string(settings.from_address),
        from_name: normalize_non_empty_string(settings.from_name),
        tls_mode: normalize_non_empty_string(settings.tls_mode),
        helo_host: normalize_non_empty_string(settings.helo_host),
    }
}

fn normalize_non_empty_string(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

fn decode_smtp_password(
    value: Option<String>,
    aad: &str,
    scope: &'static str,
    tenant_id: Option<Uuid>,
) -> Option<String> {
    let raw = normalize_non_empty_string(value)?;

    if uptrakit_crypto::is_encrypted(&raw) {
        return match uptrakit_crypto::decrypt_str(&raw, aad) {
            Ok(value) => normalize_non_empty_string(Some(value)),
            Err(error) => {
                if let Some(tenant_id) = tenant_id {
                    tracing::warn!(
                        error = ?error,
                        %tenant_id,
                        scope,
                        "failed to decrypt SMTP password while loading typed SMTP settings; using empty fallback"
                    );
                } else {
                    tracing::warn!(
                        error = ?error,
                        scope,
                        "failed to decrypt SMTP password while loading typed SMTP settings; using empty fallback"
                    );
                }
                None
            }
        };
    }

    Some(raw)
}

async fn apply_tenant_smtp_patch(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    patch: EmailSmtpSettingsPatch,
) -> uptrakit_plugin_infrastructure_registry::PluginResult<()> {
    if let Some(value) = patch.host {
        upsert_tenant_setting_raw(
            db,
            tenant_id,
            KEY_SMTP_HOST,
            value.map(serde_json::Value::String),
        )
        .await?;
    }
    if let Some(value) = patch.port {
        upsert_tenant_setting_raw(
            db,
            tenant_id,
            KEY_SMTP_PORT,
            value.map(|port| serde_json::json!(port)),
        )
        .await?;
    }
    if let Some(value) = patch.username {
        upsert_tenant_setting_raw(
            db,
            tenant_id,
            KEY_SMTP_USERNAME,
            value.map(serde_json::Value::String),
        )
        .await?;
    }
    if let Some(value) = patch.password {
        let password_value = match value {
            Some(password) => Some(serde_json::Value::String(
                uptrakit_crypto::encrypt_str(password.as_str(), SMTP_PASSWORD_AAD)
                    .map_err(plugin_internal_error)?,
            )),
            None => None,
        };
        upsert_tenant_setting_raw(db, tenant_id, KEY_SMTP_PASSWORD, password_value).await?;
    }
    if let Some(value) = patch.from_address {
        upsert_tenant_setting_raw(
            db,
            tenant_id,
            KEY_SMTP_FROM_ADDRESS,
            value.map(serde_json::Value::String),
        )
        .await?;
    }
    if let Some(value) = patch.from_name {
        upsert_tenant_setting_raw(
            db,
            tenant_id,
            KEY_SMTP_FROM_NAME,
            value.map(serde_json::Value::String),
        )
        .await?;
    }
    if let Some(value) = patch.tls_mode {
        upsert_tenant_setting_raw(
            db,
            tenant_id,
            KEY_SMTP_TLS_MODE,
            value.map(serde_json::Value::String),
        )
        .await?;
    }
    if let Some(value) = patch.helo_host {
        upsert_tenant_setting_raw(
            db,
            tenant_id,
            KEY_SMTP_HELO_HOST,
            value.map(serde_json::Value::String),
        )
        .await?;
    }
    Ok(())
}

async fn apply_global_smtp_patch(
    db: &sea_orm::DatabaseConnection,
    patch: EmailSmtpSettingsPatch,
) -> uptrakit_plugin_infrastructure_registry::PluginResult<()> {
    if let Some(value) = patch.host {
        upsert_global_setting_raw(
            db,
            KEY_GLOBAL_SMTP_HOST,
            value.map(serde_json::Value::String),
        )
        .await?;
    }
    if let Some(value) = patch.port {
        upsert_global_setting_raw(
            db,
            KEY_GLOBAL_SMTP_PORT,
            value.map(|port| serde_json::json!(port)),
        )
        .await?;
    }
    if let Some(value) = patch.username {
        upsert_global_setting_raw(
            db,
            KEY_GLOBAL_SMTP_USERNAME,
            value.map(serde_json::Value::String),
        )
        .await?;
    }
    if let Some(value) = patch.password {
        let password_value = match value {
            Some(password) => Some(serde_json::Value::String(
                uptrakit_crypto::encrypt_str(password.as_str(), GLOBAL_SMTP_PASSWORD_AAD)
                    .map_err(plugin_internal_error)?,
            )),
            None => None,
        };
        upsert_global_setting_raw(db, KEY_GLOBAL_SMTP_PASSWORD, password_value).await?;
    }
    if let Some(value) = patch.from_address {
        upsert_global_setting_raw(
            db,
            KEY_GLOBAL_SMTP_FROM_ADDRESS,
            value.map(serde_json::Value::String),
        )
        .await?;
    }
    if let Some(value) = patch.from_name {
        upsert_global_setting_raw(
            db,
            KEY_GLOBAL_SMTP_FROM_NAME,
            value.map(serde_json::Value::String),
        )
        .await?;
    }
    if let Some(value) = patch.tls_mode {
        upsert_global_setting_raw(
            db,
            KEY_GLOBAL_SMTP_TLS_MODE,
            value.map(serde_json::Value::String),
        )
        .await?;
    }
    if let Some(value) = patch.helo_host {
        upsert_global_setting_raw(
            db,
            KEY_GLOBAL_SMTP_HELO_HOST,
            value.map(serde_json::Value::String),
        )
        .await?;
    }
    Ok(())
}

async fn upsert_tenant_setting_raw(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    key: &str,
    value: Option<serde_json::Value>,
) -> uptrakit_plugin_infrastructure_registry::PluginResult<()> {
    let payload = value.unwrap_or(serde_json::Value::Null);
    uptrakit_web_api_auth::settings_store::upsert_setting_raw(db, tenant_id, key, payload)
        .await
        .map_err(plugin_internal_error)
}

async fn upsert_global_setting_raw(
    db: &sea_orm::DatabaseConnection,
    key: &str,
    value: Option<serde_json::Value>,
) -> uptrakit_plugin_infrastructure_registry::PluginResult<()> {
    let payload = value.unwrap_or(serde_json::Value::Null);
    uptrakit_web_api_auth::settings_store::upsert_global_setting_raw(db, key, payload)
        .await
        .map_err(plugin_internal_error)
}

pub(crate) fn notification_channel_type_for_surface_id(surface_id: &str) -> Option<&'static str> {
    match surface_id {
        "notifications.email" => Some("email"),
        "notifications.telegram" => Some("telegram"),
        "notifications.webhook" => Some("webhook"),
        _ => None,
    }
}

fn allowlisted_notification_channel_provider(provider_id: &str, channel_type: &str) -> bool {
    match channel_type {
        "email" => matches!(provider_id, "plugin.email" | "plugin.notifications_email"),
        "telegram" => matches!(
            provider_id,
            "plugin.telegram" | "plugin.notifications_telegram"
        ),
        "webhook" => matches!(
            provider_id,
            "plugin.webhook" | "plugin.notifications_webhook"
        ),
        _ => false,
    }
}

pub(crate) fn allowlisted_notification_channel_controller_local_action(
    provider_id: &str,
    surface_id: &str,
    interaction_id: &str,
) -> Option<&'static str> {
    if !matches!(interaction_id, "create" | "edit" | "test" | "delete") {
        return None;
    }
    let channel_type = notification_channel_type_for_surface_id(surface_id)?;
    allowlisted_notification_channel_provider(provider_id, channel_type).then_some(channel_type)
}

pub(crate) fn allowlisted_proxmox_provider(provider_id: &str) -> bool {
    matches!(
        provider_id,
        "plugin.infrastructure_proxmox" | "infrastructure_proxmox"
    )
}

pub(crate) fn allowlisted_proxmox_add_config_controller_local_action(
    surface_id: &str,
    interaction_id: &str,
) -> bool {
    surface_id == "proxmox.hosts" && interaction_id == "add-config"
}

pub(crate) async fn execute_allowlisted_notification_channel_action(
    tenant_db: &uptrakit_web_api_queries::TenantDb,
    plugin_ops: &dyn PluginOps,
    channel_type: &str,
    interaction_id: &str,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, String> {
    use uptrakit_web_api_types::validation::Validate as _;

    match interaction_id {
        "create" => {
            let req = build_notification_channel_create_request(channel_type, params)?;
            req.validate().map_err(|error| error.to_string())?;
            let response =
                crate::queries::notifications::create_channel(tenant_db, &req, plugin_ops)
                    .await
                    .map_err(|error| error.to_string())?;
            serde_json::to_value(response)
                .map_err(|error| format!("failed to serialize create response: {error}"))
        }
        "edit" => {
            let channel_id = required_uuid_param(params, "id")?;
            require_notification_channel_type(tenant_db, channel_id, channel_type).await?;
            let req = build_notification_channel_update_request(channel_type, params)?;
            req.validate().map_err(|error| error.to_string())?;
            let response = crate::queries::notifications::update_channel(
                tenant_db, channel_id, &req, plugin_ops,
            )
            .await
            .map_err(|error| error.to_string())?;
            let Some(response) = response else {
                return Err("Channel not found".to_string());
            };
            serde_json::to_value(response)
                .map_err(|error| format!("failed to serialize update response: {error}"))
        }
        "delete" => {
            let channel_id = required_uuid_param(params, "id")?;
            require_notification_channel_type(tenant_db, channel_id, channel_type).await?;
            let deleted = crate::queries::notifications::delete_channel(tenant_db, channel_id)
                .await
                .map_err(|error| error.to_string())?;
            if !deleted {
                return Err("Channel not found".to_string());
            }
            Ok(serde_json::json!({}))
        }
        "test" => {
            let channel_id = required_uuid_param(params, "id")?;
            execute_notification_channel_test_action(
                tenant_db,
                plugin_ops,
                channel_id,
                channel_type,
            )
            .await
        }
        _ => Err(format!(
            "action `{interaction_id}` is not allowlisted for notification controller_local execution"
        )),
    }
}

pub(crate) async fn execute_allowlisted_proxmox_add_config_action(
    tenant_db: &uptrakit_web_api_queries::TenantDb,
    plugin_ops: &dyn PluginOps,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, SurfaceProxyError> {
    use uptrakit_web_api_types::validation::Validate as _;

    let request = build_proxmox_add_config_create_request(params)
        .map_err(SurfaceProxyError::SchemaValidationFailed)?;
    request
        .validate()
        .map_err(|error| SurfaceProxyError::SchemaValidationFailed(error.to_string()))?;
    plugin_ops
        .validate_config(&request.plugin_type, &request.config)
        .map_err(|error| SurfaceProxyError::SchemaValidationFailed(error.to_string()))?;
    let response = uptrakit_web_api_queries::queries::plugin_configs::create_plugin_config(
        plugin_ops, tenant_db, request,
    )
    .await
    .map_err(|error| match error.current_context() {
        uptrakit_web_api_queries::queries::plugin_configs::PluginConfigError::DuplicateName => {
            SurfaceProxyError::Conflict {
                message: error.to_string(),
                code: "duplicate_name",
            }
        }
        _ => SurfaceProxyError::SchemaValidationFailed(error.to_string()),
    })?;
    serde_json::to_value(response).map_err(|error| {
        SurfaceProxyError::SchemaValidationFailed(format!(
            "failed to serialize proxmox add-config response: {error}"
        ))
    })
}

pub(crate) fn emit_proxmox_add_config_audit_event(
    caller_user_id: Option<Uuid>,
    tenant_id: Uuid,
    result: &serde_json::Value,
) {
    let Some(caller_user_id) = caller_user_id else {
        return;
    };
    let Some(plugin_config_id) = result.get("id").and_then(|value| value.as_str()) else {
        return;
    };
    let Some(config_name) = result.get("name").and_then(|value| value.as_str()) else {
        return;
    };
    tracing::warn!(
        target: "security_audit",
        user_id = %caller_user_id,
        tenant_id = %tenant_id,
        plugin_config_id = %plugin_config_id,
        plugin_type = "infrastructure_proxmox",
        config_name = %config_name,
        "plugin config created"
    );
}

async fn execute_notification_channel_test_action(
    tenant_db: &uptrakit_web_api_queries::TenantDb,
    plugin_ops: &dyn PluginOps,
    channel_id: Uuid,
    expected_channel_type: &str,
) -> Result<serde_json::Value, String> {
    let channel =
        require_notification_channel_type(tenant_db, channel_id, expected_channel_type).await?;
    let config_json: serde_json::Value = serde_json::from_str(channel.config.expose_secret())
        .map_err(|error| format!("Failed to parse channel config: {error}"))?;
    let channel_type_id = uptrakit_shared_types::PluginTypeId::new(&channel.channel_type);
    let channel_transport = plugin_ops
        .transport(&channel_type_id)
        .ok_or_else(|| format!("Unsupported channel type: {}", channel.channel_type))?;

    let settings_bag =
        crate::notifications::dispatcher::build_settings_bag(tenant_db.db(), tenant_db.tenant_id)
            .await;
    let test_msg = uptrakit_plugin_infrastructure_registry::DeliveryMessage::new(
        "Test Notification",
        "This is a test notification from Uptrakit.",
        None,
        serde_json::json!({"test": true}),
        vec![],
    );

    channel_transport
        .deliver(&config_json, &settings_bag, &test_msg)
        .await
        .map_err(|error| error.to_string())?;

    serde_json::to_value(
        uptrakit_web_api_types::notifications::TestNotificationResponse {
            success: true,
            message: "Test notification delivered successfully".to_string(),
        },
    )
    .map_err(|error| format!("failed to serialize test response: {error}"))
}

async fn require_notification_channel_type(
    tenant_db: &uptrakit_web_api_queries::TenantDb,
    channel_id: Uuid,
    expected_channel_type: &str,
) -> Result<uptrakit_shared_db::entity::notification_channel::Model, String> {
    let model = tenant_db
        .find_by_id::<uptrakit_shared_db::entity::notification_channel::Entity, _>(channel_id)
        .one(tenant_db.db())
        .await
        .map_err(|error| format!("failed to load notification channel: {error}"))?;
    let Some(model) = model else {
        return Err("Channel not found".to_string());
    };
    if model.channel_type != expected_channel_type {
        return Err("Channel type mismatch for selected notification surface".to_string());
    }
    Ok(model)
}

pub(crate) fn build_notification_channel_create_request(
    channel_type: &str,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<uptrakit_web_api_types::notifications::CreateNotificationChannelRequest, String> {
    validate_or_reject_mismatched_channel_type(channel_type, params)?;

    Ok(
        uptrakit_web_api_types::notifications::CreateNotificationChannelRequest {
            name: required_string_param(params, "name")?,
            channel_type: channel_type.to_string(),
            config: resolve_notification_channel_config(channel_type, params)?,
            enabled: strict_bool_param_with_default(params, "enabled", true)?,
        },
    )
}

pub(crate) fn build_proxmox_add_config_create_request(
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<uptrakit_web_api_types::plugin_configs::CreatePluginConfigRequest, String> {
    Ok(
        uptrakit_web_api_types::plugin_configs::CreatePluginConfigRequest {
            name: required_string_param(params, "name")?,
            plugin_type: uptrakit_shared_types::plugin_ids::INFRASTRUCTURE_PROXMOX.clone(),
            config: resolve_proxmox_add_config(params)?,
            enabled: true,
        },
    )
}

pub(crate) fn build_notification_channel_update_request(
    channel_type: &str,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<uptrakit_web_api_types::notifications::UpdateNotificationChannelRequest, String> {
    Ok(
        uptrakit_web_api_types::notifications::UpdateNotificationChannelRequest {
            name: optional_string_param(params, "name")?,
            config: Some(resolve_notification_channel_config(channel_type, params)?),
            enabled: strict_optional_bool_param(params, "enabled")?,
        },
    )
}

fn validate_or_reject_mismatched_channel_type(
    expected_channel_type: &str,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    let Some(channel_type) = params.get("channel_type") else {
        return Ok(());
    };
    let Some(channel_type) = channel_type.as_str() else {
        return Err("field `channel_type` must be a string".to_string());
    };
    if channel_type != expected_channel_type {
        return Err(format!(
            "field `channel_type` must be `{expected_channel_type}` for this surface"
        ));
    }
    Ok(())
}

fn resolve_notification_channel_config(
    channel_type: &str,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<uptrakit_web_api_types::notifications::channels::JsonObjectInput, String> {
    if let Some(config) = params.get("config") {
        let Some(config) = config.as_object() else {
            return Err("field `config` must be a JSON object".to_string());
        };
        let value = match channel_type {
            "email" => {
                let to_addresses = parse_to_addresses_param(config, "to_addresses")?;
                serde_json::json!({ "to_addresses": to_addresses })
            }
            _ => serde_json::Value::Object(config.clone()),
        };
        return notification_channel_config_input(value);
    }
    notification_channel_config_input(build_notification_channel_config_from_flat_params(
        channel_type,
        params,
    )?)
}

fn notification_channel_config_input(
    value: serde_json::Value,
) -> Result<uptrakit_web_api_types::notifications::channels::JsonObjectInput, String> {
    uptrakit_web_api_types::notifications::channels::JsonObjectMap::try_from(value)
        .map(Into::into)
        .map_err(|error| error.message)
}

fn build_notification_channel_config_from_flat_params(
    channel_type: &str,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, String> {
    match channel_type {
        "email" => {
            let to_addresses = parse_to_addresses_param(params, "to_addresses")?;
            Ok(serde_json::json!({ "to_addresses": to_addresses }))
        }
        "telegram" => {
            let chat_id = required_string_param(params, "chat_id")?;
            let mut config = serde_json::Map::from_iter([(
                "chat_id".to_string(),
                serde_json::Value::String(chat_id),
            )]);
            if let Some(bot_token) = optional_string_param(params, "bot_token")? {
                config.insert(
                    "bot_token".to_string(),
                    serde_json::Value::String(bot_token),
                );
            }
            Ok(serde_json::Value::Object(config))
        }
        "webhook" => {
            let url = required_string_param(params, "url")?;
            let mut config =
                serde_json::Map::from_iter([("url".to_string(), serde_json::Value::String(url))]);
            if let Some(secret) = optional_string_param(params, "secret")? {
                config.insert("secret".to_string(), serde_json::Value::String(secret));
            }
            Ok(serde_json::Value::Object(config))
        }
        _ => Err(format!(
            "channel type `{channel_type}` is not allowlisted for controller-local execution"
        )),
    }
}

fn resolve_proxmox_add_config(
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, String> {
    if let Some(config) = params.get("config") {
        let Some(config) = config.as_object() else {
            return Err("field `config` must be a JSON object".to_string());
        };
        return build_proxmox_config_from_params(config);
    }
    build_proxmox_config_from_params(params)
}

fn build_proxmox_config_from_params(
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "api_url": required_string_param(params, "api_url")?,
        "api_token": required_string_param(params, "api_token")?,
        "verify_tls": proxmox_verify_tls_param_with_default(params, "verify_tls", true)?,
        "node_filter": parse_csv_array_or_string_array_param(params, "node_filter")?,
    }))
}

fn required_string_param(
    params: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<String, String> {
    let Some(value) = params.get(key) else {
        return Err(format!("missing required field `{key}`"));
    };
    let Some(value) = value.as_str() else {
        return Err(format!("field `{key}` must be a string"));
    };
    if value.trim().is_empty() {
        return Err(format!("field `{key}` must not be empty"));
    }
    Ok(value.to_string())
}

fn optional_string_param(
    params: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Option<String>, String> {
    let Some(value) = params.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(value) = value.as_str() else {
        return Err(format!("field `{key}` must be a string"));
    };
    Ok(Some(value.to_string()))
}

fn required_uuid_param(
    params: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Uuid, String> {
    let value = required_string_param(params, key)?;
    Uuid::parse_str(value.as_str())
        .map_err(|error| format!("field `{key}` must be a UUID: {error}"))
}

fn strict_bool_param_with_default(
    params: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    default: bool,
) -> Result<bool, String> {
    let Some(value) = params.get(key) else {
        return Ok(default);
    };
    let Some(value) = value.as_bool() else {
        return Err(format!("field `{key}` must be a boolean"));
    };
    Ok(value)
}

fn strict_optional_bool_param(
    params: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Option<bool>, String> {
    let Some(value) = params.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(value) = value.as_bool() else {
        return Err(format!("field `{key}` must be a boolean"));
    };
    Ok(Some(value))
}

fn proxmox_verify_tls_param_with_default(
    params: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    default: bool,
) -> Result<bool, String> {
    let Some(value) = params.get(key) else {
        return Ok(default);
    };
    match value {
        serde_json::Value::Bool(value) => Ok(*value),
        serde_json::Value::String(value) => match value.trim() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(format!(
                "field `{key}` must be a boolean or the string `true`/`false`"
            )),
        },
        _ => Err(format!(
            "field `{key}` must be a boolean or the string `true`/`false`"
        )),
    }
}

fn parse_to_addresses_param(
    params: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Vec<String>, String> {
    let Some(value) = params.get(key) else {
        return Err(format!("missing required field `{key}`"));
    };

    match value {
        serde_json::Value::String(text) => {
            let addresses = text
                .split([',', '\n'])
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            if addresses.is_empty() {
                return Err(format!("field `{key}` must include at least one address"));
            }
            Ok(addresses)
        }
        serde_json::Value::Array(values) => {
            let mut addresses = Vec::new();
            for value in values {
                let Some(value) = value.as_str() else {
                    return Err(format!("field `{key}` array entries must be strings"));
                };
                let value = value.trim();
                if !value.is_empty() {
                    addresses.push(value.to_string());
                }
            }
            if addresses.is_empty() {
                return Err(format!("field `{key}` must include at least one address"));
            }
            Ok(addresses)
        }
        _ => Err(format!(
            "field `{key}` must be either a string or an array of strings"
        )),
    }
}

fn parse_csv_array_or_string_array_param(
    params: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Vec<String>, String> {
    let Some(value) = params.get(key) else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }

    match value {
        serde_json::Value::String(text) => Ok(text
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(str::to_string)
            .collect()),
        serde_json::Value::Array(values) => {
            let mut parsed = Vec::new();
            for value in values {
                let Some(value) = value.as_str() else {
                    return Err(format!("field `{key}` array entries must be strings"));
                };
                let value = value.trim();
                if !value.is_empty() {
                    parsed.push(value.to_string());
                }
            }
            Ok(parsed)
        }
        _ => Err(format!(
            "field `{key}` must be either a csv string or an array of strings"
        )),
    }
}
