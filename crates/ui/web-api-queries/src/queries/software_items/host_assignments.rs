//! Host assignment management for software items.

use rootcause::prelude::*;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, ModelTrait, QueryFilter, Set};
use time::OffsetDateTime;
use uptrakit_plugin_infrastructure_registry::{PluginConfigOps, RoleKey};
use uptrakit_shared_db::begin_immediate;
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, plugin_config, prelude::*,
};
use uptrakit_shared_types::HostCapabilities;
use uptrakit_web_api_types::software_items::{
    AssignHostsRequest, HostPluginRoleAssignment, JsonObjectMap, JsonObjectMapPatch,
    SoftwareItemDetailResponse, UpdateHostAssignmentRequest,
};
use uuid::Uuid;

use crate::queries::plugin_configs::find_raw_active_config_txn;
use crate::tenant_db::TenantDb;
use crate::token_utils::generate_uuid;

use super::{
    SoftwareItemQueryError, build_detail_response, find_active_item, load_latest_version_for_item,
    load_plugins, try_load_item_hosts,
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

fn config_override_to_value(config_override: &JsonObjectMap) -> serde_json::Value {
    serde_json::Value::Object(config_override.as_object().clone())
}

fn parse_stored_config_override(
    value: Option<serde_json::Value>,
) -> super::Result<Option<JsonObjectMap>> {
    value
        .map(JsonObjectMap::try_from)
        .transpose()
        .map_err(|err| {
            report!(SoftwareItemQueryError::InvalidConfigOverride(
                err.message.clone()
            ))
        })
}

fn resolve_type_only_inline_override(
    config_override: &JsonObjectMapPatch,
    existing_override: Option<JsonObjectMap>,
) -> Option<JsonObjectMap> {
    config_override.clone().resolve(existing_override)
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
    config_override: Option<&JsonObjectMap>,
) -> super::Result<()> {
    let id = uptrakit_shared_types::PluginTypeId::new(plugin_type);
    if let Err(e) = ops.validate_package_identifier(&id, package_identifier) {
        bail!(SoftwareItemQueryError::InvalidPackageIdentifier(
            e.to_string()
        ));
    }

    if let Some(override_val) = config_override {
        let override_value = config_override_to_value(override_val);
        if let Some(base) = base_config {
            if let Err(e) = validate_config_override(ops, plugin_type, base, &override_value) {
                bail!(SoftwareItemQueryError::InvalidConfigOverride(e.to_string()));
            }
        } else if let Err(e) = ops.validate_config(&id, &override_value) {
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
            // Same locked profile write-path order as the dedicated create route:
            // validate → prune → sentinel-assert.
            let mut config = inline.config.clone();
            let _pruned = ops.prune_stale_sensitive_keys(&id, &mut config);
            if let Err(e) = ops.assert_no_sentinel(&id, &config) {
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
                config: Set(config),
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
///
/// Precondition: the caller MUST have already validated that `host_id` belongs to the
/// acting tenant (e.g. via a `Host::find_by_id(host_id).filter(host::Column::TenantId.eq(...))`
/// lookup). This helper trusts `host_id` and performs no tenant check of its own.
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
        last_discovered_at: Set(None),
        discovery_source: Set(None),
        missing_since: Set(None),
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
#[expect(
    clippy::too_many_arguments,
    reason = "query function requires all filter parameters"
)]
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
            active.config = Set(role_assignment.config_override.clone().map(Into::into));
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
                config: Set(role_assignment.config_override.clone().map(Into::into)),
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

// ---------------------------------------------------------------------------
// Transaction-aware _in_tx variants (for emit_stateful callers)
// ---------------------------------------------------------------------------

/// Assign hosts inside a caller-managed `BEGIN IMMEDIATE` transaction.
///
/// Inserts or upserts host links and role assignments; does not reload the full
/// detail response — that is the caller's responsibility.
///
/// # Errors
///
/// Returns `SoftwareItemQueryError::NotFound`, `SoftwareItemQueryError::HostNotFound`,
/// or other validation / DB errors.
pub async fn assign_hosts_in_tx(
    ops: &dyn PluginConfigOps,
    txn: &sea_orm::DatabaseTransaction,
    tenant_id: Uuid,
    id: Uuid,
    req: &AssignHostsRequest,
) -> super::Result<()> {
    let now = OffsetDateTime::now_utc();

    for assignment in &req.host_assignments {
        let host_id = assignment.host_id;

        let host_model = Host::find_by_id(host_id)
            .filter(host::Column::TenantId.eq(tenant_id))
            .filter(host::Column::DeactivatedAt.is_null())
            .one(txn)
            .await
            .context_to()?
            .ok_or_else(|| report!(SoftwareItemQueryError::HostNotFound(host_id)))?;

        let hsi_id = ensure_host_link(txn, host_id, id, now).await?;

        for role_assignment in &assignment.plugins {
            upsert_role_assignment(
                ops,
                txn,
                tenant_id,
                &host_model,
                id,
                hsi_id,
                role_assignment,
                now,
            )
            .await?;
        }
    }

    Ok(())
}

/// Update a host assignment inside a caller-managed `BEGIN IMMEDIATE` transaction.
///
/// Does not reload the full detail response — that is the caller's responsibility.
///
/// # Errors
///
/// Returns `SoftwareItemQueryError::NotFound`, validation errors, or DB errors.
pub async fn update_host_assignment_in_tx(
    ops: &dyn PluginConfigOps,
    txn: &sea_orm::DatabaseTransaction,
    tenant_id: Uuid,
    id: Uuid,
    host_id: Uuid,
    req: UpdateHostAssignmentRequest,
) -> super::Result<()> {
    let hsi_link = HostSoftwareItem::find()
        .filter(host_software_item::Column::HostId.eq(host_id))
        .filter(host_software_item::Column::SoftwareItemId.eq(id))
        .one(txn)
        .await
        .context_to()?
        .ok_or_else(|| report!(SoftwareItemQueryError::NotFound))?;
    let hsi_id = hsi_link.id;

    let existing_plugin = HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(id))
        .filter(host_software_item_plugin::Column::Role.eq(req.role.as_str()))
        .filter(host_software_item_plugin::Column::Ordinal.eq(req.ordinal))
        .one(txn)
        .await
        .context_to()?;

    let (existing_pcid, existing_pkg, existing_override, existing_exec_site) =
        if let Some(ref ep) = existing_plugin {
            (
                ep.plugin_config_id,
                Some(ep.package_identifier.clone()),
                parse_stored_config_override(ep.config.clone())?,
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
        .filter(host::Column::TenantId.eq(tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(txn)
        .await
        .context_to()?
        .ok_or_else(|| report!(SoftwareItemQueryError::HostNotFound(host_id)))?;

    let now = OffsetDateTime::now_utc();

    let existing_is_type_only = existing_plugin
        .as_ref()
        .is_some_and(|ep| ep.plugin_config_id.is_none());
    let request_is_zero_source =
        req.plugin_type.is_none() && req.plugin_config_id.is_none() && req.plugin_config.is_none();
    // "Keep the existing plugin source" must hold for both row shapes: a
    // zero-source request on a type-only row routes to the type-only branch
    // with the stored plugin_type (a no-row role/ordinal keeps rejecting).
    let effective_plugin_type: Option<String> = match req.plugin_type {
        Some(pt) => Some(pt.to_string()),
        None if request_is_zero_source && existing_is_type_only => {
            existing_plugin.as_ref().map(|ep| ep.plugin_type.clone())
        }
        None => None,
    };

    if let Some(pt) = effective_plugin_type {
        // Type-only inline assignment: no plugin_configs row is created.
        let effective_override =
            resolve_type_only_inline_override(&req.config_override, existing_override.clone());

        validate_assignment(
            ops,
            pt.as_str(),
            None,
            &effective_pkg,
            effective_override.as_ref(),
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
                if !req.config_override.is_keep() {
                    active.config = Set(req.config_override.clone().into_option().map(Into::into));
                }
                active.execution_site = Set(effective_exec_site);
                active.updated_at = Set(now);
                active.update(txn).await.context_to()?;
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
                    config: Set(req.config_override.into_option().map(Into::into)),
                    execution_site: Set(effective_exec_site),
                    created_at: Set(now),
                    updated_at: Set(now),
                };
                plugin_row.insert(txn).await.context_to()?;
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
            config_override: req.config_override.clone().resolve(existing_override),
            execution_site: effective_exec_site.clone(),
        };

        if synthetic.plugin_config_id.is_none() && synthetic.plugin_config.is_none() {
            bail!(SoftwareItemQueryError::MissingPluginSource(format!(
                "role={}, ordinal={}",
                req.role.as_str(),
                req.ordinal
            )));
        }

        let (plugin_config_id, config) =
            resolve_plugin_config_txn(ops, txn, tenant_id, &synthetic).await?;

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

                if !req.config_override.is_keep() {
                    active.config = Set(req.config_override.clone().into_option().map(Into::into));
                }

                active.execution_site = Set(synthetic.execution_site);
                active.updated_at = Set(now);
                active.update(txn).await.context_to()?;
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
                    config: Set(synthetic.config_override.map(Into::into)),
                    execution_site: Set(synthetic.execution_site),
                    created_at: Set(now),
                    updated_at: Set(now),
                };
                plugin_row.insert(txn).await.context_to()?;
            }
        }
    }

    Ok(())
}

/// Unassign a host inside a caller-managed `BEGIN IMMEDIATE` transaction.
///
/// Returns `true` if a link was found and deleted, `false` if not found.
///
/// # Errors
///
/// Returns `SoftwareItemQueryError::Db` on DB failures.
pub async fn unassign_host_in_tx(
    txn: &sea_orm::DatabaseTransaction,
    id: Uuid,
    host_id: Uuid,
) -> super::Result<bool> {
    let link = HostSoftwareItem::find()
        .filter(host_software_item::Column::HostId.eq(host_id))
        .filter(host_software_item::Column::SoftwareItemId.eq(id))
        .one(txn)
        .await
        .context_to()?;

    match link {
        Some(l) => {
            l.delete(txn).await.context_to()?;
            Ok(true)
        }
        None => Ok(false),
    }
}

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
    find_active_item(tenant_db.db(), tenant_db.tenant_id(), id)
        .await
        .ok_or_else(|| report!(SoftwareItemQueryError::NotFound))?;

    let txn = begin_immediate(tenant_db.db()).await.context_to()?;
    let now = OffsetDateTime::now_utc();

    for assignment in &req.host_assignments {
        let host_id = assignment.host_id;

        let host_model = tenant_db
            .find_by_id::<host::Entity, _>(host_id)
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
                tenant_db.tenant_id(),
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

    let item = find_active_item(tenant_db.db(), tenant_db.tenant_id(), id)
        .await
        .ok_or_else(|| report!(SoftwareItemQueryError::NotFound))?;

    let hosts = try_load_item_hosts(tenant_db.db(), tenant_db.tenant_id(), id).await?;
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
    if find_active_item(tenant_db.db(), tenant_db.tenant_id(), id)
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

#[cfg(test)]
mod tests {
    use super::resolve_type_only_inline_override;
    use sea_orm::{ActiveModelTrait, Database, DatabaseConnection, Set};
    use time::OffsetDateTime;
    use uptrakit_shared_db::begin_immediate;
    use uptrakit_shared_db::entity::{host, software_item, tenant};
    use uptrakit_web_api_types::software_items::{
        HostSoftwareAssignment, JsonObjectMap, JsonObjectMapPatch,
    };
    use uuid::Uuid;

    // Minimal PluginConfigOps double. The IDOR lookups fail before any ops method is
    // reached, so the bodies are never executed — they exist only to satisfy the trait.
    struct StubOps;

    impl uptrakit_plugin_infrastructure_registry::PluginMetadataOps for StubOps {
        fn get(
            &self,
            _id: &uptrakit_shared_types::PluginTypeId,
        ) -> Option<&uptrakit_plugin_infrastructure_registry::PluginDescriptor> {
            None
        }
        fn all(&self) -> Vec<&uptrakit_plugin_infrastructure_registry::PluginDescriptor> {
            vec![]
        }
        fn instance_enabled(&self, _id: &uptrakit_shared_types::PluginTypeId) -> bool {
            true
        }
    }

    impl uptrakit_plugin_infrastructure_registry::PluginConfigOps for StubOps {}

    async fn idor_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("migrations");
        db
    }

    /// Seed one tenant with one software_item and return (tenant_id, item_id).
    async fn seed_tenant_with_item(db: &DatabaseConnection, name: &str) -> (Uuid, Uuid) {
        let now = OffsetDateTime::now_utc();
        let tenant_id = Uuid::now_v7();
        let item_id = Uuid::now_v7();
        tenant::ActiveModel {
            id: Set(tenant_id),
            name: Set(name.to_string()),
            slug: Set(format!("t-{tenant_id}")),
            is_default: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert tenant");
        software_item::ActiveModel {
            id: Set(item_id),
            tenant_id: Set(tenant_id),
            name: Set("item".to_string()),
            featured: Set(false),
            icon_url: Set(None),
            last_checked_at: Set(None),
            awaiting_restart_timeout: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert software_item");
        (tenant_id, item_id)
    }

    /// Seed one host under the given tenant and return its id.
    async fn seed_host(db: &DatabaseConnection, tenant_id: Uuid) -> Uuid {
        let now = OffsetDateTime::now_utc();
        let host_id = Uuid::now_v7();
        host::ActiveModel {
            id: Set(host_id),
            tenant_id: Set(tenant_id),
            machine_id: Set(format!("machine-{host_id}")),
            hostname: Set("h".to_string()),
            friendly_name: Set("H".to_string()),
            os_type: Set(None),
            os_version: Set(None),
            architecture: Set(None),
            ip_address: Set(None),
            host_features: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert host");
        host_id
    }

    #[tokio::test]
    async fn assign_hosts_in_tx_accepts_own_tenant_host() {
        let db = idor_db().await;
        let (tenant_a, item_a) = seed_tenant_with_item(&db, "a").await;
        let host_a = seed_host(&db, tenant_a).await; // host belongs to tenant A

        // Assign host_a to item_a while acting as tenant A — must succeed.
        let req = super::AssignHostsRequest {
            host_assignments: vec![HostSoftwareAssignment {
                host_id: host_a,
                plugins: vec![],
            }],
        };

        let txn = begin_immediate(&db).await.expect("begin");
        let result = super::assign_hosts_in_tx(&StubOps, &txn, tenant_a, item_a, &req).await;

        assert!(
            result.is_ok(),
            "host belonging to the acting tenant must be accepted, got {result:?}"
        );
    }

    #[tokio::test]
    async fn assign_hosts_in_tx_rejects_foreign_tenant_host() {
        let db = idor_db().await;
        let (tenant_a, item_a) = seed_tenant_with_item(&db, "a").await;
        let (tenant_b, _item_b) = seed_tenant_with_item(&db, "b").await;
        let host_b = seed_host(&db, tenant_b).await; // host belongs to tenant B

        // Attempt to attach host_b (tenant B) to item_a while acting as tenant A.
        let req = super::AssignHostsRequest {
            host_assignments: vec![HostSoftwareAssignment {
                host_id: host_b,
                plugins: vec![],
            }],
        };

        let txn = begin_immediate(&db).await.expect("begin");
        let result = super::assign_hosts_in_tx(&StubOps, &txn, tenant_a, item_a, &req).await;

        assert!(
            matches!(
                result.as_ref().map_err(|e| e.current_context()),
                Err(super::SoftwareItemQueryError::HostNotFound(id)) if *id == host_b
            ),
            "cross-tenant host must be rejected as HostNotFound, got {result:?}"
        );
    }

    /// Insert a `host_software_item` link with no tenant check, so that
    /// `update_host_assignment*`'s first lookup (host_id + item_id) succeeds and
    /// the host-tenant filter becomes the load-bearing gate under test.
    async fn seed_link(db: &DatabaseConnection, host_id: Uuid, item_id: Uuid) {
        use uptrakit_shared_db::entity::host_software_item;
        let now = OffsetDateTime::now_utc();
        host_software_item::ActiveModel {
            id: Set(Uuid::now_v7()),
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
            last_discovered_at: Set(None),
            discovery_source: Set(None),
            missing_since: Set(None),
        }
        .insert(db)
        .await
        .expect("insert rogue host_software_item link");
    }

    /// Minimal `UpdateHostAssignmentRequest` that reaches the host lookup:
    /// `execution_site` resolves to `"auto"`, which passes `validate_execution_site`
    /// for any role, so the tenant-scoped host lookup is the first gate that can fail.
    fn foreign_update_req() -> super::UpdateHostAssignmentRequest {
        super::UpdateHostAssignmentRequest {
            role: uptrakit_web_api_types::PluginRole::DetectVersion,
            ordinal: 0,
            plugin_config_id: None,
            plugin_config: None,
            plugin_type: None,
            package_identifier: None,
            config_override: JsonObjectMapPatch::Keep,
            execution_site: None,
        }
    }

    #[tokio::test]
    async fn assign_hosts_rejects_foreign_tenant_host() {
        use crate::tenant_db::TenantDb;
        let db = idor_db().await;
        let (tenant_a, item_a) = seed_tenant_with_item(&db, "a").await;
        let (tenant_b, _item_b) = seed_tenant_with_item(&db, "b").await;
        let host_b = seed_host(&db, tenant_b).await; // host belongs to tenant B

        // Act as tenant A; attempt to attach tenant B's host to tenant A's item
        // through the public (non-tx) entry point.
        let tenant_db = TenantDb::new(db.clone(), tenant_a);
        let req = super::AssignHostsRequest {
            host_assignments: vec![HostSoftwareAssignment {
                host_id: host_b,
                plugins: vec![],
            }],
        };

        let result = super::assign_hosts(&StubOps, &tenant_db, item_a, req).await;

        // `SoftwareItemDetailResponse` has no `Debug`, so match on the error context
        // directly rather than `{result:?}`-formatting the whole result.
        assert!(
            matches!(
                result.as_ref().map_err(|e| e.current_context()),
                Err(super::SoftwareItemQueryError::HostNotFound(id)) if *id == host_b
            ),
            "cross-tenant host must be rejected as HostNotFound(host_b), got Ok or a different error"
        );
    }

    #[tokio::test]
    async fn update_host_assignment_in_tx_rejects_foreign_tenant_host() {
        let db = idor_db().await;
        let (tenant_a, item_a) = seed_tenant_with_item(&db, "a").await;
        let (tenant_b, _item_b) = seed_tenant_with_item(&db, "b").await;
        let host_b = seed_host(&db, tenant_b).await;
        // Rogue link so the host-tenant filter (not the missing link) is what rejects host_b.
        seed_link(&db, host_b, item_a).await;

        let txn = begin_immediate(&db).await.expect("begin");
        let result = super::update_host_assignment_in_tx(
            &StubOps,
            &txn,
            tenant_a,
            item_a,
            host_b,
            foreign_update_req(),
        )
        .await;

        assert!(
            matches!(
                result.as_ref().map_err(|e| e.current_context()),
                Err(super::SoftwareItemQueryError::HostNotFound(id)) if *id == host_b
            ),
            "cross-tenant host must be rejected as HostNotFound, got {result:?}"
        );
    }

    #[test]
    fn type_only_inline_override_keep_preserves_existing_override() {
        let existing = Some(
            JsonObjectMap::try_from(serde_json::json!({
                "asset_patterns": ["nginx.*linux"]
            }))
            .expect("object config_override"),
        );

        let resolved =
            resolve_type_only_inline_override(&JsonObjectMapPatch::Keep, existing.clone());

        assert_eq!(resolved, existing);
    }

    #[test]
    fn type_only_inline_override_clear_and_set_behave_explicitly() {
        let existing = Some(
            JsonObjectMap::try_from(serde_json::json!({
                "asset_patterns": ["nginx.*linux"]
            }))
            .expect("object config_override"),
        );

        assert_eq!(
            resolve_type_only_inline_override(&JsonObjectMapPatch::Clear, existing.clone()),
            None
        );

        let replacement = JsonObjectMap::try_from(serde_json::json!({
            "channel": "stable"
        }))
        .expect("object config_override");

        assert_eq!(
            resolve_type_only_inline_override(
                &JsonObjectMapPatch::Set(replacement.clone()),
                existing,
            ),
            Some(replacement)
        );
    }
}
