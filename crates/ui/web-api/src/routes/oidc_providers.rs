use crate::auth::token::generate_uuid;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::permission::{CanManageAuthSettings, CanViewSettings};
use crate::tenant_db::TenantDb;
use axum::{
    Json,
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{ActiveModelTrait, ColumnTrait, QueryFilter, QueryOrder, Set};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{oidc_provider, oidc_provider::RoleMapping};

use crate::auth::AuthMethod;
use uuid::Uuid;

pub use uptrakit_web_api_types::oidc_providers::{
    CreateOidcProviderRequest, OidcProviderResponse, UpdateOidcProviderRequest,
};

fn effective_allow_private_network_issuers(
    stored_value: bool,
    multi_tenancy_enabled: bool,
) -> bool {
    stored_value && !multi_tenancy_enabled
}

fn resolve_allow_private_network_issuers_for_create(
    requested: Option<bool>,
    multi_tenancy_enabled: bool,
) -> Result<bool, &'static str> {
    match (requested, multi_tenancy_enabled) {
        (Some(true), true) => {
            Err("Private-network OIDC issuers are not allowed in multi-tenant mode")
        }
        (Some(value), _) => Ok(value),
        (None, true) => Ok(false),
        (None, false) => Ok(true),
    }
}

fn resolve_allow_private_network_issuers_for_update(
    requested: Option<bool>,
    multi_tenancy_enabled: bool,
) -> Result<Option<bool>, &'static str> {
    match (requested, multi_tenancy_enabled) {
        (Some(true), true) => {
            Err("Private-network OIDC issuers are not allowed in multi-tenant mode")
        }
        _ => Ok(requested),
    }
}

fn oidc_provider_response_from(
    m: oidc_provider::Model,
    multi_tenancy_enabled: bool,
) -> OidcProviderResponse {
    OidcProviderResponse {
        id: m.id,
        name: m.name,
        slug: m.slug,
        logo_url: m.logo_url,
        issuer_url: m.issuer_url,
        client_id: m.client_id,
        has_client_secret: !m.client_secret.expose_secret().is_empty(),
        scopes: m.scopes,
        auto_create_users: m.auto_create_users,
        allow_private_network_issuers: effective_allow_private_network_issuers(
            m.allow_private_network_issuers,
            multi_tenancy_enabled,
        ),
        role_claim_path: m.role_claim_path,
        role_mapping: m.role_mapping.0,
        is_active: m.is_active,
        created_at: m.created_at,
        updated_at: m.updated_at,
    }
}

/// Create a new OIDC provider (inactive by default)
#[utoipa::path(
    post,
    path = "/api/v1/settings/oidc-providers",
    request_body = CreateOidcProviderRequest,
    extensions(("x-required-permission" = json!("manage_auth_settings"))),
    responses(
        (status = 201, description = "Provider created", body = OidcProviderResponse),
        (status = 400, description = "Invalid input"),
        (status = 409, description = "Slug already exists")
    ),
    tag = "OIDC Providers",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn create_provider(
    tenant_db: TenantDb,
    CanManageAuthSettings(_user): CanManageAuthSettings,
    Validated(req): Validated<CreateOidcProviderRequest>,
) -> Response {
    let multi_tenancy_enabled =
        match crate::settings_store::is_multi_tenancy_enabled(tenant_db.db()).await {
            Ok(enabled) => enabled,
            Err(e) => {
                tracing::error!("Failed to load multi-tenancy mode: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };
    let allow_private_network_issuers = match resolve_allow_private_network_issuers_for_create(
        req.allow_private_network_issuers,
        multi_tenancy_enabled,
    ) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::BAD_REQUEST, message),
    };

    // Check slug uniqueness among non-deleted providers within tenant
    let existing = tenant_db
        .find::<oidc_provider::Entity>()
        .filter(oidc_provider::Column::Slug.eq(&req.slug))
        .filter(oidc_provider::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await;

    if let Ok(Some(_)) = existing {
        return error_response(StatusCode::CONFLICT, "Slug already exists");
    }

    let encrypted_secret = match uptrakit_crypto::EncryptedString::new(
        req.client_secret.expose_secret().to_string(),
        "uptrakit:oidc_providers:client_secret",
    ) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("encryption failed: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let now = OffsetDateTime::now_utc();
    let provider = oidc_provider::ActiveModel {
        id: Set(generate_uuid()),
        tenant_id: Set(tenant_db.tenant_id),
        name: Set(req.name),
        slug: Set(req.slug),
        logo_url: Set(req.logo_url),
        issuer_url: Set(req.issuer_url),
        client_id: Set(req.client_id),
        client_secret: Set(encrypted_secret),
        scopes: Set(req.scopes),
        auto_create_users: Set(req.auto_create_users),
        allow_private_network_issuers: Set(allow_private_network_issuers),
        role_claim_path: Set(req.role_claim_path),
        role_mapping: Set(RoleMapping(req.role_mapping)),
        is_active: Set(false),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    };

    match provider.insert(tenant_db.db()).await {
        Ok(model) => (
            StatusCode::CREATED,
            Json(oidc_provider_response_from(model, multi_tenancy_enabled)),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to create OIDC provider: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// List all non-deleted OIDC providers
#[utoipa::path(
    get,
    path = "/api/v1/settings/oidc-providers",
    extensions(("x-required-permission" = json!("view_settings"))),
    responses(
        (status = 200, description = "List of OIDC providers", body = Vec<OidcProviderResponse>),
    ),
    tag = "OIDC Providers",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_providers(
    tenant_db: TenantDb,
    CanViewSettings(_user): CanViewSettings,
) -> Response {
    let multi_tenancy_enabled =
        match crate::settings_store::is_multi_tenancy_enabled(tenant_db.db()).await {
            Ok(enabled) => enabled,
            Err(e) => {
                tracing::error!("Failed to load multi-tenancy mode: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };
    match tenant_db
        .find::<oidc_provider::Entity>()
        .filter(oidc_provider::Column::DeactivatedAt.is_null())
        .order_by_asc(oidc_provider::Column::Name)
        .all(tenant_db.db())
        .await
    {
        Ok(providers) => {
            let resp: Vec<OidcProviderResponse> = providers
                .into_iter()
                .map(|provider| oidc_provider_response_from(provider, multi_tenancy_enabled))
                .collect();
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to list OIDC providers: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Get a single OIDC provider
#[utoipa::path(
    get,
    path = "/api/v1/settings/oidc-providers/{id}",
    params(("id" = Uuid, Path, description = "Provider ID")),
    extensions(("x-required-permission" = json!("view_settings"))),
    responses(
        (status = 200, description = "Provider details", body = OidcProviderResponse),
        (status = 404, description = "Provider not found")
    ),
    tag = "OIDC Providers",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_provider(
    tenant_db: TenantDb,
    Path(provider_id): Path<Uuid>,
    CanViewSettings(_user): CanViewSettings,
) -> Response {
    let multi_tenancy_enabled =
        match crate::settings_store::is_multi_tenancy_enabled(tenant_db.db()).await {
            Ok(enabled) => enabled,
            Err(e) => {
                tracing::error!("Failed to load multi-tenancy mode: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };
    match find_non_deleted_provider(&tenant_db, provider_id).await {
        Some(provider) => (
            StatusCode::OK,
            Json(oidc_provider_response_from(provider, multi_tenancy_enabled)),
        )
            .into_response(),
        None => error_response(StatusCode::NOT_FOUND, "Provider not found"),
    }
}

/// Update an OIDC provider (partial update)
#[utoipa::path(
    put,
    path = "/api/v1/settings/oidc-providers/{id}",
    params(("id" = Uuid, Path, description = "Provider ID")),
    request_body = UpdateOidcProviderRequest,
    extensions(("x-required-permission" = json!("manage_auth_settings"))),
    responses(
        (status = 200, description = "Provider updated", body = OidcProviderResponse),
        (status = 404, description = "Provider not found")
    ),
    tag = "OIDC Providers",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_provider(
    tenant_db: TenantDb,
    Path(provider_id): Path<Uuid>,
    CanManageAuthSettings(_user): CanManageAuthSettings,
    Json(req): Json<UpdateOidcProviderRequest>,
) -> Response {
    let multi_tenancy_enabled =
        match crate::settings_store::is_multi_tenancy_enabled(tenant_db.db()).await {
            Ok(enabled) => enabled,
            Err(e) => {
                tracing::error!("Failed to load multi-tenancy mode: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };
    let provider = match find_non_deleted_provider(&tenant_db, provider_id).await {
        Some(p) => p,
        None => return error_response(StatusCode::NOT_FOUND, "Provider not found"),
    };

    // Check slug uniqueness if changing
    if let Some(ref new_slug) = req.slug
        && *new_slug != provider.slug
    {
        let existing = tenant_db
            .find::<oidc_provider::Entity>()
            .filter(oidc_provider::Column::Slug.eq(new_slug.as_str()))
            .filter(oidc_provider::Column::DeactivatedAt.is_null())
            .filter(oidc_provider::Column::Id.ne(provider_id))
            .one(tenant_db.db())
            .await;
        if let Ok(Some(_)) = existing {
            return error_response(StatusCode::CONFLICT, "Slug already exists");
        }
    }

    let now = OffsetDateTime::now_utc();
    let mut model: oidc_provider::ActiveModel = provider.into();
    let allow_private_network_issuers = match resolve_allow_private_network_issuers_for_update(
        req.allow_private_network_issuers,
        multi_tenancy_enabled,
    ) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::BAD_REQUEST, message),
    };

    if let Some(name) = req.name {
        model.name = Set(name);
    }
    if let Some(slug) = req.slug {
        model.slug = Set(slug);
    }
    if let Some(logo_url) = req.logo_url {
        model.logo_url = Set(Some(logo_url));
    }
    if let Some(issuer_url) = req.issuer_url {
        model.issuer_url = Set(issuer_url);
    }
    if let Some(client_id) = req.client_id {
        model.client_id = Set(client_id);
    }
    if let Some(client_secret) = req.client_secret {
        let encrypted_secret = match uptrakit_crypto::EncryptedString::new(
            client_secret.expose_secret().to_string(),
            "uptrakit:oidc_providers:client_secret",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("encryption failed: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };
        model.client_secret = Set(encrypted_secret);
    }
    if let Some(scopes) = req.scopes {
        model.scopes = Set(scopes);
    }
    if let Some(auto_create_users) = req.auto_create_users {
        model.auto_create_users = Set(auto_create_users);
    }
    if let Some(allow_private_network_issuers) = allow_private_network_issuers {
        model.allow_private_network_issuers = Set(allow_private_network_issuers);
    }
    if let Some(role_claim_path) = req.role_claim_path {
        model.role_claim_path = Set(Some(role_claim_path));
    }
    if let Some(role_mapping) = req.role_mapping {
        model.role_mapping = Set(RoleMapping(role_mapping));
    }
    model.updated_at = Set(now);

    match model.update(tenant_db.db()).await {
        Ok(updated) => (
            StatusCode::OK,
            Json(oidc_provider_response_from(updated, multi_tenancy_enabled)),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to update OIDC provider: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Soft-delete an OIDC provider
#[utoipa::path(
    delete,
    path = "/api/v1/settings/oidc-providers/{id}",
    params(("id" = Uuid, Path, description = "Provider ID")),
    extensions(("x-required-permission" = json!("manage_auth_settings"))),
    responses(
        (status = 204, description = "Provider deleted"),
        (status = 404, description = "Provider not found"),
        (status = 409, description = "Cannot delete: safety check failed")
    ),
    tag = "OIDC Providers",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn delete_provider(
    tenant_db: TenantDb,
    Path(provider_id): Path<Uuid>,
    CanManageAuthSettings(user): CanManageAuthSettings,
) -> Response {
    let provider = match find_non_deleted_provider(&tenant_db, provider_id).await {
        Some(p) => p,
        None => return error_response(StatusCode::NOT_FOUND, "Provider not found"),
    };

    // Safety: cannot soft-delete if admin is logged in via this provider
    if let AuthMethod::Oidc {
        provider_id: session_pid,
    } = &user.auth_method
        && *session_pid == provider_id
        && provider.is_active
    {
        return error_response(
            StatusCode::CONFLICT,
            "Cannot delete the OIDC provider used by your current session",
        );
    }

    let now = OffsetDateTime::now_utc();
    let mut model: oidc_provider::ActiveModel = provider.into();
    model.deactivated_at = Set(Some(now));
    model.is_active = Set(false);
    model.updated_at = Set(now);

    match model.update(tenant_db.db()).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!("Failed to soft-delete OIDC provider: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Activate an OIDC provider (deactivates all others)
#[utoipa::path(
    post,
    path = "/api/v1/settings/oidc-providers/{id}/activate",
    params(("id" = Uuid, Path, description = "Provider ID")),
    extensions(("x-required-permission" = json!("manage_auth_settings"))),
    responses(
        (status = 200, description = "Provider activated", body = OidcProviderResponse),
        (status = 404, description = "Provider not found"),
        (status = 409, description = "Cannot activate: provider is deleted or config incomplete")
    ),
    tag = "OIDC Providers",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn activate_provider(
    tenant_db: TenantDb,
    Path(provider_id): Path<Uuid>,
    CanManageAuthSettings(_user): CanManageAuthSettings,
) -> Response {
    let multi_tenancy_enabled =
        match crate::settings_store::is_multi_tenancy_enabled(tenant_db.db()).await {
            Ok(enabled) => enabled,
            Err(e) => {
                tracing::error!("Failed to load multi-tenancy mode: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };
    let provider = match find_non_deleted_provider(&tenant_db, provider_id).await {
        Some(p) => p,
        None => return error_response(StatusCode::NOT_FOUND, "Provider not found"),
    };

    // Validate config completeness
    if provider.issuer_url.is_empty()
        || provider.client_id.is_empty()
        || provider.client_secret.expose_secret().is_empty()
    {
        return error_response(StatusCode::CONFLICT, "Provider configuration is incomplete");
    }

    let now = OffsetDateTime::now_utc();

    // Deactivate all other providers within the tenant
    let all_active = tenant_db
        .find::<oidc_provider::Entity>()
        .filter(oidc_provider::Column::IsActive.eq(true))
        .filter(oidc_provider::Column::Id.ne(provider_id))
        .filter(oidc_provider::Column::DeactivatedAt.is_null())
        .all(tenant_db.db())
        .await;

    if let Ok(others) = all_active {
        for other in others {
            let mut m: oidc_provider::ActiveModel = other.into();
            m.is_active = Set(false);
            m.updated_at = Set(now);
            let _ = m.update(tenant_db.db()).await;
        }
    }

    // Activate this provider
    let mut model: oidc_provider::ActiveModel = provider.into();
    model.is_active = Set(true);
    model.updated_at = Set(now);

    match model.update(tenant_db.db()).await {
        Ok(updated) => (
            StatusCode::OK,
            Json(oidc_provider_response_from(updated, multi_tenancy_enabled)),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to activate OIDC provider: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Deactivate an OIDC provider
#[utoipa::path(
    post,
    path = "/api/v1/settings/oidc-providers/{id}/deactivate",
    params(("id" = Uuid, Path, description = "Provider ID")),
    extensions(("x-required-permission" = json!("manage_auth_settings"))),
    responses(
        (status = 200, description = "Provider deactivated", body = OidcProviderResponse),
        (status = 404, description = "Provider not found"),
        (status = 409, description = "Cannot deactivate: safety check failed")
    ),
    tag = "OIDC Providers",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn deactivate_provider(
    tenant_db: TenantDb,
    Path(provider_id): Path<Uuid>,
    CanManageAuthSettings(user): CanManageAuthSettings,
) -> Response {
    let multi_tenancy_enabled =
        match crate::settings_store::is_multi_tenancy_enabled(tenant_db.db()).await {
            Ok(enabled) => enabled,
            Err(e) => {
                tracing::error!("Failed to load multi-tenancy mode: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };
    let provider = match find_non_deleted_provider(&tenant_db, provider_id).await {
        Some(p) => p,
        None => return error_response(StatusCode::NOT_FOUND, "Provider not found"),
    };

    // Safety: cannot deactivate if admin's session is via this provider
    if let AuthMethod::Oidc {
        provider_id: session_pid,
    } = &user.auth_method
        && *session_pid == provider_id
    {
        return error_response(
            StatusCode::CONFLICT,
            "Cannot deactivate the OIDC provider used by your current session",
        );
    }

    let now = OffsetDateTime::now_utc();
    let mut model: oidc_provider::ActiveModel = provider.into();
    model.is_active = Set(false);
    model.updated_at = Set(now);

    match model.update(tenant_db.db()).await {
        Ok(updated) => (
            StatusCode::OK,
            Json(oidc_provider_response_from(updated, multi_tenancy_enabled)),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to deactivate OIDC provider: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

async fn find_non_deleted_provider(
    tenant_db: &TenantDb,
    id: uuid::Uuid,
) -> Option<oidc_provider::Model> {
    tenant_db
        .find_by_id::<oidc_provider::Entity, _>(id)
        .filter(oidc_provider::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::{
        effective_allow_private_network_issuers, resolve_allow_private_network_issuers_for_create,
        resolve_allow_private_network_issuers_for_update,
    };

    #[test]
    fn create_defaults_to_true_in_single_tenant_mode() {
        let result = resolve_allow_private_network_issuers_for_create(None, false)
            .expect("single-tenant create should default to true");
        assert!(result);
    }

    #[test]
    fn create_defaults_to_false_in_multi_tenant_mode() {
        let result = resolve_allow_private_network_issuers_for_create(None, true)
            .expect("multi-tenant create should default to false");
        assert!(!result);
    }

    #[test]
    fn create_rejects_explicit_true_in_multi_tenant_mode() {
        let error = resolve_allow_private_network_issuers_for_create(Some(true), true)
            .expect_err("multi-tenant mode must reject explicit private-network allowance");
        assert!(error.contains("multi-tenant"));
    }

    #[test]
    fn update_rejects_explicit_true_in_multi_tenant_mode() {
        let error = resolve_allow_private_network_issuers_for_update(Some(true), true)
            .expect_err("multi-tenant mode must reject explicit private-network allowance");
        assert!(error.contains("multi-tenant"));
    }

    #[test]
    fn update_allows_false_in_multi_tenant_mode() {
        let result = resolve_allow_private_network_issuers_for_update(Some(false), true)
            .expect("multi-tenant mode should allow explicit false");
        assert_eq!(result, Some(false));
    }

    #[test]
    fn effective_value_is_forced_false_in_multi_tenant_mode() {
        assert!(!effective_allow_private_network_issuers(true, true));
        assert!(!effective_allow_private_network_issuers(false, true));
    }

    #[test]
    fn effective_value_matches_stored_flag_in_single_tenant_mode() {
        assert!(effective_allow_private_network_issuers(true, false));
        assert!(!effective_allow_private_network_issuers(false, false));
    }
}
