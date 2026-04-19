use crate::AppState;
use crate::auth::token::generate_uuid;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::permission::{CanManageAuthSettings, CanViewSettings};
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{ActiveModelTrait, ColumnTrait, QueryFilter, QueryOrder, Set};
use std::sync::Arc;
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

fn emit_oidc_provider_audit(
    state: &AppState,
    tenant_id: Uuid,
    user: &AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
    target_provider_id: Option<String>,
    target_provider_name: Option<String>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(user, api_token_id);
    let mut builder = uptrakit_audit_log::AuditEntry::builder(action_type)
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .outcome(outcome)
        .details(details);

    if target_provider_id.is_some() || target_provider_name.is_some() {
        builder = builder.target_opt(
            Some("oidc_provider".to_string()),
            target_provider_id,
            target_provider_name,
        );
    }

    if let Ok(entry) = builder.build() {
        state.audit_emitter.emit_best_effort(entry);
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
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageAuthSettings(user): CanManageAuthSettings,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Validated(req): Validated<CreateOidcProviderRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let provider_name = req.name.clone();
    let provider_slug = req.slug.clone();
    let scopes_count = req.scopes.split_whitespace().count();
    let role_mapping_count = req.role_mapping.len();
    let has_logo_url = req.logo_url.is_some();
    let has_role_claim_path = req.role_claim_path.is_some();
    let has_client_secret = !req.client_secret.expose_secret().is_empty();

    let multi_tenancy_enabled =
        match crate::settings_store::is_multi_tenancy_enabled(tenant_db.db()).await {
            Ok(enabled) => enabled,
            Err(e) => {
                tracing::error!("Failed to load multi-tenancy mode: {e}");
                emit_oidc_provider_audit(
                    &state,
                    tenant_db.tenant_id,
                    &user,
                    api_token_id,
                    uptrakit_audit_log::AuditActionType::from_static(
                        uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_CREATE,
                    ),
                    None,
                    Some(provider_name.clone()),
                    uptrakit_audit_log::AuditOutcome::Failed,
                    serde_json::json!({
                        "reason_code": "multi_tenancy_lookup_failed",
                    }),
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };
    let allow_private_network_issuers = match resolve_allow_private_network_issuers_for_create(
        req.allow_private_network_issuers,
        multi_tenancy_enabled,
    ) {
        Ok(value) => value,
        Err(message) => {
            emit_oidc_provider_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::from_static(
                    uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_CREATE,
                ),
                None,
                Some(provider_name.clone()),
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                serde_json::json!({
                    "reason_code": "private_network_issuer_disallowed",
                }),
            );
            return error_response(StatusCode::BAD_REQUEST, message);
        }
    };

    // Check slug uniqueness among non-deleted providers within tenant
    let existing = tenant_db
        .find::<oidc_provider::Entity>()
        .filter(oidc_provider::Column::Slug.eq(&req.slug))
        .filter(oidc_provider::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await;

    if let Ok(Some(_)) = existing {
        emit_oidc_provider_audit(
            &state,
            tenant_db.tenant_id,
            &user,
            api_token_id,
            uptrakit_audit_log::AuditActionType::from_static(
                uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_CREATE,
            ),
            None,
            Some(provider_name.clone()),
            uptrakit_audit_log::AuditOutcome::Denied,
            serde_json::json!({
                "reason_code": "slug_already_exists",
                "slug": provider_slug,
            }),
        );
        return error_response(StatusCode::CONFLICT, "Slug already exists");
    }

    let encrypted_secret = match uptrakit_crypto::EncryptedString::new(
        req.client_secret.expose_secret().to_string(),
        "uptrakit:oidc_providers:client_secret",
    ) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("encryption failed: {e}");
            emit_oidc_provider_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::from_static(
                    uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_CREATE,
                ),
                None,
                Some(provider_name.clone()),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "client_secret_encryption_failed",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let now = OffsetDateTime::now_utc();
    let provider_id = generate_uuid();
    let provider = oidc_provider::ActiveModel {
        id: Set(provider_id),
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
        Ok(model) => {
            emit_oidc_provider_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::from_static(
                    uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_CREATE,
                ),
                Some(model.id.to_string()),
                Some(model.name.clone()),
                uptrakit_audit_log::AuditOutcome::Success,
                serde_json::json!({
                    "slug": model.slug,
                    "auto_create_users": model.auto_create_users,
                    "allow_private_network_issuers": allow_private_network_issuers,
                    "scopes_count": scopes_count,
                    "has_logo_url": has_logo_url,
                    "has_role_claim_path": has_role_claim_path,
                    "role_mapping_count": role_mapping_count,
                    "has_client_secret": has_client_secret,
                }),
            );
            (
                StatusCode::CREATED,
                Json(oidc_provider_response_from(model, multi_tenancy_enabled)),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("Failed to create OIDC provider: {e}");
            emit_oidc_provider_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::from_static(
                    uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_CREATE,
                ),
                Some(provider_id.to_string()),
                Some(provider_name),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "provider_insert_failed",
                }),
            );
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
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    Path(provider_id): Path<Uuid>,
    CanManageAuthSettings(user): CanManageAuthSettings,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Json(req): Json<UpdateOidcProviderRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let mut updated_fields: Vec<&'static str> = Vec::new();
    if req.name.is_some() {
        updated_fields.push("name");
    }
    if req.slug.is_some() {
        updated_fields.push("slug");
    }
    if req.logo_url.is_some() {
        updated_fields.push("logo_url");
    }
    if req.issuer_url.is_some() {
        updated_fields.push("issuer_url");
    }
    if req.client_id.is_some() {
        updated_fields.push("client_id");
    }
    let client_secret_updated = req.client_secret.is_some();
    if client_secret_updated {
        updated_fields.push("client_secret");
    }
    if req.scopes.is_some() {
        updated_fields.push("scopes");
    }
    if req.auto_create_users.is_some() {
        updated_fields.push("auto_create_users");
    }
    if req.allow_private_network_issuers.is_some() {
        updated_fields.push("allow_private_network_issuers");
    }
    if req.role_claim_path.is_some() {
        updated_fields.push("role_claim_path");
    }
    if req.role_mapping.is_some() {
        updated_fields.push("role_mapping");
    }

    let multi_tenancy_enabled =
        match crate::settings_store::is_multi_tenancy_enabled(tenant_db.db()).await {
            Ok(enabled) => enabled,
            Err(e) => {
                tracing::error!("Failed to load multi-tenancy mode: {e}");
                emit_oidc_provider_audit(
                    &state,
                    tenant_db.tenant_id,
                    &user,
                    api_token_id,
                    uptrakit_audit_log::AuditActionType::from_static(
                        uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
                    ),
                    Some(provider_id.to_string()),
                    None,
                    uptrakit_audit_log::AuditOutcome::Failed,
                    serde_json::json!({
                        "reason_code": "multi_tenancy_lookup_failed",
                    }),
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };
    let provider = match find_non_deleted_provider(&tenant_db, provider_id).await {
        Some(p) => p,
        None => {
            emit_oidc_provider_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::from_static(
                    uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
                ),
                Some(provider_id.to_string()),
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "reason_code": "provider_not_found",
                }),
            );
            return error_response(StatusCode::NOT_FOUND, "Provider not found");
        }
    };
    let provider_name = provider.name.clone();

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
            emit_oidc_provider_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::from_static(
                    uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
                ),
                Some(provider_id.to_string()),
                Some(provider_name.clone()),
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "reason_code": "slug_already_exists",
                    "slug": new_slug,
                }),
            );
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
        Err(message) => {
            emit_oidc_provider_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::from_static(
                    uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
                ),
                Some(provider_id.to_string()),
                Some(provider_name.clone()),
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                serde_json::json!({
                    "reason_code": "private_network_issuer_disallowed",
                }),
            );
            return error_response(StatusCode::BAD_REQUEST, message);
        }
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
                emit_oidc_provider_audit(
                    &state,
                    tenant_db.tenant_id,
                    &user,
                    api_token_id,
                    uptrakit_audit_log::AuditActionType::from_static(
                        uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
                    ),
                    Some(provider_id.to_string()),
                    Some(provider_name.clone()),
                    uptrakit_audit_log::AuditOutcome::Failed,
                    serde_json::json!({
                        "reason_code": "client_secret_encryption_failed",
                    }),
                );
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
        Ok(updated) => {
            emit_oidc_provider_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::from_static(
                    uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
                ),
                Some(updated.id.to_string()),
                Some(updated.name.clone()),
                uptrakit_audit_log::AuditOutcome::Success,
                serde_json::json!({
                    "updated_fields": updated_fields,
                    "updated_field_count": updated_fields.len(),
                    "client_secret_updated": client_secret_updated,
                }),
            );
            (
                StatusCode::OK,
                Json(oidc_provider_response_from(updated, multi_tenancy_enabled)),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("Failed to update OIDC provider: {e}");
            emit_oidc_provider_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::from_static(
                    uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
                ),
                Some(provider_id.to_string()),
                Some(provider_name),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "provider_update_failed",
                    "updated_field_count": updated_fields.len(),
                    "client_secret_updated": client_secret_updated,
                }),
            );
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
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    Path(provider_id): Path<Uuid>,
    CanManageAuthSettings(user): CanManageAuthSettings,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let provider = match find_non_deleted_provider(&tenant_db, provider_id).await {
        Some(p) => p,
        None => {
            emit_oidc_provider_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::from_static(
                    uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_DELETE,
                ),
                Some(provider_id.to_string()),
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "reason_code": "provider_not_found",
                }),
            );
            return error_response(StatusCode::NOT_FOUND, "Provider not found");
        }
    };

    // Safety: cannot soft-delete if admin is logged in via this provider
    if let AuthMethod::Oidc {
        provider_id: session_pid,
    } = &user.auth_method
        && *session_pid == provider_id
        && provider.is_active
    {
        emit_oidc_provider_audit(
            &state,
            tenant_db.tenant_id,
            &user,
            api_token_id,
            uptrakit_audit_log::AuditActionType::from_static(
                uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_DELETE,
            ),
            Some(provider_id.to_string()),
            Some(provider.name.clone()),
            uptrakit_audit_log::AuditOutcome::Denied,
            serde_json::json!({
                "reason_code": "active_session_provider",
            }),
        );
        return error_response(
            StatusCode::CONFLICT,
            "Cannot delete the OIDC provider used by your current session",
        );
    }

    let now = OffsetDateTime::now_utc();
    let provider_name = provider.name.clone();
    let was_active = provider.is_active;
    let mut model: oidc_provider::ActiveModel = provider.into();
    model.deactivated_at = Set(Some(now));
    model.is_active = Set(false);
    model.updated_at = Set(now);

    match model.update(tenant_db.db()).await {
        Ok(_) => {
            emit_oidc_provider_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::from_static(
                    uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_DELETE,
                ),
                Some(provider_id.to_string()),
                Some(provider_name),
                uptrakit_audit_log::AuditOutcome::Success,
                serde_json::json!({
                    "was_active": was_active,
                }),
            );
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => {
            tracing::error!("Failed to soft-delete OIDC provider: {e}");
            emit_oidc_provider_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::from_static(
                    uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_DELETE,
                ),
                Some(provider_id.to_string()),
                Some(provider_name),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "provider_delete_failed",
                    "was_active": was_active,
                }),
            );
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
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    Path(provider_id): Path<Uuid>,
    CanManageAuthSettings(user): CanManageAuthSettings,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let multi_tenancy_enabled =
        match crate::settings_store::is_multi_tenancy_enabled(tenant_db.db()).await {
            Ok(enabled) => enabled,
            Err(e) => {
                tracing::error!("Failed to load multi-tenancy mode: {e}");
                emit_oidc_provider_audit(
                    &state,
                    tenant_db.tenant_id,
                    &user,
                    api_token_id,
                    uptrakit_audit_log::AuditActionType::from_static(
                        uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
                    ),
                    Some(provider_id.to_string()),
                    None,
                    uptrakit_audit_log::AuditOutcome::Failed,
                    serde_json::json!({
                        "reason_code": "multi_tenancy_lookup_failed",
                        "is_active": true,
                    }),
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };
    let provider = match find_non_deleted_provider(&tenant_db, provider_id).await {
        Some(p) => p,
        None => {
            emit_oidc_provider_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::from_static(
                    uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
                ),
                Some(provider_id.to_string()),
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "reason_code": "provider_not_found",
                    "is_active": true,
                }),
            );
            return error_response(StatusCode::NOT_FOUND, "Provider not found");
        }
    };

    // Validate config completeness
    if provider.issuer_url.is_empty()
        || provider.client_id.is_empty()
        || provider.client_secret.expose_secret().is_empty()
    {
        emit_oidc_provider_audit(
            &state,
            tenant_db.tenant_id,
            &user,
            api_token_id,
            uptrakit_audit_log::AuditActionType::from_static(
                uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
            ),
            Some(provider_id.to_string()),
            Some(provider.name.clone()),
            uptrakit_audit_log::AuditOutcome::Denied,
            serde_json::json!({
                "reason_code": "provider_configuration_incomplete",
                "is_active": true,
            }),
        );
        return error_response(StatusCode::CONFLICT, "Provider configuration is incomplete");
    }

    let now = OffsetDateTime::now_utc();
    let provider_name = provider.name.clone();

    // Deactivate all other providers within the tenant
    let all_active = tenant_db
        .find::<oidc_provider::Entity>()
        .filter(oidc_provider::Column::IsActive.eq(true))
        .filter(oidc_provider::Column::Id.ne(provider_id))
        .filter(oidc_provider::Column::DeactivatedAt.is_null())
        .all(tenant_db.db())
        .await;

    let others = match all_active {
        Ok(others) => others,
        Err(err) => {
            tracing::error!("Failed to load previously active OIDC providers: {err}");
            emit_oidc_provider_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::from_static(
                    uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
                ),
                Some(provider_id.to_string()),
                Some(provider_name.clone()),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "active_provider_query_failed",
                    "is_active": true,
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    for other in others {
        let other_id = other.id;
        let other_name = other.name.clone();
        let mut m: oidc_provider::ActiveModel = other.into();
        m.is_active = Set(false);
        m.updated_at = Set(now);
        match m.update(tenant_db.db()).await {
            Ok(_) => emit_oidc_provider_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::from_static(
                    uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
                ),
                Some(other_id.to_string()),
                Some(other_name),
                uptrakit_audit_log::AuditOutcome::Success,
                serde_json::json!({
                    "is_active": false,
                    "reason_code": "replaced_by_provider_activation",
                    "activated_provider_id": provider_id,
                }),
            ),
            Err(err) => {
                tracing::error!("Failed to deactivate previously active OIDC provider: {err}");
                emit_oidc_provider_audit(
                    &state,
                    tenant_db.tenant_id,
                    &user,
                    api_token_id,
                    uptrakit_audit_log::AuditActionType::from_static(
                        uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
                    ),
                    Some(other_id.to_string()),
                    Some(other_name),
                    uptrakit_audit_log::AuditOutcome::Failed,
                    serde_json::json!({
                        "is_active": false,
                        "reason_code": "replacement_deactivation_failed",
                        "activated_provider_id": provider_id,
                    }),
                );
            }
        }
    }

    // Activate this provider
    let mut model: oidc_provider::ActiveModel = provider.into();
    model.is_active = Set(true);
    model.updated_at = Set(now);

    match model.update(tenant_db.db()).await {
        Ok(updated) => {
            emit_oidc_provider_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::from_static(
                    uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
                ),
                Some(updated.id.to_string()),
                Some(updated.name.clone()),
                uptrakit_audit_log::AuditOutcome::Success,
                serde_json::json!({
                    "is_active": true,
                }),
            );
            (
                StatusCode::OK,
                Json(oidc_provider_response_from(updated, multi_tenancy_enabled)),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("Failed to activate OIDC provider: {e}");
            emit_oidc_provider_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::from_static(
                    uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
                ),
                Some(provider_id.to_string()),
                Some(provider_name),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "provider_activate_failed",
                    "is_active": true,
                }),
            );
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
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    Path(provider_id): Path<Uuid>,
    CanManageAuthSettings(user): CanManageAuthSettings,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let multi_tenancy_enabled =
        match crate::settings_store::is_multi_tenancy_enabled(tenant_db.db()).await {
            Ok(enabled) => enabled,
            Err(e) => {
                tracing::error!("Failed to load multi-tenancy mode: {e}");
                emit_oidc_provider_audit(
                    &state,
                    tenant_db.tenant_id,
                    &user,
                    api_token_id,
                    uptrakit_audit_log::AuditActionType::from_static(
                        uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
                    ),
                    Some(provider_id.to_string()),
                    None,
                    uptrakit_audit_log::AuditOutcome::Failed,
                    serde_json::json!({
                        "reason_code": "multi_tenancy_lookup_failed",
                        "is_active": false,
                    }),
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };
    let provider = match find_non_deleted_provider(&tenant_db, provider_id).await {
        Some(p) => p,
        None => {
            emit_oidc_provider_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::from_static(
                    uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
                ),
                Some(provider_id.to_string()),
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "reason_code": "provider_not_found",
                    "is_active": false,
                }),
            );
            return error_response(StatusCode::NOT_FOUND, "Provider not found");
        }
    };

    // Safety: cannot deactivate if admin's session is via this provider
    if let AuthMethod::Oidc {
        provider_id: session_pid,
    } = &user.auth_method
        && *session_pid == provider_id
    {
        emit_oidc_provider_audit(
            &state,
            tenant_db.tenant_id,
            &user,
            api_token_id,
            uptrakit_audit_log::AuditActionType::from_static(
                uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
            ),
            Some(provider_id.to_string()),
            Some(provider.name.clone()),
            uptrakit_audit_log::AuditOutcome::Denied,
            serde_json::json!({
                "reason_code": "active_session_provider",
                "is_active": false,
            }),
        );
        return error_response(
            StatusCode::CONFLICT,
            "Cannot deactivate the OIDC provider used by your current session",
        );
    }

    let now = OffsetDateTime::now_utc();
    let provider_name = provider.name.clone();
    let mut model: oidc_provider::ActiveModel = provider.into();
    model.is_active = Set(false);
    model.updated_at = Set(now);

    match model.update(tenant_db.db()).await {
        Ok(updated) => {
            emit_oidc_provider_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::from_static(
                    uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
                ),
                Some(updated.id.to_string()),
                Some(updated.name.clone()),
                uptrakit_audit_log::AuditOutcome::Success,
                serde_json::json!({
                    "is_active": false,
                }),
            );
            (
                StatusCode::OK,
                Json(oidc_provider_response_from(updated, multi_tenancy_enabled)),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("Failed to deactivate OIDC provider: {e}");
            emit_oidc_provider_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::from_static(
                    uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
                ),
                Some(provider_id.to_string()),
                Some(provider_name),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "provider_deactivate_failed",
                    "is_active": false,
                }),
            );
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

#[cfg(all(test, feature = "db-sqlite"))]
mod audit_tests {
    use crate::test_harness::TestApp;
    use crate::test_harness::fixtures;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
    use uptrakit_shared_db::entity::audit_log;
    use uuid::Uuid;

    async fn tenant_audit_row_for_action(
        db: &sea_orm::DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
    ) -> audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = audit_log::Entity::find()
                .filter(audit_log::Column::ActionType.eq(action_type))
                .order_by_desc(audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("expected tenant audit row for action {action_type}");
    }

    async fn tenant_provider_update_row_for_state(
        db: &sea_orm::DatabaseConnection,
        provider_id: &str,
        is_active: bool,
    ) -> audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = audit_log::Entity::find()
                .filter(
                    audit_log::Column::ActionType
                        .eq(uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE),
                )
                .filter(audit_log::Column::TargetId.eq(provider_id))
                .order_by_desc(audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query audit rows")
            {
                let matches_state = row
                    .details_json
                    .as_ref()
                    .and_then(|details| details.get("is_active"))
                    == Some(&serde_json::json!(is_active));
                if matches_state {
                    return row;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("expected tenant provider.update audit row for {provider_id} state {is_active}");
    }

    #[tokio::test]
    async fn create_provider_writes_oidc_provider_create_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let token = fixtures::register_and_get_token(&client).await;
        let client_secret = "ultra-secret-client-secret";

        let (status, created): (http::StatusCode, serde_json::Value) = client
            .post_json(
                "/api/v1/settings/oidc-providers",
                &serde_json::json!({
                    "name": "Keycloak",
                    "slug": "keycloak",
                    "issuer_url": "https://auth.example.com/realms/main",
                    "client_id": "uptrakit",
                    "client_secret": client_secret,
                    "scopes": "openid email profile groups",
                    "auto_create_users": true,
                    "allow_private_network_issuers": false,
                    "role_mapping": {
                        "admin": "owner"
                    }
                }),
            )
            .bearer(&token)
            .send_json()
            .await;

        assert_eq!(status, http::StatusCode::CREATED);
        let provider_id = created["id"].as_str().expect("provider id in response");

        let row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_CREATE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::User.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("oidc_provider"));
        assert_eq!(row.target_id.as_deref(), Some(provider_id));
        let details = row.details_json.expect("audit details");
        assert_eq!(details["slug"], serde_json::json!("keycloak"));
        assert_eq!(details["role_mapping_count"], serde_json::json!(1));
        assert!(
            !details.to_string().contains(client_secret),
            "client secret must never be present in audit details"
        );
    }

    #[tokio::test]
    async fn create_provider_with_api_token_uses_api_token_actor_id() {
        let app = TestApp::new().await;
        let client = app.client();
        let user_token = fixtures::register_and_get_token(&client).await;

        let (create_token_status, created_token): (http::StatusCode, serde_json::Value) = client
            .post_json(
                "/api/v1/auth/api-tokens",
                &serde_json::json!({ "name": "oidc-provider-test-token" }),
            )
            .bearer(&user_token)
            .send_json()
            .await;
        assert_eq!(create_token_status, http::StatusCode::CREATED);
        let token_id = Uuid::parse_str(created_token["id"].as_str().expect("api token id"))
            .expect("api token id should be a uuid");
        let raw_api_token = created_token["token"].as_str().expect("raw api token");

        let (status, _created): (http::StatusCode, serde_json::Value) = client
            .post_json(
                "/api/v1/settings/oidc-providers",
                &serde_json::json!({
                    "name": "Token Managed Provider",
                    "slug": "token-managed-provider",
                    "issuer_url": "https://auth.example.com/realms/token",
                    "client_id": "uptrakit",
                    "client_secret": "api-token-secret",
                    "allow_private_network_issuers": false
                }),
            )
            .bearer(raw_api_token)
            .send_json()
            .await;

        assert_eq!(status, http::StatusCode::CREATED);
        let row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_CREATE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::ApiToken.as_str()
        );
        assert_eq!(row.actor_id, Some(token_id));
    }

    #[tokio::test]
    async fn update_and_delete_provider_write_audit_events() {
        let app = TestApp::new().await;
        let client = app.client();
        let token = fixtures::register_and_get_token(&client).await;
        let updated_secret = "updated-client-secret";

        let (create_status, created): (http::StatusCode, serde_json::Value) = client
            .post_json(
                "/api/v1/settings/oidc-providers",
                &serde_json::json!({
                    "name": "Initial SSO",
                    "slug": "initial-sso",
                    "issuer_url": "https://auth.example.com",
                    "client_id": "uptrakit",
                    "client_secret": "initial-secret",
                    "allow_private_network_issuers": false
                }),
            )
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(create_status, http::StatusCode::CREATED);
        let provider_id = created["id"].as_str().expect("provider id in response");

        let update_status = client
            .put_json(
                &format!("/api/v1/settings/oidc-providers/{provider_id}"),
                &serde_json::json!({
                    "name": "Updated SSO",
                    "client_secret": updated_secret,
                    "auto_create_users": false
                }),
            )
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(update_status, http::StatusCode::OK);

        let update_row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
        )
        .await;
        assert_eq!(
            update_row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(update_row.target_type.as_deref(), Some("oidc_provider"));
        assert_eq!(update_row.target_id.as_deref(), Some(provider_id));
        let update_details = update_row.details_json.expect("update details");
        assert_eq!(
            update_details["client_secret_updated"],
            serde_json::json!(true)
        );
        assert!(
            !update_details.to_string().contains(updated_secret),
            "updated client secret must never be present in audit details"
        );

        let delete_status = client
            .delete(&format!("/api/v1/settings/oidc-providers/{provider_id}"))
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(delete_status, http::StatusCode::NO_CONTENT);

        let delete_row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_DELETE,
        )
        .await;
        assert_eq!(
            delete_row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(delete_row.target_type.as_deref(), Some("oidc_provider"));
        assert_eq!(delete_row.target_id.as_deref(), Some(provider_id));
        let delete_details = delete_row.details_json.expect("delete details");
        assert!(delete_details.get("was_active").is_some());
    }

    #[tokio::test]
    async fn activate_and_deactivate_provider_write_update_audit_events() {
        let app = TestApp::new().await;
        let client = app.client();
        let token = fixtures::register_and_get_token(&client).await;

        let (create_status, created): (http::StatusCode, serde_json::Value) = client
            .post_json(
                "/api/v1/settings/oidc-providers",
                &serde_json::json!({
                    "name": "Activatable SSO",
                    "slug": "activatable-sso",
                    "issuer_url": "https://auth.example.com",
                    "client_id": "uptrakit",
                    "client_secret": "initial-secret",
                    "allow_private_network_issuers": false
                }),
            )
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(create_status, http::StatusCode::CREATED);
        let provider_id = created["id"].as_str().expect("provider id in response");

        let activate_status = client
            .post_empty(&format!(
                "/api/v1/settings/oidc-providers/{provider_id}/activate"
            ))
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(activate_status, http::StatusCode::OK);

        let activate_row = tenant_provider_update_row_for_state(&app.db, provider_id, true).await;
        assert_eq!(
            activate_row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        let activate_details = activate_row.details_json.expect("activate details");
        assert_eq!(activate_details["is_active"], serde_json::json!(true));

        let deactivate_status = client
            .post_empty(&format!(
                "/api/v1/settings/oidc-providers/{provider_id}/deactivate"
            ))
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(deactivate_status, http::StatusCode::OK);

        let deactivate_row =
            tenant_provider_update_row_for_state(&app.db, provider_id, false).await;
        assert_eq!(
            deactivate_row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        let deactivate_details = deactivate_row.details_json.expect("deactivate details");
        assert_eq!(deactivate_details["is_active"], serde_json::json!(false));
    }
}
