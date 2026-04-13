//! Host assignment management for software items.

use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, ModelTrait, QueryFilter, Set, TransactionTrait,
};
use time::OffsetDateTime;
use uptrakit_plugin_infrastructure_registry::{PluginConfigOps, RoleKey};
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, plugin_config, prelude::*,
};
use uptrakit_shared_types::HostCapabilities;
use uptrakit_web_api_types::software_items::{
    AssignHostsRequest, HostPluginRoleAssignment, SoftwareItemDetailResponse,
    UpdateHostAssignmentRequest,
};
use uuid::Uuid;

use crate::queries::plugin_configs::find_raw_active_config_txn;
use crate::tenant_db::TenantDb;
use crate::token_utils::generate_uuid;

use super::{
    SoftwareItemQueryError, build_detail_response, find_active_item, load_item_hosts,
    load_latest_version_for_item, load_plugins,
};

// ---------------------------------------------------------------------------
// Private validation helpers (also used by tests in crud.rs)
// ---------------------------------------------------------------------------

/// Error returned when `config_override` validation fails.
#[derive(Debug, thiserror::Error)]
pub(super) enum ConfigOverrideError {
    #[error("config_override must be a JSON object")]
    NotAnObject,
    #[error("plugin validation failed: {0}")]
    PluginValidation(String),
}

/// Validate `config_override` by merging it with the base plugin config and running
/// plugin-specific validation. The merged document must satisfy the plugin's schema.
pub(super) fn validate_config_override(
    ops: &dyn PluginConfigOps,
    plugin_type: &str,
    base_config: &serde_json::Value,
    override_config: &serde_json::Value,
) -> std::result::Result<(), ConfigOverrideError> {
    let mut merged = base_config.clone();
    if let (Some(base_obj), Some(over_obj)) = (merged.as_object_mut(), override_config.as_object())
    {
        for (k, v) in over_obj {
            base_obj.insert(k.clone(), v.clone());
        }
    } else {
        return Err(ConfigOverrideError::NotAnObject);
    }

    let id = uptrakit_shared_types::PluginTypeId::new(plugin_type);
    ops.validate_config(&id, &merged)
        .map_err(|e| ConfigOverrideError::PluginValidation(e.to_string()))
}

/// Validate that `execution_site` is one of the allowed values and that
/// "controller" is only used with the "fetch_releases" role.
pub(super) fn validate_execution_site(
    execution_site: &str,
    role: &uptrakit_web_api_types::PluginRole,
) -> super::Result<()> {
    match execution_site {
        "auto" | "agent" => Ok(()),
        "controller" => {
            if *role == uptrakit_web_api_types::PluginRole::FetchReleases {
                Ok(())
            } else {
                Err(report!(SoftwareItemQueryError::InvalidExecutionSite(
                    format!(
                        "execution_site \"controller\" is only valid for the \"fetch_releases\" role, got \"{}\"",
                        role,
                    )
                )))
            }
        }
        other => Err(report!(SoftwareItemQueryError::InvalidExecutionSite(
            format!(
                "invalid execution_site value \"{other}\"; must be \"auto\", \"agent\", or \"controller\""
            )
        ))),
    }
}

/// Validate plugin type, package identifier, and config/config_override for a host assignment.
fn validate_assignment(
    ops: &dyn PluginConfigOps,
    plugin_type: &str,
    base_config: Option<&serde_json::Value>,
    package_identifier: &str,
    config_override: Option<&serde_json::Value>,
) -> super::Result<()> {
    let id = uptrakit_shared_types::PluginTypeId::new(plugin_type);
    if let Err(e) = ops.validate_package_identifier(&id, package_identifier) {
        bail!(SoftwareItemQueryError::InvalidPackageIdentifier(e));
    }

    if let Some(override_val) = config_override {
        if let Some(base) = base_config {
            if let Err(e) = validate_config_override(ops, plugin_type, base, override_val) {
                bail!(SoftwareItemQueryError::InvalidConfigOverride(e.to_string()));
            }
        } else if let Err(e) = ops.validate_config(&id, override_val) {
            bail!(SoftwareItemQueryError::InvalidConfigOverride(e.to_string()));
        }
    }

    Ok(())
}

/// Resolve plugin config from either an existing ID or an inline create request,
/// within a transaction. Returns `(plugin_config_id, plugin_config::Model)`.
async fn resolve_plugin_config_txn(
    ops: &dyn PluginConfigOps,
    txn: &sea_orm::DatabaseTransaction,
    tenant_id: Uuid,
    assignment: &HostPluginRoleAssignment,
) -> super::Result<(Uuid, plugin_config::Model)> {
    match (&assignment.plugin_config_id, &assignment.plugin_config) {
        (Some(pcid), None) => {
            let pcid = *pcid;
            let c = find_raw_active_config_txn(txn, tenant_id, pcid)
                .await
                .map_err(|e| {
                    report!(SoftwareItemQueryError::Db(sea_orm::DbErr::Custom(
                        e.to_string()
                    )))
                })?
                .ok_or_else(|| report!(SoftwareItemQueryError::PluginConfigNotFound))?;
            Ok((pcid, c))
        }
        (None, Some(inline)) => {
            if inline.name.is_empty() {
                bail!(SoftwareItemQueryError::InvalidInlinePluginConfig(
                    "name must not be empty".to_string(),
                ));
            }
            let id = uptrakit_shared_types::PluginTypeId::new(inline.plugin_type.as_str());
            if let Err(e) = ops.validate_config(&id, &inline.config) {
                bail!(SoftwareItemQueryError::InvalidInlinePluginConfig(
                    e.to_string()
                ));
            }
            let now = OffsetDateTime::now_utc();
            let pcid = generate_uuid();
            let model = plugin_config::ActiveModel {
                id: Set(pcid),
                tenant_id: Set(tenant_id),
                name: Set(inline.name.clone()),
                plugin_type: Set(inline.plugin_type.to_string()),
                config: Set(inline.config.clone()),
                enabled: Set(inline.enabled),
                created_at: Set(now),
                updated_at: Set(now),
                deactivated_at: Set(None),
            };
            let inserted = model.insert(txn).await.context_to()?;
            Ok((pcid, inserted))
        }
        _ => Err(report!(SoftwareItemQueryError::PluginConfigNotFound)),
    }
}

// ---------------------------------------------------------------------------
// Private link helpers
// ---------------------------------------------------------------------------

/// Ensure a `host_software_item` link row exists for the given host and item.
/// Returns the link's ID (existing or newly created).
async fn ensure_host_link(
    txn: &impl sea_orm::ConnectionTrait,
    host_id: Uuid,
    item_id: Uuid,
    now: OffsetDateTime,
) -> super::Result<Uuid> {
    let existing_link = HostSoftwareItem::find()
        .filter(host_software_item::Column::HostId.eq(host_id))
        .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
        .one(txn)
        .await
        .context_to()?;

    if let Some(ref link) = existing_link {
        return Ok(link.id);
    }

    let new_hsi_id = Uuid::now_v7();
    let link = host_software_item::ActiveModel {
        id: Set(new_hsi_id),
        host_id: Set(host_id),
        software_item_id: Set(item_id),
        qualifier: Set(None),
        plugin_config_id: Set(None),
        package_identifier: Set(None),
        installed_version: Set(None),
        installed_version_detected_at: Set(None),
        installed_display_version: Set(None),
        latest_version: Set(None),
        latest_version_fetched_at: Set(None),
        latest_release_metadata: Set(None),
        last_updated_at: Set(None),
        linked_at: Set(now),
        update_category: Set("unknown".to_string()),
        deactivated_at: Set(None),
    };
    link.insert(txn).await.context_to()?;
    Ok(new_hsi_id)
}

/// Validate host compatibility for a role assignment.
///
/// Skips validation when:
/// - `execution_site` is `"controller"` (controller-only roles don't run on hosts)
/// - `RoleKey::from_plugin_role()` returns `None` (unknown/future roles)
fn validate_host_compatibility(
    ops: &dyn PluginConfigOps,
    host_model: &host::Model,
    plugin_type: &str,
    role: &uptrakit_web_api_types::PluginRole,
    execution_site: &str,
) -> super::Result<()> {
    if execution_site == "controller" {
        return Ok(());
    }

    let Some(role_key) = RoleKey::from_plugin_role(role) else {
        return Ok(());
    };

    let caps = HostCapabilities::from_json_features(
        host_model.os_type.as_deref(),
        host_model.os_version.as_deref(),
        host_model.architecture.as_deref(),
        host_model.host_features.as_deref(),
    );

    let plugin_type_id = uptrakit_shared_types::PluginTypeId::new(plugin_type);
    ops.validate_role_compatibility(&plugin_type_id, role_key, &caps)
        .map_err(|e| report!(SoftwareItemQueryError::IncompatibleHost(e.to_string())))
}

/// Validate, resolve, and upsert a single role assignment for a host-software-item pair.
#[allow(clippy::too_many_arguments)]
async fn upsert_role_assignment(
    ops: &dyn PluginConfigOps,
    txn: &sea_orm::DatabaseTransaction,
    tenant_id: Uuid,
    host_model: &host::Model,
    item_id: Uuid,
    hsi_id: Uuid,
    role_assignment: &HostPluginRoleAssignment,
    now: OffsetDateTime,
) -> super::Result<()> {
    let role = &role_assignment.role;
    let execution_site = &role_assignment.execution_site;

    validate_execution_site(execution_site, role)?;

    let (plugin_config_id, config) =
        resolve_plugin_config_txn(ops, txn, tenant_id, role_assignment).await?;

    validate_host_compatibility(ops, host_model, &config.plugin_type, role, execution_site)?;

    validate_assignment(
        ops,
        &config.plugin_type,
        Some(&config.config),
        &role_assignment.package_identifier,
        role_assignment.config_override.as_ref(),
    )?;

    let host_id = host_model.id;

    let existing_plugin = HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(item_id))
        .filter(host_software_item_plugin::Column::Role.eq(role.as_str()))
        .filter(host_software_item_plugin::Column::Ordinal.eq(role_assignment.ordinal))
        .one(txn)
        .await
        .context_to()?;

    match existing_plugin {
        Some(existing) => {
            let mut active: host_software_item_plugin::ActiveModel = existing.into();
            active.plugin_config_id = Set(Some(plugin_config_id));
            active.plugin_type = Set(config.plugin_type.clone());
            active.package_identifier = Set(role_assignment.package_identifier.clone());
            active.config = Set(role_assignment.config_override.clone());
            active.execution_site = Set(execution_site.clone());
            active.updated_at = Set(now);
            active.update(txn).await.context_to()?;
        }
        None => {
            let plugin_row = host_software_item_plugin::ActiveModel {
                id: Set(generate_uuid()),
                host_id: Set(host_id),
                software_item_id: Set(item_id),
                host_software_item_id: Set(hsi_id),
                plugin_config_id: Set(Some(plugin_config_id)),
                plugin_type: Set(config.plugin_type.clone()),
                role: Set(role.as_str().to_string()),
                ordinal: Set(role_assignment.ordinal),
                package_identifier: Set(role_assignment.package_identifier.clone()),
                config: Set(role_assignment.config_override.clone()),
                execution_site: Set(execution_site.clone()),
                created_at: Set(now),
                updated_at: Set(now),
            };
            plugin_row.insert(txn).await.map_err(|e| {
                // Check if this is a unique constraint violation
                // (host_id, software_item_id, role, ordinal).
                if matches!(e, sea_orm::DbErr::Query(..))
                    || e.to_string().contains("UNIQUE")
                    || e.to_string().contains("duplicate")
                {
                    report!(SoftwareItemQueryError::DuplicateHostAssignment)
                } else {
                    report!(SoftwareItemQueryError::Db(e))
                }
            })?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Public query functions
// ---------------------------------------------------------------------------

/// Assign hosts to a software item. Each host carries its own list of role-specific
/// plugin assignments. Returns the updated detail response, or an error if the item
/// or a host is not found.
#[tracing::instrument(skip_all, fields(%id))]
pub async fn assign_hosts(
    ops: &dyn PluginConfigOps,
    tenant_db: &TenantDb,
    id: Uuid,
    req: AssignHostsRequest,
) -> super::Result<SoftwareItemDetailResponse> {
    find_active_item(tenant_db.db(), tenant_db.tenant_id, id)
        .await
        .ok_or_else(|| report!(SoftwareItemQueryError::NotFound))?;

    let txn = tenant_db.db().begin().await.context_to()?;
    let now = OffsetDateTime::now_utc();

    for assignment in &req.host_assignments {
        let host_id = assignment.host_id;

        let host_model = Host::find_by_id(host_id)
            .filter(host::Column::DeactivatedAt.is_null())
            .one(&txn)
            .await
            .context_to()?
            .ok_or_else(|| report!(SoftwareItemQueryError::HostNotFound(host_id)))?;

        let hsi_id = ensure_host_link(&txn, host_id, id, now).await?;

        for role_assignment in &assignment.plugins {
            upsert_role_assignment(
                ops,
                &txn,
                tenant_db.tenant_id,
                &host_model,
                id,
                hsi_id,
                role_assignment,
                now,
            )
            .await?;
        }
    }

    txn.commit().await.context_to()?;

    let item = find_active_item(tenant_db.db(), tenant_db.tenant_id, id)
        .await
        .ok_or_else(|| report!(SoftwareItemQueryError::NotFound))?;

    let hosts = load_item_hosts(tenant_db.db(), id).await;
    let host_count = hosts.len() as u64;
    let plugins = load_plugins(tenant_db.db(), id).await;
    let latest_version = load_latest_version_for_item(tenant_db.db(), id).await;
    let update_available = hosts.iter().any(|h| h.update_available);

    Ok(build_detail_response(
        item,
        plugins,
        host_count,
        latest_version,
        update_available,
        hosts,
    ))
}

/// Update a single role assignment for an existing host-software-item pair.
#[tracing::instrument(skip_all, fields(%id, %host_id))]
pub async fn update_host_assignment(
    ops: &dyn PluginConfigOps,
    tenant_db: &TenantDb,
    id: Uuid,
    host_id: Uuid,
    req: UpdateHostAssignmentRequest,
) -> super::Result<SoftwareItemDetailResponse> {
    find_active_item(tenant_db.db(), tenant_db.tenant_id, id)
        .await
        .ok_or_else(|| report!(SoftwareItemQueryError::NotFound))?;

    let hsi_link = HostSoftwareItem::find()
        .filter(host_software_item::Column::HostId.eq(host_id))
        .filter(host_software_item::Column::SoftwareItemId.eq(id))
        .one(tenant_db.db())
        .await
        .context_to()?
        .ok_or_else(|| report!(SoftwareItemQueryError::NotFound))?;
    let hsi_id = hsi_link.id;

    let existing_plugin = HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(id))
        .filter(host_software_item_plugin::Column::Role.eq(req.role.as_str()))
        .filter(host_software_item_plugin::Column::Ordinal.eq(req.ordinal))
        .one(tenant_db.db())
        .await
        .context_to()?;

    let (existing_pcid, existing_pkg, existing_override, existing_exec_site) =
        if let Some(ref ep) = existing_plugin {
            (
                ep.plugin_config_id,
                Some(ep.package_identifier.clone()),
                ep.config.clone(),
                Some(ep.execution_site.clone()),
            )
        } else {
            (None, None, None, None)
        };

    let effective_pkg = req
        .package_identifier
        .clone()
        .or(existing_pkg.clone())
        .unwrap_or_default();
    let effective_exec_site = req
        .execution_site
        .clone()
        .or(existing_exec_site)
        .unwrap_or_else(|| "auto".to_string());

    validate_execution_site(&effective_exec_site, &req.role)?;

    // Load host model for compatibility validation.
    let host_model = Host::find_by_id(host_id)
        .filter(host::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
        .context_to()?
        .ok_or_else(|| report!(SoftwareItemQueryError::HostNotFound(host_id)))?;

    let txn = tenant_db.db().begin().await.context_to()?;
    let now = OffsetDateTime::now_utc();

    if let Some(pt) = req.plugin_type {
        // Type-only inline assignment: no plugin_configs row is created.
        validate_assignment(
            ops,
            pt.as_str(),
            None,
            &effective_pkg,
            req.config_override.as_ref(),
        )?;

        validate_host_compatibility(
            ops,
            &host_model,
            pt.as_str(),
            &req.role,
            &effective_exec_site,
        )?;

        match existing_plugin {
            Some(existing) => {
                let mut active: host_software_item_plugin::ActiveModel = existing.into();
                active.plugin_config_id = Set(None);
                active.plugin_type = Set(pt.to_string());
                active.package_identifier = Set(effective_pkg);
                if let Some(ref ov) = req.config_override {
                    active.config = Set(if ov.is_null() { None } else { Some(ov.clone()) });
                }
                active.execution_site = Set(effective_exec_site);
                active.updated_at = Set(now);
                active.update(&txn).await.context_to()?;
            }
            None => {
                let plugin_row = host_software_item_plugin::ActiveModel {
                    id: Set(generate_uuid()),
                    host_id: Set(host_id),
                    software_item_id: Set(id),
                    host_software_item_id: Set(hsi_id),
                    plugin_config_id: Set(None),
                    plugin_type: Set(pt.to_string()),
                    role: Set(req.role.as_str().to_string()),
                    ordinal: Set(req.ordinal),
                    package_identifier: Set(effective_pkg),
                    config: Set(req.config_override),
                    execution_site: Set(effective_exec_site),
                    created_at: Set(now),
                    updated_at: Set(now),
                };
                plugin_row.insert(&txn).await.context_to()?;
            }
        }
    } else {
        // Config-based assignment.
        let synthetic = HostPluginRoleAssignment {
            role: req.role.clone(),
            ordinal: req.ordinal,
            plugin_config_id: req.plugin_config_id.or(existing_pcid),
            plugin_config: req.plugin_config,
            package_identifier: effective_pkg.clone(),
            config_override: req.config_override.clone().or(existing_override),
            execution_site: effective_exec_site.clone(),
        };

        let (plugin_config_id, config) =
            resolve_plugin_config_txn(ops, &txn, tenant_db.tenant_id, &synthetic).await?;

        validate_assignment(
            ops,
            &config.plugin_type,
            Some(&config.config),
            &synthetic.package_identifier,
            synthetic.config_override.as_ref(),
        )?;

        validate_host_compatibility(
            ops,
            &host_model,
            &config.plugin_type,
            &req.role,
            &effective_exec_site,
        )?;

        match existing_plugin {
            Some(existing) => {
                let mut active: host_software_item_plugin::ActiveModel = existing.into();
                active.plugin_config_id = Set(Some(plugin_config_id));
                active.plugin_type = Set(config.plugin_type.clone());
                active.package_identifier = Set(synthetic.package_identifier);

                if let Some(ref override_val) = req.config_override {
                    if override_val.is_null() {
                        active.config = Set(None);
                    } else {
                        active.config = Set(Some(override_val.clone()));
                    }
                }

                active.execution_site = Set(synthetic.execution_site);
                active.updated_at = Set(now);
                active.update(&txn).await.context_to()?;
            }
            None => {
                let plugin_row = host_software_item_plugin::ActiveModel {
                    id: Set(generate_uuid()),
                    host_id: Set(host_id),
                    software_item_id: Set(id),
                    host_software_item_id: Set(hsi_id),
                    plugin_config_id: Set(Some(plugin_config_id)),
                    plugin_type: Set(config.plugin_type.clone()),
                    role: Set(req.role.as_str().to_string()),
                    ordinal: Set(req.ordinal),
                    package_identifier: Set(synthetic.package_identifier),
                    config: Set(synthetic.config_override),
                    execution_site: Set(synthetic.execution_site),
                    created_at: Set(now),
                    updated_at: Set(now),
                };
                plugin_row.insert(&txn).await.context_to()?;
            }
        }
    }

    txn.commit().await.context_to()?;

    let item = find_active_item(tenant_db.db(), tenant_db.tenant_id, id)
        .await
        .ok_or_else(|| report!(SoftwareItemQueryError::NotFound))?;

    let hosts = load_item_hosts(tenant_db.db(), id).await;
    let host_count = hosts.len() as u64;
    let plugins = load_plugins(tenant_db.db(), id).await;
    let latest_version = load_latest_version_for_item(tenant_db.db(), id).await;
    let update_available = hosts.iter().any(|h| h.update_available);

    Ok(build_detail_response(
        item,
        plugins,
        host_count,
        latest_version,
        update_available,
        hosts,
    ))
}

/// Unassign a host from a software item.
/// Returns `true` if removed, `false` if the software item or link was not found.
/// Cascade deletes will remove the associated `host_software_item_plugins` rows.
#[tracing::instrument(skip_all, fields(%id, %host_id))]
pub async fn unassign_host(tenant_db: &TenantDb, id: Uuid, host_id: Uuid) -> super::Result<bool> {
    if find_active_item(tenant_db.db(), tenant_db.tenant_id, id)
        .await
        .is_none()
    {
        return Ok(false);
    }

    let link = HostSoftwareItem::find()
        .filter(host_software_item::Column::HostId.eq(host_id))
        .filter(host_software_item::Column::SoftwareItemId.eq(id))
        .one(tenant_db.db())
        .await
        .context_to()?;

    match link {
        Some(l) => {
            l.delete(tenant_db.db()).await.context_to()?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Load the host_software_item link for a specific host assignment.
/// Used by route handlers to verify the assignment exists.
#[tracing::instrument(skip_all, fields(%host_id, %software_item_id))]
pub async fn load_host_assignment(
    db: &sea_orm::DatabaseConnection,
    host_id: Uuid,
    software_item_id: Uuid,
) -> Option<host_software_item::Model> {
    HostSoftwareItem::find()
        .filter(host_software_item::Column::HostId.eq(host_id))
        .filter(host_software_item::Column::SoftwareItemId.eq(software_item_id))
        .one(db)
        .await
        .ok()
        .flatten()
}
