//! Extension manifests and action handler dispatch for the Docker plugin.
//!
//! Registers a `ContextMenuGroup` extension on `software-item-host` entities
//! that lets users switch the tracked Docker image tag for a specific host
//! assignment without touching any other hosts.
//!
//! ## Action flow
//!
//! 1. The frontend opens the context menu on a host row that has a Docker
//!    plugin assignment.
//! 2. The user clicks **Switch Tag** — the form pre-loads the current image
//!    reference (without `#container` suffix) via `get-current-tag`.
//! 3. The user edits the tag and submits — `switch-tag` updates all
//!    `host_software_item_plugin` rows and the `host_software_item` row,
//!    then clears stale version data so the next check reflects the new tag.
//!
//! All Docker-specific logic (`ImageRef` parsing, `#container` suffix handling,
//! `validate_identifier` SSRF guard) lives here and does not leak to callers.

use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set,
    TransactionTrait,
};
use uuid::Uuid;

use uptrakit_extension_framework::*;
use uptrakit_shared_types::Permission;

use crate::image_ref::{ImageRef, validate_identifier};

// ── Public API ───────────────────────────────────────────────────────────────

/// Returns the extension manifests registered by the Docker plugin.
pub fn extension_manifests() -> Vec<ExtensionManifest> {
    vec![item_host_actions_manifest()]
}

/// Returns the action library for the Docker plugin.
///
/// All action IDs referenced by UI definitions in the manifests must be
/// defined here.
pub fn extension_actions() -> Vec<ActionDef> {
    vec![switch_tag_action(), get_current_tag_action()]
}

// ── Manifest definitions ─────────────────────────────────────────────────────

/// Context menu group extension for `software-item-host` rows.
fn item_host_actions_manifest() -> ExtensionManifest {
    ExtensionManifest::new(
        "docker.item-host-actions",
        "Docker",
        100,
        ExtensionPlacement::ContextMenuGroup {
            target_entity: "software-item-host".to_string(),
            group_label: "Docker".to_string(),
        },
        ExtensionUi::Actions {
            actions: vec!["switch-tag".to_string()],
        },
    )
    .with_permission(Permission::UpdateSoftware)
}

// ── Action definitions ───────────────────────────────────────────────────────

/// Form action: switch the Docker image tag for a specific host assignment.
fn switch_tag_action() -> ActionDef {
    ActionDef::new("switch-tag", "Switch Tag")
        .with_permission(Permission::UpdateSoftware)
        .with_ui(ActionUi::Form(
            FormDef::new(vec![
                FieldDef::new("software_item_id", "")
                    .with_type(FieldType::Hidden)
                    .required(),
                FieldDef::new("host_id", "")
                    .with_type(FieldType::Hidden)
                    .required(),
                FieldDef::new("new_image_ref", "New Image Reference")
                    .required()
                    .with_placeholder("ghcr.io/example/app:26.2.6")
                    .with_help_text(
                        "Enter the full image reference with the new tag. \
                         The container name is preserved automatically.",
                    ),
            ])
            .with_pre_load_action("get-current-tag"),
        ))
}

/// Pre-load helper: returns the current image reference for form pre-population.
///
/// Not shown as a button in the UI — invoked automatically when the Switch Tag
/// form opens to populate the `new_image_ref` field with the current value.
fn get_current_tag_action() -> ActionDef {
    ActionDef::new("get-current-tag", "").with_permission(Permission::UpdateSoftware)
}

// ── Extension action handler ─────────────────────────────────────────────────

/// Dispatch an extension action for the Docker plugin.
///
/// Routes based on `(extension_id, action_id)` to the appropriate handler.
///
/// The `ctx.db` field is `&dyn Any`; we downcast to `&DatabaseConnection`
/// once at the top so individual handlers keep a concrete typed reference.
#[tracing::instrument(skip_all, fields(extension_id, action_id))]
pub async fn handle_action(
    ctx: &uptrakit_plugin_infrastructure_core::descriptor::ExtensionActionContext<'_>,
    extension_id: &str,
    action_id: &str,
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    tracing::debug!("dispatching Docker extension action");

    let db = ctx
        .db
        .downcast_ref::<DatabaseConnection>()
        .ok_or_else(|| "internal error: expected DatabaseConnection".to_string())?;

    let result = match (extension_id, action_id) {
        ("docker.item-host-actions", "switch-tag") => handle_switch_tag(db, params).await,
        ("docker.item-host-actions", "get-current-tag") => handle_get_current_tag(db, params).await,
        _ => Err(format!(
            "unknown action '{action_id}' for extension '{extension_id}'"
        )),
    };

    match &result {
        Ok(_) => tracing::debug!("Docker extension action succeeded"),
        Err(e) => tracing::warn!(error = %e, "Docker extension action failed"),
    }

    result
}

// ── Action handlers ──────────────────────────────────────────────────────────

/// Pre-load handler: return the current image reference (without `#container`
/// suffix) so the Switch Tag form can pre-populate the `new_image_ref` field.
async fn handle_get_current_tag(
    db: &DatabaseConnection,
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    use uptrakit_shared_db::entity::host_software_item_plugin;

    let host_id = parse_uuid_param(&params, "host_id")?;
    let software_item_id = parse_uuid_param(&params, "software_item_id")?;

    tracing::debug!(%host_id, %software_item_id, "fetching current Docker tag");

    // Find the first Docker plugin row for this (host, software_item) pair.
    // Any row will do — all rows sharing the same image ref differ only by
    // container name suffix and role.
    let plugin_rows = host_software_item_plugin::Entity::find()
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(software_item_id))
        .filter(host_software_item_plugin::Column::PluginType.eq("releases_docker"))
        .all(db)
        .await
        .map_err(|e| format!("database error: {e}"))?;

    let image_ref = plugin_rows
        .into_iter()
        .next()
        .map(|row| strip_container_suffix(&row.package_identifier))
        .unwrap_or_default();

    tracing::debug!(%host_id, %software_item_id, image_ref = %image_ref, "resolved current Docker tag");

    Ok(serde_json::json!({ "new_image_ref": image_ref }))
}

/// Switch tag handler: update all Docker plugin rows for a specific
/// `(host_id, software_item_id)` pair and clear stale version data.
///
/// Preserves the `#container_name` suffix on each plugin row so subsequent
/// update operations still target the correct named container.
async fn handle_switch_tag(
    db: &DatabaseConnection,
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    use uptrakit_shared_db::entity::{host_software_item, host_software_item_plugin};

    let host_id = parse_uuid_param(&params, "host_id")?;
    let software_item_id = parse_uuid_param(&params, "software_item_id")?;

    let new_image_ref = params
        .get("new_image_ref")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required parameter 'new_image_ref'".to_string())?
        .trim()
        .to_string();

    tracing::debug!(%host_id, %software_item_id, new_image_ref = %new_image_ref, "switching Docker tag");

    // Validate format first (parses the image ref) then SSRF check.
    let new_ref: ImageRef = new_image_ref
        .parse()
        .map_err(|e| format!("invalid image reference: {e}"))?;

    validate_identifier(&new_image_ref).map_err(|e| format!("invalid image reference: {e}"))?;

    // Load all plugin rows for this (host, software_item) pair.
    let plugin_rows = host_software_item_plugin::Entity::find()
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(software_item_id))
        .all(db)
        .await
        .map_err(|e| format!("database error loading plugin rows: {e}"))?;

    if plugin_rows.is_empty() {
        return Err("no plugin assignments found for this host".to_string());
    }

    // Load the host_software_item row.
    let hsi_row = host_software_item::Entity::find()
        .filter(host_software_item::Column::HostId.eq(host_id))
        .filter(host_software_item::Column::SoftwareItemId.eq(software_item_id))
        .one(db)
        .await
        .map_err(|e| format!("database error loading host_software_item: {e}"))?
        .ok_or_else(|| {
            format!("host_software_item not found for host {host_id} / item {software_item_id}")
        })?;

    let txn = db
        .begin()
        .await
        .map_err(|e| format!("failed to begin transaction: {e}"))?;

    // Update each plugin row: preserve #container suffix, replace image ref.
    for row in plugin_rows {
        // Only update Docker plugin rows; leave hooks and other plugin types alone.
        if row.plugin_type != "releases_docker" {
            continue;
        }

        let new_pkg_id = match extract_container_suffix(&row.package_identifier) {
            Some(container) => format!("{}#{container}", new_ref.full_ref),
            None => new_ref.full_ref.clone(),
        };

        let mut active: host_software_item_plugin::ActiveModel = row.into();
        active.package_identifier = Set(new_pkg_id);
        active
            .update(&txn)
            .await
            .map_err(|e| format!("failed to update plugin row: {e}"))?;
    }

    // Update host_software_item: new package_identifier and clear stale version data.
    let mut hsi_active: host_software_item::ActiveModel = hsi_row.into();
    hsi_active.package_identifier = Set(Some(new_ref.full_ref.clone()));
    hsi_active.installed_version = Set(None);
    hsi_active.installed_display_version = Set(None);
    hsi_active.installed_version_detected_at = Set(None);
    hsi_active.latest_version = Set(None);
    hsi_active.latest_version_fetched_at = Set(None);
    hsi_active.latest_release_metadata = Set(None);
    hsi_active.update_category = Set("unknown".to_string());
    hsi_active
        .update(&txn)
        .await
        .map_err(|e| format!("failed to update host_software_item: {e}"))?;

    txn.commit()
        .await
        .map_err(|e| format!("failed to commit transaction: {e}"))?;

    tracing::info!(
        %host_id,
        %software_item_id,
        new_image_ref = %new_ref.full_ref,
        "Docker tag switched successfully"
    );

    Ok(serde_json::json!({
        "ok": true,
        "message": "Tag switched. Run a version check to update status.",
    }))
}

// ── Private helpers ──────────────────────────────────────────────────────────

/// Strip the `#container_name` suffix from a `package_identifier`.
///
/// Returns the image ref part (before `#`), or the full string if no `#` is
/// present.
fn strip_container_suffix(id: &str) -> String {
    match id.find('#') {
        Some(pos) => id[..pos].to_string(),
        None => id.to_string(),
    }
}

/// Extract the container name from a `package_identifier`, if present.
///
/// Returns `Some(container_name)` when the identifier contains `#`, or `None`.
fn extract_container_suffix(id: &str) -> Option<&str> {
    id.find('#').map(|pos| &id[pos + 1..])
}

/// Parse a UUID parameter from JSON params.
fn parse_uuid_param(params: &serde_json::Value, key: &str) -> std::result::Result<Uuid, String> {
    let val = params
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("missing required parameter '{key}'"))?;
    Uuid::parse_str(val).map_err(|e| format!("invalid UUID for '{key}': {e}"))
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_manifests_returns_one() {
        let manifests = extension_manifests();
        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].id, "docker.item-host-actions");
    }

    #[test]
    fn extension_actions_returns_two() {
        let actions = extension_actions();
        assert_eq!(actions.len(), 2);
        let ids: Vec<&str> = actions.iter().map(|a| a.action_id.as_str()).collect();
        assert!(ids.contains(&"switch-tag"));
        assert!(ids.contains(&"get-current-tag"));
    }

    #[test]
    fn item_host_actions_is_context_menu_group() {
        let manifest = item_host_actions_manifest();
        assert!(matches!(
            manifest.placement,
            ExtensionPlacement::ContextMenuGroup { .. }
        ));
        assert!(matches!(manifest.ui, ExtensionUi::Actions { .. }));
    }

    #[test]
    fn context_menu_group_target_entity() {
        let manifest = item_host_actions_manifest();
        if let ExtensionPlacement::ContextMenuGroup { target_entity, .. } = &manifest.placement {
            assert_eq!(target_entity, "software-item-host");
        } else {
            panic!("expected ContextMenuGroup placement");
        }
    }

    #[test]
    fn switch_tag_action_has_form_with_pre_load() {
        let action = switch_tag_action();
        assert!(action.ui.is_some());
        if let Some(ActionUi::Form(form)) = &action.ui {
            assert_eq!(form.pre_load_action.as_deref(), Some("get-current-tag"));
            // Hidden fields present
            let keys: Vec<&str> = form.fields.iter().map(|f| f.key.as_str()).collect();
            assert!(keys.contains(&"software_item_id"));
            assert!(keys.contains(&"host_id"));
            assert!(keys.contains(&"new_image_ref"));
        } else {
            panic!("expected Form UI");
        }
    }

    #[test]
    fn get_current_tag_action_has_no_ui() {
        let action = get_current_tag_action();
        assert!(action.ui.is_none(), "pre-load helper must have no UI");
    }

    #[test]
    fn strip_container_suffix_with_suffix() {
        assert_eq!(
            strip_container_suffix("ghcr.io/xtls/xray-core:25.8.3#xray"),
            "ghcr.io/xtls/xray-core:25.8.3"
        );
    }

    #[test]
    fn strip_container_suffix_without_suffix() {
        assert_eq!(
            strip_container_suffix("ghcr.io/xtls/xray-core:25.8.3"),
            "ghcr.io/xtls/xray-core:25.8.3"
        );
    }

    #[test]
    fn extract_container_suffix_with_suffix() {
        assert_eq!(
            extract_container_suffix("ghcr.io/xtls/xray-core:25.8.3#xray"),
            Some("xray")
        );
    }

    #[test]
    fn extract_container_suffix_without_suffix() {
        assert_eq!(
            extract_container_suffix("ghcr.io/xtls/xray-core:25.8.3"),
            None
        );
    }

    #[test]
    fn parse_uuid_param_valid() {
        let params = serde_json::json!({ "id": "01944c3c-6a3a-7000-8000-000000000001" });
        assert!(parse_uuid_param(&params, "id").is_ok());
    }

    #[test]
    fn parse_uuid_param_missing() {
        let params = serde_json::json!({});
        assert!(parse_uuid_param(&params, "id").is_err());
    }

    #[test]
    fn parse_uuid_param_invalid() {
        let params = serde_json::json!({ "id": "not-a-uuid" });
        assert!(parse_uuid_param(&params, "id").is_err());
    }
}
