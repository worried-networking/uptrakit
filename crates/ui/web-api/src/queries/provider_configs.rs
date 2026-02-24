use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use time::OffsetDateTime;
use uptrakit_provider_registry::ProviderOps;
use uptrakit_shared_db::entity::provider_config;
use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
use uptrakit_web_api_types::provider_configs::{
    CreateProviderConfigRequest, ProviderConfigResponse, UpdateProviderConfigRequest,
};
use uuid::Uuid;

use crate::auth::token::generate_uuid;
use crate::tenant_db::TenantDb;

/// Error returned by [`update_provider_config`].
#[derive(Debug)]
pub enum UpdateProviderConfigError {
    /// No active provider config with this ID exists for the tenant.
    NotFound,
    /// `name` was explicitly set to an empty string.
    EmptyName,
    /// Provider-specific config validation failed.
    ConfigValidation(String),
    /// Hook parameter validation failed (command-injection prevention).
    HookValidation(String),
    /// A database error occurred.
    Db(sea_orm::DbErr),
}

// --- Private helpers ---

fn provider_config_to_response(
    ops: &dyn ProviderOps,
    m: provider_config::Model,
) -> Option<ProviderConfigResponse> {
    let provider_type: uptrakit_provider_registry::ProviderType = match m.provider_type.parse() {
        Ok(pt) => pt,
        Err(_) => {
            tracing::error!(
                id = %m.id,
                provider_type = %m.provider_type,
                "provider config has invalid provider_type in database, skipping"
            );
            return None;
        }
    };
    let config = ops.mask_config_secrets_str(provider_type.as_str(), &m.config);
    Some(ProviderConfigResponse {
        id: m.id,
        name: m.name,
        provider_type,
        config,
        enabled: m.enabled,
        created_at: m.created_at,
        updated_at: m.updated_at,
    })
}

/// Validate hooks configuration embedded in a provider config or config_override JSON.
///
/// Parses the `"hooks"` key and validates all predefined hook parameters
/// to reject shell metacharacters. Exposed as `pub(crate)` so other query
/// modules (e.g. `software_items`) can reuse the check.
pub(crate) fn validate_hooks_internal(
    config: &serde_json::Value,
) -> Result<(), uptrakit_web_api_types::update_hooks::HookValidationError> {
    if let Some(hooks_val) = config.get("hooks")
        && let Ok(hooks_config) = serde_json::from_value::<
            uptrakit_web_api_types::update_hooks::HooksConfig,
        >(hooks_val.clone())
    {
        hooks_config.validate()?;
    }
    Ok(())
}

// --- pub(crate) helpers ---

/// Find a non-deactivated provider config by ID, scoped to a tenant.
/// Returns the raw model — intended for use when the secrets must remain unmasked.
pub(crate) async fn find_raw_active_config(
    tenant_db: &TenantDb,
    id: Uuid,
) -> Option<provider_config::Model> {
    tenant_db
        .find_by_id::<provider_config::Entity, _>(id)
        .filter(provider_config::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
        .ok()
        .flatten()
}

/// Same as [`find_raw_active_config`] but accepts an arbitrary `ConnectionTrait`
/// so it can be called inside transactions.
pub(crate) async fn find_raw_active_config_txn(
    db: &impl ConnectionTrait,
    tenant_id: Uuid,
    id: Uuid,
) -> Option<provider_config::Model> {
    use sea_orm::EntityTrait;
    provider_config::Entity::find_by_id(id)
        .filter(provider_config::Column::TenantId.eq(tenant_id))
        .filter(provider_config::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .ok()
        .flatten()
}

// --- Public query functions ---

/// Errors returned by [`create_provider_config`].
#[derive(Debug)]
pub enum CreateProviderConfigError {
    /// An active provider config with the same name already exists for the tenant.
    DuplicateName,
    /// A database error occurred.
    Db(sea_orm::DbErr),
}

/// Create a new provider configuration and return the masked response.
/// Validation (name, provider-specific config, hooks) is the caller's responsibility.
pub async fn create_provider_config(
    ops: &dyn ProviderOps,
    tenant_db: &TenantDb,
    req: CreateProviderConfigRequest,
) -> Result<ProviderConfigResponse, CreateProviderConfigError> {
    let now = OffsetDateTime::now_utc();
    let model = provider_config::ActiveModel {
        id: Set(generate_uuid()),
        tenant_id: Set(tenant_db.tenant_id),
        name: Set(req.name),
        provider_type: Set(req.provider_type.to_string()),
        config: Set(req.config),
        enabled: Set(req.enabled),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    };

    let inserted = model.insert(tenant_db.db()).await.map_err(|e| {
        if is_unique_name_violation(&e) {
            CreateProviderConfigError::DuplicateName
        } else {
            CreateProviderConfigError::Db(e)
        }
    })?;
    Ok(provider_config_to_response(ops, inserted)
        .unwrap_or_else(|| unreachable!("provider_type was just validated by the caller")))
}

fn is_unique_name_violation(e: &sea_orm::DbErr) -> bool {
    let msg = e.to_string().to_lowercase();
    (msg.contains("unique") || msg.contains("duplicate"))
        && (msg.contains("name") || msg.contains("uq_provider_configs_active_name"))
}

pub async fn list_provider_configs(
    ops: &dyn ProviderOps,
    tenant_db: &TenantDb,
    params: &PaginationParams,
) -> Result<PaginatedResponse<ProviderConfigResponse>, sea_orm::DbErr> {
    let pagination = params.resolve();

    let base_query = tenant_db
        .find::<provider_config::Entity>()
        .filter(provider_config::Column::DeactivatedAt.is_null())
        .order_by_asc(provider_config::Column::Name);

    let total = base_query.clone().count(tenant_db.db()).await?;

    let configs = base_query
        .offset(Some(pagination.offset()))
        .limit(Some(pagination.per_page))
        .all(tenant_db.db())
        .await?;

    let items: Vec<ProviderConfigResponse> = configs
        .into_iter()
        .filter_map(|m| provider_config_to_response(ops, m))
        .collect();

    Ok(PaginatedResponse::new(items, total, pagination))
}

/// Returns `None` if the config is not found or is deactivated.
pub async fn get_provider_config(
    ops: &dyn ProviderOps,
    tenant_db: &TenantDb,
    id: Uuid,
) -> Result<Option<ProviderConfigResponse>, sea_orm::DbErr> {
    let config = match find_raw_active_config(tenant_db, id).await {
        Some(c) => c,
        None => return Ok(None),
    };
    Ok(provider_config_to_response(ops, config))
}

/// Partial update. Handles secret restoration and provider-specific validation internally.
/// Returns the updated response, or an error describing what went wrong.
pub async fn update_provider_config(
    ops: &dyn ProviderOps,
    tenant_db: &TenantDb,
    id: Uuid,
    req: UpdateProviderConfigRequest,
) -> Result<ProviderConfigResponse, UpdateProviderConfigError> {
    let existing = match find_raw_active_config(tenant_db, id).await {
        Some(c) => c,
        None => return Err(UpdateProviderConfigError::NotFound),
    };

    let provider_type = existing.provider_type.clone();

    // Validate name if changing.
    if let Some(ref name) = req.name
        && name.is_empty()
    {
        return Err(UpdateProviderConfigError::EmptyName);
    }

    // Validate new config if provided; restore masked secrets from the existing value.
    if let Some(ref mut new_config) = req.config.clone() {
        ops.restore_config_secrets_str(&provider_type, new_config, &existing.config);

        if let Err(e) = ops.validate_config_str(&provider_type, new_config) {
            return Err(UpdateProviderConfigError::ConfigValidation(e.to_string()));
        }

        if let Err(e) = validate_hooks_internal(new_config) {
            return Err(UpdateProviderConfigError::HookValidation(e.to_string()));
        }
    }

    let now = OffsetDateTime::now_utc();
    let mut model: provider_config::ActiveModel = existing.into();

    if let Some(name) = req.name {
        model.name = Set(name);
    }
    if let Some(mut config) = req.config {
        // Re-apply secret restoration on the actual value being persisted.
        ops.restore_config_secrets_str(&provider_type, &mut config, model.config.as_ref());
        model.config = Set(config);
    }
    if let Some(enabled) = req.enabled {
        model.enabled = Set(enabled);
    }
    model.updated_at = Set(now);

    let updated = model
        .update(tenant_db.db())
        .await
        .map_err(UpdateProviderConfigError::Db)?;

    provider_config_to_response(ops, updated).ok_or_else(|| {
        UpdateProviderConfigError::ConfigValidation(
            "updated record has unrecognised provider_type".to_string(),
        )
    })
}

/// Soft-delete a provider configuration.
/// Returns `true` if deleted, `false` if not found.
pub async fn delete_provider_config(
    tenant_db: &TenantDb,
    id: Uuid,
) -> Result<bool, sea_orm::DbErr> {
    let config = match find_raw_active_config(tenant_db, id).await {
        Some(c) => c,
        None => return Ok(false),
    };

    let now = OffsetDateTime::now_utc();
    let mut model: provider_config::ActiveModel = config.into();
    model.deactivated_at = Set(Some(now));
    model.enabled = Set(false);
    model.updated_at = Set(now);
    model.update(tenant_db.db()).await?;
    Ok(true)
}
