//! Extension manifests and action handler dispatch for the Proxmox VE plugin.

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use uuid::Uuid;

use uptrakit_internal_wire::extension::*;
use uptrakit_shared_types::Permission;

use crate::client::ProxmoxClient;
use crate::config::ProxmoxConfig;

/// Returns the extension manifests registered by the Proxmox VE plugin.
pub fn extension_manifests() -> Vec<ExtensionManifest> {
    vec![hosts_page_manifest(), host_info_panel_manifest()]
}

/// Full-page extension: Proxmox Hosts data table.
fn hosts_page_manifest() -> ExtensionManifest {
    let add_config_action = ActionDef::new("add-config", "Add Configuration")
        .with_permission(Permission::ManageHosts)
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
                .with_help_text("PVE API token in USER@REALM!TOKENID=SECRET format."),
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
        );

    ExtensionManifest::new(
        "proxmox.hosts",
        "Proxmox VE Hosts",
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
            ],
            data_action: "list".to_string(),
            row_actions: vec![
                ActionDef::new("match", "Manual Match")
                    .with_ui(ActionUi::Form(FormDef::new(vec![
                        FieldDef::new("mapping_id", "Mapping ID")
                            .with_type(FieldType::Hidden)
                            .required(),
                        FieldDef::new("host_id", "Host")
                            .with_type(FieldType::Select)
                            .required()
                            .with_placeholder("Select a host"),
                    ])))
                    .with_permission(Permission::ManageHosts),
                ActionDef::new("unmatch", "Remove Match")
                    .with_permission(Permission::ManageHosts)
                    .destructive(),
            ],
            primary_actions: vec![
                ActionDef::new("discover", "Discover")
                    .with_permission(Permission::ManageHosts)
                    .with_timeout(120),
                ActionDef::new("test-connection", "Test Connection")
                    .with_permission(Permission::ManageHosts)
                    .with_timeout(30),
            ],
            context_selector: Some(Box::new(
                ContextSelectorDef::new(
                    "plugin_config_id",
                    "Configuration",
                    ContextSelectorSource::PluginConfigs {
                        plugin_type: "infrastructure_proxmox".to_string(),
                    },
                )
                .with_add_action(add_config_action)
                .with_empty_message("No Proxmox VE configurations found. Add one to get started."),
            )),
        },
    )
    .with_permission(Permission::ManageHosts)
}

/// Panel extension: Proxmox host info on host detail page.
fn host_info_panel_manifest() -> ExtensionManifest {
    ExtensionManifest::new(
        "proxmox.host-info",
        "Proxmox VE Info",
        ExtensionPlacement::Panel {
            target_page: "host-detail".to_string(),
            position: PanelPosition::default(),
        },
        ExtensionUi::KeyValue {
            data_action: "get-info".to_string(),
        },
    )
}

/// Handle an extension action for the Proxmox plugin.
///
/// Dispatches based on `(extension_id, action_id)` to the appropriate handler.
pub async fn handle_action(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    extension_id: &str,
    action_id: &str,
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    match (extension_id, action_id) {
        ("proxmox.hosts", "list") => handle_list(db, tenant_id, params).await,
        ("proxmox.hosts", "discover") => handle_discover(db, tenant_id, params).await,
        ("proxmox.hosts", "test-connection") => handle_test_connection(db, params).await,
        ("proxmox.hosts", "match") => handle_match(db, params).await,
        ("proxmox.hosts", "unmatch") => handle_unmatch(db, params).await,
        ("proxmox.host-info", "get-info") => handle_get_info(db, tenant_id, params).await,
        _ => Err(format!(
            "unknown action '{action_id}' for extension '{extension_id}'"
        )),
    }
}

/// List all discovered Proxmox host mappings.
async fn handle_list(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    use uptrakit_shared_db::entity::proxmox_host_mapping;

    let plugin_config_id = parse_uuid_param(&params, "plugin_config_id")?;

    let mut query = proxmox_host_mapping::Entity::find()
        .filter(proxmox_host_mapping::Column::PluginConfigId.eq(plugin_config_id));

    if let Some(tid) = tenant_id {
        query = query.filter(proxmox_host_mapping::Column::TenantId.eq(tid));
    }

    let mappings = query
        .all(db)
        .await
        .map_err(|e| format!("database error: {e}"))?;

    let rows: Vec<serde_json::Value> = mappings
        .into_iter()
        .map(|m| {
            serde_json::json!({
                "id": m.id.to_string(),
                "name": m.proxmox_name,
                "node": m.proxmox_node,
                "vmid": m.proxmox_vmid,
                "type": m.proxmox_type,
                "status": m.proxmox_status,
                "hostname": m.hostname,
                "ip_addresses": m.ip_addresses,
                "matched_host": m.host_id.map(|id| id.to_string()),
                "match_method": m.match_method,
            })
        })
        .collect();

    Ok(serde_json::json!({ "rows": rows }))
}

/// Trigger discovery for a Proxmox plugin configuration.
async fn handle_discover(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    let plugin_config_id = parse_uuid_param(&params, "plugin_config_id")?;
    let tenant_id = tenant_id.ok_or_else(|| "tenant context required for discovery".to_string())?;

    let config = load_proxmox_config(db, plugin_config_id).await?;
    let client =
        ProxmoxClient::new(&config).map_err(|e| format!("failed to create client: {e}"))?;

    let guests = crate::discovery::discover_guests(&client, &config.node_filter)
        .await
        .map_err(|e| format!("discovery failed: {e}"))?;

    let persisted =
        crate::discovery::persist_discovered_guests(db, tenant_id, plugin_config_id, &guests)
            .await
            .map_err(|e| format!("failed to persist discovered guests: {e}"))?;

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
    let config = load_proxmox_config(db, plugin_config_id).await?;
    let client =
        ProxmoxClient::new(&config).map_err(|e| format!("failed to create client: {e}"))?;

    let version = client
        .test_connection()
        .await
        .map_err(|e| format!("connection test failed: {e}"))?;

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

    crate::matching::manual_match(db, mapping_id, host_id)
        .await
        .map_err(|e| format!("manual match failed: {e}"))?;

    Ok(serde_json::json!({ "success": true }))
}

/// Remove a match from a mapping.
async fn handle_unmatch(
    db: &DatabaseConnection,
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    let mapping_id = parse_uuid_param(&params, "mapping_id")?;

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
        Some(m) => Ok(serde_json::json!({
            "id": m.id.to_string(),
            "node": m.proxmox_node,
            "vmid": m.proxmox_vmid,
            "type": m.proxmox_type,
            "name": m.proxmox_name,
            "status": m.proxmox_status,
            "hostname": m.hostname,
            "ip_addresses": m.ip_addresses,
            "match_method": m.match_method,
        })),
        None => Ok(serde_json::json!({ "linked": false })),
    }
}

/// Load `ProxmoxConfig` from the `plugin_configs` table.
async fn load_proxmox_config(
    db: &DatabaseConnection,
    plugin_config_id: Uuid,
) -> std::result::Result<ProxmoxConfig, String> {
    use uptrakit_shared_db::entity::plugin_config;

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
    fn hosts_page_is_data_table() {
        let manifest = hosts_page_manifest();
        assert!(matches!(manifest.ui, ExtensionUi::DataTable { .. }));
        assert!(matches!(
            manifest.placement,
            ExtensionPlacement::Page { .. }
        ));
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
            assert!(
                cs.add_action.is_some(),
                "add_action should be set for config creation"
            );
        } else {
            panic!("expected DataTable UI");
        }
    }

    #[test]
    fn hosts_page_primary_actions_do_not_require_config_field() {
        let manifest = hosts_page_manifest();
        if let ExtensionUi::DataTable {
            primary_actions, ..
        } = &manifest.ui
        {
            for action in primary_actions {
                if let Some(ActionUi::Form(form)) = &action.ui {
                    let has_config_field = form.fields.iter().any(|f| f.key == "plugin_config_id");
                    assert!(
                        !has_config_field,
                        "primary action '{}' should not have plugin_config_id field (injected by context selector)",
                        action.action_id
                    );
                }
            }
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
}
