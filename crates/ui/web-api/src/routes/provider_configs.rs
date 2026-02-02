use crate::AppState;
use crate::auth::permissions::Permission;
use crate::auth::token::generate_uuid;
use crate::middleware::require_auth::AuthenticatedUser;
use crate::middleware::tenant_context::TenantContext;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use std::sync::Arc;
use time::OffsetDateTime;
use uptrakit_provider_core::ProviderType;
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::provider_config;

/// Sentinel value used to indicate a masked secret in API responses.
const SECRET_MASK: &str = "***";

pub use uptrakit_web_api_types::provider_configs::{
    CreateProviderConfigRequest, ProviderConfigResponse, UpdateProviderConfigRequest,
};

fn provider_config_response_from(m: provider_config::Model) -> ProviderConfigResponse {
    ProviderConfigResponse {
        id: m.id.to_string(),
        name: m.name,
        provider_type: m.provider_type.clone(),
        config: mask_secrets(&m.provider_type, &m.config),
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

/// Mask secret fields in provider config JSON before returning to the client.
fn mask_secrets(provider_type: &str, config: &serde_json::Value) -> serde_json::Value {
    let mut masked = config.clone();
    match provider_type {
        "github_releases" => {
            if let Some(obj) = masked.as_object_mut()
                && let Some(token) = obj.get("auth_token")
                && !token.is_null()
            {
                obj.insert(
                    "auth_token".to_string(),
                    serde_json::Value::String(SECRET_MASK.to_string()),
                );
            }
        }
        "docker_registry" => {
            if let Some(obj) = masked.as_object_mut()
                && let Some(auth) = obj.get_mut("auth")
                && let Some(auth_obj) = auth.as_object_mut()
            {
                if let Some(password) = auth_obj.get("password")
                    && !password.is_null()
                {
                    auth_obj.insert(
                        "password".to_string(),
                        serde_json::Value::String(SECRET_MASK.to_string()),
                    );
                }
                if let Some(token) = auth_obj.get("token")
                    && !token.is_null()
                {
                    auth_obj.insert(
                        "token".to_string(),
                        serde_json::Value::String(SECRET_MASK.to_string()),
                    );
                }
            }
        }
        _ => {}
    }
    masked
}

/// Restore preserved secrets from the existing DB value when the client sends the mask sentinel.
fn restore_secrets(
    provider_type: &str,
    incoming: &mut serde_json::Value,
    existing: &serde_json::Value,
) {
    match provider_type {
        "github_releases" => {
            if let (Some(incoming_obj), Some(existing_obj)) =
                (incoming.as_object_mut(), existing.as_object())
                && let Some(token) = incoming_obj.get("auth_token")
                && token.as_str() == Some(SECRET_MASK)
                && let Some(existing_token) = existing_obj.get("auth_token")
            {
                incoming_obj.insert("auth_token".to_string(), existing_token.clone());
            }
        }
        "docker_registry" => {
            if let (Some(incoming_obj), Some(existing_obj)) =
                (incoming.as_object_mut(), existing.as_object())
                && let Some(incoming_auth) = incoming_obj.get_mut("auth")
                && let Some(incoming_auth_obj) = incoming_auth.as_object_mut()
                && let Some(existing_auth) = existing_obj.get("auth")
                && let Some(existing_auth_obj) = existing_auth.as_object()
            {
                if let Some(password) = incoming_auth_obj.get("password")
                    && password.as_str() == Some(SECRET_MASK)
                    && let Some(existing_password) = existing_auth_obj.get("password")
                {
                    incoming_auth_obj.insert("password".to_string(), existing_password.clone());
                }
                if let Some(token) = incoming_auth_obj.get("token")
                    && token.as_str() == Some(SECRET_MASK)
                    && let Some(existing_token) = existing_auth_obj.get("token")
                {
                    incoming_auth_obj.insert("token".to_string(), existing_token.clone());
                }
            }
        }
        _ => {}
    }
}

/// Validate the provider-specific config blob by deserializing and calling `.validate()`.
pub fn validate_provider_config(
    provider_type: &str,
    config: &serde_json::Value,
) -> std::result::Result<(), String> {
    match provider_type {
        "github_releases" => {
            let gh_config: uptrakit_provider_github::GitHubConfig =
                serde_json::from_value(config.clone())
                    .map_err(|e| format!("invalid GitHub config: {e}"))?;
            gh_config
                .validate()
                .map_err(|e| format!("GitHub config validation failed: {e}"))?;
            Ok(())
        }
        "proxmox_helper_scripts" => {
            // No validation yet for this provider type
            Ok(())
        }
        "docker_registry" => {
            let dr_config: uptrakit_provider_docker_registry::DockerRegistryConfig =
                serde_json::from_value(config.clone())
                    .map_err(|e| format!("invalid Docker Registry config: {e}"))?;
            dr_config
                .validate()
                .map_err(|e| format!("Docker Registry config validation failed: {e}"))?;
            Ok(())
        }
        _ => Err(format!("unknown provider_type: {provider_type}")),
    }
}

/// Parse a provider_type string into a `ProviderType`, returning None if unknown.
fn parse_provider_type(s: &str) -> Option<ProviderType> {
    serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
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
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    if req.name.is_empty() {
        return (StatusCode::BAD_REQUEST, "name must not be empty").into_response();
    }

    if parse_provider_type(&req.provider_type).is_none() {
        return (
            StatusCode::BAD_REQUEST,
            format!("unknown provider_type: {}", req.provider_type),
        )
            .into_response();
    }

    if let Err(e) = validate_provider_config(&req.provider_type, &req.config) {
        return (StatusCode::BAD_REQUEST, e).into_response();
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
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// List all non-deactivated provider configurations.
#[utoipa::path(
    get,
    path = "/api/v1/provider-configs",
    responses(
        (status = 200, description = "List of provider configs", body = Vec<ProviderConfigResponse>),
    ),
    tag = "Provider Configs",
    security(("bearer_token" = []))
)]
pub async fn list_provider_configs(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    axum::Extension(user): axum::Extension<AuthenticatedUser>,
) -> Response {
    if !user.has_permission(Permission::ViewSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    match ProviderConfig::find()
        .filter(provider_config::Column::TenantId.eq(tenant.tenant_id))
        .filter(provider_config::Column::DeactivatedAt.is_null())
        .order_by_asc(provider_config::Column::Name)
        .all(&state.db)
        .await
    {
        Ok(configs) => {
            let resp: Vec<ProviderConfigResponse> = configs
                .into_iter()
                .map(provider_config_response_from)
                .collect();
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to list provider configs: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
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
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let config_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid UUID").into_response(),
    };

    match find_active_config(&state.db, tenant.tenant_id, config_id).await {
        Some(config) => {
            (StatusCode::OK, Json(provider_config_response_from(config))).into_response()
        }
        None => (StatusCode::NOT_FOUND, "Provider config not found").into_response(),
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
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let config_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid UUID").into_response(),
    };

    let existing = match find_active_config(&state.db, tenant.tenant_id, config_id).await {
        Some(c) => c,
        None => return (StatusCode::NOT_FOUND, "Provider config not found").into_response(),
    };

    let provider_type = existing.provider_type.clone();

    // Validate new config if provided
    if let Some(ref mut new_config) = req.config.clone() {
        // Restore masked secrets from existing config
        restore_secrets(&provider_type, new_config, &existing.config);

        if let Err(e) = validate_provider_config(&provider_type, new_config) {
            return (StatusCode::BAD_REQUEST, e).into_response();
        }
    }

    let now = OffsetDateTime::now_utc();
    let mut model: provider_config::ActiveModel = existing.into();

    if let Some(name) = req.name {
        if name.is_empty() {
            return (StatusCode::BAD_REQUEST, "name must not be empty").into_response();
        }
        model.name = Set(name);
    }
    if let Some(mut config) = req.config {
        // Re-apply secret restoration on the actual value being saved
        restore_secrets(&provider_type, &mut config, model.config.as_ref());
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
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
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
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let config_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid UUID").into_response(),
    };

    let config = match find_active_config(&state.db, tenant.tenant_id, config_id).await {
        Some(c) => c,
        None => return (StatusCode::NOT_FOUND, "Provider config not found").into_response(),
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
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_github_auth_token() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world",
            "auth_token": "ghp_secret123"
        });
        let masked = mask_secrets("github_releases", &config);
        assert_eq!(masked["auth_token"], SECRET_MASK);
        assert_eq!(masked["owner"], "octocat");
    }

    #[test]
    fn mask_preserves_null_token() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world",
            "auth_token": null
        });
        let masked = mask_secrets("github_releases", &config);
        assert!(masked["auth_token"].is_null());
    }

    #[test]
    fn mask_without_token_field() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        let masked = mask_secrets("github_releases", &config);
        // No auth_token field should be added
        assert!(masked.get("auth_token").is_none());
    }

    #[test]
    fn mask_unknown_provider_type() {
        let config = serde_json::json!({"key": "value"});
        let masked = mask_secrets("unknown_type", &config);
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
        restore_secrets("github_releases", &mut incoming, &existing);
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
        restore_secrets("github_releases", &mut incoming, &existing);
        assert_eq!(incoming["auth_token"], "ghp_new_token");
    }

    #[test]
    fn validate_valid_github_config() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        assert!(validate_provider_config("github_releases", &config).is_ok());
    }

    #[test]
    fn validate_invalid_github_config() {
        let config = serde_json::json!({
            "owner": "",
            "repo": "hello-world"
        });
        assert!(validate_provider_config("github_releases", &config).is_err());
    }

    #[test]
    fn validate_unknown_provider_type() {
        let config = serde_json::json!({});
        assert!(validate_provider_config("nonexistent", &config).is_err());
    }

    #[test]
    fn parse_known_provider_types() {
        assert!(parse_provider_type("github_releases").is_some());
        assert!(parse_provider_type("proxmox_helper_scripts").is_some());
        assert!(parse_provider_type("docker_registry").is_some());
        assert!(parse_provider_type("unknown").is_none());
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
        let masked = mask_secrets("docker_registry", &config);
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
        let masked = mask_secrets("docker_registry", &config);
        assert_eq!(masked["auth"]["token"], SECRET_MASK);
    }

    #[test]
    fn mask_docker_registry_no_auth() {
        let config = serde_json::json!({
            "image": "nginx"
        });
        let masked = mask_secrets("docker_registry", &config);
        assert!(masked.get("auth").is_none());
    }

    #[test]
    fn mask_docker_registry_null_auth() {
        let config = serde_json::json!({
            "image": "nginx",
            "auth": null
        });
        let masked = mask_secrets("docker_registry", &config);
        assert!(masked["auth"].is_null());
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
        restore_secrets("docker_registry", &mut incoming, &existing);
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
        restore_secrets("docker_registry", &mut incoming, &existing);
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
        restore_secrets("docker_registry", &mut incoming, &existing);
        assert_eq!(incoming["auth"]["password"], "new_password");
    }

    #[test]
    fn validate_valid_docker_registry_config() {
        let config = serde_json::json!({
            "image": "nginx"
        });
        assert!(validate_provider_config("docker_registry", &config).is_ok());
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
        assert!(validate_provider_config("docker_registry", &config).is_ok());
    }

    #[test]
    fn validate_invalid_docker_registry_config_empty_image() {
        let config = serde_json::json!({
            "image": ""
        });
        assert!(validate_provider_config("docker_registry", &config).is_err());
    }

    #[test]
    fn validate_invalid_docker_registry_config_bad_regex() {
        let config = serde_json::json!({
            "image": "nginx",
            "tag_patterns": ["[invalid"]
        });
        assert!(validate_provider_config("docker_registry", &config).is_err());
    }
}
