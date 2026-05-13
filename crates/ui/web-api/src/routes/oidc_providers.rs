use crate::AppState;
use crate::auth::token::generate_uuid;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::permission::{CanManageAuthSettings, CanViewSettings};
use crate::middleware::require_auth::{AuthenticatedApiTokenId, authenticated_user_audit_actor};
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{
    ColumnTrait, QueryFilter, QueryOrder, SqliteTransactionMode, TransactionOptions,
    TransactionTrait,
};
use std::sync::Arc;
use time::OffsetDateTime;
use uptrakit_audit_log::{AbsentView, AuditEntry, AuditOutcome, Event, Stateful};
use uptrakit_shared_db::entity::oidc_provider;
use uptrakit_web_api_queries::queries::oidc_providers::{
    CreateOidcProviderParams, OidcProviderView, UpdateOidcProviderParams,
    create_oidc_provider_in_tx, delete_oidc_provider_in_tx, set_provider_active_in_tx,
    update_oidc_provider_in_tx,
};
use uuid::Uuid;

use crate::auth::AuthMethod;

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
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageAuthSettings(user): CanManageAuthSettings,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Validated(req): Validated<CreateOidcProviderRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();
    let provider_name = req.name.clone();
    let provider_slug = req.slug.clone();
    let scopes_count = req.scopes.split_whitespace().count();
    let role_mapping_count = req.role_mapping.len();
    let has_logo_url = req.logo_url.is_some();
    let has_role_claim_path = req.role_claim_path.is_some();
    let has_client_secret = !req.client_secret.expose_secret().is_empty();

    // ── Pre-tx reads and validation ───────────────────────────────────────────

    let multi_tenancy_enabled =
        match crate::settings_store::is_multi_tenancy_enabled(tenant_db.db()).await {
            Ok(enabled) => enabled,
            Err(e) => {
                tracing::error!("Failed to load multi-tenancy mode: {e}");
                if let Ok(entry) = AuditEntry::<Event>::builder_event(
                    uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_CREATE,
                )
                .tenant_scope(tenant_id)
                .actor(actor_type, actor_id)
                .target("oidc_provider", String::new(), Some(provider_name.clone()))
                .outcome(AuditOutcome::Failed)
                .details(serde_json::json!({
                    "reason_code": "multi_tenancy_lookup_failed",
                }))
                .build()
                {
                    state.audit_emitter.emit_event(entry);
                }
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    let allow_private_network_issuers = match resolve_allow_private_network_issuers_for_create(
        req.allow_private_network_issuers,
        multi_tenancy_enabled,
    ) {
        Ok(value) => value,
        Err(message) => {
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_CREATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("oidc_provider", String::new(), Some(provider_name.clone()))
            .outcome(AuditOutcome::ValidationFailed)
            .details(serde_json::json!({
                "reason_code": "private_network_issuer_disallowed",
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
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
        if let Ok(entry) = AuditEntry::<Event>::builder_event(
            uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_CREATE,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .target("oidc_provider", String::new(), Some(provider_name.clone()))
        .outcome(AuditOutcome::Denied)
        .details(serde_json::json!({
            "reason_code": "slug_already_exists",
            "slug": provider_slug,
        }))
        .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        return error_response(StatusCode::CONFLICT, "Slug already exists");
    }

    let encrypted_secret = match uptrakit_crypto::EncryptedString::new(
        req.client_secret.expose_secret().to_string(),
        "uptrakit:oidc_providers:client_secret",
    ) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("encryption failed: {e}");
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_CREATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("oidc_provider", String::new(), Some(provider_name.clone()))
            .outcome(AuditOutcome::Failed)
            .details(serde_json::json!({
                "reason_code": "client_secret_encryption_failed",
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // ── BEGIN IMMEDIATE tx ────────────────────────────────────────────────────

    let now = OffsetDateTime::now_utc();
    let provider_id = generate_uuid();

    let tx = match state
        .db()
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
    {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Failed to begin transaction for oidc provider create: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let model = match create_oidc_provider_in_tx(
        &tx,
        CreateOidcProviderParams {
            id: provider_id,
            tenant_id,
            name: req.name,
            slug: req.slug,
            logo_url: req.logo_url,
            issuer_url: req.issuer_url,
            client_id: req.client_id,
            client_secret: encrypted_secret,
            scopes: req.scopes,
            auto_create_users: req.auto_create_users,
            allow_private_network_issuers,
            role_claim_path: req.role_claim_path,
            role_mapping: req.role_mapping,
            now,
        },
    )
    .await
    {
        Ok(m) => m,
        Err(e) => {
            drop(tx);
            tracing::error!("Failed to create OIDC provider: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let after_view = OidcProviderView::from(&model);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry =
        match AuditEntry::<Stateful>::oidc_provider_create(&AbsentView(&after_view), &after_view)
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Success)
            .details(serde_json::json!({
                "slug": model.slug,
                "auto_create_users": model.auto_create_users,
                "allow_private_network_issuers": allow_private_network_issuers,
                "scopes_count": scopes_count,
                "has_logo_url": has_logo_url,
                "has_role_claim_path": has_role_claim_path,
                "role_mapping_count": role_mapping_count,
                "has_client_secret": has_client_secret,
            }))
            .build()
        {
            Ok(entry) => entry,
            Err(e) => {
                tracing::error!("Failed to build audit entry for oidc provider create: {e}");
                drop(tx);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!("Failed to emit audit entry for oidc provider create: {e}");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit oidc provider create: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    (
        StatusCode::CREATED,
        Json(oidc_provider_response_from(model, multi_tenancy_enabled)),
    )
        .into_response()
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
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

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

    // ── Pre-tx reads and validation ───────────────────────────────────────────

    let multi_tenancy_enabled =
        match crate::settings_store::is_multi_tenancy_enabled(tenant_db.db()).await {
            Ok(enabled) => enabled,
            Err(e) => {
                tracing::error!("Failed to load multi-tenancy mode: {e}");
                if let Ok(entry) = AuditEntry::<Event>::builder_event(
                    uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
                )
                .tenant_scope(tenant_id)
                .actor(actor_type, actor_id)
                .target("oidc_provider", provider_id.to_string(), None)
                .outcome(AuditOutcome::Failed)
                .details(serde_json::json!({
                    "reason_code": "multi_tenancy_lookup_failed",
                }))
                .build()
                {
                    state.audit_emitter.emit_event(entry);
                }
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    let provider = match find_non_deleted_provider(&tenant_db, provider_id).await {
        Some(p) => p,
        None => {
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("oidc_provider", provider_id.to_string(), None)
            .outcome(AuditOutcome::Denied)
            .details(serde_json::json!({
                "reason_code": "provider_not_found",
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::NOT_FOUND, "Provider not found");
        }
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
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target(
                "oidc_provider",
                provider_id.to_string(),
                Some(provider.name.clone()),
            )
            .outcome(AuditOutcome::Denied)
            .details(serde_json::json!({
                "reason_code": "slug_already_exists",
                "slug": new_slug,
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::CONFLICT, "Slug already exists");
        }
    }

    let allow_private_network_issuers = match resolve_allow_private_network_issuers_for_update(
        req.allow_private_network_issuers,
        multi_tenancy_enabled,
    ) {
        Ok(value) => value,
        Err(message) => {
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target(
                "oidc_provider",
                provider_id.to_string(),
                Some(provider.name.clone()),
            )
            .outcome(AuditOutcome::ValidationFailed)
            .details(serde_json::json!({
                "reason_code": "private_network_issuer_disallowed",
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::BAD_REQUEST, message);
        }
    };

    // Encrypt new client secret outside the tx if provided
    let encrypted_secret = match req.client_secret {
        Some(ref secret) => match uptrakit_crypto::EncryptedString::new(
            secret.expose_secret().to_string(),
            "uptrakit:oidc_providers:client_secret",
        ) {
            Ok(s) => Some(s),
            Err(e) => {
                tracing::error!("encryption failed: {e}");
                if let Ok(entry) = AuditEntry::<Event>::builder_event(
                    uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
                )
                .tenant_scope(tenant_id)
                .actor(actor_type, actor_id)
                .target(
                    "oidc_provider",
                    provider_id.to_string(),
                    Some(provider.name.clone()),
                )
                .outcome(AuditOutcome::Failed)
                .details(serde_json::json!({
                    "reason_code": "client_secret_encryption_failed",
                }))
                .build()
                {
                    state.audit_emitter.emit_event(entry);
                }
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        },
        None => None,
    };

    // ── BEGIN IMMEDIATE tx ────────────────────────────────────────────────────

    let now = OffsetDateTime::now_utc();
    let before_view = OidcProviderView::from(&provider);

    let tx = match state
        .db()
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
    {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Failed to begin transaction for oidc provider update: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let updated = match update_oidc_provider_in_tx(
        &tx,
        provider,
        UpdateOidcProviderParams {
            name: req.name,
            slug: req.slug,
            logo_url: req.logo_url,
            issuer_url: req.issuer_url,
            client_id: req.client_id,
            encrypted_secret,
            scopes: req.scopes,
            auto_create_users: req.auto_create_users,
            allow_private_network_issuers,
            role_claim_path: req.role_claim_path,
            role_mapping: req.role_mapping,
            now,
        },
    )
    .await
    {
        Ok(m) => m,
        Err(e) => {
            drop(tx);
            tracing::error!("Failed to update OIDC provider: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let after_view = OidcProviderView::from(&updated);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::oidc_provider_update(&before_view, &after_view)
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Success)
        .details(serde_json::json!({
            "updated_fields": updated_fields,
            "updated_field_count": updated_fields.len(),
            "client_secret_updated": client_secret_updated,
        }))
        .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!("Failed to build audit entry for oidc provider update: {e}");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!("Failed to emit audit entry for oidc provider update: {e}");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit oidc provider update: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    (
        StatusCode::OK,
        Json(oidc_provider_response_from(updated, multi_tenancy_enabled)),
    )
        .into_response()
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
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    // ── Pre-tx reads and validation ───────────────────────────────────────────

    let provider = match find_non_deleted_provider(&tenant_db, provider_id).await {
        Some(p) => p,
        None => {
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_DELETE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("oidc_provider", provider_id.to_string(), None)
            .outcome(AuditOutcome::Denied)
            .details(serde_json::json!({
                "reason_code": "provider_not_found",
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
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
        if let Ok(entry) = AuditEntry::<Event>::builder_event(
            uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_DELETE,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .target(
            "oidc_provider",
            provider_id.to_string(),
            Some(provider.name.clone()),
        )
        .outcome(AuditOutcome::Denied)
        .details(serde_json::json!({
            "reason_code": "active_session_provider",
        }))
        .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        return error_response(
            StatusCode::CONFLICT,
            "Cannot delete the OIDC provider used by your current session",
        );
    }

    let now = OffsetDateTime::now_utc();
    let was_active = provider.is_active;
    let before_view = OidcProviderView::from(&provider);

    // ── BEGIN IMMEDIATE tx ────────────────────────────────────────────────────

    let tx = match state
        .db()
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
    {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Failed to begin transaction for oidc provider delete: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let deleted = match delete_oidc_provider_in_tx(&tx, provider, now).await {
        Ok(m) => m,
        Err(e) => {
            drop(tx);
            tracing::error!("Failed to soft-delete OIDC provider: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let after_view = OidcProviderView::from(&deleted);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::oidc_provider_delete(&before_view, &after_view)
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Success)
        .details(serde_json::json!({
            "was_active": was_active,
        }))
        .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!("Failed to build audit entry for oidc provider delete: {e}");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!("Failed to emit audit entry for oidc provider delete: {e}");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit oidc provider delete: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    StatusCode::NO_CONTENT.into_response()
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
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    // ── Pre-tx reads and validation ───────────────────────────────────────────

    let multi_tenancy_enabled =
        match crate::settings_store::is_multi_tenancy_enabled(tenant_db.db()).await {
            Ok(enabled) => enabled,
            Err(e) => {
                tracing::error!("Failed to load multi-tenancy mode: {e}");
                if let Ok(entry) = AuditEntry::<Event>::builder_event(
                    uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
                )
                .tenant_scope(tenant_id)
                .actor(actor_type, actor_id)
                .target("oidc_provider", provider_id.to_string(), None)
                .outcome(AuditOutcome::Failed)
                .details(serde_json::json!({
                    "reason_code": "multi_tenancy_lookup_failed",
                    "is_active": true,
                }))
                .build()
                {
                    state.audit_emitter.emit_event(entry);
                }
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    let provider = match find_non_deleted_provider(&tenant_db, provider_id).await {
        Some(p) => p,
        None => {
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("oidc_provider", provider_id.to_string(), None)
            .outcome(AuditOutcome::Denied)
            .details(serde_json::json!({
                "reason_code": "provider_not_found",
                "is_active": true,
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::NOT_FOUND, "Provider not found");
        }
    };

    // Validate config completeness
    if provider.issuer_url.is_empty()
        || provider.client_id.is_empty()
        || provider.client_secret.expose_secret().is_empty()
    {
        if let Ok(entry) = AuditEntry::<Event>::builder_event(
            uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .target(
            "oidc_provider",
            provider_id.to_string(),
            Some(provider.name.clone()),
        )
        .outcome(AuditOutcome::Denied)
        .details(serde_json::json!({
            "reason_code": "provider_configuration_incomplete",
            "is_active": true,
        }))
        .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        return error_response(StatusCode::CONFLICT, "Provider configuration is incomplete");
    }

    // Query all currently-active other providers
    let others = match tenant_db
        .find::<oidc_provider::Entity>()
        .filter(oidc_provider::Column::IsActive.eq(true))
        .filter(oidc_provider::Column::Id.ne(provider_id))
        .filter(oidc_provider::Column::DeactivatedAt.is_null())
        .all(tenant_db.db())
        .await
    {
        Ok(others) => others,
        Err(err) => {
            tracing::error!("Failed to load previously active OIDC providers: {err}");
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target(
                "oidc_provider",
                provider_id.to_string(),
                Some(provider.name.clone()),
            )
            .outcome(AuditOutcome::Failed)
            .details(serde_json::json!({
                "reason_code": "active_provider_query_failed",
                "is_active": true,
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // ── BEGIN IMMEDIATE tx ────────────────────────────────────────────────────

    let now = OffsetDateTime::now_utc();

    let tx = match state
        .db()
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
    {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Failed to begin transaction for oidc provider activate: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let hook = state.audit_emitter.commit_hook();

    // Deactivate all other active providers, emitting one audit row per provider
    for other in others {
        let other_before_view = OidcProviderView::from(&other);
        let other_id = other.id;
        let deactivated = match set_provider_active_in_tx(&tx, other, false, now).await {
            Ok(m) => m,
            Err(err) => {
                drop(tx);
                tracing::error!("Failed to deactivate previously active OIDC provider: {err}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };
        let other_after_view = OidcProviderView::from(&deactivated);
        let other_entry = match AuditEntry::<Stateful>::oidc_provider_update(
            &other_before_view,
            &other_after_view,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Success)
        .details(serde_json::json!({
            "is_active": false,
            "reason_code": "replaced_by_provider_activation",
            "activated_provider_id": other_id,
        }))
        .build()
        {
            Ok(entry) => entry,
            Err(e) => {
                tracing::error!(
                    "Failed to build audit entry for oidc provider deactivate-other: {e}"
                );
                drop(tx);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };
        if let Err(e) = state
            .audit_emitter
            .emit_stateful(&tx, &hook, other_entry)
            .await
        {
            tracing::error!("Failed to emit audit entry for oidc provider deactivate-other: {e}");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    }

    // Activate the target provider
    let before_view = OidcProviderView::from(&provider);
    let activated = match set_provider_active_in_tx(&tx, provider, true, now).await {
        Ok(m) => m,
        Err(e) => {
            drop(tx);
            tracing::error!("Failed to activate OIDC provider: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let after_view = OidcProviderView::from(&activated);
    let audit_entry = match AuditEntry::<Stateful>::oidc_provider_update(&before_view, &after_view)
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Success)
        .details(serde_json::json!({
            "is_active": true,
        }))
        .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!("Failed to build audit entry for oidc provider activate: {e}");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!("Failed to emit audit entry for oidc provider activate: {e}");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit oidc provider activate: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    (
        StatusCode::OK,
        Json(oidc_provider_response_from(
            activated,
            multi_tenancy_enabled,
        )),
    )
        .into_response()
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
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    // ── Pre-tx reads and validation ───────────────────────────────────────────

    let multi_tenancy_enabled =
        match crate::settings_store::is_multi_tenancy_enabled(tenant_db.db()).await {
            Ok(enabled) => enabled,
            Err(e) => {
                tracing::error!("Failed to load multi-tenancy mode: {e}");
                if let Ok(entry) = AuditEntry::<Event>::builder_event(
                    uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
                )
                .tenant_scope(tenant_id)
                .actor(actor_type, actor_id)
                .target("oidc_provider", provider_id.to_string(), None)
                .outcome(AuditOutcome::Failed)
                .details(serde_json::json!({
                    "reason_code": "multi_tenancy_lookup_failed",
                    "is_active": false,
                }))
                .build()
                {
                    state.audit_emitter.emit_event(entry);
                }
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    let provider = match find_non_deleted_provider(&tenant_db, provider_id).await {
        Some(p) => p,
        None => {
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("oidc_provider", provider_id.to_string(), None)
            .outcome(AuditOutcome::Denied)
            .details(serde_json::json!({
                "reason_code": "provider_not_found",
                "is_active": false,
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::NOT_FOUND, "Provider not found");
        }
    };

    // Safety: cannot deactivate if admin's session is via this provider
    if let AuthMethod::Oidc {
        provider_id: session_pid,
    } = &user.auth_method
        && *session_pid == provider_id
    {
        if let Ok(entry) = AuditEntry::<Event>::builder_event(
            uptrakit_audit_log::AuditActionType::OIDC_PROVIDER_UPDATE,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .target(
            "oidc_provider",
            provider_id.to_string(),
            Some(provider.name.clone()),
        )
        .outcome(AuditOutcome::Denied)
        .details(serde_json::json!({
            "reason_code": "active_session_provider",
            "is_active": false,
        }))
        .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        return error_response(
            StatusCode::CONFLICT,
            "Cannot deactivate the OIDC provider used by your current session",
        );
    }

    // ── BEGIN IMMEDIATE tx ────────────────────────────────────────────────────

    let now = OffsetDateTime::now_utc();
    let before_view = OidcProviderView::from(&provider);

    let tx = match state
        .db()
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
    {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Failed to begin transaction for oidc provider deactivate: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let deactivated = match set_provider_active_in_tx(&tx, provider, false, now).await {
        Ok(m) => m,
        Err(e) => {
            drop(tx);
            tracing::error!("Failed to deactivate OIDC provider: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let after_view = OidcProviderView::from(&deactivated);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::oidc_provider_update(&before_view, &after_view)
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Success)
        .details(serde_json::json!({
            "is_active": false,
        }))
        .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!("Failed to build audit entry for oidc provider deactivate: {e}");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!("Failed to emit audit entry for oidc provider deactivate: {e}");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit oidc provider deactivate: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    (
        StatusCode::OK,
        Json(oidc_provider_response_from(
            deactivated,
            multi_tenancy_enabled,
        )),
    )
        .into_response()
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
    #![expect(
        clippy::expect_used,
        reason = "test code: panics on failure are acceptable"
    )]
    #![expect(clippy::panic, reason = "test code: panics on failure are acceptable")]

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
        let before = row.before_snapshot.expect("before_snapshot");
        let after = row.after_snapshot.expect("after_snapshot");
        assert!(
            before.get("client_secret").is_none(),
            "client_secret key must not appear in before_snapshot"
        );
        assert!(
            after.get("client_secret").is_none(),
            "client_secret key must not appear in after_snapshot"
        );
        assert!(
            !before.to_string().contains(client_secret),
            "client_secret plaintext must not appear in before_snapshot JSON"
        );
        assert!(
            !after.to_string().contains(client_secret),
            "client_secret plaintext must not appear in after_snapshot JSON"
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
        let update_before = update_row.before_snapshot.expect("update before_snapshot");
        let update_after = update_row.after_snapshot.expect("update after_snapshot");
        assert!(
            update_before.get("client_secret").is_none(),
            "client_secret key must not appear in update before_snapshot"
        );
        assert!(
            update_after.get("client_secret").is_none(),
            "client_secret key must not appear in update after_snapshot"
        );
        assert!(
            !update_before.to_string().contains(updated_secret),
            "updated client_secret must not appear in before_snapshot JSON"
        );
        assert!(
            !update_after.to_string().contains(updated_secret),
            "updated client_secret must not appear in after_snapshot JSON"
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
