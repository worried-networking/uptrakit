//! Surface manifests and action handler dispatch for the Proxmox VE plugin.

use std::future::Future;
use std::pin::Pin;

use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use serde::de::DeserializeOwned;
use serde_json::json;
use uuid::Uuid;

use uptrakit_plugin_infrastructure_core::{
    ApiSubmitDescriptor, FormFieldDescriptor, FormFieldType, FormSelectSourceDescriptor,
    ProxmoxApproveMatchRequest, ProxmoxGlobalDefaultsSaveRequest, ProxmoxHostInfoRequest,
    ProxmoxHostMappingsRequest, ProxmoxItemOverridePreloadRequest, ProxmoxItemOverrideSaveRequest,
    ProxmoxManualMatchRequest, ProxmoxMappingRequest, ProxmoxPluginConfigRequest,
    ProxmoxScopeSelectionRequest, ProxmoxUnmatchedGuestsRequest, SurfaceActionContext,
    SurfaceActionDescriptor, SurfaceActionError, SurfaceActionUi, SurfaceFormDescriptor,
    SurfaceRowCondition,
};
use uptrakit_shared_types::Permission;

use crate::client::ProxmoxClient;
use crate::config::ProxmoxConfig;
use crate::policy_store::{
    ProtectionMode, ProtectionPolicy, delete_item_override, find_cached_backup_target,
    list_cached_backup_targets, load_global_default, load_item_override, upsert_global_default,
    upsert_item_override,
};

const SURFACE_SETTINGS_UPDATE_PROTECTION: &str = "proxmox.settings.update-protection";
const SURFACE_SOFTWARE_ITEM_UPDATE_PROTECTION: &str = "proxmox.software-item.update-protection";

const ACTION_PRELOAD_GLOBAL_DEFAULTS: &str = "preload-global-defaults";
const ACTION_SAVE_GLOBAL_DEFAULTS: &str = "save-global-defaults";
const ACTION_PRELOAD_ITEM_OVERRIDES: &str = "preload-item-overrides";
const ACTION_SAVE_ITEM_OVERRIDES: &str = "save-item-overrides";
const ACTION_LOAD_BACKUP_TARGET_OPTIONS: &str = "load-backup-target-options";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControllerSurfaceAction {
    ListHostMappings,
    DiscoverHosts,
    TestConnection,
    MatchHost,
    ApproveMatch,
    UnmatchHost,
    ListAllUnmatched,
    GetHostInfo,
    PreloadGlobalDefaults,
    SaveGlobalDefaults,
    PreloadItemOverrides,
    SaveItemOverrides,
    LoadBackupTargetOptions,
}

fn resolve_controller_surface_action(
    surface_id: &str,
    action_id: &str,
) -> Option<ControllerSurfaceAction> {
    match (surface_id, action_id) {
        ("proxmox.hosts", "list") => Some(ControllerSurfaceAction::ListHostMappings),
        ("proxmox.hosts", "discover") => Some(ControllerSurfaceAction::DiscoverHosts),
        ("proxmox.hosts", "test-connection") => Some(ControllerSurfaceAction::TestConnection),
        ("proxmox.hosts", "match") => Some(ControllerSurfaceAction::MatchHost),
        ("proxmox.hosts", "approve-match") => Some(ControllerSurfaceAction::ApproveMatch),
        ("proxmox.hosts", "unmatch") => Some(ControllerSurfaceAction::UnmatchHost),
        ("proxmox.hosts", "list-all-unmatched") => Some(ControllerSurfaceAction::ListAllUnmatched),
        ("proxmox.host-info", "get-info") => Some(ControllerSurfaceAction::GetHostInfo),
        (SURFACE_SETTINGS_UPDATE_PROTECTION, ACTION_PRELOAD_GLOBAL_DEFAULTS) => {
            Some(ControllerSurfaceAction::PreloadGlobalDefaults)
        }
        (SURFACE_SETTINGS_UPDATE_PROTECTION, ACTION_SAVE_GLOBAL_DEFAULTS) => {
            Some(ControllerSurfaceAction::SaveGlobalDefaults)
        }
        (SURFACE_SETTINGS_UPDATE_PROTECTION, ACTION_LOAD_BACKUP_TARGET_OPTIONS)
        | (SURFACE_SOFTWARE_ITEM_UPDATE_PROTECTION, ACTION_LOAD_BACKUP_TARGET_OPTIONS) => {
            Some(ControllerSurfaceAction::LoadBackupTargetOptions)
        }
        (SURFACE_SOFTWARE_ITEM_UPDATE_PROTECTION, ACTION_PRELOAD_ITEM_OVERRIDES) => {
            Some(ControllerSurfaceAction::PreloadItemOverrides)
        }
        (SURFACE_SOFTWARE_ITEM_UPDATE_PROTECTION, ACTION_SAVE_ITEM_OVERRIDES) => {
            Some(ControllerSurfaceAction::SaveItemOverrides)
        }
        _ => None,
    }
}

/// Returns the surface action library for the Proxmox VE plugin.
///
/// All actions referenced by shared-surface interaction IDs must be defined
/// here.
pub fn surface_actions() -> Vec<SurfaceActionDescriptor> {
    vec![
        add_config_action(),
        match_action(),
        approve_match_action(),
        unmatch_action(),
        discover_action(),
        test_connection_action(),
        list_all_unmatched_action(),
        get_info_action(),
        preload_global_defaults_action(),
        save_global_defaults_action(),
        preload_item_overrides_action(),
        save_item_overrides_action(),
        load_backup_target_options_action(),
    ]
}

// ── Action definitions ──────────────────────────────────────────────────────

fn add_config_action() -> SurfaceActionDescriptor {
    SurfaceActionDescriptor::new("add-config", "Add Configuration")
        .with_permission(Permission::UpdateHosts)
        .with_ui(SurfaceActionUi::Form(SurfaceFormDescriptor::new(vec![
            FormFieldDescriptor::new("name", "Configuration Name")
                .required()
                .with_placeholder("My Proxmox Cluster"),
            FormFieldDescriptor::new("api_url", "Proxmox VE URL")
                .required()
                .with_placeholder("https://pve.example.com:8006")
                .with_help_text("HTTPS URL to your Proxmox VE API (port 8006 by default)."),
            FormFieldDescriptor::new("api_token", "API Token")
                .with_type(FormFieldType::Password)
                .required()
                .with_placeholder("user@realm!tokenid=secret")
                .with_help_text(
                    "PVE API token in USER@REALM!TOKENID=SECRET format. \
                     Required privileges: Sys.Audit (list nodes) and VM.Audit \
                     (list and read VM/CT config); the built-in PVEAuditor role \
                     covers both. VM.Monitor on /vms is optional and enables IP \
                     discovery via the QEMU guest agent. \
                     Without privilege separation: assign PVEAuditor on / to the user. \
                     With privilege separation: assign PVEAuditor on / to the token \
                     directly (Datacenter → Permissions → API Token Permissions, or \
                     pveum acl modify / --tokens USER@REALM!TOKENID --roles PVEAuditor).",
                ),
            FormFieldDescriptor::new("verify_tls", "Verify TLS Certificate")
                .with_type(FormFieldType::Toggle)
                .with_help_text("Disable if your Proxmox VE uses a self-signed certificate."),
            FormFieldDescriptor::new("node_filter", "Node Filter")
                .with_placeholder("pve1,pve2")
                .with_help_text(
                    "Comma-separated list of node names to include. Leave blank for all nodes.",
                ),
        ])))
        .with_api_submit(
            ApiSubmitDescriptor::new(
                "POST",
                "/api/v1/plugin-configs",
                serde_json::json!({
                    "name": "{{name}}",
                    "plugin_type": "infrastructure_proxmox",
                    "enabled": true,
                    "config": {
                        "api_url": "{{api_url}}",
                        "api_token": "{{api_token}}",
                        "verify_tls": "{{verify_tls:bool}}",
                        "node_filter": "{{node_filter:csv_array}}"
                    }
                }),
            )
            .with_response_id_field("id")
            .with_response_label_field("name"),
        )
}

fn match_action() -> SurfaceActionDescriptor {
    SurfaceActionDescriptor::new("match", "Manual Match")
        .with_ui(SurfaceActionUi::Form(SurfaceFormDescriptor::new(vec![
            FormFieldDescriptor::new("mapping_id", "Mapping ID")
                .with_type(FormFieldType::Hidden)
                .required(),
            FormFieldDescriptor::new("host_id", "Host")
                .with_type(FormFieldType::Select)
                .required()
                .with_placeholder("Select a host")
                .with_select_source(FormSelectSourceDescriptor::RestApi {
                    path: "/api/v1/hosts".to_string(),
                    value_field: "id".to_string(),
                    label_field: "friendly_name".to_string(),
                }),
        ])))
        .with_permission(Permission::UpdateHosts)
}

fn approve_match_action() -> SurfaceActionDescriptor {
    SurfaceActionDescriptor::new("approve-match", "Approve Match")
        .with_permission(Permission::UpdateHosts)
        .with_row_visible_when("suggested_host_id", SurfaceRowCondition::Present)
}

fn unmatch_action() -> SurfaceActionDescriptor {
    SurfaceActionDescriptor::new("unmatch", "Remove Match")
        .with_permission(Permission::UpdateHosts)
        .destructive()
        .with_confirm_entity_field("proxmox_name")
        .with_row_visible_when("matched_host", SurfaceRowCondition::Present)
}

fn discover_action() -> SurfaceActionDescriptor {
    SurfaceActionDescriptor::new("discover", "Discover")
        .with_permission(Permission::UpdateHosts)
        .with_timeout(120)
}

fn test_connection_action() -> SurfaceActionDescriptor {
    SurfaceActionDescriptor::new("test-connection", "Test Connection")
        .with_permission(Permission::UpdateHosts)
        .with_timeout(30)
}

fn list_all_unmatched_action() -> SurfaceActionDescriptor {
    SurfaceActionDescriptor::new("list-all-unmatched", "List All Unmatched Guests")
        .with_permission(Permission::UpdateHosts)
        .with_timeout(10)
}

fn get_info_action() -> SurfaceActionDescriptor {
    SurfaceActionDescriptor::new("get-info", "Get Info")
        .with_permission(Permission::UpdateHosts)
        .with_timeout(10)
}

fn preload_global_defaults_action() -> SurfaceActionDescriptor {
    SurfaceActionDescriptor::new(ACTION_PRELOAD_GLOBAL_DEFAULTS, "Preload Global Defaults")
        .with_permission(Permission::ManageGlobalSettings)
}

fn save_global_defaults_action() -> SurfaceActionDescriptor {
    SurfaceActionDescriptor::new(ACTION_SAVE_GLOBAL_DEFAULTS, "Save Global Defaults")
        .with_permission(Permission::ManageGlobalSettings)
}

fn preload_item_overrides_action() -> SurfaceActionDescriptor {
    SurfaceActionDescriptor::new(ACTION_PRELOAD_ITEM_OVERRIDES, "Preload Per-item Overrides")
        .with_permission(Permission::ViewSoftware)
}

fn save_item_overrides_action() -> SurfaceActionDescriptor {
    SurfaceActionDescriptor::new(ACTION_SAVE_ITEM_OVERRIDES, "Save Per-item Overrides")
        .with_permission(Permission::UpdateSoftware)
}

fn load_backup_target_options_action() -> SurfaceActionDescriptor {
    SurfaceActionDescriptor::new(
        ACTION_LOAD_BACKUP_TARGET_OPTIONS,
        "Load Backup Target Dropdown Options",
    )
    .with_permission(Permission::ViewSoftware)
}

/// Handle a surface action for the Proxmox plugin.
///
/// Dispatches based on `(surface_id, action_id)` to the appropriate handler.
///
/// The function item matches `SurfaceActionHandler` so it can be used directly
/// as a function pointer in `declare_plugin!`.
pub fn handle_surface_action<'a>(
    ctx: &'a SurfaceActionContext<'a>,
    surface_id: &'a str,
    action_id: &'a str,
    params: serde_json::Value,
) -> Pin<
    Box<
        dyn Future<Output = std::result::Result<serde_json::Value, SurfaceActionError>> + Send + 'a,
    >,
> {
    Box::pin(handle_action_inner(ctx, surface_id, action_id, params))
}

#[tracing::instrument(skip_all, fields(surface_id, action_id))]
async fn handle_action_inner(
    ctx: &SurfaceActionContext<'_>,
    surface_id: &str,
    action_id: &str,
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, SurfaceActionError> {
    tracing::debug!("dispatching Proxmox surface action");

    let store = require_proxmox_surface_store(ctx)?;

    let Some(route) = resolve_controller_surface_action(surface_id, action_id) else {
        return Err(SurfaceActionError::InvalidInput(format!(
            "unknown action '{action_id}' for surface '{surface_id}'"
        )));
    };

    let result = match route {
        ControllerSurfaceAction::ListHostMappings => store
            .list_host_mappings(parse_action_params::<ProxmoxHostMappingsRequest>(
                params, action_id,
            )?)
            .await
            .map_err(map_store_error),
        ControllerSurfaceAction::DiscoverHosts => store
            .discover_hosts(parse_action_params::<ProxmoxPluginConfigRequest>(
                params, action_id,
            )?)
            .await
            .map_err(map_store_error),
        ControllerSurfaceAction::TestConnection => store
            .test_connection(parse_action_params::<ProxmoxPluginConfigRequest>(
                params, action_id,
            )?)
            .await
            .map_err(map_store_error),
        ControllerSurfaceAction::MatchHost => store
            .match_host(parse_action_params::<ProxmoxManualMatchRequest>(
                params, action_id,
            )?)
            .await
            .map_err(map_store_error),
        ControllerSurfaceAction::ApproveMatch => store
            .approve_match(parse_approve_match_request(params)?)
            .await
            .map_err(map_store_error),
        ControllerSurfaceAction::UnmatchHost => store
            .unmatch_host(parse_action_params::<ProxmoxMappingRequest>(
                params, action_id,
            )?)
            .await
            .map_err(map_store_error),
        ControllerSurfaceAction::ListAllUnmatched => store
            .list_all_unmatched(parse_action_params::<ProxmoxUnmatchedGuestsRequest>(
                params, action_id,
            )?)
            .await
            .map_err(map_store_error),
        ControllerSurfaceAction::GetHostInfo => store
            .get_host_info(parse_action_params::<ProxmoxHostInfoRequest>(
                params, action_id,
            )?)
            .await
            .map_err(map_store_error),
        ControllerSurfaceAction::PreloadGlobalDefaults => store
            .preload_global_defaults(parse_action_params::<ProxmoxScopeSelectionRequest>(
                params, action_id,
            )?)
            .await
            .map_err(map_store_error),
        ControllerSurfaceAction::SaveGlobalDefaults => store
            .save_global_defaults(parse_action_params::<ProxmoxGlobalDefaultsSaveRequest>(
                params, action_id,
            )?)
            .await
            .map_err(map_store_error),
        ControllerSurfaceAction::LoadBackupTargetOptions => store
            .load_backup_target_options(parse_action_params::<ProxmoxScopeSelectionRequest>(
                params, action_id,
            )?)
            .await
            .map_err(map_store_error),
        ControllerSurfaceAction::PreloadItemOverrides => store
            .preload_item_overrides(parse_action_params::<ProxmoxItemOverridePreloadRequest>(
                params, action_id,
            )?)
            .await
            .map_err(map_store_error),
        ControllerSurfaceAction::SaveItemOverrides => store
            .save_item_overrides(parse_action_params::<ProxmoxItemOverrideSaveRequest>(
                params, action_id,
            )?)
            .await
            .map_err(map_store_error),
    };

    match &result {
        Ok(_) => tracing::debug!("Proxmox surface action succeeded"),
        Err(e) => tracing::warn!(error = %e, "Proxmox surface action failed"),
    }

    result
}

fn require_proxmox_surface_store<'a>(
    ctx: &'a SurfaceActionContext<'a>,
) -> std::result::Result<
    &'a dyn uptrakit_plugin_infrastructure_core::ProxmoxSurfaceStore,
    SurfaceActionError,
> {
    ctx.controller.proxmox_surface_store().ok_or_else(|| {
        SurfaceActionError::ControllerIntegration(
            "proxmox surface store is not available".to_string(),
        )
    })
}

/// Execute a Proxmox controller surface action using the canonical DB-backed handlers.
///
/// Kept for compatibility with string-based dispatch entry points.
pub async fn execute_controller_surface_action(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    surface_id: &str,
    action_id: &str,
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    execute_controller_surface_action_typed(db, tenant_id, surface_id, action_id, params)
        .await
        .map_err(|error| error.to_string())
}

async fn execute_controller_surface_action_typed(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    surface_id: &str,
    action_id: &str,
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, SurfaceActionError> {
    let Some(route) = resolve_controller_surface_action(surface_id, action_id) else {
        return Err(SurfaceActionError::InvalidInput(format!(
            "unknown action '{action_id}' for surface '{surface_id}'"
        )));
    };

    match route {
        ControllerSurfaceAction::ListHostMappings => execute_controller_list_host_mappings(
            db,
            tenant_id,
            parse_action_params::<ProxmoxHostMappingsRequest>(params, action_id)?,
        )
        .await
        .map_err(map_controller_action_error),
        ControllerSurfaceAction::DiscoverHosts => execute_controller_discover_hosts(
            db,
            tenant_id,
            parse_action_params::<ProxmoxPluginConfigRequest>(params, action_id)?,
        )
        .await
        .map_err(map_controller_action_error),
        ControllerSurfaceAction::TestConnection => execute_controller_test_connection(
            db,
            parse_action_params::<ProxmoxPluginConfigRequest>(params, action_id)?,
        )
        .await
        .map_err(map_controller_action_error),
        ControllerSurfaceAction::MatchHost => execute_controller_manual_match(
            db,
            parse_action_params::<ProxmoxManualMatchRequest>(params, action_id)?,
        )
        .await
        .map_err(map_controller_action_error),
        ControllerSurfaceAction::ApproveMatch => {
            execute_controller_approve_match(db, parse_approve_match_request(params)?)
                .await
                .map_err(map_controller_action_error)
        }
        ControllerSurfaceAction::UnmatchHost => execute_controller_unmatch_host(
            db,
            parse_action_params::<ProxmoxMappingRequest>(params, action_id)?,
        )
        .await
        .map_err(map_controller_action_error),
        ControllerSurfaceAction::ListAllUnmatched => execute_controller_list_all_unmatched(
            db,
            tenant_id,
            parse_action_params::<ProxmoxUnmatchedGuestsRequest>(params, action_id)?,
        )
        .await
        .map_err(map_controller_action_error),
        ControllerSurfaceAction::GetHostInfo => execute_controller_get_host_info(
            db,
            tenant_id,
            parse_action_params::<ProxmoxHostInfoRequest>(params, action_id)?,
        )
        .await
        .map_err(map_controller_action_error),
        ControllerSurfaceAction::PreloadGlobalDefaults => {
            execute_controller_preload_global_defaults(
                db,
                tenant_id,
                parse_action_params::<ProxmoxScopeSelectionRequest>(params, action_id)?,
            )
            .await
            .map_err(map_controller_action_error)
        }
        ControllerSurfaceAction::SaveGlobalDefaults => execute_controller_save_global_defaults(
            db,
            tenant_id,
            parse_action_params::<ProxmoxGlobalDefaultsSaveRequest>(params, action_id)?,
        )
        .await
        .map_err(map_controller_action_error),
        ControllerSurfaceAction::PreloadItemOverrides => execute_controller_preload_item_overrides(
            db,
            tenant_id,
            parse_action_params::<ProxmoxItemOverridePreloadRequest>(params, action_id)?,
        )
        .await
        .map_err(map_controller_action_error),
        ControllerSurfaceAction::SaveItemOverrides => execute_controller_save_item_overrides(
            db,
            tenant_id,
            parse_action_params::<ProxmoxItemOverrideSaveRequest>(params, action_id)?,
        )
        .await
        .map_err(map_controller_action_error),
        ControllerSurfaceAction::LoadBackupTargetOptions => {
            execute_controller_load_backup_target_options(
                db,
                tenant_id,
                parse_action_params::<ProxmoxScopeSelectionRequest>(params, action_id)?,
            )
            .await
            .map_err(map_controller_action_error)
        }
    }
}

pub async fn execute_controller_list_host_mappings(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    request: ProxmoxHostMappingsRequest,
) -> std::result::Result<serde_json::Value, String> {
    handle_list(db, tenant_id, request).await
}

pub async fn execute_controller_discover_hosts(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    request: ProxmoxPluginConfigRequest,
) -> std::result::Result<serde_json::Value, String> {
    handle_discover(db, tenant_id, request).await
}

pub async fn execute_controller_test_connection(
    db: &DatabaseConnection,
    request: ProxmoxPluginConfigRequest,
) -> std::result::Result<serde_json::Value, String> {
    handle_test_connection(db, request).await
}

pub async fn execute_controller_manual_match(
    db: &DatabaseConnection,
    request: ProxmoxManualMatchRequest,
) -> std::result::Result<serde_json::Value, String> {
    handle_match(db, request).await
}

pub async fn execute_controller_approve_match(
    db: &DatabaseConnection,
    request: ProxmoxApproveMatchRequest,
) -> std::result::Result<serde_json::Value, String> {
    handle_approve_match(db, request).await
}

pub async fn execute_controller_unmatch_host(
    db: &DatabaseConnection,
    request: ProxmoxMappingRequest,
) -> std::result::Result<serde_json::Value, String> {
    handle_unmatch(db, request).await
}

pub async fn execute_controller_list_all_unmatched(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    request: ProxmoxUnmatchedGuestsRequest,
) -> std::result::Result<serde_json::Value, String> {
    handle_list_all_unmatched(db, tenant_id, request).await
}

pub async fn execute_controller_get_host_info(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    request: ProxmoxHostInfoRequest,
) -> std::result::Result<serde_json::Value, String> {
    handle_get_info(db, tenant_id, request).await
}

pub async fn execute_controller_preload_global_defaults(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    request: ProxmoxScopeSelectionRequest,
) -> std::result::Result<serde_json::Value, String> {
    handle_preload_global_defaults(db, tenant_id, request).await
}

pub async fn execute_controller_save_global_defaults(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    request: ProxmoxGlobalDefaultsSaveRequest,
) -> std::result::Result<serde_json::Value, String> {
    handle_save_global_defaults(db, tenant_id, request).await
}

pub async fn execute_controller_preload_item_overrides(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    request: ProxmoxItemOverridePreloadRequest,
) -> std::result::Result<serde_json::Value, String> {
    handle_preload_item_overrides(db, tenant_id, request).await
}

pub async fn execute_controller_save_item_overrides(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    request: ProxmoxItemOverrideSaveRequest,
) -> std::result::Result<serde_json::Value, String> {
    handle_save_item_overrides(db, tenant_id, request).await
}

pub async fn execute_controller_load_backup_target_options(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    request: ProxmoxScopeSelectionRequest,
) -> std::result::Result<serde_json::Value, String> {
    handle_load_backup_target_options(db, tenant_id, request).await
}

fn parse_action_params<T>(
    params: serde_json::Value,
    action_id: &str,
) -> Result<T, SurfaceActionError>
where
    T: DeserializeOwned,
{
    serde_json::from_value(params).map_err(|error| {
        SurfaceActionError::InvalidInput(format!(
            "invalid params for action '{action_id}': {error}"
        ))
    })
}

fn parse_approve_match_request(
    params: serde_json::Value,
) -> Result<ProxmoxApproveMatchRequest, SurfaceActionError> {
    let mapping_id =
        parse_uuid_param(&params, "mapping_id").map_err(SurfaceActionError::InvalidInput)?;
    let host_id = parse_uuid_param_with_fallback(&params, "host_id", "suggested_host_id")
        .map_err(SurfaceActionError::InvalidInput)?;
    let match_method = params
        .get("match_method")
        .or_else(|| params.get("suggested_match_method"))
        .and_then(|value| value.as_str())
        .unwrap_or("suggested_hostname")
        .to_string();

    Ok(ProxmoxApproveMatchRequest {
        mapping_id,
        host_id,
        match_method,
    })
}

fn map_store_error(
    error: rootcause::Report<uptrakit_plugin_infrastructure_core::PluginError>,
) -> SurfaceActionError {
    SurfaceActionError::ControllerIntegration(error.to_string())
}

fn map_controller_action_error(error: String) -> SurfaceActionError {
    SurfaceActionError::ControllerIntegration(error)
}

/// List discovered Proxmox host mappings with pagination and inline match suggestions.
async fn handle_list(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    request: ProxmoxHostMappingsRequest,
) -> std::result::Result<serde_json::Value, String> {
    use uptrakit_shared_db::entity::{host, proxmox_host_mapping};

    let plugin_config_id = request.plugin_config_id;
    let page = request.page.unwrap_or(1).max(1);
    let per_page = request.per_page.unwrap_or(50).clamp(1, 1000);

    tracing::debug!(%plugin_config_id, %page, %per_page, "listing Proxmox host mappings");

    let mut base_query = proxmox_host_mapping::Entity::find()
        .filter(proxmox_host_mapping::Column::PluginConfigId.eq(plugin_config_id));

    if let Some(tid) = tenant_id {
        base_query = base_query.filter(proxmox_host_mapping::Column::TenantId.eq(tid));
    }

    let base_query = base_query
        .order_by(
            sea_orm::sea_query::Func::lower(sea_orm::sea_query::Expr::col(
                proxmox_host_mapping::Column::ProxmoxName,
            )),
            sea_orm::sea_query::Order::Asc,
        )
        .order_by_asc(proxmox_host_mapping::Column::ProxmoxVmid);

    let total = base_query
        .clone()
        .count(db)
        .await
        .map_err(|e| format!("database error counting mappings: {e}"))?;

    let offset = (page.saturating_sub(1)) * per_page;
    let mappings = base_query
        .offset(Some(offset))
        .limit(Some(per_page))
        .all(db)
        .await
        .map_err(|e| format!("database error: {e}"))?;

    let total_pages = if per_page == 0 {
        0
    } else {
        total.div_ceil(per_page)
    };

    // Collect IDs of already-matched hosts on this page for suggestion filtering
    let matched_host_ids: std::collections::HashSet<Uuid> =
        mappings.iter().filter_map(|m| m.host_id).collect();

    // Collect unmatched mappings on this page for suggestion computation
    let unmatched_mappings: Vec<&proxmox_host_mapping::Model> =
        mappings.iter().filter(|m| m.host_id.is_none()).collect();

    // Load active hosts for suggestions (only if there are unmatched mappings on this page)
    let suggestion_map = if !unmatched_mappings.is_empty() {
        if let Some(tid) = tenant_id {
            let all_hosts: Vec<host::Model> = host::Entity::find()
                .filter(host::Column::TenantId.eq(tid))
                .filter(host::Column::DeactivatedAt.is_null())
                .all(db)
                .await
                .map_err(|e| format!("database error loading hosts: {e}"))?;

            // Exclude hosts already matched to any mapping
            let available_hosts: Vec<host::Model> = all_hosts
                .into_iter()
                .filter(|h| !matched_host_ids.contains(&h.id))
                .collect();

            let unmatched_owned: Vec<proxmox_host_mapping::Model> =
                unmatched_mappings.into_iter().cloned().collect();

            let suggestions =
                crate::matching::compute_suggestions(&unmatched_owned, &available_hosts);
            crate::matching::suggestions_by_mapping_id(suggestions)
        } else {
            std::collections::HashMap::new()
        }
    } else {
        std::collections::HashMap::new()
    };

    let items: Vec<serde_json::Value> = mappings
        .into_iter()
        .map(|m| {
            let mapping_id = m.id;
            let mut row = serde_json::json!({
                "id": m.id.to_string(),
                "mapping_id": m.id.to_string(),
                "name": m.proxmox_name,
                "node": m.proxmox_node,
                "vmid": m.proxmox_vmid,
                "type": m.proxmox_type,
                "status": m.proxmox_status,
                "hostname": m.hostname,
                "ip_addresses": m.ip_addresses,
                "matched_host": m.host_id.map(|id| id.to_string()),
                "match_method": m.match_method,
            });

            // Add suggestion data if available
            if let Some(suggestion) = suggestion_map.get(&mapping_id) {
                row["suggested_host"] = serde_json::json!(suggestion.host_name);
                row["suggested_host_id"] = serde_json::json!(suggestion.host_id.to_string());
                row["match_confidence"] = serde_json::json!(suggestion.confidence.as_str());
                row["match_reason"] = serde_json::json!(suggestion.reason);
                row["suggested_match_method"] = serde_json::json!(suggestion.match_method.as_str());
            }

            row
        })
        .collect();

    tracing::debug!(%plugin_config_id, item_count = items.len(), %total, "host mappings listed");
    Ok(serde_json::json!({
        "items": items,
        "total": total,
        "page": page,
        "per_page": per_page,
        "total_pages": total_pages,
    }))
}

/// Trigger discovery for a Proxmox plugin configuration.
async fn handle_discover(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    request: ProxmoxPluginConfigRequest,
) -> std::result::Result<serde_json::Value, String> {
    let plugin_config_id = request.plugin_config_id;
    let tenant_id = tenant_id.ok_or_else(|| "tenant context required for discovery".to_string())?;

    tracing::info!(%plugin_config_id, %tenant_id, "starting Proxmox discovery action");

    let config = load_proxmox_config(db, plugin_config_id).await?;
    let client =
        ProxmoxClient::new(&config).map_err(|e| format!("failed to create client: {e}"))?;

    #[cfg(feature = "agent-infra")]
    {
        let _ = (db, tenant_id, plugin_config_id, &client, &config);
        Err("controller discovery is unavailable in agent-infra builds".to_string())
    }

    #[cfg(not(feature = "agent-infra"))]
    {
        let persisted = crate::discovery::discover_and_persist(
            db,
            tenant_id,
            plugin_config_id,
            &client,
            &config.node_filter,
        )
        .await
        .map_err(|e| format!("discovery failed: {e}"))?;

        tracing::info!(
            %plugin_config_id,
            guests_upserted = persisted.guests_upserted,
            backup_targets_upserted = persisted.backup_targets_upserted,
            "Proxmox discovery action complete"
        );

        Ok(serde_json::json!({
            "discovered": persisted.guests_upserted,
            "backup_targets_discovered": persisted.backup_targets_upserted,
        }))
    }
}

/// Test connectivity to the Proxmox VE API.
async fn handle_test_connection(
    db: &DatabaseConnection,
    request: ProxmoxPluginConfigRequest,
) -> std::result::Result<serde_json::Value, String> {
    let plugin_config_id = request.plugin_config_id;
    tracing::debug!(%plugin_config_id, "testing Proxmox VE connection");

    let config = load_proxmox_config(db, plugin_config_id).await?;
    let client =
        ProxmoxClient::new(&config).map_err(|e| format!("failed to create client: {e}"))?;

    let version = client
        .test_connection()
        .await
        .map_err(|e| format!("connection test failed: {e}"))?;

    tracing::info!(%plugin_config_id, "Proxmox connection test succeeded");

    Ok(serde_json::json!({
        "success": true,
        "version": version,
    }))
}

/// Manually match a mapping to a host.
async fn handle_match(
    db: &DatabaseConnection,
    request: ProxmoxManualMatchRequest,
) -> std::result::Result<serde_json::Value, String> {
    let mapping_id = request.mapping_id;
    let host_id = request.host_id;

    tracing::info!(%mapping_id, %host_id, "manually matching Proxmox guest to host");

    crate::matching::manual_match(db, mapping_id, host_id)
        .await
        .map_err(|e| format!("manual match failed: {e}"))?;

    Ok(serde_json::json!({ "success": true }))
}

/// Approve a suggested match.
async fn handle_approve_match(
    db: &DatabaseConnection,
    request: ProxmoxApproveMatchRequest,
) -> std::result::Result<serde_json::Value, String> {
    let mapping_id = request.mapping_id;
    let host_id = request.host_id;
    let match_method_str = request.match_method.as_str();

    let method: crate::matching::MatchMethod = match_method_str
        .parse()
        .map_err(|e| format!("invalid match method: {e}"))?;

    tracing::info!(
        %mapping_id,
        %host_id,
        method = match_method_str,
        "approving suggested Proxmox guest-to-host match"
    );

    crate::matching::apply_suggested_match(db, mapping_id, host_id, method)
        .await
        .map_err(|e| format!("approve match failed: {e}"))?;

    Ok(serde_json::json!({ "success": true }))
}

/// Remove a match from a mapping.
async fn handle_unmatch(
    db: &DatabaseConnection,
    request: ProxmoxMappingRequest,
) -> std::result::Result<serde_json::Value, String> {
    let mapping_id = request.mapping_id;

    tracing::info!(%mapping_id, "removing Proxmox guest-to-host match");

    crate::matching::unmatch(db, mapping_id)
        .await
        .map_err(|e| format!("unmatch failed: {e}"))?;

    Ok(serde_json::json!({ "success": true }))
}

/// Get Proxmox info for a specific host.
async fn handle_get_info(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    request: ProxmoxHostInfoRequest,
) -> std::result::Result<serde_json::Value, String> {
    use uptrakit_shared_db::entity::proxmox_host_mapping;

    let host_id = request.host_id;
    tracing::debug!(%host_id, "fetching Proxmox info for host");

    let mut query = proxmox_host_mapping::Entity::find()
        .filter(proxmox_host_mapping::Column::HostId.eq(host_id));

    if let Some(tid) = tenant_id {
        query = query.filter(proxmox_host_mapping::Column::TenantId.eq(tid));
    }

    let mapping = query
        .one(db)
        .await
        .map_err(|e| format!("database error: {e}"))?;

    match mapping {
        Some(m) => {
            tracing::debug!(
                %host_id,
                node = %m.proxmox_node,
                vmid = m.proxmox_vmid,
                "found Proxmox mapping for host"
            );
            Ok(serde_json::json!({
                "id": m.id.to_string(),
                "node": m.proxmox_node,
                "vmid": m.proxmox_vmid,
                "type": m.proxmox_type,
                "name": m.proxmox_name,
                "status": m.proxmox_status,
                "hostname": m.hostname,
                "ip_addresses": m.ip_addresses,
                "match_method": m.match_method,
            }))
        }
        None => {
            tracing::debug!(%host_id, "no Proxmox mapping found for host");
            Ok(serde_json::json!({ "linked": false }))
        }
    }
}

/// List all unmatched discovered guests across ALL Proxmox configs for a tenant.
///
/// Returns a paginated list of unmatched guests. Each item includes `value`/`label`
/// fields for use in surface action dropdowns (e.g., SSH agent's "Bootstrap via
/// Discovered Guest").
async fn handle_list_all_unmatched(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    request: ProxmoxUnmatchedGuestsRequest,
) -> std::result::Result<serde_json::Value, String> {
    use uptrakit_shared_db::entity::proxmox_host_mapping;

    let tenant_id = tenant_id.ok_or_else(|| "tenant context required".to_string())?;
    let page = request.page.unwrap_or(1).max(1);
    let per_page = request.per_page.unwrap_or(50).clamp(1, 1000);

    let base_query = proxmox_host_mapping::Entity::find()
        .filter(proxmox_host_mapping::Column::TenantId.eq(tenant_id))
        .filter(proxmox_host_mapping::Column::HostId.is_null())
        .order_by(
            sea_orm::sea_query::Func::lower(sea_orm::sea_query::Expr::col(
                proxmox_host_mapping::Column::ProxmoxName,
            )),
            sea_orm::sea_query::Order::Asc,
        )
        .order_by_asc(proxmox_host_mapping::Column::ProxmoxVmid);

    let total = base_query
        .clone()
        .count(db)
        .await
        .map_err(|e| format!("database error counting unmatched: {e}"))?;

    let offset = (page.saturating_sub(1)) * per_page;
    let mappings = base_query
        .offset(Some(offset))
        .limit(Some(per_page))
        .all(db)
        .await
        .map_err(|e| format!("database error: {e}"))?;

    let total_pages = if per_page == 0 {
        0
    } else {
        total.div_ceil(per_page)
    };

    let items: Vec<serde_json::Value> = mappings
        .into_iter()
        .map(|m| {
            let name = m.proxmox_name.as_deref().unwrap_or("unnamed");
            let label = format!(
                "{name} ({}/{} VMID {})",
                m.proxmox_node, m.proxmox_type, m.proxmox_vmid
            );
            serde_json::json!({
                "value": m.id.to_string(),
                "label": label,
                "mapping_id": m.id.to_string(),
                "plugin_config_id": m.plugin_config_id.to_string(),
                "proxmox_node": m.proxmox_node,
                "proxmox_vmid": m.proxmox_vmid,
                "proxmox_type": m.proxmox_type,
                "proxmox_name": m.proxmox_name,
                "hostname": m.hostname,
            })
        })
        .collect();

    tracing::debug!(
        %tenant_id,
        item_count = items.len(),
        %total,
        "listed all unmatched Proxmox guests"
    );

    Ok(serde_json::json!({
        "items": items,
        "total": total,
        "page": page,
        "per_page": per_page,
        "total_pages": total_pages,
    }))
}

#[derive(Debug, Clone)]
struct ProxmoxConfigOption {
    id: Uuid,
    name: String,
}

async fn handle_preload_global_defaults(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    request: ProxmoxScopeSelectionRequest,
) -> std::result::Result<serde_json::Value, String> {
    let tenant_id = require_tenant_id(tenant_id, "global defaults preload")?;
    let configs = resolve_scope_plugin_configs(db, tenant_id, &request).await?;

    let Some(selected_config) = configs.first() else {
        return Ok(json!({
            "plugin_config_id": "",
            "mode": ProtectionMode::DoNothing.as_str(),
            "backup_target_option": "",
        }));
    };

    let policy = load_global_default(db, tenant_id, selected_config.id)
        .await
        .map_err(|e| format!("failed to load global defaults: {e}"))?
        .unwrap_or_else(ProtectionPolicy::do_nothing);

    Ok(json!({
        "plugin_config_id": selected_config.id.to_string(),
        "mode": policy.mode.as_str(),
        "backup_target_option": policy
            .backup_target_key
            .as_deref()
            .map(|target_key| encode_backup_target_option(selected_config.id, target_key))
            .unwrap_or_default(),
    }))
}

async fn handle_save_global_defaults(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    request: ProxmoxGlobalDefaultsSaveRequest,
) -> std::result::Result<serde_json::Value, String> {
    let tenant_id = require_tenant_id(tenant_id, "global defaults save")?;
    let plugin_config_id = request.plugin_config_id;
    let mode_raw = normalize_required_mode(request.mode.as_str())?;
    let mode = parse_protection_mode(&mode_raw)?;

    ensure_proxmox_plugin_config_exists(db, tenant_id, plugin_config_id).await?;

    let backup_target_key = match mode {
        ProtectionMode::Backup => {
            let (selected_plugin_config_id, target_key) =
                parse_required_backup_target_option(request.backup_target_option.as_deref())?;
            if selected_plugin_config_id != plugin_config_id {
                return Err(
                    "selected backup target belongs to a different Proxmox configuration"
                        .to_string(),
                );
            }
            ensure_cached_backup_target_exists(db, plugin_config_id, &target_key).await?;
            Some(target_key)
        }
        ProtectionMode::DoNothing | ProtectionMode::Snapshot => None,
    };

    upsert_global_default(
        db,
        tenant_id,
        plugin_config_id,
        &ProtectionPolicy {
            mode,
            backup_target_key,
        },
    )
    .await
    .map_err(|e| format!("failed to save global defaults: {e}"))?;

    Ok(json!({
        "success": true,
        "plugin_config_id": plugin_config_id.to_string(),
    }))
}

async fn handle_preload_item_overrides(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    request: ProxmoxItemOverridePreloadRequest,
) -> std::result::Result<serde_json::Value, String> {
    let tenant_id = require_tenant_id(tenant_id, "item override preload")?;
    let software_item_id = request.software_item_id;
    let configs = resolve_scope_plugin_configs(
        db,
        tenant_id,
        &ProxmoxScopeSelectionRequest {
            plugin_config_id: request.plugin_config_id,
            software_item_id: Some(software_item_id),
        },
    )
    .await?;

    let Some(selected_config) = configs.first() else {
        return Ok(json!({
            "software_item_id": software_item_id.to_string(),
            "plugin_config_id": "",
            "mode": "inherit_global",
            "backup_target_option": "",
        }));
    };

    let item_override = load_item_override(db, software_item_id, selected_config.id)
        .await
        .map_err(|e| format!("failed to load per-item override: {e}"))?;

    let (mode, backup_target_option) = match item_override {
        Some(policy) => (
            policy.mode.as_str().to_string(),
            policy
                .backup_target_key
                .as_deref()
                .map(|target_key| encode_backup_target_option(selected_config.id, target_key))
                .unwrap_or_default(),
        ),
        None => ("inherit_global".to_string(), String::new()),
    };

    Ok(json!({
        "software_item_id": software_item_id.to_string(),
        "plugin_config_id": selected_config.id.to_string(),
        "mode": mode,
        "backup_target_option": backup_target_option,
    }))
}

async fn handle_save_item_overrides(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    request: ProxmoxItemOverrideSaveRequest,
) -> std::result::Result<serde_json::Value, String> {
    let tenant_id = require_tenant_id(tenant_id, "item override save")?;
    let software_item_id = request.software_item_id;
    let plugin_config_id = request.plugin_config_id;
    let mode_raw = normalize_required_mode(request.mode.as_str())?;

    ensure_proxmox_plugin_config_exists(db, tenant_id, plugin_config_id).await?;
    ensure_plugin_config_assigned_to_software_item(
        db,
        tenant_id,
        software_item_id,
        plugin_config_id,
    )
    .await?;

    if mode_raw == "inherit_global" {
        delete_item_override(db, software_item_id, plugin_config_id)
            .await
            .map_err(|e| format!("failed to clear per-item override: {e}"))?;
        return Ok(json!({
            "success": true,
            "cleared": true,
            "software_item_id": software_item_id.to_string(),
            "plugin_config_id": plugin_config_id.to_string(),
        }));
    }

    let mode = parse_protection_mode(&mode_raw)?;
    let backup_target_key = match mode {
        ProtectionMode::Backup => {
            let (selected_plugin_config_id, target_key) =
                parse_required_backup_target_option(request.backup_target_option.as_deref())?;
            if selected_plugin_config_id != plugin_config_id {
                return Err(
                    "selected backup target belongs to a different Proxmox configuration"
                        .to_string(),
                );
            }
            ensure_cached_backup_target_exists(db, plugin_config_id, &target_key).await?;
            Some(target_key)
        }
        ProtectionMode::DoNothing | ProtectionMode::Snapshot => None,
    };

    upsert_item_override(
        db,
        software_item_id,
        plugin_config_id,
        &ProtectionPolicy {
            mode,
            backup_target_key,
        },
    )
    .await
    .map_err(|e| format!("failed to save per-item override: {e}"))?;

    Ok(json!({
        "success": true,
        "cleared": false,
        "software_item_id": software_item_id.to_string(),
        "plugin_config_id": plugin_config_id.to_string(),
    }))
}

async fn handle_load_backup_target_options(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    request: ProxmoxScopeSelectionRequest,
) -> std::result::Result<serde_json::Value, String> {
    let tenant_id = require_tenant_id(tenant_id, "backup target options")?;
    let configs = resolve_scope_plugin_configs(db, tenant_id, &request).await?;

    if configs.is_empty() {
        return Ok(json!({
            "options": [],
            "state": "empty",
            "message": "No Proxmox configurations are available for this context.",
        }));
    }

    let mut options: Vec<(String, String)> = Vec::new();
    for config in configs {
        let targets = list_cached_backup_targets(db, config.id)
            .await
            .map_err(|e| format!("failed to list cached backup targets: {e}"))?;
        for target in targets {
            let value = encode_backup_target_option(config.id, &target.target_key);
            let label = format!(
                "{} • {} / {} ({})",
                config.name, target.node, target.storage_id, target.storage_type
            );
            options.push((value, label));
        }
    }

    options.sort_unstable_by(|left, right| left.1.cmp(&right.1));

    let is_empty = options.is_empty();
    let message = if is_empty {
        "No cached backup targets yet. Run Discover on Proxmox VE Hosts to populate this dropdown."
    } else {
        ""
    };

    Ok(json!({
        "options": options
            .into_iter()
            .map(|(value, label)| json!({ "value": value, "label": label }))
            .collect::<Vec<_>>(),
        "state": if is_empty { "empty" } else { "ready" },
        "message": message,
    }))
}

async fn ensure_proxmox_plugin_config_exists(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    plugin_config_id: Uuid,
) -> std::result::Result<(), String> {
    let config = uptrakit_shared_db::entity::plugin_config::Entity::find_by_id(plugin_config_id)
        .filter(uptrakit_shared_db::entity::plugin_config::Column::TenantId.eq(tenant_id))
        .filter(
            uptrakit_shared_db::entity::plugin_config::Column::PluginType
                .eq("infrastructure_proxmox"),
        )
        .filter(uptrakit_shared_db::entity::plugin_config::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .map_err(|e| format!("database error validating Proxmox config: {e}"))?;

    if config.is_none() {
        return Err(format!(
            "Proxmox plugin configuration '{plugin_config_id}' was not found in tenant scope"
        ));
    }
    Ok(())
}

async fn ensure_plugin_config_assigned_to_software_item(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    software_item_id: Uuid,
    plugin_config_id: Uuid,
) -> std::result::Result<(), String> {
    let scoped =
        list_proxmox_plugin_configs_for_software_item(db, tenant_id, software_item_id).await?;
    let assigned = scoped.iter().any(|config| config.id == plugin_config_id);
    if !assigned {
        return Err(format!(
            "Proxmox plugin configuration '{plugin_config_id}' is not assigned to software item '{software_item_id}'"
        ));
    }
    Ok(())
}

async fn ensure_cached_backup_target_exists(
    db: &DatabaseConnection,
    plugin_config_id: Uuid,
    target_key: &str,
) -> std::result::Result<(), String> {
    let exists = find_cached_backup_target(db, plugin_config_id, target_key)
        .await
        .map_err(|e| format!("failed to validate cached backup target: {e}"))?;
    if exists.is_none() {
        return Err(
            "selected backup target is not present in cache; run Discover to refresh storage targets"
                .to_string(),
        );
    }
    Ok(())
}

async fn resolve_scope_plugin_configs(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    request: &ProxmoxScopeSelectionRequest,
) -> std::result::Result<Vec<ProxmoxConfigOption>, String> {
    if let Some(plugin_config_id) = request.plugin_config_id {
        let selected =
            list_proxmox_plugin_configs_by_ids(db, tenant_id, &[plugin_config_id]).await?;
        return Ok(selected);
    }

    if let Some(software_item_id) = request.software_item_id {
        return list_proxmox_plugin_configs_for_software_item(db, tenant_id, software_item_id)
            .await;
    }

    list_all_proxmox_plugin_configs(db, tenant_id).await
}

async fn list_all_proxmox_plugin_configs(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> std::result::Result<Vec<ProxmoxConfigOption>, String> {
    use uptrakit_shared_db::entity::plugin_config;

    let rows = plugin_config::Entity::find()
        .filter(plugin_config::Column::TenantId.eq(tenant_id))
        .filter(plugin_config::Column::PluginType.eq("infrastructure_proxmox"))
        .filter(plugin_config::Column::DeactivatedAt.is_null())
        .order_by(
            sea_orm::sea_query::Func::lower(sea_orm::sea_query::Expr::col(
                plugin_config::Column::Name,
            )),
            sea_orm::sea_query::Order::Asc,
        )
        .all(db)
        .await
        .map_err(|e| format!("database error loading Proxmox plugin configs: {e}"))?;

    Ok(rows
        .into_iter()
        .map(|row| ProxmoxConfigOption {
            id: row.id,
            name: row.name,
        })
        .collect())
}

async fn list_proxmox_plugin_configs_by_ids(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    ids: &[Uuid],
) -> std::result::Result<Vec<ProxmoxConfigOption>, String> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    use uptrakit_shared_db::entity::plugin_config;
    let rows = plugin_config::Entity::find()
        .filter(plugin_config::Column::TenantId.eq(tenant_id))
        .filter(plugin_config::Column::PluginType.eq("infrastructure_proxmox"))
        .filter(plugin_config::Column::DeactivatedAt.is_null())
        .filter(plugin_config::Column::Id.is_in(ids.iter().copied()))
        .order_by(
            sea_orm::sea_query::Func::lower(sea_orm::sea_query::Expr::col(
                plugin_config::Column::Name,
            )),
            sea_orm::sea_query::Order::Asc,
        )
        .all(db)
        .await
        .map_err(|e| format!("database error loading scoped Proxmox plugin configs: {e}"))?;

    Ok(rows
        .into_iter()
        .map(|row| ProxmoxConfigOption {
            id: row.id,
            name: row.name,
        })
        .collect())
}

async fn list_proxmox_plugin_configs_for_software_item(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    software_item_id: Uuid,
) -> std::result::Result<Vec<ProxmoxConfigOption>, String> {
    use uptrakit_shared_db::entity::host_software_item_plugin;

    let assignments = host_software_item_plugin::Entity::find()
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(software_item_id))
        .filter(host_software_item_plugin::Column::PluginType.eq("infrastructure_proxmox"))
        .all(db)
        .await
        .map_err(|e| format!("database error loading software-item plugin assignments: {e}"))?;

    let mut ids: Vec<Uuid> = assignments
        .into_iter()
        .filter_map(|model| model.plugin_config_id)
        .collect();
    ids.sort_unstable();
    ids.dedup();

    list_proxmox_plugin_configs_by_ids(db, tenant_id, &ids).await
}

fn encode_backup_target_option(plugin_config_id: Uuid, target_key: &str) -> String {
    format!("{plugin_config_id}|{target_key}")
}

fn decode_backup_target_option(value: &str) -> std::result::Result<(Uuid, String), String> {
    let (plugin_config_raw, target_key_raw) = value
        .split_once('|')
        .ok_or_else(|| "invalid backup target selection format".to_string())?;
    let plugin_config_id = Uuid::parse_str(plugin_config_raw)
        .map_err(|e| format!("invalid plugin config in backup target selection: {e}"))?;
    let target_key = target_key_raw.trim();
    if target_key.is_empty() {
        return Err("backup target selection is missing target key".to_string());
    }
    Ok((plugin_config_id, target_key.to_string()))
}

fn parse_required_backup_target_option(
    value: Option<&str>,
) -> std::result::Result<(Uuid, String), String> {
    let raw = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "missing required parameter 'backup_target_option'".to_string())?;
    decode_backup_target_option(raw)
}

fn normalize_required_mode(value: &str) -> std::result::Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("missing required parameter 'mode'".to_string());
    }
    Ok(trimmed.to_string())
}

fn parse_protection_mode(value: &str) -> std::result::Result<ProtectionMode, String> {
    match value {
        "do_nothing" => Ok(ProtectionMode::DoNothing),
        "snapshot" => Ok(ProtectionMode::Snapshot),
        "backup" => Ok(ProtectionMode::Backup),
        _ => Err(format!("invalid protection mode '{value}'")),
    }
}

fn require_tenant_id(
    tenant_id: Option<Uuid>,
    action_context: &str,
) -> std::result::Result<Uuid, String> {
    tenant_id.ok_or_else(|| format!("tenant context required for {action_context}"))
}

/// Load `ProxmoxConfig` from the `plugin_configs` table.
async fn load_proxmox_config(
    db: &DatabaseConnection,
    plugin_config_id: Uuid,
) -> std::result::Result<ProxmoxConfig, String> {
    use uptrakit_shared_db::entity::plugin_config;

    tracing::trace!(%plugin_config_id, "loading Proxmox plugin config from DB");

    let pc = plugin_config::Entity::find_by_id(plugin_config_id)
        .one(db)
        .await
        .map_err(|e| format!("database error: {e}"))?
        .ok_or_else(|| format!("plugin config {plugin_config_id} not found"))?;

    if pc.plugin_type != "infrastructure_proxmox" {
        return Err(format!(
            "plugin config {plugin_config_id} is type '{}', expected 'infrastructure_proxmox'",
            pc.plugin_type
        ));
    }

    serde_json::from_value::<ProxmoxConfig>(pc.config)
        .map_err(|e| format!("failed to parse Proxmox config: {e}"))
}

/// Parse a UUID parameter from JSON params.
fn parse_uuid_param(params: &serde_json::Value, key: &str) -> std::result::Result<Uuid, String> {
    let val = params
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("missing required parameter '{key}'"))?;

    Uuid::parse_str(val).map_err(|e| format!("invalid UUID for '{key}': {e}"))
}

/// Parse a UUID parameter with a fallback key.
///
/// Tries `primary_key` first, then `fallback_key`. This allows row actions
/// to work when the frontend passes row data where the field has a different
/// name than what the handler originally expected (e.g., `suggested_host_id`
/// instead of `host_id`).
fn parse_uuid_param_with_fallback(
    params: &serde_json::Value,
    primary_key: &str,
    fallback_key: &str,
) -> std::result::Result<Uuid, String> {
    let val = params
        .get(primary_key)
        .and_then(|v| v.as_str())
        .or_else(|| params.get(fallback_key).and_then(|v| v.as_str()))
        .ok_or_else(|| format!("missing required parameter '{primary_key}'"))?;

    Uuid::parse_str(val).map_err(|e| format!("invalid UUID for '{primary_key}': {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{DatabaseConnection, DbBackend, MockDatabase};
    use time::OffsetDateTime;

    #[test]
    fn surface_actions_include_host_and_policy_actions_with_permissions() {
        let actions = surface_actions();
        assert_eq!(actions.len(), 13);
        let ids: Vec<&str> = actions.iter().map(|a| a.action_id.as_str()).collect();
        assert!(ids.contains(&"add-config"));
        assert!(ids.contains(&"match"));
        assert!(ids.contains(&"approve-match"));
        assert!(ids.contains(&"unmatch"));
        assert!(ids.contains(&"discover"));
        assert!(ids.contains(&"test-connection"));
        assert!(ids.contains(&"list-all-unmatched"));
        assert!(ids.contains(&"get-info"));
        assert!(ids.contains(&ACTION_PRELOAD_GLOBAL_DEFAULTS));
        assert!(ids.contains(&ACTION_SAVE_GLOBAL_DEFAULTS));
        assert!(ids.contains(&ACTION_PRELOAD_ITEM_OVERRIDES));
        assert!(ids.contains(&ACTION_SAVE_ITEM_OVERRIDES));
        assert!(ids.contains(&ACTION_LOAD_BACKUP_TARGET_OPTIONS));

        let get_info = actions
            .iter()
            .find(|action| action.action_id == "get-info")
            .expect("get-info action must be exported");
        assert_eq!(get_info.permission, "update_hosts");

        let save_global = actions
            .iter()
            .find(|action| action.action_id == ACTION_SAVE_GLOBAL_DEFAULTS)
            .expect("save-global-defaults action must be exported");
        assert_eq!(
            save_global.permission,
            Permission::ManageGlobalSettings.as_str()
        );

        let save_item = actions
            .iter()
            .find(|action| action.action_id == ACTION_SAVE_ITEM_OVERRIDES)
            .expect("save-item-overrides action must be exported");
        assert_eq!(save_item.permission, Permission::UpdateSoftware.as_str());
    }

    #[test]
    fn parse_uuid_param_valid() {
        let params = serde_json::json!({"id": "01944c3c-6a3a-7000-8000-000000000001"});
        assert!(parse_uuid_param(&params, "id").is_ok());
    }

    #[test]
    fn parse_uuid_param_missing() {
        let params = serde_json::json!({});
        assert!(parse_uuid_param(&params, "id").is_err());
    }

    #[test]
    fn parse_uuid_param_invalid() {
        let params = serde_json::json!({"id": "not-a-uuid"});
        assert!(parse_uuid_param(&params, "id").is_err());
    }

    #[test]
    fn parse_uuid_param_with_fallback_primary() {
        let params = serde_json::json!({"host_id": "01944c3c-6a3a-7000-8000-000000000001"});
        assert!(parse_uuid_param_with_fallback(&params, "host_id", "suggested_host_id").is_ok());
    }

    #[test]
    fn parse_uuid_param_with_fallback_uses_fallback() {
        let params =
            serde_json::json!({"suggested_host_id": "01944c3c-6a3a-7000-8000-000000000001"});
        let result = parse_uuid_param_with_fallback(&params, "host_id", "suggested_host_id");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_uuid_param_with_fallback_both_missing() {
        let params = serde_json::json!({});
        assert!(parse_uuid_param_with_fallback(&params, "host_id", "suggested_host_id").is_err());
    }

    #[test]
    fn approve_match_action_has_row_visibility() {
        let action = approve_match_action();
        let rvw = action
            .row_visible_when
            .expect("approve-match should have row_visible_when");
        assert_eq!(rvw.field, "suggested_host_id");
        assert_eq!(rvw.condition, SurfaceRowCondition::Present);
    }

    #[test]
    fn unmatch_action_has_row_visibility() {
        let action = unmatch_action();
        let rvw = action
            .row_visible_when
            .expect("unmatch should have row_visible_when");
        assert_eq!(rvw.field, "matched_host");
        assert_eq!(rvw.condition, SurfaceRowCondition::Present);
    }

    #[test]
    fn backup_target_option_roundtrip() {
        let plugin_config_id =
            Uuid::parse_str("01944c3c-6a3a-7000-8000-000000000001").expect("valid test uuid");
        let encoded = encode_backup_target_option(plugin_config_id, "pve1:local-zfs:zfspool");
        let decoded = decode_backup_target_option(&encoded).expect("decode should succeed");
        assert_eq!(decoded.0, plugin_config_id);
        assert_eq!(decoded.1, "pve1:local-zfs:zfspool");
    }

    #[test]
    fn backup_target_option_decode_rejects_invalid_shape() {
        let err = decode_backup_target_option("invalid").expect_err("expected parse error");
        assert!(err.contains("invalid backup target selection format"));
    }

    #[test]
    fn parse_required_backup_target_option_rejects_missing_or_blank() {
        assert!(parse_required_backup_target_option(None).is_err());
        assert!(parse_required_backup_target_option(Some("   ")).is_err());
    }

    #[test]
    fn parse_required_backup_target_option_accepts_valid_value() {
        let plugin_config_id =
            Uuid::parse_str("01944c3c-6a3a-7000-8000-000000000001").expect("valid test uuid");
        let encoded = encode_backup_target_option(plugin_config_id, "node:store:key");
        let (parsed_id, parsed_key) = parse_required_backup_target_option(Some(&encoded))
            .expect("backup target option should parse");
        assert_eq!(parsed_id, plugin_config_id);
        assert_eq!(parsed_key, "node:store:key");
    }

    #[test]
    fn parse_protection_mode_accepts_known_values() {
        assert_eq!(
            parse_protection_mode("do_nothing").expect("mode should parse"),
            ProtectionMode::DoNothing
        );
        assert_eq!(
            parse_protection_mode("snapshot").expect("mode should parse"),
            ProtectionMode::Snapshot
        );
        assert_eq!(
            parse_protection_mode("backup").expect("mode should parse"),
            ProtectionMode::Backup
        );
    }

    #[test]
    fn parse_protection_mode_rejects_unknown_value() {
        let err = parse_protection_mode("inherit_global").expect_err("expected parse error");
        assert!(err.contains("invalid protection mode"));
    }

    #[tokio::test]
    async fn save_item_overrides_rejects_unassigned_plugin_config_for_software_item() {
        let tenant_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        let requested_plugin_config_id = Uuid::now_v7();
        let assigned_plugin_config_id = Uuid::now_v7();

        let db = MockDatabase::new(DbBackend::MySql)
            // ensure_proxmox_plugin_config_exists(requested)
            .append_query_results([vec![mock_plugin_config_model(
                tenant_id,
                requested_plugin_config_id,
            )]])
            // list_proxmox_plugin_configs_for_software_item -> host_software_item_plugins
            .append_query_results([vec![mock_host_software_item_plugin_model(
                software_item_id,
                assigned_plugin_config_id,
            )]])
            // list_proxmox_plugin_configs_by_ids(assigned)
            .append_query_results([vec![mock_plugin_config_model(
                tenant_id,
                assigned_plugin_config_id,
            )]])
            .into_connection();

        let result = handle_save_item_overrides(
            &db,
            Some(tenant_id),
            ProxmoxItemOverrideSaveRequest {
                software_item_id,
                plugin_config_id: requested_plugin_config_id,
                mode: "inherit_global".to_string(),
                backup_target_option: None,
            },
        )
        .await;

        let err = result.expect_err("save should reject unrelated plugin config");
        assert!(err.contains("is not assigned to software item"));
    }

    #[tokio::test]
    async fn preload_item_overrides_does_not_fallback_to_all_configs_when_item_has_no_proxmox_assignment()
     {
        let tenant_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();

        let db = MockDatabase::new(DbBackend::MySql)
            // list_proxmox_plugin_configs_for_software_item -> no assignments
            .append_query_results([Vec::<
                uptrakit_shared_db::entity::host_software_item_plugin::Model,
            >::new()])
            .into_connection();

        let result = handle_preload_item_overrides(
            &db,
            Some(tenant_id),
            ProxmoxItemOverridePreloadRequest {
                software_item_id,
                plugin_config_id: None,
            },
        )
        .await
        .expect("preload should succeed");

        assert_eq!(result["software_item_id"], software_item_id.to_string());
        assert_eq!(result["plugin_config_id"], "");
        assert_eq!(result["mode"], "inherit_global");
        assert_eq!(result["backup_target_option"], "");
    }

    #[tokio::test]
    async fn save_global_defaults_rejects_deactivated_plugin_config() {
        let tenant_id = Uuid::now_v7();
        let plugin_config_id = Uuid::now_v7();
        let db = mock_empty_plugin_config_validation_db();

        let result = handle_save_global_defaults(
            &db,
            Some(tenant_id),
            ProxmoxGlobalDefaultsSaveRequest {
                plugin_config_id,
                mode: "do_nothing".to_string(),
                backup_target_option: None,
            },
        )
        .await;

        let err = result.expect_err("save should reject deactivated config");
        assert!(err.contains("not found in tenant scope"));
    }

    #[tokio::test]
    async fn save_item_overrides_rejects_deactivated_plugin_config() {
        let tenant_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        let plugin_config_id = Uuid::now_v7();
        let db = mock_empty_plugin_config_validation_db();

        let result = handle_save_item_overrides(
            &db,
            Some(tenant_id),
            ProxmoxItemOverrideSaveRequest {
                software_item_id,
                plugin_config_id,
                mode: "inherit_global".to_string(),
                backup_target_option: None,
            },
        )
        .await;

        let err = result.expect_err("save should reject deactivated config");
        assert!(err.contains("not found in tenant scope"));
    }

    fn mock_empty_plugin_config_validation_db() -> DatabaseConnection {
        MockDatabase::new(DbBackend::MySql)
            .append_query_results([Vec::<uptrakit_shared_db::entity::plugin_config::Model>::new()])
            .into_connection()
    }

    fn mock_plugin_config_model(
        tenant_id: Uuid,
        plugin_config_id: Uuid,
    ) -> uptrakit_shared_db::entity::plugin_config::Model {
        uptrakit_shared_db::entity::plugin_config::Model {
            id: plugin_config_id,
            tenant_id,
            name: "PVE Main".to_string(),
            plugin_type: "infrastructure_proxmox".to_string(),
            config: serde_json::json!({}),
            enabled: true,
            created_at: OffsetDateTime::now_utc(),
            updated_at: OffsetDateTime::now_utc(),
            deactivated_at: None,
        }
    }

    fn mock_host_software_item_plugin_model(
        software_item_id: Uuid,
        plugin_config_id: Uuid,
    ) -> uptrakit_shared_db::entity::host_software_item_plugin::Model {
        let now = OffsetDateTime::now_utc();
        uptrakit_shared_db::entity::host_software_item_plugin::Model {
            id: Uuid::now_v7(),
            host_id: Uuid::now_v7(),
            software_item_id,
            host_software_item_id: Uuid::now_v7(),
            plugin_config_id: Some(plugin_config_id),
            plugin_type: "infrastructure_proxmox".to_string(),
            role: "execute_update".to_string(),
            ordinal: 0,
            package_identifier: "pkg".to_string(),
            config: None,
            execution_site: "auto".to_string(),
            created_at: now,
            updated_at: now,
        }
    }
}
