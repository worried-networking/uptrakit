//! Extension manifests and action handler dispatch for the Proxmox VE plugin.

use std::future::Future;
use std::pin::Pin;

use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use uuid::Uuid;

use uptrakit_plugin_infrastructure_core::{
    ActionDef, ActionUi, ApiSubmitDef, ContextSelectorDef, ContextSelectorSource,
    ExtensionManifest, ExtensionPlacement, ExtensionUi, FieldDef, FieldType, FormDef,
    PanelPosition, RowCondition, SelectSource, TableColumn,
};
use uptrakit_shared_types::Permission;

use crate::client::ProxmoxClient;
use crate::config::ProxmoxConfig;

/// Returns the extension manifests registered by the Proxmox VE plugin.
pub fn extension_manifests() -> Vec<ExtensionManifest> {
    vec![hosts_page_manifest(), host_info_panel_manifest()]
}

/// Returns the action library for the Proxmox VE plugin.
///
/// All actions referenced by `action_id` strings in the extension manifests
/// must be defined here.
pub fn extension_actions() -> Vec<ActionDef> {
    vec![
        add_config_action(),
        match_action(),
        approve_match_action(),
        unmatch_action(),
        discover_action(),
        test_connection_action(),
        list_all_unmatched_action(),
        get_info_action(),
    ]
}

// ── Action definitions ──────────────────────────────────────────────────────

fn add_config_action() -> ActionDef {
    ActionDef::new("add-config", "Add Configuration")
        .with_permission(Permission::UpdateHosts)
        .with_ui(ActionUi::Form(FormDef::new(vec![
            FieldDef::new("name", "Configuration Name")
                .required()
                .with_placeholder("My Proxmox Cluster"),
            FieldDef::new("api_url", "Proxmox VE URL")
                .required()
                .with_placeholder("https://pve.example.com:8006")
                .with_help_text("HTTPS URL to your Proxmox VE API (port 8006 by default)."),
            FieldDef::new("api_token", "API Token")
                .with_type(FieldType::Password)
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
            FieldDef::new("verify_tls", "Verify TLS Certificate")
                .with_type(FieldType::Toggle)
                .with_help_text("Disable if your Proxmox VE uses a self-signed certificate."),
            FieldDef::new("node_filter", "Node Filter")
                .with_placeholder("pve1,pve2")
                .with_help_text(
                    "Comma-separated list of node names to include. Leave blank for all nodes.",
                ),
        ])))
        .with_api_submit(
            ApiSubmitDef::new(
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

fn match_action() -> ActionDef {
    ActionDef::new("match", "Manual Match")
        .with_ui(ActionUi::Form(FormDef::new(vec![
            FieldDef::new("mapping_id", "Mapping ID")
                .with_type(FieldType::Hidden)
                .required(),
            FieldDef::new("host_id", "Host")
                .with_type(FieldType::Select)
                .required()
                .with_placeholder("Select a host")
                .with_select_source(SelectSource::RestApi {
                    path: "/api/v1/hosts".to_string(),
                    value_field: "id".to_string(),
                    label_field: "friendly_name".to_string(),
                }),
        ])))
        .with_permission(Permission::UpdateHosts)
}

fn approve_match_action() -> ActionDef {
    ActionDef::new("approve-match", "Approve Match")
        .with_permission(Permission::UpdateHosts)
        .with_row_visible_when("suggested_host_id", RowCondition::Present)
}

fn unmatch_action() -> ActionDef {
    ActionDef::new("unmatch", "Remove Match")
        .with_permission(Permission::UpdateHosts)
        .destructive()
        .with_confirm_entity_field("proxmox_name")
        .with_row_visible_when("matched_host", RowCondition::Present)
}

fn discover_action() -> ActionDef {
    ActionDef::new("discover", "Discover")
        .with_permission(Permission::UpdateHosts)
        .with_timeout(120)
}

fn test_connection_action() -> ActionDef {
    ActionDef::new("test-connection", "Test Connection")
        .with_permission(Permission::UpdateHosts)
        .with_timeout(30)
}

fn list_all_unmatched_action() -> ActionDef {
    ActionDef::new("list-all-unmatched", "List All Unmatched Guests")
        .with_permission(Permission::UpdateHosts)
        .with_timeout(10)
}

fn get_info_action() -> ActionDef {
    ActionDef::new("get-info", "Get Info")
        .with_permission(Permission::UpdateHosts)
        .with_timeout(10)
}

// ── Manifest definitions ────────────────────────────────────────────────────

/// Full-page extension: Proxmox Hosts data table.
fn hosts_page_manifest() -> ExtensionManifest {
    ExtensionManifest::new(
        "proxmox.hosts",
        "Proxmox VE Hosts",
        650,
        ExtensionPlacement::Page {
            nav_section: "infrastructure".to_string(),
            icon: Some("server".to_string()),
        },
        ExtensionUi::DataTable {
            columns: vec![
                TableColumn::new("name", "Name").sortable(),
                TableColumn::new("node", "Node").sortable(),
                TableColumn::new("vmid", "VMID").sortable(),
                TableColumn::new("type", "Type").sortable(),
                TableColumn::new("status", "Status").sortable(),
                TableColumn::new("matched_host", "Matched Host"),
                TableColumn::new("match_method", "Match Method"),
                TableColumn::new("suggested_host", "Suggested Match"),
                TableColumn::new("match_confidence", "Confidence"),
            ],
            data_action: "list".to_string(),
            row_actions: vec![
                "match".to_string(),
                "approve-match".to_string(),
                "unmatch".to_string(),
            ],
            primary_actions: vec!["discover".to_string(), "test-connection".to_string()],
            default_per_page: Some(50),
            context_selector: Some(Box::new(
                ContextSelectorDef::new(
                    "plugin_config_id",
                    "Configuration",
                    ContextSelectorSource::PluginConfigs {
                        plugin_type: "infrastructure_proxmox".to_string(),
                    },
                )
                .with_add_action("add-config")
                .with_empty_message("No Proxmox VE configurations found. Add one to get started."),
            )),
        },
    )
    .with_permission(Permission::UpdateHosts)
}

/// Panel extension: Proxmox host info on host detail page.
fn host_info_panel_manifest() -> ExtensionManifest {
    ExtensionManifest::new(
        "proxmox.host-info",
        "Proxmox VE Info",
        0,
        ExtensionPlacement::Panel {
            target_page: "host-detail".to_string(),
            position: PanelPosition::default(),
            tab_group: None,
        },
        ExtensionUi::KeyValue {
            data_action: "get-info".to_string(),
        },
    )
    .with_permission(Permission::UpdateHosts)
}

/// Handle an extension action for the Proxmox plugin.
///
/// Dispatches based on `(extension_id, action_id)` to the appropriate handler.
///
/// The `ctx.db` field is `&dyn Any` and is downcast to `&DatabaseConnection`
/// at the start of this function. The return type matches
/// `ExtensionActionHandler` so it can be used directly as a function pointer
/// in `declare_plugin!`.
pub fn handle_action<'a>(
    ctx: &'a uptrakit_plugin_infrastructure_core::ExtensionActionContext<'a>,
    extension_id: &'a str,
    action_id: &'a str,
    params: serde_json::Value,
) -> Pin<Box<dyn Future<Output = std::result::Result<serde_json::Value, String>> + Send + 'a>> {
    Box::pin(handle_action_inner(ctx, extension_id, action_id, params))
}

#[tracing::instrument(skip_all, fields(extension_id, action_id))]
async fn handle_action_inner(
    ctx: &uptrakit_plugin_infrastructure_core::ExtensionActionContext<'_>,
    extension_id: &str,
    action_id: &str,
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    tracing::debug!("dispatching Proxmox extension action");

    let db: &DatabaseConnection = ctx
        .db
        .downcast_ref::<DatabaseConnection>()
        .ok_or_else(|| "internal error: expected DatabaseConnection".to_string())?;
    let tenant_id = ctx.tenant_id;

    let result = match (extension_id, action_id) {
        ("proxmox.hosts", "list") => handle_list(db, tenant_id, params).await,
        ("proxmox.hosts", "discover") => handle_discover(db, tenant_id, params).await,
        ("proxmox.hosts", "test-connection") => handle_test_connection(db, params).await,
        ("proxmox.hosts", "match") => handle_match(db, params).await,
        ("proxmox.hosts", "approve-match") => handle_approve_match(db, params).await,
        ("proxmox.hosts", "unmatch") => handle_unmatch(db, params).await,
        ("proxmox.hosts", "list-all-unmatched") => {
            handle_list_all_unmatched(db, tenant_id, params).await
        }
        ("proxmox.host-info", "get-info") => handle_get_info(db, tenant_id, params).await,
        _ => Err(format!(
            "unknown action '{action_id}' for extension '{extension_id}'"
        )),
    };

    match &result {
        Ok(_) => tracing::debug!("Proxmox extension action succeeded"),
        Err(e) => tracing::warn!(error = %e, "Proxmox extension action failed"),
    }

    result
}

/// List discovered Proxmox host mappings with pagination and inline match suggestions.
async fn handle_list(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    use uptrakit_shared_db::entity::{host, proxmox_host_mapping};

    let plugin_config_id = parse_uuid_param(&params, "plugin_config_id")?;
    let page = parse_pagination_page(&params);
    let per_page = parse_pagination_per_page(&params);

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
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    let plugin_config_id = parse_uuid_param(&params, "plugin_config_id")?;
    let tenant_id = tenant_id.ok_or_else(|| "tenant context required for discovery".to_string())?;

    tracing::info!(%plugin_config_id, %tenant_id, "starting Proxmox discovery action");

    let config = load_proxmox_config(db, plugin_config_id).await?;
    let client =
        ProxmoxClient::new(&config).map_err(|e| format!("failed to create client: {e}"))?;

    let guests = crate::discovery::discover_guests(&client, &config.node_filter)
        .await
        .map_err(|e| format!("discovery failed: {e}"))?;

    tracing::debug!(%plugin_config_id, guest_count = guests.len(), "persisting discovered guests");

    let persisted =
        crate::discovery::persist_discovered_guests(db, tenant_id, plugin_config_id, &guests)
            .await
            .map_err(|e| format!("failed to persist discovered guests: {e}"))?;

    tracing::info!(%plugin_config_id, upserted = persisted, "Proxmox discovery action complete");

    Ok(serde_json::json!({
        "discovered": persisted,
    }))
}

/// Test connectivity to the Proxmox VE API.
async fn handle_test_connection(
    db: &DatabaseConnection,
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    let plugin_config_id = parse_uuid_param(&params, "plugin_config_id")?;
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
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    let mapping_id = parse_uuid_param(&params, "mapping_id")?;
    let host_id = parse_uuid_param(&params, "host_id")?;

    tracing::info!(%mapping_id, %host_id, "manually matching Proxmox guest to host");

    crate::matching::manual_match(db, mapping_id, host_id)
        .await
        .map_err(|e| format!("manual match failed: {e}"))?;

    Ok(serde_json::json!({ "success": true }))
}

/// Approve a suggested match.
async fn handle_approve_match(
    db: &DatabaseConnection,
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    let mapping_id = parse_uuid_param(&params, "mapping_id")?;
    // The frontend passes row data as params; the host ID lives in
    // `suggested_host_id` (from the suggestion) rather than `host_id`.
    let host_id = parse_uuid_param_with_fallback(&params, "host_id", "suggested_host_id")?;
    let match_method_str = params
        .get("match_method")
        .or_else(|| params.get("suggested_match_method"))
        .and_then(|v| v.as_str())
        .unwrap_or("suggested_hostname");

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
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    let mapping_id = parse_uuid_param(&params, "mapping_id")?;

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
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    use uptrakit_shared_db::entity::proxmox_host_mapping;

    let host_id = parse_uuid_param(&params, "host_id")?;
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
/// fields for use in extension action dropdowns (e.g., SSH agent's "Bootstrap via
/// Discovered Guest").
async fn handle_list_all_unmatched(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    use uptrakit_shared_db::entity::proxmox_host_mapping;

    let tenant_id = tenant_id.ok_or_else(|| "tenant context required".to_string())?;
    let page = parse_pagination_page(&params);
    let per_page = parse_pagination_per_page(&params);

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

/// Extract the `page` parameter from action params (1-indexed, default 1).
fn parse_pagination_page(params: &serde_json::Value) -> u64 {
    params
        .get("page")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .max(1)
}

/// Extract the `per_page` parameter from action params (default 50, max 1000).
fn parse_pagination_per_page(params: &serde_json::Value) -> u64 {
    params
        .get("per_page")
        .and_then(|v| v.as_u64())
        .unwrap_or(50)
        .clamp(1, 1000)
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

    #[test]
    fn extension_manifests_returns_two() {
        let manifests = extension_manifests();
        assert_eq!(manifests.len(), 2);
        assert_eq!(manifests[0].id, "proxmox.hosts");
        assert_eq!(manifests[1].id, "proxmox.host-info");
    }

    #[test]
    fn extension_actions_include_host_info_data_load_action_with_permission() {
        let actions = extension_actions();
        assert_eq!(actions.len(), 8);
        let ids: Vec<&str> = actions.iter().map(|a| a.action_id.as_str()).collect();
        assert!(ids.contains(&"add-config"));
        assert!(ids.contains(&"match"));
        assert!(ids.contains(&"approve-match"));
        assert!(ids.contains(&"unmatch"));
        assert!(ids.contains(&"discover"));
        assert!(ids.contains(&"test-connection"));
        assert!(ids.contains(&"list-all-unmatched"));
        assert!(ids.contains(&"get-info"));
        let get_info = actions
            .iter()
            .find(|action| action.action_id == "get-info")
            .expect("get-info action must be exported");
        assert_eq!(get_info.permission, "update_hosts");
    }

    #[test]
    fn hosts_page_is_data_table() {
        let manifest = hosts_page_manifest();
        assert!(matches!(manifest.ui, ExtensionUi::DataTable { .. }));
        assert!(matches!(
            manifest.placement,
            ExtensionPlacement::Page { .. }
        ));
        assert_eq!(manifest.priority, 650);
    }

    #[test]
    fn hosts_page_has_context_selector() {
        let manifest = hosts_page_manifest();
        if let ExtensionUi::DataTable {
            context_selector, ..
        } = &manifest.ui
        {
            let cs = context_selector
                .as_ref()
                .expect("context_selector should be set");
            assert_eq!(cs.param_key, "plugin_config_id");
            assert!(matches!(
                cs.source,
                ContextSelectorSource::PluginConfigs { .. }
            ));
            assert_eq!(
                cs.add_action.as_deref(),
                Some("add-config"),
                "add_action should reference the add-config action"
            );
        } else {
            panic!("expected DataTable UI");
        }
    }

    #[test]
    fn hosts_page_row_actions_reference_action_library() {
        let manifest = hosts_page_manifest();
        if let ExtensionUi::DataTable { row_actions, .. } = &manifest.ui {
            assert_eq!(row_actions, &["match", "approve-match", "unmatch"]);
        } else {
            panic!("expected DataTable UI");
        }
    }

    #[test]
    fn hosts_page_primary_actions_reference_action_library() {
        let manifest = hosts_page_manifest();
        if let ExtensionUi::DataTable {
            primary_actions, ..
        } = &manifest.ui
        {
            assert_eq!(primary_actions, &["discover", "test-connection"]);
        } else {
            panic!("expected DataTable UI");
        }
    }

    #[test]
    fn host_info_is_key_value_panel() {
        let manifest = host_info_panel_manifest();
        assert!(matches!(manifest.ui, ExtensionUi::KeyValue { .. }));
        assert!(matches!(
            manifest.placement,
            ExtensionPlacement::Panel { .. }
        ));
        assert_eq!(manifest.required_permission, "update_hosts");
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
        assert_eq!(rvw.condition, RowCondition::Present);
    }

    #[test]
    fn unmatch_action_has_row_visibility() {
        let action = unmatch_action();
        let rvw = action
            .row_visible_when
            .expect("unmatch should have row_visible_when");
        assert_eq!(rvw.field, "matched_host");
        assert_eq!(rvw.condition, RowCondition::Present);
    }
}
