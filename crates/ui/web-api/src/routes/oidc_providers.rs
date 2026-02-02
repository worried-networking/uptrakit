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
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
};
use std::sync::Arc;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::{oidc_provider, oidc_provider::RoleMapping};

pub use uptrakit_web_api_types::oidc_providers::{
    CreateOidcProviderRequest, OidcProviderResponse, UpdateOidcProviderRequest,
};

fn oidc_provider_response_from(m: oidc_provider::Model) -> OidcProviderResponse {
    OidcProviderResponse {
        id: m.id.to_string(),
        name: m.name,
        slug: m.slug,
        logo_url: m.logo_url,
        issuer_url: m.issuer_url,
        client_id: m.client_id,
        has_client_secret: !m.client_secret.is_empty(),
        scopes: m.scopes,
        auto_create_users: m.auto_create_users,
        role_claim_path: m.role_claim_path,
        role_mapping: m.role_mapping.0,
        is_active: m.is_active,
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

/// Create a new OIDC provider (inactive by default)
#[utoipa::path(
    post,
    path = "/api/v1/settings/oidc-providers",
    request_body = CreateOidcProviderRequest,
    responses(
        (status = 201, description = "Provider created", body = OidcProviderResponse),
        (status = 400, description = "Invalid input"),
        (status = 409, description = "Slug already exists")
    ),
    tag = "OIDC Providers",
    security(("bearer_token" = []))
)]
pub async fn create_provider(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    axum::Extension(user): axum::Extension<AuthenticatedUser>,
    Json(req): Json<CreateOidcProviderRequest>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    if req.name.is_empty()
        || req.slug.is_empty()
        || req.issuer_url.is_empty()
        || req.client_id.is_empty()
        || req.client_secret.is_empty()
    {
        return (StatusCode::BAD_REQUEST, "Missing required fields").into_response();
    }

    // Check slug uniqueness among non-deleted providers within tenant
    let existing = OidcProvider::find()
        .filter(oidc_provider::Column::TenantId.eq(tenant.tenant_id))
        .filter(oidc_provider::Column::Slug.eq(&req.slug))
        .filter(oidc_provider::Column::DeletedAt.is_null())
        .one(&state.db)
        .await;

    if let Ok(Some(_)) = existing {
        return (StatusCode::CONFLICT, "Slug already exists").into_response();
    }

    let now = OffsetDateTime::now_utc();
    let provider = oidc_provider::ActiveModel {
        id: Set(generate_uuid()),
        tenant_id: Set(tenant.tenant_id),
        name: Set(req.name),
        slug: Set(req.slug),
        logo_url: Set(req.logo_url),
        issuer_url: Set(req.issuer_url),
        client_id: Set(req.client_id),
        client_secret: Set(req.client_secret),
        scopes: Set(req.scopes),
        auto_create_users: Set(req.auto_create_users),
        role_claim_path: Set(req.role_claim_path),
        role_mapping: Set(RoleMapping(req.role_mapping)),
        is_active: Set(false),
        created_at: Set(now),
        updated_at: Set(now),
        deleted_at: Set(None),
    };

    match provider.insert(&state.db).await {
        Ok(model) => (
            StatusCode::CREATED,
            Json(oidc_provider_response_from(model)),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to create OIDC provider: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// List all non-deleted OIDC providers
#[utoipa::path(
    get,
    path = "/api/v1/settings/oidc-providers",
    responses(
        (status = 200, description = "List of OIDC providers", body = Vec<OidcProviderResponse>),
    ),
    tag = "OIDC Providers",
    security(("bearer_token" = []))
)]
pub async fn list_providers(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    axum::Extension(user): axum::Extension<AuthenticatedUser>,
) -> Response {
    if !user.has_permission(Permission::ViewSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    match OidcProvider::find()
        .filter(oidc_provider::Column::TenantId.eq(tenant.tenant_id))
        .filter(oidc_provider::Column::DeletedAt.is_null())
        .order_by_asc(oidc_provider::Column::Name)
        .all(&state.db)
        .await
    {
        Ok(providers) => {
            let resp: Vec<OidcProviderResponse> = providers
                .into_iter()
                .map(oidc_provider_response_from)
                .collect();
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to list OIDC providers: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Get a single OIDC provider
#[utoipa::path(
    get,
    path = "/api/v1/settings/oidc-providers/{id}",
    params(("id" = String, Path, description = "Provider ID")),
    responses(
        (status = 200, description = "Provider details", body = OidcProviderResponse),
        (status = 404, description = "Provider not found")
    ),
    tag = "OIDC Providers",
    security(("bearer_token" = []))
)]
pub async fn get_provider(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Path(id): Path<String>,
    axum::Extension(user): axum::Extension<AuthenticatedUser>,
) -> Response {
    if !user.has_permission(Permission::ViewSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let provider_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid UUID").into_response(),
    };

    match find_non_deleted_provider(&state.db, tenant.tenant_id, provider_id).await {
        Some(provider) => {
            (StatusCode::OK, Json(oidc_provider_response_from(provider))).into_response()
        }
        None => (StatusCode::NOT_FOUND, "Provider not found").into_response(),
    }
}

/// Update an OIDC provider (partial update)
#[utoipa::path(
    put,
    path = "/api/v1/settings/oidc-providers/{id}",
    params(("id" = String, Path, description = "Provider ID")),
    request_body = UpdateOidcProviderRequest,
    responses(
        (status = 200, description = "Provider updated", body = OidcProviderResponse),
        (status = 404, description = "Provider not found")
    ),
    tag = "OIDC Providers",
    security(("bearer_token" = []))
)]
pub async fn update_provider(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Path(id): Path<String>,
    axum::Extension(user): axum::Extension<AuthenticatedUser>,
    Json(req): Json<UpdateOidcProviderRequest>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let provider_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid UUID").into_response(),
    };

    let provider = match find_non_deleted_provider(&state.db, tenant.tenant_id, provider_id).await {
        Some(p) => p,
        None => return (StatusCode::NOT_FOUND, "Provider not found").into_response(),
    };

    // Check slug uniqueness if changing
    if let Some(ref new_slug) = req.slug
        && *new_slug != provider.slug
    {
        let existing = OidcProvider::find()
            .filter(oidc_provider::Column::TenantId.eq(tenant.tenant_id))
            .filter(oidc_provider::Column::Slug.eq(new_slug.as_str()))
            .filter(oidc_provider::Column::DeletedAt.is_null())
            .filter(oidc_provider::Column::Id.ne(provider_id))
            .one(&state.db)
            .await;
        if let Ok(Some(_)) = existing {
            return (StatusCode::CONFLICT, "Slug already exists").into_response();
        }
    }

    let now = OffsetDateTime::now_utc();
    let mut model: oidc_provider::ActiveModel = provider.into();

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
        model.client_secret = Set(client_secret);
    }
    if let Some(scopes) = req.scopes {
        model.scopes = Set(scopes);
    }
    if let Some(auto_create_users) = req.auto_create_users {
        model.auto_create_users = Set(auto_create_users);
    }
    if let Some(role_claim_path) = req.role_claim_path {
        model.role_claim_path = Set(Some(role_claim_path));
    }
    if let Some(role_mapping) = req.role_mapping {
        model.role_mapping = Set(RoleMapping(role_mapping));
    }
    model.updated_at = Set(now);

    match model.update(&state.db).await {
        Ok(updated) => (StatusCode::OK, Json(oidc_provider_response_from(updated))).into_response(),
        Err(e) => {
            tracing::error!("Failed to update OIDC provider: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Soft-delete an OIDC provider
#[utoipa::path(
    delete,
    path = "/api/v1/settings/oidc-providers/{id}",
    params(("id" = String, Path, description = "Provider ID")),
    responses(
        (status = 204, description = "Provider deleted"),
        (status = 404, description = "Provider not found"),
        (status = 409, description = "Cannot delete: safety check failed")
    ),
    tag = "OIDC Providers",
    security(("bearer_token" = []))
)]
pub async fn delete_provider(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Path(id): Path<String>,
    axum::Extension(user): axum::Extension<AuthenticatedUser>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let provider_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid UUID").into_response(),
    };

    let provider = match find_non_deleted_provider(&state.db, tenant.tenant_id, provider_id).await {
        Some(p) => p,
        None => return (StatusCode::NOT_FOUND, "Provider not found").into_response(),
    };

    // Safety: cannot soft-delete if admin is logged in via this provider
    if let AuthMethod::Oidc {
        provider_id: session_pid,
    } = &user.auth_method
        && *session_pid == provider_id
        && provider.is_active
    {
        return (
            StatusCode::CONFLICT,
            "Cannot delete the OIDC provider used by your current session",
        )
            .into_response();
    }

    let now = OffsetDateTime::now_utc();
    let mut model: oidc_provider::ActiveModel = provider.into();
    model.deleted_at = Set(Some(now));
    model.is_active = Set(false);
    model.updated_at = Set(now);

    match model.update(&state.db).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!("Failed to soft-delete OIDC provider: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Activate an OIDC provider (deactivates all others)
#[utoipa::path(
    post,
    path = "/api/v1/settings/oidc-providers/{id}/activate",
    params(("id" = String, Path, description = "Provider ID")),
    responses(
        (status = 200, description = "Provider activated", body = OidcProviderResponse),
        (status = 404, description = "Provider not found"),
        (status = 409, description = "Cannot activate: provider is deleted or config incomplete")
    ),
    tag = "OIDC Providers",
    security(("bearer_token" = []))
)]
pub async fn activate_provider(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Path(id): Path<String>,
    axum::Extension(user): axum::Extension<AuthenticatedUser>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let provider_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid UUID").into_response(),
    };

    let provider = match find_non_deleted_provider(&state.db, tenant.tenant_id, provider_id).await {
        Some(p) => p,
        None => return (StatusCode::NOT_FOUND, "Provider not found").into_response(),
    };

    // Validate config completeness
    if provider.issuer_url.is_empty()
        || provider.client_id.is_empty()
        || provider.client_secret.is_empty()
    {
        return (StatusCode::CONFLICT, "Provider configuration is incomplete").into_response();
    }

    let now = OffsetDateTime::now_utc();

    // Deactivate all other providers within the tenant
    let all_active = OidcProvider::find()
        .filter(oidc_provider::Column::TenantId.eq(tenant.tenant_id))
        .filter(oidc_provider::Column::IsActive.eq(true))
        .filter(oidc_provider::Column::Id.ne(provider_id))
        .filter(oidc_provider::Column::DeletedAt.is_null())
        .all(&state.db)
        .await;

    if let Ok(others) = all_active {
        for other in others {
            let mut m: oidc_provider::ActiveModel = other.into();
            m.is_active = Set(false);
            m.updated_at = Set(now);
            let _ = m.update(&state.db).await;
        }
    }

    // Activate this provider
    let mut model: oidc_provider::ActiveModel = provider.into();
    model.is_active = Set(true);
    model.updated_at = Set(now);

    match model.update(&state.db).await {
        Ok(updated) => (StatusCode::OK, Json(oidc_provider_response_from(updated))).into_response(),
        Err(e) => {
            tracing::error!("Failed to activate OIDC provider: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Deactivate an OIDC provider
#[utoipa::path(
    post,
    path = "/api/v1/settings/oidc-providers/{id}/deactivate",
    params(("id" = String, Path, description = "Provider ID")),
    responses(
        (status = 200, description = "Provider deactivated", body = OidcProviderResponse),
        (status = 404, description = "Provider not found"),
        (status = 409, description = "Cannot deactivate: safety check failed")
    ),
    tag = "OIDC Providers",
    security(("bearer_token" = []))
)]
pub async fn deactivate_provider(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Path(id): Path<String>,
    axum::Extension(user): axum::Extension<AuthenticatedUser>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let provider_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid UUID").into_response(),
    };

    let provider = match find_non_deleted_provider(&state.db, tenant.tenant_id, provider_id).await {
        Some(p) => p,
        None => return (StatusCode::NOT_FOUND, "Provider not found").into_response(),
    };

    // Safety: cannot deactivate if admin's session is via this provider
    if let AuthMethod::Oidc {
        provider_id: session_pid,
    } = &user.auth_method
        && *session_pid == provider_id
    {
        return (
            StatusCode::CONFLICT,
            "Cannot deactivate the OIDC provider used by your current session",
        )
            .into_response();
    }

    let now = OffsetDateTime::now_utc();
    let mut model: oidc_provider::ActiveModel = provider.into();
    model.is_active = Set(false);
    model.updated_at = Set(now);

    match model.update(&state.db).await {
        Ok(updated) => (StatusCode::OK, Json(oidc_provider_response_from(updated))).into_response(),
        Err(e) => {
            tracing::error!("Failed to deactivate OIDC provider: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn find_non_deleted_provider(
    db: &DatabaseConnection,
    tenant_id: uuid::Uuid,
    id: uuid::Uuid,
) -> Option<oidc_provider::Model> {
    OidcProvider::find_by_id(id)
        .filter(oidc_provider::Column::TenantId.eq(tenant_id))
        .filter(oidc_provider::Column::DeletedAt.is_null())
        .one(db)
        .await
        .ok()
        .flatten()
}
