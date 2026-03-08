use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use thiserror::Error;
use time::OffsetDateTime;
use uptrakit_plugin_infrastructure_registry::PluginOps;
use uptrakit_shared_db::entity::plugin_config;
use uptrakit_shared_db::is_unique_constraint_violation;
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
use uptrakit_web_api_types::plugin_configs::{
    CreatePluginConfigRequest, PluginConfigResponse, UpdatePluginConfigRequest,
};
use uuid::Uuid;

use crate::tenant_db::TenantDb;
use crate::token_utils::generate_uuid;

/// Error returned by plugin config query helpers.
#[derive(Debug, Error)]
pub enum PluginConfigError {
    /// No active plugin config with this ID exists for the tenant.
    #[error("plugin config not found")]
    NotFound,
    /// `name` was explicitly set to an empty string.
    #[error("name must not be empty")]
    EmptyName,
    /// An active plugin config with the same name already exists for the tenant.
    #[error("a plugin config with this name already exists")]
    DuplicateName,
    /// Plugin-specific config validation failed.
    #[error("config validation error: {0}")]
    ConfigValidation(String),
    /// Hook parameter validation failed (command-injection prevention).
    #[error("hook validation error: {0}")]
    HookValidation(String),
    /// A database error occurred.
    #[error("database error: {0}")]
    Db(sea_orm::DbErr),
    /// An unexpected internal invariant was violated.
    #[error("internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, rootcause::Report<PluginConfigError>>;
impl_report_conversion!(sea_orm::DbErr => PluginConfigError::Db);

// --- Private helpers ---

fn plugin_config_to_response(
    ops: &dyn PluginOps,
    m: plugin_config::Model,
) -> Option<PluginConfigResponse> {
    let plugin_type: uptrakit_plugin_infrastructure_registry::PluginType =
        match m.plugin_type.parse() {
            Ok(pt) => pt,
            Err(_) => {
                tracing::error!(
                    id = %m.id,
                    plugin_type = %m.plugin_type,
                    "plugin config has invalid plugin_type in database, skipping"
                );
                return None;
            }
        };
    let config = ops.mask_config_secrets_str(plugin_type.as_str(), &m.config);
    let capabilities: Vec<String> = ops
        .capabilities_for_str(plugin_type.as_str())
        .into_iter()
        .filter_map(|c| {
            serde_json::to_value(c)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
        })
        .collect();
    Some(PluginConfigResponse {
        id: m.id,
        name: m.name,
        plugin_type,
        config,
        enabled: m.enabled,
        capabilities,
        created_at: m.created_at,
        updated_at: m.updated_at,
    })
}

/// Validate hooks configuration embedded in a plugin config or config_override JSON.
///
/// Parses the `"hooks"` key and validates all predefined hook parameters
/// to reject shell metacharacters. Exposed as `pub(crate)` so other query
/// modules (e.g. `software_items`) can reuse the check.
pub fn validate_hooks_internal(
    config: &serde_json::Value,
) -> std::result::Result<(), uptrakit_web_api_types::update_hooks::HookValidationError> {
    use uptrakit_web_api_types::update_hooks::HookValidationError;

    // Validate structured "hooks" key.
    if let Some(hooks_val) = config.get("hooks") {
        match serde_json::from_value::<uptrakit_web_api_types::update_hooks::HooksConfig>(
            hooks_val.clone(),
        ) {
            Ok(hooks_config) => hooks_config.validate()?,
            Err(e) => {
                return Err(HookValidationError {
                    field: "hooks",
                    message: format!("invalid hooks format: {e}"),
                });
            }
        }
    }

    // Validate legacy flat hook arrays (pre_update_commands, post_update_commands).
    for field in ["pre_update_commands", "post_update_commands"] {
        if let Some(arr) = config.get(field).and_then(|v| v.as_array()) {
            if arr.len() > uptrakit_shared_types::command_validation::MAX_HOOK_COMMANDS_PER_PHASE {
                return Err(HookValidationError {
                    field: if field == "pre_update_commands" {
                        "pre_update_commands"
                    } else {
                        "post_update_commands"
                    },
                    message: format!(
                        "too many commands ({}, max {})",
                        arr.len(),
                        uptrakit_shared_types::command_validation::MAX_HOOK_COMMANDS_PER_PHASE,
                    ),
                });
            }
            for (i, item) in arr.iter().enumerate() {
                let cmd = item.as_str().ok_or_else(|| HookValidationError {
                    field: if field == "pre_update_commands" {
                        "pre_update_commands"
                    } else {
                        "post_update_commands"
                    },
                    message: format!("{field}[{i}] must be a string"),
                })?;
                if let Err(msg) = uptrakit_shared_types::command_validation::validate_command_length(
                    cmd,
                    &format!("{field}[{i}]"),
                ) {
                    return Err(HookValidationError {
                        field: if field == "pre_update_commands" {
                            "pre_update_commands"
                        } else {
                            "post_update_commands"
                        },
                        message: msg,
                    });
                }
            }
        }
    }

    Ok(())
}

// --- pub(crate) helpers ---

/// Find a non-deactivated plugin config by ID, scoped to a tenant.
/// Returns the raw model — intended for use when the secrets must remain unmasked.
#[tracing::instrument(skip_all)]
pub async fn find_raw_active_config(
    tenant_db: &TenantDb,
    id: Uuid,
) -> Result<Option<plugin_config::Model>> {
    tenant_db
        .find_by_id::<plugin_config::Entity, _>(id)
        .filter(plugin_config::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
        .context_to()
}

/// Same as [`find_raw_active_config`] but accepts an arbitrary `ConnectionTrait`
/// so it can be called inside transactions.
#[tracing::instrument(skip_all, fields(%tenant_id))]
pub(crate) async fn find_raw_active_config_txn(
    db: &impl ConnectionTrait,
    tenant_id: Uuid,
    id: Uuid,
) -> Result<Option<plugin_config::Model>> {
    use sea_orm::EntityTrait;
    plugin_config::Entity::find_by_id(id)
        .filter(plugin_config::Column::TenantId.eq(tenant_id))
        .filter(plugin_config::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context_to()
}

// --- Public query functions ---

/// Create a new plugin configuration and return the masked response.
/// Validation (name, plugin-specific config, hooks) is the caller's responsibility.
#[tracing::instrument(skip_all)]
pub async fn create_plugin_config(
    ops: &dyn PluginOps,
    tenant_db: &TenantDb,
    req: CreatePluginConfigRequest,
) -> Result<PluginConfigResponse> {
    let now = OffsetDateTime::now_utc();
    let model = plugin_config::ActiveModel {
        id: Set(generate_uuid()),
        tenant_id: Set(tenant_db.tenant_id),
        name: Set(req.name),
        plugin_type: Set(req.plugin_type.to_string()),
        config: Set(req.config),
        enabled: Set(req.enabled),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    };

    let inserted = model.insert(tenant_db.db()).await.map_err(|e| {
        if is_unique_constraint_violation(&e) {
            report!(PluginConfigError::DuplicateName)
        } else {
            report!(PluginConfigError::Db(e))
        }
    })?;
    plugin_config_to_response(ops, inserted).ok_or_else(|| {
        report!(PluginConfigError::Internal(
            "inserted plugin_config has unrecognised plugin_type".to_string()
        ))
    })
}

#[tracing::instrument(skip_all)]
pub async fn list_plugin_configs(
    ops: &dyn PluginOps,
    tenant_db: &TenantDb,
    params: &PaginationParams,
) -> Result<PaginatedResponse<PluginConfigResponse>> {
    let pagination = params.resolve();

    let base_query = tenant_db
        .find::<plugin_config::Entity>()
        .filter(plugin_config::Column::DeactivatedAt.is_null())
        .order_by_asc(plugin_config::Column::Name);

    let total = base_query
        .clone()
        .count(tenant_db.db())
        .await
        .context_to()?;

    let configs = base_query
        .offset(Some(pagination.offset()))
        .limit(Some(pagination.per_page))
        .all(tenant_db.db())
        .await
        .context_to()?;

    let items: Vec<PluginConfigResponse> = configs
        .into_iter()
        .filter_map(|m| plugin_config_to_response(ops, m))
        .collect();

    Ok(PaginatedResponse::new(items, total, pagination))
}

/// Returns `None` if the config is not found or is deactivated.
#[tracing::instrument(skip_all)]
pub async fn get_plugin_config(
    ops: &dyn PluginOps,
    tenant_db: &TenantDb,
    id: Uuid,
) -> Result<Option<PluginConfigResponse>> {
    let config = match find_raw_active_config(tenant_db, id).await? {
        Some(c) => c,
        None => return Ok(None),
    };
    Ok(plugin_config_to_response(ops, config))
}

/// Partial update. Handles secret restoration and plugin-specific validation internally.
/// Returns the updated response, or an error describing what went wrong.
#[tracing::instrument(skip_all)]
pub async fn update_plugin_config(
    ops: &dyn PluginOps,
    tenant_db: &TenantDb,
    id: Uuid,
    req: UpdatePluginConfigRequest,
) -> Result<PluginConfigResponse> {
    let existing = find_raw_active_config(tenant_db, id)
        .await?
        .ok_or_else(|| report!(PluginConfigError::NotFound))?;

    let plugin_type = existing.plugin_type.clone();

    // Validate name if changing.
    if let Some(ref name) = req.name
        && name.is_empty()
    {
        bail!(PluginConfigError::EmptyName);
    }

    // Validate new config if provided; restore masked secrets from the existing value.
    if let Some(ref mut new_config) = req.config.clone() {
        ops.restore_config_secrets_str(&plugin_type, new_config, &existing.config);

        if let Err(e) = ops.validate_config_str(&plugin_type, new_config) {
            bail!(PluginConfigError::ConfigValidation(e.to_string()));
        }

        if let Err(e) = validate_hooks_internal(new_config) {
            bail!(PluginConfigError::HookValidation(e.to_string()));
        }
    }

    let now = OffsetDateTime::now_utc();
    let mut model: plugin_config::ActiveModel = existing.into();

    if let Some(name) = req.name {
        model.name = Set(name);
    }
    if let Some(mut config) = req.config {
        // Re-apply secret restoration on the actual value being persisted.
        ops.restore_config_secrets_str(&plugin_type, &mut config, model.config.as_ref());
        model.config = Set(config);
    }
    if let Some(enabled) = req.enabled {
        model.enabled = Set(enabled);
    }
    model.updated_at = Set(now);

    let updated = model.update(tenant_db.db()).await.context_to()?;

    plugin_config_to_response(ops, updated).ok_or_else(|| {
        report!(PluginConfigError::ConfigValidation(
            "updated record has unrecognised plugin_type".to_string(),
        ))
    })
}

// ---------------------------------------------------------------------------
// Batch operations
// ---------------------------------------------------------------------------

/// Soft-delete multiple plugin configs.
#[allow(clippy::type_complexity)]
#[tracing::instrument(skip_all)]
pub async fn batch_delete_plugin_configs(
    tenant_db: &TenantDb,
    ids: &[Uuid],
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>)> {
    let configs = tenant_db
        .find::<plugin_config::Entity>()
        .filter(plugin_config::Column::Id.is_in(ids.iter().copied()))
        .filter(plugin_config::Column::DeactivatedAt.is_null())
        .all(tenant_db.db())
        .await
        .context_to()?;

    let found: std::collections::HashMap<Uuid, plugin_config::Model> =
        configs.into_iter().map(|c| (c.id, c)).collect();

    let mut succeeded = Vec::new();
    let mut failed: Vec<(Uuid, String)> = Vec::new();

    for id in ids {
        if !found.contains_key(id) {
            failed.push((*id, "not found".to_string()));
        }
    }

    let now = OffsetDateTime::now_utc();

    for (id, config) in &found {
        let mut active: plugin_config::ActiveModel = config.clone().into();
        active.deactivated_at = Set(Some(now));
        active.enabled = Set(false);
        active.updated_at = Set(now);
        active.update(tenant_db.db()).await.context_to()?;
        succeeded.push(*id);
    }

    Ok((succeeded, failed))
}

/// Soft-delete a plugin configuration.
/// Returns `true` if deleted, `false` if not found.
#[tracing::instrument(skip_all)]
pub async fn delete_plugin_config(tenant_db: &TenantDb, id: Uuid) -> Result<bool> {
    let config = match find_raw_active_config(tenant_db, id).await? {
        Some(c) => c,
        None => return Ok(false),
    };

    let now = OffsetDateTime::now_utc();
    let mut model: plugin_config::ActiveModel = config.into();
    model.deactivated_at = Set(Some(now));
    model.enabled = Set(false);
    model.updated_at = Set(now);
    model.update(tenant_db.db()).await.context_to()?;
    Ok(true)
}
