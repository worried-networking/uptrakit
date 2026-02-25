use crate::AppState;
use crate::error_response::error_response;
use crate::middleware::permission::{CanManageSoftware, CanViewSoftware};
use crate::queries::autodiscovery as autodiscovery_queries;
use crate::queries::provider_configs::{self as pc_queries, UpdateProviderConfigError};
use crate::tenant_db::TenantDb;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{
    ColumnTrait, EntityTrait, FromQueryResult, JoinType, QueryFilter, QuerySelect, RelationTrait,
};
use std::sync::Arc;
use uptrakit_shared_db::entity::{host, plugin_config as provider_config, prelude::*, service, service_host};
use uptrakit_web_api_types::autodiscovery::{DiscardDiscoveredResponse, TriggerDiscoveryResponse};
use uptrakit_web_api_types::validation::Validate;

pub use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
pub use uptrakit_web_api_types::provider_configs::{
    CreateProviderConfigRequest, ProviderConfigResponse, UpdateProviderConfigRequest,
};

/// Create a new provider configuration.
#[utoipa::path(
    post,
    path = "/api/v1/provider-configs",
    request_body = CreateProviderConfigRequest,
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 201, description = "Provider config created", body = ProviderConfigResponse),
        (status = 400, description = "Invalid input"),
        (status = 409, description = "A provider config with this name already exists")
    ),
    tag = "Provider Configs",
    security(("bearer_token" = []))
)]
pub async fn create_provider_config(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Json(req): Json<CreateProviderConfigRequest>,
) -> Response {
    if let Err(e) = req.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    match pc_queries::create_provider_config(state.provider_ops.as_ref(), &tenant_db, req).await {
        Ok(resp) => (StatusCode::CREATED, Json(resp)).into_response(),
        Err(pc_queries::CreateProviderConfigError::DuplicateName) => error_response(
            StatusCode::CONFLICT,
            "A provider config with this name already exists",
        ),
        Err(pc_queries::CreateProviderConfigError::Db(e)) => {
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
    extensions(("x-required-permission" = json!("view_software"))),
    responses(
        (status = 200, description = "Paginated list of provider configs", body = PaginatedResponse<ProviderConfigResponse>),
    ),
    tag = "Provider Configs",
    security(("bearer_token" = []))
)]
pub async fn list_provider_configs(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
    Query(params): Query<PaginationParams>,
) -> Response {
    match pc_queries::list_provider_configs(state.provider_ops.as_ref(), &tenant_db, &params).await
    {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
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
    extensions(("x-required-permission" = json!("view_software"))),
    responses(
        (status = 200, description = "Provider config details", body = ProviderConfigResponse),
        (status = 404, description = "Provider config not found")
    ),
    tag = "Provider Configs",
    security(("bearer_token" = []))
)]
pub async fn get_provider_config(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    Path(id): Path<String>,
    CanViewSoftware(_user): CanViewSoftware,
) -> Response {
    let config_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid UUID"),
    };

    match pc_queries::get_provider_config(state.provider_ops.as_ref(), &tenant_db, config_id).await
    {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Provider config not found"),
        Err(e) => {
            tracing::error!("Failed to get provider config: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Update a provider configuration (partial update).
#[utoipa::path(
    put,
    path = "/api/v1/provider-configs/{id}",
    params(("id" = String, Path, description = "Provider config ID")),
    request_body = UpdateProviderConfigRequest,
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 200, description = "Provider config updated", body = ProviderConfigResponse),
        (status = 404, description = "Provider config not found")
    ),
    tag = "Provider Configs",
    security(("bearer_token" = []))
)]
pub async fn update_provider_config(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    Path(id): Path<String>,
    CanManageSoftware(_user): CanManageSoftware,
    Json(req): Json<UpdateProviderConfigRequest>,
) -> Response {
    let config_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid UUID"),
    };

    match pc_queries::update_provider_config(
        state.provider_ops.as_ref(),
        &tenant_db,
        config_id,
        req,
    )
    .await
    {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(UpdateProviderConfigError::NotFound) => {
            error_response(StatusCode::NOT_FOUND, "Provider config not found")
        }
        Err(UpdateProviderConfigError::EmptyName) => {
            error_response(StatusCode::BAD_REQUEST, "name must not be empty")
        }
        Err(UpdateProviderConfigError::ConfigValidation(msg)) => {
            error_response(StatusCode::BAD_REQUEST, msg)
        }
        Err(UpdateProviderConfigError::HookValidation(msg)) => {
            error_response(StatusCode::BAD_REQUEST, msg)
        }
        Err(UpdateProviderConfigError::Db(e)) => {
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
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 204, description = "Provider config deleted"),
        (status = 404, description = "Provider config not found")
    ),
    tag = "Provider Configs",
    security(("bearer_token" = []))
)]
pub async fn delete_provider_config(
    tenant_db: TenantDb,
    Path(id): Path<String>,
    CanManageSoftware(_user): CanManageSoftware,
) -> Response {
    let config_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid UUID"),
    };

    match pc_queries::delete_provider_config(&tenant_db, config_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "Provider config not found"),
        Err(e) => {
            tracing::error!("Failed to delete provider config: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

// ── Autodiscovery endpoints ───────────────────────────────────────────────────

/// Trigger autodiscovery for a specific provider configuration.
///
/// Sends a `DiscoverSoftware` assignment to all connected agents.
/// Returns an error if the provider type does not support discovery.
#[utoipa::path(
    post,
    path = "/api/v1/provider-configs/{id}/discover",
    params(("id" = String, Path, description = "Provider config UUID")),
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 200, description = "Discovery triggered", body = TriggerDiscoveryResponse),
        (status = 400, description = "Provider type does not support discovery"),
        (status = 404, description = "Provider config not found")
    ),
    tag = "Provider Configs",
    security(("bearer_token" = []))
)]
pub async fn discover_provider_config(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path(id): Path<String>,
) -> Response {
    let config_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid UUID"),
    };

    // Load the provider config and verify it belongs to the tenant.
    let cfg = match ProviderConfig::find_by_id(config_id)
        .filter(provider_config::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(provider_config::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(c)) => c,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Provider config not found"),
        Err(e) => {
            tracing::error!("DB error: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Validate provider supports discovery.
    let provider_type: uptrakit_internal_wire::PluginType = match cfg.plugin_type.parse() {
        Ok(pt) => pt,
        Err(_) => {
            return error_response(StatusCode::BAD_REQUEST, "Unknown provider type");
        }
    };

    if !state
        .provider_ops
        .discovery_plugin_types()
        .contains(&provider_type)
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!(
                "Provider type '{}' does not support autodiscovery",
                cfg.plugin_type
            ),
        );
    }

    // Single JOIN query: service_host → service (tenant-scoped) → host
    // This prevents cross-tenant data leaks and eliminates the N+1 pattern.
    #[derive(FromQueryResult)]
    struct AgentHostRow {
        service_id: uuid::Uuid,
        machine_id: String,
    }

    let rows: Vec<AgentHostRow> = match tenant_db
        .find_via_tenant_join::<service_host::Entity, service::Entity>(
            service_host::Relation::Service.def(),
        )
        .join(JoinType::InnerJoin, service_host::Relation::Host.def())
        .select_only()
        .column(service_host::Column::ServiceId)
        .column(host::Column::MachineId)
        .filter(service::Column::DeactivatedAt.is_null())
        .filter(host::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .into_model::<AgentHostRow>()
        .all(tenant_db.db())
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to query service-host links: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Group machine_ids by service_id.
    let mut by_service: std::collections::HashMap<uuid::Uuid, Vec<String>> =
        std::collections::HashMap::new();
    for row in rows {
        by_service
            .entry(row.service_id)
            .or_default()
            .push(row.machine_id);
    }

    let agents_notified = by_service.len() as u32;

    // One DiscoverSoftware message per (agent, host) pair.
    for (agent_id, machine_ids) in &by_service {
        for machine_id in machine_ids {
            let msg = uptrakit_internal_wire::ControllerMessage::DiscoverSoftware(
                uptrakit_internal_wire::DiscoverSoftwarePayload {
                    host_machine_id: machine_id.clone(),
                    providers: vec![uptrakit_internal_wire::DiscoveryPluginAssignment {
                        plugin_config_id: Some(cfg.id),
                        plugin_type: provider_type.clone(),
                        config: cfg.config.clone(),
                    }],
                },
            );
            state.notification_service.send(agent_id, msg).await;
        }
    }

    (
        StatusCode::OK,
        Json(TriggerDiscoveryResponse {
            providers_queued: agents_notified,
            message: format!(
                "Discovery triggered for provider config '{}' on {} agent(s)",
                cfg.name, agents_notified
            ),
        }),
    )
        .into_response()
}

/// Bulk-discard all pending discovered software items for a provider configuration.
///
/// No autodiscovery ignore rules are created.
#[utoipa::path(
    delete,
    path = "/api/v1/provider-configs/{id}/discovered",
    params(("id" = String, Path, description = "Provider config UUID")),
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 200, description = "Pending items discarded", body = DiscardDiscoveredResponse),
        (status = 404, description = "Provider config not found")
    ),
    tag = "Provider Configs",
    security(("bearer_token" = []))
)]
pub async fn discard_provider_config_discovered(
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path(id): Path<String>,
) -> Response {
    let config_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid UUID"),
    };

    // Verify provider config belongs to tenant.
    let exists = match ProviderConfig::find_by_id(config_id)
        .filter(provider_config::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(provider_config::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(e) => {
            tracing::error!("DB error: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if !exists {
        return error_response(StatusCode::NOT_FOUND, "Provider config not found");
    }

    match autodiscovery_queries::discard_pending_items(
        tenant_db.db(),
        tenant_db.tenant_id,
        None,
        Some(config_id),
    )
    .await
    {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!("Failed to discard pending items: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_plugin_registry::PluginRegistry;

    /// Sentinel value used to indicate a masked secret in API responses.
    const SECRET_MASK: &str = "***";

    #[test]
    fn mask_github_auth_token() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world",
            "auth_token": "ghp_secret123"
        });
        let masked = PluginRegistry::mask_config_secrets_str("github_releases", &config);
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
        let masked = PluginRegistry::mask_config_secrets_str("github_releases", &config);
        // with_secrets_masked always sets auth_token to "***"
        assert_eq!(masked["auth_token"], SECRET_MASK);
    }

    #[test]
    fn mask_without_token_field_adds_masked() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        let masked = PluginRegistry::mask_config_secrets_str("github_releases", &config);
        // with_secrets_masked always adds auth_token as "***"
        assert_eq!(masked["auth_token"], SECRET_MASK);
    }

    #[test]
    fn mask_unknown_provider_type() {
        let config = serde_json::json!({"key": "value"});
        let masked = PluginRegistry::mask_config_secrets_str("unknown_type", &config);
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
        PluginRegistry::restore_config_secrets_str("github_releases", &mut incoming, &existing);
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
        PluginRegistry::restore_config_secrets_str("github_releases", &mut incoming, &existing);
        assert_eq!(incoming["auth_token"], "ghp_new_token");
    }

    #[test]
    fn validate_valid_github_config() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        assert!(PluginRegistry::validate_config_str("github_releases", &config).is_ok());
    }

    #[test]
    fn validate_invalid_github_config() {
        let config = serde_json::json!({
            "owner": "",
            "repo": "hello-world"
        });
        assert!(PluginRegistry::validate_config_str("github_releases", &config).is_err());
    }

    #[test]
    fn validate_unknown_provider_type() {
        let config = serde_json::json!({});
        assert!(PluginRegistry::validate_config_str("nonexistent", &config).is_err());
    }

    #[test]
    fn parse_known_provider_types() {
        let github_config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        assert!(PluginRegistry::validate_config_str("github_releases", &github_config).is_ok());

        let proxmox_config = serde_json::json!({
            "script_url": "https://example.com/update.sh"
        });
        assert!(
            PluginRegistry::validate_config_str("proxmox_helper_scripts", &proxmox_config)
                .is_ok()
        );

        let docker_config = serde_json::json!({});
        assert!(PluginRegistry::validate_config_str("docker", &docker_config).is_ok());

        let homebrew_config = serde_json::json!({});
        assert!(PluginRegistry::validate_config_str("homebrew", &homebrew_config).is_ok());

        assert!(PluginRegistry::validate_config_str("unknown", &homebrew_config).is_err());
    }

    // --- Homebrew provider tests ---

    #[test]
    fn validate_valid_homebrew_config() {
        let config = serde_json::json!({});
        assert!(PluginRegistry::validate_config_str("homebrew", &config).is_ok());
    }

    #[test]
    fn validate_homebrew_config_with_cask() {
        let config = serde_json::json!({"package_type": "cask"});
        assert!(PluginRegistry::validate_config_str("homebrew", &config).is_ok());
    }

    #[test]
    fn mask_homebrew_config_unchanged() {
        let config = serde_json::json!({"package_type": "formula"});
        let masked = PluginRegistry::mask_config_secrets_str("homebrew", &config);
        // No secrets to mask — config returned unchanged
        assert_eq!(masked, config);
    }

    // --- Docker provider tests ---

    #[test]
    fn mask_docker_basic_password() {
        let config = serde_json::json!({
            "auth": {
                "type": "basic",
                "username": "user",
                "password": "secret123"
            }
        });
        let masked = PluginRegistry::mask_config_secrets_str("docker", &config);
        assert_eq!(masked["auth"]["password"], SECRET_MASK);
        assert_eq!(masked["auth"]["username"], "user");
    }

    #[test]
    fn mask_docker_bearer_token() {
        let config = serde_json::json!({
            "auth": {
                "type": "bearer",
                "token": "ghcr_token_secret"
            }
        });
        let masked = PluginRegistry::mask_config_secrets_str("docker", &config);
        assert_eq!(masked["auth"]["token"], SECRET_MASK);
    }

    #[test]
    fn mask_docker_no_auth() {
        let config = serde_json::json!({});
        let masked = PluginRegistry::mask_config_secrets_str("docker", &config);
        // None auth stays absent (serialized with skip_serializing_if)
        assert!(masked.get("auth").is_none());
    }

    #[test]
    fn mask_docker_null_auth() {
        let config = serde_json::json!({ "auth": null });
        let masked = PluginRegistry::mask_config_secrets_str("docker", &config);
        // JSON null deserializes to None, which stays absent after masking
        assert!(masked.get("auth").is_none());
    }

    #[test]
    fn restore_docker_masked_password() {
        let mut incoming = serde_json::json!({
            "auth": {
                "type": "basic",
                "username": "user",
                "password": "***"
            }
        });
        let existing = serde_json::json!({
            "auth": {
                "type": "basic",
                "username": "user",
                "password": "real_password"
            }
        });
        PluginRegistry::restore_config_secrets_str("docker", &mut incoming, &existing);
        assert_eq!(incoming["auth"]["password"], "real_password");
    }

    #[test]
    fn restore_docker_masked_token() {
        let mut incoming = serde_json::json!({
            "auth": {
                "type": "bearer",
                "token": "***"
            }
        });
        let existing = serde_json::json!({
            "auth": {
                "type": "bearer",
                "token": "real_token"
            }
        });
        PluginRegistry::restore_config_secrets_str("docker", &mut incoming, &existing);
        assert_eq!(incoming["auth"]["token"], "real_token");
    }

    #[test]
    fn restore_docker_new_password_not_masked() {
        let mut incoming = serde_json::json!({
            "auth": {
                "type": "basic",
                "username": "user",
                "password": "new_password"
            }
        });
        let existing = serde_json::json!({
            "auth": {
                "type": "basic",
                "username": "user",
                "password": "old_password"
            }
        });
        PluginRegistry::restore_config_secrets_str("docker", &mut incoming, &existing);
        assert_eq!(incoming["auth"]["password"], "new_password");
    }

    #[test]
    fn validate_valid_docker_config() {
        // Empty config is valid — no required fields
        let config = serde_json::json!({});
        assert!(PluginRegistry::validate_config_str("docker", &config).is_ok());
    }

    #[test]
    fn validate_docker_config_with_auth() {
        let config = serde_json::json!({
            "tracking_mode": "digest_tracking",
            "tracked_tag": "main",
            "auth": {
                "type": "bearer",
                "token": "ghcr_token"
            }
        });
        assert!(PluginRegistry::validate_config_str("docker", &config).is_ok());
    }

    #[test]
    fn validate_invalid_docker_config_zero_page_size() {
        let config = serde_json::json!({ "page_size": 0 });
        assert!(PluginRegistry::validate_config_str("docker", &config).is_err());
    }

    #[test]
    fn validate_invalid_docker_config_bad_regex() {
        let config = serde_json::json!({
            "tag_patterns": ["[invalid"]
        });
        assert!(PluginRegistry::validate_config_str("docker", &config).is_err());
    }
}
