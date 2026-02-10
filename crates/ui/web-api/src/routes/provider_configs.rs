use crate::AppState;
use crate::auth::permissions::Permission;
use crate::auth::token::generate_uuid;
use crate::error_response::error_response;
use crate::middleware::require_auth::AuthenticatedUser;
use crate::middleware::tenant_context::TenantContext;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use std::sync::Arc;
use time::OffsetDateTime;
use uptrakit_provider_registry::ProviderRegistry;
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::provider_config;

pub use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
pub use uptrakit_web_api_types::provider_configs::{
    CreateProviderConfigRequest, ProviderConfigResponse, UpdateProviderConfigRequest,
};

fn provider_config_response_from(m: provider_config::Model) -> ProviderConfigResponse {
    ProviderConfigResponse {
        id: m.id.to_string(),
        name: m.name,
        provider_type: m.provider_type.clone(),
        config: ProviderRegistry::mask_config_secrets_str(&m.provider_type, &m.config),
        enabled: m.enabled,
        created_at: m
            .created_at
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default(),
        updated_at: m
            .updated_at
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default(),
    }
}

/// Create a new provider configuration.
#[utoipa::path(
    post,
    path = "/api/v1/provider-configs",
    request_body = CreateProviderConfigRequest,
    responses(
        (status = 201, description = "Provider config created", body = ProviderConfigResponse),
        (status = 400, description = "Invalid input")
    ),
    tag = "Provider Configs",
    security(("bearer_token" = []))
)]
pub async fn create_provider_config(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    axum::Extension(user): axum::Extension<AuthenticatedUser>,
    Json(req): Json<CreateProviderConfigRequest>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    if req.name.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "name must not be empty");
    }

    if ProviderRegistry::parse_provider_type(&req.provider_type).is_none() {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!("unknown provider_type: {}", req.provider_type),
        );
    }

    if let Err(e) = ProviderRegistry::validate_config_str(&req.provider_type, &req.config) {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    // Validate hook parameters to prevent command injection
    if let Err(e) = validate_hooks_in_config(&req.config) {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    let now = OffsetDateTime::now_utc();
    let model = provider_config::ActiveModel {
        id: Set(generate_uuid()),
        tenant_id: Set(tenant.tenant_id),
        name: Set(req.name),
        provider_type: Set(req.provider_type),
        config: Set(req.config),
        enabled: Set(req.enabled),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    };

    match model.insert(&state.db).await {
        Ok(inserted) => (
            StatusCode::CREATED,
            Json(provider_config_response_from(inserted)),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to create provider config: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// List all non-deactivated provider configurations.
#[utoipa::path(
    get,
    path = "/api/v1/provider-configs",
    params(
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)")
    ),
    responses(
        (status = 200, description = "Paginated list of provider configs", body = PaginatedResponse<ProviderConfigResponse>),
    ),
    tag = "Provider Configs",
    security(("bearer_token" = []))
)]
pub async fn list_provider_configs(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    axum::Extension(user): axum::Extension<AuthenticatedUser>,
    Query(params): Query<PaginationParams>,
) -> Response {
    if !user.has_permission(Permission::ViewSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let pagination = params.resolve();

    let base_query = ProviderConfig::find()
        .filter(provider_config::Column::TenantId.eq(tenant.tenant_id))
        .filter(provider_config::Column::DeactivatedAt.is_null())
        .order_by_asc(provider_config::Column::Name);

    let total = match base_query.clone().count(&state.db).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to count provider configs: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    match base_query
        .offset(Some(pagination.offset()))
        .limit(Some(pagination.per_page))
        .all(&state.db)
        .await
    {
        Ok(configs) => {
            let items: Vec<ProviderConfigResponse> = configs
                .into_iter()
                .map(provider_config_response_from)
                .collect();
            (
                StatusCode::OK,
                Json(PaginatedResponse::new(items, total, pagination)),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("Failed to list provider configs: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Get a specific provider configuration.
#[utoipa::path(
    get,
    path = "/api/v1/provider-configs/{id}",
    params(("id" = String, Path, description = "Provider config ID")),
    responses(
        (status = 200, description = "Provider config details", body = ProviderConfigResponse),
        (status = 404, description = "Provider config not found")
    ),
    tag = "Provider Configs",
    security(("bearer_token" = []))
)]
pub async fn get_provider_config(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Path(id): Path<String>,
    axum::Extension(user): axum::Extension<AuthenticatedUser>,
) -> Response {
    if !user.has_permission(Permission::ViewSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let config_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid UUID"),
    };

    match find_active_config(&state.db, tenant.tenant_id, config_id).await {
        Some(config) => {
            (StatusCode::OK, Json(provider_config_response_from(config))).into_response()
        }
        None => error_response(StatusCode::NOT_FOUND, "Provider config not found"),
    }
}

/// Update a provider configuration (partial update).
#[utoipa::path(
    put,
    path = "/api/v1/provider-configs/{id}",
    params(("id" = String, Path, description = "Provider config ID")),
    request_body = UpdateProviderConfigRequest,
    responses(
        (status = 200, description = "Provider config updated", body = ProviderConfigResponse),
        (status = 404, description = "Provider config not found")
    ),
    tag = "Provider Configs",
    security(("bearer_token" = []))
)]
pub async fn update_provider_config(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Path(id): Path<String>,
    axum::Extension(user): axum::Extension<AuthenticatedUser>,
    Json(req): Json<UpdateProviderConfigRequest>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let config_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid UUID"),
    };

    let existing = match find_active_config(&state.db, tenant.tenant_id, config_id).await {
        Some(c) => c,
        None => return error_response(StatusCode::NOT_FOUND, "Provider config not found"),
    };

    let provider_type = existing.provider_type.clone();

    // Validate new config if provided
    if let Some(ref mut new_config) = req.config.clone() {
        // Restore masked secrets from existing config
        ProviderRegistry::restore_config_secrets_str(&provider_type, new_config, &existing.config);

        if let Err(e) = ProviderRegistry::validate_config_str(&provider_type, new_config) {
            return error_response(StatusCode::BAD_REQUEST, e.to_string());
        }

        // Validate hook parameters to prevent command injection
        if let Err(e) = validate_hooks_in_config(new_config) {
            return error_response(StatusCode::BAD_REQUEST, e.to_string());
        }
    }

    let now = OffsetDateTime::now_utc();
    let mut model: provider_config::ActiveModel = existing.into();

    if let Some(name) = req.name {
        if name.is_empty() {
            return error_response(StatusCode::BAD_REQUEST, "name must not be empty");
        }
        model.name = Set(name);
    }
    if let Some(mut config) = req.config {
        // Re-apply secret restoration on the actual value being saved
        ProviderRegistry::restore_config_secrets_str(
            &provider_type,
            &mut config,
            model.config.as_ref(),
        );
        model.config = Set(config);
    }
    if let Some(enabled) = req.enabled {
        model.enabled = Set(enabled);
    }
    model.updated_at = Set(now);

    match model.update(&state.db).await {
        Ok(updated) => {
            (StatusCode::OK, Json(provider_config_response_from(updated))).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to update provider config: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Soft-delete a provider configuration.
#[utoipa::path(
    delete,
    path = "/api/v1/provider-configs/{id}",
    params(("id" = String, Path, description = "Provider config ID")),
    responses(
        (status = 204, description = "Provider config deleted"),
        (status = 404, description = "Provider config not found")
    ),
    tag = "Provider Configs",
    security(("bearer_token" = []))
)]
pub async fn delete_provider_config(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Path(id): Path<String>,
    axum::Extension(user): axum::Extension<AuthenticatedUser>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let config_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid UUID"),
    };

    let config = match find_active_config(&state.db, tenant.tenant_id, config_id).await {
        Some(c) => c,
        None => return error_response(StatusCode::NOT_FOUND, "Provider config not found"),
    };

    let now = OffsetDateTime::now_utc();
    let mut model: provider_config::ActiveModel = config.into();
    model.deactivated_at = Set(Some(now));
    model.enabled = Set(false);
    model.updated_at = Set(now);

    match model.update(&state.db).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!("Failed to soft-delete provider config: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Find a non-deactivated provider config by ID, scoped to a tenant.
async fn find_active_config(
    db: &sea_orm::DatabaseConnection,
    tenant_id: uuid::Uuid,
    id: uuid::Uuid,
) -> Option<provider_config::Model> {
    ProviderConfig::find_by_id(id)
        .filter(provider_config::Column::TenantId.eq(tenant_id))
        .filter(provider_config::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .ok()
        .flatten()
}

/// Validate hooks configuration embedded in a provider config or config_override JSON.
///
/// Parses the `"hooks"` key and validates all predefined hook parameters
/// to reject shell metacharacters.
pub(crate) fn validate_hooks_in_config(
    config: &serde_json::Value,
) -> std::result::Result<(), uptrakit_web_api_types::update_hooks::HookValidationError> {
    if let Some(hooks_val) = config.get("hooks")
        && let Ok(hooks_config) = serde_json::from_value::<
            uptrakit_web_api_types::update_hooks::HooksConfig,
        >(hooks_val.clone())
    {
        hooks_config.validate()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sentinel value used to indicate a masked secret in API responses.
    const SECRET_MASK: &str = "***";

    #[test]
    fn mask_github_auth_token() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world",
            "auth_token": "ghp_secret123"
        });
        let masked = ProviderRegistry::mask_config_secrets_str("github_releases", &config);
        assert_eq!(masked["auth_token"], SECRET_MASK);
        assert_eq!(masked["owner"], "octocat");
    }

    #[test]
    fn mask_null_token_becomes_masked() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world",
            "auth_token": null
        });
        let masked = ProviderRegistry::mask_config_secrets_str("github_releases", &config);
        // with_secrets_masked always sets auth_token to "***"
        assert_eq!(masked["auth_token"], SECRET_MASK);
    }

    #[test]
    fn mask_without_token_field_adds_masked() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        let masked = ProviderRegistry::mask_config_secrets_str("github_releases", &config);
        // with_secrets_masked always adds auth_token as "***"
        assert_eq!(masked["auth_token"], SECRET_MASK);
    }

    #[test]
    fn mask_unknown_provider_type() {
        let config = serde_json::json!({"key": "value"});
        let masked = ProviderRegistry::mask_config_secrets_str("unknown_type", &config);
        assert_eq!(masked, config);
    }

    #[test]
    fn restore_masked_token() {
        let mut incoming = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world",
            "auth_token": "***"
        });
        let existing = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world",
            "auth_token": "ghp_real_token"
        });
        ProviderRegistry::restore_config_secrets_str("github_releases", &mut incoming, &existing);
        assert_eq!(incoming["auth_token"], "ghp_real_token");
    }

    #[test]
    fn restore_new_token_not_masked() {
        let mut incoming = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world",
            "auth_token": "ghp_new_token"
        });
        let existing = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world",
            "auth_token": "ghp_old_token"
        });
        ProviderRegistry::restore_config_secrets_str("github_releases", &mut incoming, &existing);
        assert_eq!(incoming["auth_token"], "ghp_new_token");
    }

    #[test]
    fn validate_valid_github_config() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        assert!(ProviderRegistry::validate_config_str("github_releases", &config).is_ok());
    }

    #[test]
    fn validate_invalid_github_config() {
        let config = serde_json::json!({
            "owner": "",
            "repo": "hello-world"
        });
        assert!(ProviderRegistry::validate_config_str("github_releases", &config).is_err());
    }

    #[test]
    fn validate_unknown_provider_type() {
        let config = serde_json::json!({});
        assert!(ProviderRegistry::validate_config_str("nonexistent", &config).is_err());
    }

    #[test]
    fn parse_known_provider_types() {
        assert!(ProviderRegistry::parse_provider_type("github_releases").is_some());
        assert!(ProviderRegistry::parse_provider_type("proxmox_helper_scripts").is_some());
        assert!(ProviderRegistry::parse_provider_type("docker_registry").is_some());
        assert!(ProviderRegistry::parse_provider_type("homebrew").is_some());
        assert!(ProviderRegistry::parse_provider_type("unknown").is_none());
    }

    // --- Homebrew provider tests ---

    #[test]
    fn validate_valid_homebrew_config() {
        let config = serde_json::json!({});
        assert!(ProviderRegistry::validate_config_str("homebrew", &config).is_ok());
    }

    #[test]
    fn validate_homebrew_config_with_cask() {
        let config = serde_json::json!({"package_type": "cask"});
        assert!(ProviderRegistry::validate_config_str("homebrew", &config).is_ok());
    }

    #[test]
    fn mask_homebrew_config_unchanged() {
        let config = serde_json::json!({"package_type": "formula"});
        let masked = ProviderRegistry::mask_config_secrets_str("homebrew", &config);
        // No secrets to mask — config returned unchanged
        assert_eq!(masked, config);
    }

    // --- Docker Registry provider tests ---

    #[test]
    fn mask_docker_registry_basic_password() {
        let config = serde_json::json!({
            "image": "nginx",
            "auth": {
                "type": "basic",
                "username": "user",
                "password": "secret123"
            }
        });
        let masked = ProviderRegistry::mask_config_secrets_str("docker_registry", &config);
        assert_eq!(masked["auth"]["password"], SECRET_MASK);
        assert_eq!(masked["auth"]["username"], "user");
    }

    #[test]
    fn mask_docker_registry_bearer_token() {
        let config = serde_json::json!({
            "image": "nginx",
            "auth": {
                "type": "bearer",
                "token": "ghcr_token_secret"
            }
        });
        let masked = ProviderRegistry::mask_config_secrets_str("docker_registry", &config);
        assert_eq!(masked["auth"]["token"], SECRET_MASK);
    }

    #[test]
    fn mask_docker_registry_no_auth() {
        let config = serde_json::json!({
            "image": "nginx"
        });
        let masked = ProviderRegistry::mask_config_secrets_str("docker_registry", &config);
        // None auth stays absent (serialized with skip_serializing_if)
        assert!(masked.get("auth").is_none());
    }

    #[test]
    fn mask_docker_registry_null_auth() {
        let config = serde_json::json!({
            "image": "nginx",
            "auth": null
        });
        let masked = ProviderRegistry::mask_config_secrets_str("docker_registry", &config);
        // JSON null deserializes to None, which stays absent after masking
        assert!(masked.get("auth").is_none());
    }

    #[test]
    fn restore_docker_registry_masked_password() {
        let mut incoming = serde_json::json!({
            "image": "nginx",
            "auth": {
                "type": "basic",
                "username": "user",
                "password": "***"
            }
        });
        let existing = serde_json::json!({
            "image": "nginx",
            "auth": {
                "type": "basic",
                "username": "user",
                "password": "real_password"
            }
        });
        ProviderRegistry::restore_config_secrets_str("docker_registry", &mut incoming, &existing);
        assert_eq!(incoming["auth"]["password"], "real_password");
    }

    #[test]
    fn restore_docker_registry_masked_token() {
        let mut incoming = serde_json::json!({
            "image": "nginx",
            "auth": {
                "type": "bearer",
                "token": "***"
            }
        });
        let existing = serde_json::json!({
            "image": "nginx",
            "auth": {
                "type": "bearer",
                "token": "real_token"
            }
        });
        ProviderRegistry::restore_config_secrets_str("docker_registry", &mut incoming, &existing);
        assert_eq!(incoming["auth"]["token"], "real_token");
    }

    #[test]
    fn restore_docker_registry_new_password_not_masked() {
        let mut incoming = serde_json::json!({
            "image": "nginx",
            "auth": {
                "type": "basic",
                "username": "user",
                "password": "new_password"
            }
        });
        let existing = serde_json::json!({
            "image": "nginx",
            "auth": {
                "type": "basic",
                "username": "user",
                "password": "old_password"
            }
        });
        ProviderRegistry::restore_config_secrets_str("docker_registry", &mut incoming, &existing);
        assert_eq!(incoming["auth"]["password"], "new_password");
    }

    #[test]
    fn validate_valid_docker_registry_config() {
        let config = serde_json::json!({
            "image": "nginx"
        });
        assert!(ProviderRegistry::validate_config_str("docker_registry", &config).is_ok());
    }

    #[test]
    fn validate_docker_registry_config_full() {
        let config = serde_json::json!({
            "image": "ghcr.io/owner/repo",
            "registry": "ghcr.io",
            "tracking_mode": "digest_tracking",
            "tracked_tag": "main",
            "auth": {
                "type": "bearer",
                "token": "ghcr_token"
            }
        });
        assert!(ProviderRegistry::validate_config_str("docker_registry", &config).is_ok());
    }

    #[test]
    fn validate_invalid_docker_registry_config_empty_image() {
        let config = serde_json::json!({
            "image": ""
        });
        assert!(ProviderRegistry::validate_config_str("docker_registry", &config).is_err());
    }

    #[test]
    fn validate_invalid_docker_registry_config_bad_regex() {
        let config = serde_json::json!({
            "image": "nginx",
            "tag_patterns": ["[invalid"]
        });
        assert!(ProviderRegistry::validate_config_str("docker_registry", &config).is_err());
    }
}
