use std::sync::Arc;

use axum::{
    Extension, Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{SqliteTransactionMode, TransactionOptions, TransactionTrait};
use uptrakit_audit_log::{AbsentView, AuditEntry, AuditOutcome, Event, Stateful};
use uptrakit_web_api_queries::queries::tenant_settings::TenantSettingView;
use uptrakit_web_api_types::settings_access::{
    AccessSettingsResponse, UpdateAccessSettingsRequest,
};
use uptrakit_web_api_types::validation::Validate;

use crate::AppState;
use crate::auth::AuthMethod;
use crate::auth::registration::RegistrationMode;
use crate::error_response::error_response;
use crate::middleware::permission::{CanManageAuthSettings, CanViewSettings};
use crate::middleware::require_auth::{AuthenticatedApiTokenId, authenticated_user_audit_actor};
#[cfg(feature = "oidc")]
use {
    crate::tenant_db::TenantDb,
    sea_orm::{ColumnTrait, QueryFilter},
    uptrakit_shared_db::entity::oidc_provider,
};

fn current_response(state: &AppState) -> AccessSettingsResponse {
    let reg = state.settings.registration();
    let auth = state.settings.authentication();
    AccessSettingsResponse::new(
        reg.mode,
        reg.require_token_for_oidc,
        auth.password_auth_enabled,
        auth.two_factor_required,
    )
}

/// Get access settings (registration + authentication)
#[utoipa::path(
    get,
    path = "/api/v1/settings/access",
    responses(
        (status = 200, description = "Current access settings", body = AccessSettingsResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    extensions(("x-required-permission" = json!("view_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_access_settings(
    State(state): State<Arc<AppState>>,
    CanViewSettings(_user): CanViewSettings,
) -> Response {
    (StatusCode::OK, Json(current_response(&state))).into_response()
}

/// Update access settings (registration + authentication in one transaction)
#[utoipa::path(
    put,
    path = "/api/v1/settings/access",
    request_body = UpdateAccessSettingsRequest,
    responses(
        (status = 200, description = "Access settings updated", body = AccessSettingsResponse),
        (status = 409, description = "Safety check failed (e.g., disabling password auth while using it)"),
        (status = 422, description = "Validation error (e.g., invite mode without token)")
    ),
    tag = "Settings",
    extensions(("x-required-permission" = json!("manage_auth_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_access_settings(
    State(state): State<Arc<AppState>>,
    CanManageAuthSettings(user): CanManageAuthSettings,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    #[cfg(feature = "oidc")] tenant_db: TenantDb,
    Json(req): Json<UpdateAccessSettingsRequest>,
) -> Response {
    // ── 1. Type-level validation ─────────────────────────────────────────────
    if let Err(e) = req.validate() {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("{}: {}", e.field, e.message),
        );
    }

    let api_token_id = api_token_id.map(|v| v.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = state.default_tenant_id;

    // ── 2. Safety checks (reads; run BEFORE opening BEGIN IMMEDIATE) ─────────
    if let Some(false) = req.password_auth_enabled {
        let previous_enabled = state.settings.authentication().password_auth_enabled;

        if user.auth_method == AuthMethod::Password {
            emit_auth_event(
                &state,
                actor_type,
                actor_id,
                AuditOutcome::Denied,
                "cannot_disable_password_auth_while_using_password",
                previous_enabled,
                false,
            );
            return error_response(
                StatusCode::CONFLICT,
                "Cannot disable password authentication while logged in with a password",
            );
        }

        #[cfg(feature = "oidc")]
        {
            let active_providers = match tenant_db
                .find::<oidc_provider::Entity>()
                .filter(oidc_provider::Column::IsActive.eq(true))
                .filter(oidc_provider::Column::DeactivatedAt.is_null())
                .all(tenant_db.db())
                .await
            {
                Ok(providers) => providers,
                Err(e) => {
                    tracing::error!("Failed to query OIDC providers: {e}");
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            };

            if active_providers.is_empty() {
                emit_auth_event(
                    &state,
                    actor_type,
                    actor_id,
                    AuditOutcome::Denied,
                    "cannot_disable_password_auth_without_active_oidc_providers",
                    previous_enabled,
                    false,
                );
                return error_response(
                    StatusCode::CONFLICT,
                    "Cannot disable password authentication with no active OIDC providers",
                );
            }
        }

        if !cfg!(feature = "oidc") {
            emit_auth_event(
                &state,
                actor_type,
                actor_id,
                AuditOutcome::Denied,
                "cannot_disable_password_auth_without_oidc_support",
                previous_enabled,
                false,
            );
            return error_response(
                StatusCode::CONFLICT,
                "Cannot disable password authentication: OIDC support is not enabled",
            );
        }
    }

    // ── 3. Capture before-state for audit ────────────────────────────────────
    let before_reg = state.settings.registration();
    let before_auth = state.settings.authentication();

    let before_reg_view = TenantSettingView {
        key: "registration".to_string(),
        value: serde_json::json!({
            "mode": before_reg.mode.as_str(),
            "require_token_for_oidc": before_reg.require_token_for_oidc,
        }),
    };
    let before_auth_view = TenantSettingView {
        key: "authentication".to_string(),
        value: serde_json::json!({
            "password_auth_enabled": before_auth.password_auth_enabled,
            "two_factor_required": before_auth.two_factor_required,
        }),
    };

    let had_existing_reg = before_reg.token_hash.is_some()
        || before_reg.mode != RegistrationMode::Closed
        || before_reg.require_token_for_oidc;

    // ── 4. BEGIN IMMEDIATE transaction — write both settings atomically ───────
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
            tracing::error!("Failed to begin tx for access settings update: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };
    let hook = state.audit_emitter.commit_hook();

    // Write registration settings
    let mut reg = state.settings.registration();
    if let Err(e) = reg
        .update(
            &tx,
            tenant_id,
            req.mode,
            req.token.as_ref().map(|t| t.expose_secret().to_string()),
            req.require_token_for_oidc,
        )
        .await
    {
        tracing::error!(error = ?e, "Failed to update registration settings");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    // Write authentication settings
    let mut auth = state.settings.authentication();
    if let Some(enabled) = req.password_auth_enabled {
        auth.password_auth_enabled = enabled;
    }
    if let Some(required) = req.two_factor_required {
        auth.two_factor_required = required;
    }
    if let Err(e) = auth.save(&tx, tenant_id).await {
        tracing::error!("Failed to save authentication settings: {e:?}");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    // ── 5. Build after-state views ────────────────────────────────────────────
    let after_reg_view = TenantSettingView {
        key: "registration".to_string(),
        value: serde_json::json!({
            "mode": reg.mode.as_str(),
            "require_token_for_oidc": reg.require_token_for_oidc,
        }),
    };
    let after_auth_view = TenantSettingView {
        key: "authentication".to_string(),
        value: serde_json::json!({
            "password_auth_enabled": auth.password_auth_enabled,
            "two_factor_required": auth.two_factor_required,
        }),
    };

    // ── 6. Emit stateful audit events ─────────────────────────────────────────
    let reg_entry_result = if had_existing_reg {
        AuditEntry::<Stateful>::tenant_setting_update(&before_reg_view, &after_reg_view)
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Success)
            .details(serde_json::json!({ "setting_area": "registration" }))
            .build()
    } else {
        AuditEntry::<Stateful>::tenant_setting_update(&AbsentView(&after_reg_view), &after_reg_view)
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Success)
            .details(serde_json::json!({ "setting_area": "registration" }))
            .build()
    };
    let reg_entry = match reg_entry_result {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("Failed to build registration audit entry: {e}");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, reg_entry)
        .await
    {
        tracing::error!("Failed to emit registration audit: {e}");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    // Only emit auth audit event when the request actually touched auth fields.
    // Emitting with identical before/after produces spurious "changed" audit entries.
    let auth_fields_touched =
        req.password_auth_enabled.is_some() || req.two_factor_required.is_some();
    if auth_fields_touched {
        let auth_entry = match AuditEntry::<Stateful>::tenant_setting_update(
            &before_auth_view,
            &after_auth_view,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Success)
        .details(serde_json::json!({ "setting_area": "authentication" }))
        .build()
        {
            Ok(e) => e,
            Err(e) => {
                tracing::error!("Failed to build authentication audit entry: {e}");
                drop(tx);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };
        if let Err(e) = state
            .audit_emitter
            .emit_stateful(&tx, &hook, auth_entry)
            .await
        {
            tracing::error!("Failed to emit authentication audit: {e}");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    }

    // ── 7. Commit ─────────────────────────────────────────────────────────────
    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit access settings update: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    state.settings.set_registration(reg).await;
    state.settings.set_authentication(auth).await;

    (StatusCode::OK, Json(current_response(&state))).into_response()
}

// ── Audit helpers ─────────────────────────────────────────────────────────────

fn emit_auth_event(
    state: &AppState,
    actor_type: uptrakit_audit_log::AuditActorType,
    actor_id: Option<uuid::Uuid>,
    outcome: AuditOutcome,
    reason_code: &'static str,
    previous_enabled: bool,
    new_enabled: bool,
) {
    if let Ok(entry) = AuditEntry::<Event>::builder_event(
        uptrakit_audit_log::AuditActionType::TENANT_SETTING_UPDATE,
    )
    .tenant_scope(state.default_tenant_id)
    .actor(actor_type, actor_id)
    .target(
        "tenant_setting",
        "authentication".to_string(),
        Some("authentication".to_string()),
    )
    .outcome(outcome)
    .details(serde_json::json!({
        "reason_code": reason_code,
        "previous_enabled": previous_enabled,
        "new_enabled": new_enabled,
    }))
    .build()
    {
        state.audit_emitter.emit_event(entry);
    }
}
