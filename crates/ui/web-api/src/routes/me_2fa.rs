//! Route handlers for 2FA enrollment, status, disable, and recovery-code management.
//!
//! Endpoints:
//! - `GET  /api/v1/auth/me/2fa`                          — [`mfa_status`]
//! - `POST /api/v1/auth/me/2fa/totp/enroll`              — [`totp_enroll`]
//! - `POST /api/v1/auth/me/2fa/totp/confirm`             — [`totp_confirm`]
//! - `POST /api/v1/auth/me/2fa/totp/disable`             — [`totp_disable`]
//! - `POST /api/v1/auth/me/2fa/recovery-codes/regenerate`— [`regenerate_recovery_codes`]
#![expect(
    clippy::let_underscore_must_use,
    reason = "rollback results on error paths are fire-and-forget; the original error is propagated"
)]

use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter, Set,
    SqliteTransactionMode, TransactionOptions, TransactionTrait,
};

use crate::AppState;
use crate::auth::mfa_challenge::{generate_recovery_codes, replace_recovery_codes};
use crate::auth::totp::{build_otpauth_uri, generate_totp_secret, verify_totp_code};
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::require_auth::{AuthenticatedUser, FullSessionUser, SetupRequired};
use crate::routes::mfa::build_full_session;
use uptrakit_crypto::EncryptedString;
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::{user_recovery_code, user_totp};
use uptrakit_web_api_types::mfa::{
    DisableTotpRequest, MfaMethod, MfaStatusResponse, RegenerateRecoveryCodesRequest,
    RegenerateRecoveryCodesResponse, TotpConfirmRequest, TotpConfirmResponse, TotpEnrollResponse,
};

// ── Audit helper ─────────────────────────────────────────────────────────────

fn emit_mfa_audit(
    state: &AppState,
    action: uptrakit_audit_log::RegisteredAuditAction,
    user_id: uuid::Uuid,
    outcome: uptrakit_audit_log::AuditOutcome,
) {
    let builder = uptrakit_audit_log::AuditEntry::builder(action)
        .tenant_scope(state.default_tenant_id)
        .actor(uptrakit_audit_log::AuditActorType::User, Some(user_id))
        .outcome(outcome)
        .target("user", user_id.to_string(), None);

    if let Ok(entry) = builder.build() {
        state.audit_emitter.emit_best_effort(entry);
    }
}

// ── re_auth_ok ────────────────────────────────────────────────────────────────

/// Verify the caller's identity using either a password or a TOTP code.
///
/// Returns `true` when the credential is accepted, `false` otherwise.
/// Never returns an error: DB failures are logged and treated as rejected auth.
async fn re_auth_ok(
    state: &AppState,
    user_id: uuid::Uuid,
    password: Option<&uptrakit_shared_types::SecretString>,
    totp_code: Option<&str>,
) -> bool {
    if let Some(pw) = password {
        // Load user's password hash.
        let user = match User::find_by_id(user_id).one(state.db()).await {
            Ok(Some(u)) => u,
            Ok(None) => {
                tracing::warn!(%user_id, "re_auth_ok: user not found");
                return false;
            }
            Err(e) => {
                tracing::error!("re_auth_ok: failed to load user: {e}");
                return false;
            }
        };

        let Some(hash_secret) = user.password_hash else {
            // No password set (e.g. OIDC-only user).
            return false;
        };

        let pw_str = pw.expose_secret().to_string();
        let hash_str = hash_secret.expose_secret().to_string();

        match tokio::task::spawn_blocking(move || {
            crate::auth::password::verify_password(&pw_str, &hash_str)
        })
        .await
        {
            Ok(Ok(valid)) => valid,
            Ok(Err(e)) => {
                tracing::error!("re_auth_ok: password verify error: {:?}", e);
                false
            }
            Err(e) => {
                tracing::error!("re_auth_ok: spawn_blocking panicked: {:?}", e);
                false
            }
        }
    } else if let Some(code) = totp_code {
        // BEGIN IMMEDIATE: load active TOTP row and update last_used_step.
        let txn = match state
            .db()
            .begin_with_options(TransactionOptions {
                sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
                ..Default::default()
            })
            .await
        {
            Ok(t) => t,
            Err(e) => {
                tracing::error!("re_auth_ok: failed to begin transaction: {e}");
                return false;
            }
        };

        let totp_row = match UserTotp::find()
            .filter(user_totp::Column::UserId.eq(user_id))
            .filter(user_totp::Column::IsActive.eq(true))
            .one(&txn)
            .await
        {
            Ok(Some(row)) => row,
            Ok(None) => {
                let _ = txn.rollback().await;
                return false;
            }
            Err(e) => {
                tracing::error!("re_auth_ok: failed to load TOTP row: {e}");
                let _ = txn.rollback().await;
                return false;
            }
        };

        let secret = totp_row.secret.expose_secret().to_string();
        let code_owned = code.to_string();

        let step = tokio::task::spawn_blocking(move || verify_totp_code(&secret, &code_owned))
            .await
            .unwrap_or(None);

        let Some(step) = step else {
            let _ = txn.rollback().await;
            return false;
        };

        // Anti-replay: reject if step was already used.
        if totp_row.last_used_step.is_some_and(|last| step <= last) {
            let _ = txn.rollback().await;
            return false;
        }

        // Persist the new last_used_step.
        let mut active = totp_row.into_active_model();
        active.last_used_step = Set(Some(step));
        if let Err(e) = active.update(&txn).await {
            tracing::error!("re_auth_ok: failed to update last_used_step: {e}");
            let _ = txn.rollback().await;
            return false;
        }

        if let Err(e) = txn.commit().await {
            tracing::error!("re_auth_ok: failed to commit TOTP step update: {e}");
            return false;
        }

        true
    } else {
        false
    }
}

// ── GET /api/v1/auth/me/2fa ──────────────────────────────────────────────────

/// Return the current 2FA enrolment status for the authenticated user.
///
/// Accessible from both setup-required and full sessions.
#[utoipa::path(
    get,
    path = "/api/v1/auth/me/2fa",
    responses(
        (status = 200, description = "2FA status", body = uptrakit_web_api_types::mfa::MfaStatusResponse),
        (status = 401, description = "Not authenticated")
    ),
    tag = "Authentication",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn mfa_status(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<AuthenticatedUser>,
) -> Response {
    let user_id = user.user_id;

    let totp_row = match UserTotp::find()
        .filter(user_totp::Column::UserId.eq(user_id))
        .filter(user_totp::Column::IsActive.eq(true))
        .one(state.db())
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("mfa_status: failed to query user_totp: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let totp_enrolled = totp_row.is_some();

    let recovery_codes_count = if totp_enrolled {
        match UserRecoveryCode::find()
            .filter(user_recovery_code::Column::UserId.eq(user_id))
            .filter(user_recovery_code::Column::UsedAt.is_null())
            .count(state.db())
            .await
        {
            Ok(n) => u32::try_from(n).unwrap_or(u32::MAX),
            Err(e) => {
                tracing::error!("mfa_status: failed to count recovery codes: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        }
    } else {
        0
    };

    let response = MfaStatusResponse::new(
        totp_enrolled,
        recovery_codes_count,
        vec![MfaMethod::Totp, MfaMethod::Email],
    );

    (StatusCode::OK, Json(response)).into_response()
}

// ── POST /api/v1/auth/me/2fa/totp/enroll ────────────────────────────────────

/// Begin TOTP enrolment: generate a new secret and return the `otpauth://` URI.
///
/// Accessible from setup-required sessions. Replaces any existing pending row.
#[utoipa::path(
    post,
    path = "/api/v1/auth/me/2fa/totp/enroll",
    responses(
        (status = 200, description = "TOTP enrolment started — returns otpauth URI and secret", body = uptrakit_web_api_types::mfa::TotpEnrollResponse),
        (status = 401, description = "Not authenticated"),
        (status = 409, description = "TOTP already active")
    ),
    tag = "Authentication",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn totp_enroll(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<AuthenticatedUser>,
) -> Response {
    let user_id = user.user_id;

    // Generate a fresh TOTP secret.
    let secret = generate_totp_secret();

    let enc_secret = match EncryptedString::new(secret.clone(), "uptrakit:user_totp:secret") {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("totp_enroll: failed to encrypt secret: {:?}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // BEGIN IMMEDIATE: delete any pending row, insert the new one.
    let txn = match state
        .db()
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("totp_enroll: failed to begin transaction: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Delete any existing pending (is_active=false) TOTP rows.
    if let Err(e) = UserTotp::delete_many()
        .filter(user_totp::Column::UserId.eq(user_id))
        .filter(user_totp::Column::IsActive.eq(false))
        .exec(&txn)
        .await
    {
        tracing::error!("totp_enroll: failed to delete pending TOTP rows: {e}");
        let _ = txn.rollback().await;
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    let now = time::OffsetDateTime::now_utc();

    let new_row = user_totp::ActiveModel {
        id: Set(uuid::Uuid::now_v7()),
        user_id: Set(user_id),
        secret: Set(enc_secret),
        is_active: Set(false),
        enrolled_at: Set(None),
        last_used_step: Set(None),
        created_at: Set(now),
    };
    if let Err(e) = new_row.insert(&txn).await {
        tracing::error!("totp_enroll: failed to insert pending TOTP row: {e}");
        let _ = txn.rollback().await;
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = txn.commit().await {
        tracing::error!("totp_enroll: failed to commit: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    // Load the user's email to build the otpauth URI.
    let user_row = match User::find_by_id(user_id).one(state.db()).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            return error_response(StatusCode::UNAUTHORIZED, "User not found");
        }
        Err(e) => {
            tracing::error!("totp_enroll: failed to load user: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let user_email = user_row.email.expose_email().to_string();
    let otpauth_uri = match build_otpauth_uri(&secret, &user_email, "uptrakit") {
        Some(uri) => uri,
        None => {
            tracing::error!("totp_enroll: build_otpauth_uri returned None for generated secret");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    (
        StatusCode::OK,
        Json(TotpEnrollResponse::new(otpauth_uri, secret)),
    )
        .into_response()
}

// ── POST /api/v1/auth/me/2fa/totp/confirm ───────────────────────────────────

/// Confirm TOTP enrolment: verify the first code and activate the pending row.
///
/// Accessible from setup-required sessions. If the session was setup-required, a
/// full-session JWT is returned so the caller can proceed without re-login.
#[utoipa::path(
    post,
    path = "/api/v1/auth/me/2fa/totp/confirm",
    request_body = uptrakit_web_api_types::mfa::TotpConfirmRequest,
    responses(
        (status = 200, description = "TOTP confirmed — recovery codes returned; session included when upgrading from setup-required", body = uptrakit_web_api_types::mfa::TotpConfirmResponse),
        (status = 400, description = "No pending TOTP enrolment"),
        (status = 401, description = "Not authenticated / wrong code")
    ),
    tag = "Authentication",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn totp_confirm(
    State(state): State<Arc<AppState>>,
    session_svc: crate::extract::SessionSvc,
    axum::Extension(user): axum::Extension<AuthenticatedUser>,
    axum::Extension(setup_required): axum::Extension<SetupRequired>,
    Validated(req): Validated<TotpConfirmRequest>,
) -> Response {
    let user_id = user.user_id;

    // BEGIN IMMEDIATE: load pending row, verify code, activate, replace recovery codes.
    let txn = match state
        .db()
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("totp_confirm: failed to begin transaction: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Load the pending (is_active=false) TOTP row.
    let totp_row = match UserTotp::find()
        .filter(user_totp::Column::UserId.eq(user_id))
        .filter(user_totp::Column::IsActive.eq(false))
        .one(&txn)
        .await
    {
        Ok(Some(row)) => row,
        Ok(None) => {
            let _ = txn.rollback().await;
            return error_response(StatusCode::BAD_REQUEST, "No pending TOTP enrolment found");
        }
        Err(e) => {
            tracing::error!("totp_confirm: failed to load pending TOTP row: {e}");
            let _ = txn.rollback().await;
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let secret = totp_row.secret.expose_secret().to_string();
    let code_owned = req.code.clone();

    let step =
        match tokio::task::spawn_blocking(move || verify_totp_code(&secret, &code_owned)).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("totp_confirm: spawn_blocking panicked: {:?}", e);
                let _ = txn.rollback().await;
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    let Some(step) = step else {
        let _ = txn.rollback().await;
        return error_response(StatusCode::UNAUTHORIZED, "Invalid TOTP code");
    };

    // Activate the TOTP row.
    let now = time::OffsetDateTime::now_utc();
    let mut active = totp_row.into_active_model();
    active.is_active = Set(true);
    active.enrolled_at = Set(Some(now));
    active.last_used_step = Set(Some(step));

    if let Err(e) = active.update(&txn).await {
        tracing::error!("totp_confirm: failed to activate TOTP row: {e}");
        let _ = txn.rollback().await;
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    // Generate and persist recovery codes.
    let plaintext_codes = generate_recovery_codes();

    if let Err(e) = replace_recovery_codes(&txn, user_id, &plaintext_codes).await {
        tracing::error!("totp_confirm: failed to replace recovery codes: {:?}", e);
        let _ = txn.rollback().await;
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = txn.commit().await {
        tracing::error!("totp_confirm: failed to commit: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    // Emit audit event.
    emit_mfa_audit(
        &state,
        uptrakit_audit_log::AuditActionType::AUTH_MFA_ENROLLED,
        user_id,
        uptrakit_audit_log::AuditOutcome::Success,
    );

    // If this was a setup-required session, build a full session.
    let session = if setup_required.0 {
        let user_row = match User::find_by_id(user_id).one(state.db()).await {
            Ok(Some(u)) => u,
            Ok(None) => {
                return error_response(StatusCode::UNAUTHORIZED, "User not found");
            }
            Err(e) => {
                tracing::error!("totp_confirm: failed to load user for session: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

        match build_full_session(&state, &session_svc, &user_row).await {
            Ok(resp) => {
                // Extract the AuthResponse body from the response.
                // We need to parse the body to put into TotpConfirmResponse.
                // Instead, we call create_access_token/refresh directly like build_full_session does.
                // Re-use the same session construction path by delegating entirely.
                // However, build_full_session returns a Response not AuthResponse.
                // We need AuthResponse — re-implement the essentials here.
                let _ = resp; // discard the pre-built response
                match build_session_tokens(&state, &session_svc, &user_row).await {
                    Ok(auth_resp) => Some(auth_resp),
                    Err(err_resp) => return err_resp,
                }
            }
            Err(err_resp) => return err_resp,
        }
    } else {
        None
    };

    (
        StatusCode::OK,
        Json(TotpConfirmResponse::new(plaintext_codes, session)),
    )
        .into_response()
}

// ── POST /api/v1/auth/me/2fa/totp/disable ───────────────────────────────────

/// Disable TOTP for the authenticated user after re-authentication.
///
/// Requires a full session (not setup-required).
#[utoipa::path(
    post,
    path = "/api/v1/auth/me/2fa/totp/disable",
    request_body = uptrakit_web_api_types::mfa::DisableTotpRequest,
    responses(
        (status = 204, description = "TOTP disabled"),
        (status = 401, description = "Not authenticated / wrong credential"),
        (status = 403, description = "Setup-required session cannot disable TOTP")
    ),
    tag = "Authentication",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn totp_disable(
    State(state): State<Arc<AppState>>,
    FullSessionUser(user): FullSessionUser,
    Validated(req): Validated<DisableTotpRequest>,
) -> Response {
    let user_id = user.user_id;

    let password_ref = req.password.as_ref();
    let totp_ref = req.totp_code.as_deref();

    if !re_auth_ok(&state, user_id, password_ref, totp_ref).await {
        return error_response(StatusCode::UNAUTHORIZED, "Re-authentication failed");
    }

    // DELETE all user_totp rows for this user.
    if let Err(e) = UserTotp::delete_many()
        .filter(user_totp::Column::UserId.eq(user_id))
        .exec(state.db())
        .await
    {
        tracing::error!("totp_disable: failed to delete TOTP rows: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    // DELETE all recovery codes.
    if let Err(e) = UserRecoveryCode::delete_many()
        .filter(user_recovery_code::Column::UserId.eq(user_id))
        .exec(state.db())
        .await
    {
        tracing::error!("totp_disable: failed to delete recovery codes: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    emit_mfa_audit(
        &state,
        uptrakit_audit_log::AuditActionType::AUTH_MFA_DISABLED,
        user_id,
        uptrakit_audit_log::AuditOutcome::Success,
    );

    StatusCode::OK.into_response()
}

// ── POST /api/v1/auth/me/2fa/recovery-codes/regenerate ───────────────────────

/// Regenerate recovery codes after re-authentication.
///
/// Requires a full session (not setup-required).
#[utoipa::path(
    post,
    path = "/api/v1/auth/me/2fa/recovery-codes/regenerate",
    request_body = uptrakit_web_api_types::mfa::RegenerateRecoveryCodesRequest,
    responses(
        (status = 200, description = "New recovery codes", body = uptrakit_web_api_types::mfa::RegenerateRecoveryCodesResponse),
        (status = 401, description = "Not authenticated / wrong credential"),
        (status = 403, description = "Setup-required session cannot regenerate codes"),
        (status = 404, description = "No active TOTP to regenerate codes for")
    ),
    tag = "Authentication",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn regenerate_recovery_codes(
    State(state): State<Arc<AppState>>,
    FullSessionUser(user): FullSessionUser,
    Validated(req): Validated<RegenerateRecoveryCodesRequest>,
) -> Response {
    let user_id = user.user_id;

    let password_ref = req.password.as_ref();
    let totp_ref = req.totp_code.as_deref();

    if !re_auth_ok(&state, user_id, password_ref, totp_ref).await {
        return error_response(StatusCode::UNAUTHORIZED, "Re-authentication failed");
    }

    let plaintext_codes = generate_recovery_codes();

    // BEGIN IMMEDIATE: replace recovery codes.
    let txn = match state
        .db()
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("regenerate_recovery_codes: failed to begin transaction: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = replace_recovery_codes(&txn, user_id, &plaintext_codes).await {
        tracing::error!(
            "regenerate_recovery_codes: failed to replace recovery codes: {:?}",
            e
        );
        let _ = txn.rollback().await;
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = txn.commit().await {
        tracing::error!("regenerate_recovery_codes: failed to commit: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    emit_mfa_audit(
        &state,
        uptrakit_audit_log::AuditActionType::AUTH_MFA_RECOVERY_REGENERATED,
        user_id,
        uptrakit_audit_log::AuditOutcome::Success,
    );

    (
        StatusCode::OK,
        Json(RegenerateRecoveryCodesResponse::new(plaintext_codes)),
    )
        .into_response()
}

// ── Internal: build session tokens ───────────────────────────────────────────

/// Build a full-session [`AuthResponse`] for `user` after TOTP enrolment.
///
/// Mirrors the token-construction logic of [`build_full_session`] but returns
/// the typed [`AuthResponse`] value instead of a [`Response`].
async fn build_session_tokens(
    state: &AppState,
    session_svc: &crate::extract::SessionSvc,
    user: &uptrakit_shared_db::entity::user::Model,
) -> Result<uptrakit_web_api_types::auth::AuthResponse, Response> {
    use crate::auth::AuthMethod;
    use crate::middleware::require_auth::get_user_permissions;
    use crate::routes::mfa::UserResponse;
    use uptrakit_web_api_types::SecretString;
    use uptrakit_web_api_types::auth::AuthResponse;

    let permissions = match get_user_permissions(state.db(), state.default_tenant_id, user.id).await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("build_session_tokens: failed to get permissions: {:?}", e);
            vec![]
        }
    };

    let refresh_token = match session_svc
        .create_refresh_token(user.id, AuthMethod::Password, None, None)
        .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(
                "build_session_tokens: failed to create refresh token: {:?}",
                e
            );
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    let access_token =
        match state
            .auth
            .jwt
            .create_access_token(user.id, &permissions, "password", None, None)
        {
            Ok(t) => t,
            Err(e) => {
                tracing::error!(
                    "build_session_tokens: failed to create access token: {:?}",
                    e
                );
                return Err(error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error",
                ));
            }
        };

    let has_pending_email_change = uptrakit_shared_db::entity::prelude::EmailChangeRequest::find()
        .filter(uptrakit_shared_db::entity::email_change_request::Column::UserId.eq(user.id))
        .filter(
            uptrakit_shared_db::entity::email_change_request::Column::ExpiresAt
                .gt(time::OffsetDateTime::now_utc()),
        )
        .one(state.db())
        .await
        .unwrap_or(None)
        .is_some();

    Ok(AuthResponse {
        access_token: SecretString::new(access_token),
        refresh_token: SecretString::new(refresh_token),
        expires_in: state.auth.jwt.expires_in(),
        token_type: "Bearer".to_string(),
        user: UserResponse {
            id: user.id,
            email: user.email.expose_email().to_string(),
            first_name: user.first_name.clone(),
            last_name: user.last_name.clone(),
            permissions,
            has_pending_email_change,
        },
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ServiceCredentialSources;
    use crate::auth::password;
    use crate::auth::session::SessionService;
    use sea_orm::{ConnectOptions, Database, DatabaseConnection, EntityTrait, Set};
    use time::OffsetDateTime;
    use uptrakit_shared_db::entity::{tenant, user};
    use uptrakit_shared_types::MaskedEmail;

    async fn setup_test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        let db = Database::connect(opt).await.expect("test db");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("migrations");
        db
    }

    async fn insert_test_user(db: &DatabaseConnection) -> uuid::Uuid {
        let now = OffsetDateTime::now_utc();
        let hash = password::hash_password("correct-horse-battery-staple").expect("hash");
        let user_id = uuid::Uuid::now_v7();
        user::ActiveModel {
            id: Set(user_id),
            email: Set(MaskedEmail::new("2fa-test@example.com")),
            first_name: Set("Two".to_string()),
            last_name: Set("Fa".to_string()),
            password_hash: Set(Some(hash)),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert user");
        user_id
    }

    async fn test_state(db: DatabaseConnection) -> Arc<AppState> {
        use crate::auth::registration::{RegistrationMode, RegistrationSettings};
        use crate::cert_signer::{AgentCertSigner, CertSignerError, SignedCertBundle};
        use crate::settings::Settings;

        struct NoopCertSigner;
        #[async_trait::async_trait]
        impl AgentCertSigner for NoopCertSigner {
            async fn sign_agent_csr(
                &self,
                _: &str,
                _: &uuid::Uuid,
                _: time::Duration,
            ) -> std::result::Result<SignedCertBundle, rootcause::Report<CertSignerError>>
            {
                Err(rootcause::report!(CertSignerError::Signing(
                    "noop".to_string(),
                )))
            }

            fn active_ca_fingerprint(&self) -> String {
                "0".repeat(64)
            }
        }

        let ca_pem = "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n";
        let snapshot_data = crate::ca_snapshot::CaPublicSnapshot {
            active_cert_pem: ca_pem.to_string(),
            active_fingerprint: "0".repeat(64),
            previous_cert_pem: None,
            previous_fingerprint: None,
            trusted_cas: vec![crate::ca_snapshot::TrustedCaPublic {
                cert_pem: ca_pem.to_string(),
                fingerprint: "0".repeat(64),
                not_after: time::OffsetDateTime::now_utc() + time::Duration::days(365),
            }],
            trusted_ca_cns: Vec::new(),
            bundle_pem: ca_pem.to_string(),
            bundle_hash: "0".repeat(64),
            managed: true,
            active_not_after: time::OffsetDateTime::now_utc() + time::Duration::days(365),
            pki_addr: None,
        };
        let (_ca_tx, ca_rx) = tokio::sync::watch::channel(snapshot_data);
        let ca_key_store: crate::CaKeyStoreRef =
            Arc::new(tokio::sync::RwLock::new(crate::ca_snapshot::CaKeyStore {
                active_key_pem: zeroize::Zeroizing::new(String::new()),
                previous_key_pem: None,
                trusted_ca_keys: vec![],
            }));

        let rustls_cfg = {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
            let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
            let cert = rcgen::CertificateParams::new(vec!["localhost".into()])
                .unwrap()
                .self_signed(&key_pair)
                .unwrap();
            let server_config = rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(
                    vec![rustls::pki_types::CertificateDer::from(cert.der().to_vec())],
                    rustls::pki_types::PrivateKeyDer::try_from(key_pair.serialize_der()).unwrap(),
                )
                .unwrap();
            axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(server_config))
        };

        let notification_service = crate::notification_service::NotificationService::new(
            crate::service_connections::ServiceConnectionRegistry::new(),
            uuid::Uuid::nil(),
        );

        let settings = Settings::new(
            RegistrationSettings {
                mode: RegistrationMode::Open,
                token_hash: None,
                require_token_for_oidc: false,
            },
            168,
        );

        let plugin_ops: Arc<dyn uptrakit_plugin_infrastructure_registry::PluginOps> = Arc::new(
            uptrakit_plugin_infrastructure_registry::build_catalog(
                &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
                uptrakit_plugin_infrastructure_registry::InstancePluginStates::all_disabled(),
            )
            .expect("default catalog should build"),
        );

        let notification_dispatcher = crate::notifications::dispatcher::NotificationDispatcher::new(
            db.clone(),
            Arc::clone(&plugin_ops),
            "https://localhost".to_string(),
        );

        let default_tenant_id = tenant::Entity::find()
            .one(&db)
            .await
            .expect("query default tenant")
            .expect("default tenant row")
            .id;

        let (_, config_rx) = uptrakit_config_reload::RuntimeConfigChannels::from_runtime(
            &uptrakit_config_reload::RuntimeConfig::default(),
        );

        Arc::new(AppState {
            db: crate::app_state::DbState::new(db.clone()),
            cert: crate::app_state::CertState {
                ca_snapshot: ca_rx,
                ca_key_store,
                revocation_notify: Arc::new(tokio::sync::Notify::const_new()),
                crl_pem_cache: Arc::new(parking_lot::RwLock::new(String::new())),
                ca_rotation_trigger: Arc::new(tokio::sync::Notify::const_new()),
            },
            auth: crate::app_state::AuthState::new(
                Arc::new(crate::auth::jwt::JwtManager::from_secret(
                    b"test-me-2fa-secret",
                )),
                crate::auth::device_flow::DeviceFlowStore::new(db.clone()),
                crate::auth::rate_limit::RateLimitStore::new(db.clone()),
                Arc::new(crate::auth::token_denylist::TokenDenylist::new()),
            ),
            notification: crate::app_state::NotificationState::new(
                notification_service,
                notification_dispatcher,
                crate::event_broadcaster::EventBroadcaster::new(),
            ),
            broadcast: crate::app_state::BroadcastState {
                update_output_broadcaster:
                    crate::update_output_broadcaster::UpdateOutputBroadcaster::new(),
                batch_progress_broadcaster:
                    crate::batch_progress_broadcaster::BatchProgressBroadcaster::new(),
            },
            #[cfg(feature = "oidc")]
            oidc: crate::app_state::OidcState {
                oidc_flow_store: crate::auth::oidc_state::OidcFlowStore::new(db.clone()),
                account_link_store: crate::auth::oidc_state::AccountLinkStore::new(db.clone()),
                oidc_token_exchange_store: crate::auth::oidc_state::OidcTokenExchangeStore::new(
                    db.clone(),
                ),
                oidc_registration_store: crate::auth::oidc_state::OidcRegistrationStore::new(
                    db.clone(),
                ),
            },
            default_tenant_id,
            settings,
            cert_signer: Arc::new(NoopCertSigner),
            service_connections: crate::service_connections::ServiceConnectionRegistry::new(),
            controller_id: uuid::Uuid::nil(),
            plugin: crate::app_state::PluginState::new(
                plugin_ops,
                Arc::new(crate::global_providers::GlobalProviders::new(db.clone())),
            ),
            credential_sources: ServiceCredentialSources::default(),
            shutdown_token: Default::default(),
            embedded_service_notifier: None,
            audit_log_filter: uptrakit_audit_log::AuditFilter::default(),
            audit_log_dispatcher: uptrakit_audit_log::AuditLogDispatcher::new(Arc::new(
                uptrakit_audit_log::DatabaseBackend::new(db.clone()),
            )),
            audit_emitter: uptrakit_audit_log::AuditEmitter::new(
                uptrakit_audit_log::AuditLogDispatcher::new(Arc::new(
                    uptrakit_audit_log::DatabaseBackend::new(db.clone()),
                )),
            ),
            surface_proxy_deps: crate::app_state::SurfaceProxyDeps::new(
                Arc::new(crate::surface_registry::SurfaceRegistry::new(
                    crate::surface_registry::SurfaceRegistryConfig::default(),
                )),
                Arc::new(crate::surface_proxy::SurfaceProxy::new()),
            ),
            config_test_proxy: Arc::new(crate::config_test_proxy::ConfigTestProxy::new()),
            workload_claim_registry: Arc::new(crate::workload_claims::WorkloadClaimRegistry::new()),
            server: crate::app_state::ServerState::new(
                std::path::PathBuf::from("/tmp/test-pki"),
                rustls_cfg,
            ),
            reject_dangerous_commands: false,
            #[cfg(feature = "interactive")]
            interactive_sessions: crate::interactive_sessions::InteractiveSessionRegistry::new(),
            update_dispatcher: Arc::new(uptrakit_controller_core::update::NoopUpdateDispatcher),
            instance_plugin_snapshot: Arc::new(arc_swap::ArcSwap::from_pointee(
                uptrakit_web_api_queries::instance_plugin_settings::InstancePluginSnapshot::empty(),
            )),
            coordinator_handle: {
                let (tx, _) = tokio::sync::mpsc::unbounded_channel();
                uptrakit_config_reload::ReloadCoordinator::new(vec![], tx).1
            },
            settings_version_cache: uptrakit_config_reload::SettingsVersionCache::new(),
            db_config_rx: config_rx.db,
            network_config_rx: config_rx.network,
            nats_config_rx: config_rx.nats,
            tls_config_rx: config_rx.tls,
            audit_config_rx: config_rx.audit,
            log_config_rx: config_rx.log,
            master_key_config_rx: config_rx.master_key,
            embedded_services_config_rx: config_rx.embedded_services,
            zeroconf_config_rx: config_rx.zeroconf,
            oauth: crate::oauth::OAuthState::disabled(),
        })
    }

    /// Build an [`AuthenticatedUser`] extension for `user_id` with no permissions.
    fn auth_user(user_id: uuid::Uuid) -> AuthenticatedUser {
        use crate::auth::AuthMethod;
        AuthenticatedUser::new(user_id, AuthMethod::Password, vec![], None)
    }

    // ── mfa_status: unenrolled ───────────────────────────────────────────────

    #[tokio::test]
    async fn status_unenrolled_user_returns_false_and_zero_codes() {
        uptrakit_crypto::enable_plaintext_mode();

        let db = setup_test_db().await;
        let user_id = insert_test_user(&db).await;
        let state = test_state(db).await;

        let resp = mfa_status(State(state), axum::Extension(auth_user(user_id))).await;

        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body");
        let parsed: MfaStatusResponse = serde_json::from_slice(&body).expect("parse");
        assert!(!parsed.totp_enrolled);
        assert_eq!(parsed.recovery_codes_count, 0);
    }

    // ── enroll → confirm → status shows enrolled ──────────────────────────────

    #[tokio::test]
    async fn enroll_then_confirm_then_status_shows_enrolled() {
        uptrakit_crypto::enable_plaintext_mode();

        let db = setup_test_db().await;
        let user_id = insert_test_user(&db).await;
        let state = test_state(db.clone()).await;

        // Step 1: enroll.
        let enroll_resp = totp_enroll(
            State(Arc::clone(&state)),
            axum::Extension(auth_user(user_id)),
        )
        .await;
        assert_eq!(enroll_resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(enroll_resp.into_body(), usize::MAX)
            .await
            .expect("body");
        let enroll: TotpEnrollResponse = serde_json::from_slice(&body).expect("parse enroll");

        // Generate a valid TOTP code from the returned secret.
        let code = crate::auth::totp::generate_totp_code(&enroll.secret)
            .expect("should generate current TOTP code");

        // Step 2: confirm.
        let session_svc = crate::extract::SessionSvc::new(SessionService::new(db.clone()));
        let confirm_resp = totp_confirm(
            State(Arc::clone(&state)),
            session_svc,
            axum::Extension(auth_user(user_id)),
            axum::Extension(SetupRequired(false)),
            Validated(TotpConfirmRequest { code }),
        )
        .await;
        assert_eq!(confirm_resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(confirm_resp.into_body(), usize::MAX)
            .await
            .expect("body");
        let confirm: TotpConfirmResponse = serde_json::from_slice(&body).expect("parse confirm");
        assert_eq!(confirm.recovery_codes.len(), 8);
        assert!(
            confirm.session.is_none(),
            "no setup_required → session is None"
        );

        // Step 3: status.
        let status_resp = mfa_status(
            State(Arc::clone(&state)),
            axum::Extension(auth_user(user_id)),
        )
        .await;
        assert_eq!(status_resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(status_resp.into_body(), usize::MAX)
            .await
            .expect("body");
        let status: MfaStatusResponse = serde_json::from_slice(&body).expect("parse status");
        assert!(status.totp_enrolled);
        assert_eq!(status.recovery_codes_count, 8);
    }

    // ── disable with correct password → 200 ─────────────────────────────────

    #[tokio::test]
    async fn disable_with_correct_password_returns_200() {
        uptrakit_crypto::enable_plaintext_mode();

        let db = setup_test_db().await;
        let user_id = insert_test_user(&db).await;
        let state = test_state(db.clone()).await;

        // Enroll first.
        enroll_and_confirm_totp(&db, user_id, Arc::clone(&state)).await;

        let resp = totp_disable(
            State(Arc::clone(&state)),
            FullSessionUser(auth_user(user_id)),
            Validated(DisableTotpRequest {
                password: Some(uptrakit_shared_types::SecretString::new(
                    "correct-horse-battery-staple",
                )),
                totp_code: None,
            }),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::OK);
    }

    // ── disable with wrong password → 401 ───────────────────────────────────

    #[tokio::test]
    async fn disable_with_wrong_password_returns_401() {
        uptrakit_crypto::enable_plaintext_mode();

        let db = setup_test_db().await;
        let user_id = insert_test_user(&db).await;
        let state = test_state(db.clone()).await;

        enroll_and_confirm_totp(&db, user_id, Arc::clone(&state)).await;

        let resp = totp_disable(
            State(Arc::clone(&state)),
            FullSessionUser(auth_user(user_id)),
            Validated(DisableTotpRequest {
                password: Some(uptrakit_shared_types::SecretString::new("wrong-password")),
                totp_code: None,
            }),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ── disable requires full session (setup_required → 403) ─────────────────
    // This is enforced by the FullSessionUser extractor; the test exercises
    // the extractor directly by constructing a SetupRequired(true) extension
    // and calling from_request_parts.

    #[tokio::test]
    async fn full_session_user_extractor_rejects_setup_required_session() {
        use axum::extract::FromRequestParts;
        use axum::http::Request;

        let req = Request::new(axum::body::Body::empty());
        let (mut parts, _) = req.into_parts();
        parts.extensions.insert(SetupRequired(true));
        parts.extensions.insert(auth_user(uuid::Uuid::nil()));

        let result = FullSessionUser::from_request_parts(&mut parts, &()).await;
        assert!(result.is_err(), "setup_required session must be rejected");
        let rejection = result.map(|_| ()).unwrap_err();
        assert_eq!(rejection.status(), StatusCode::FORBIDDEN);
    }

    // ── regenerate with correct password → 200, new codes ───────────────────
    // Note: we use password here rather than TOTP because `enroll_and_confirm_totp`
    // already consumes the current TOTP step. Using it again within the same
    // 30-second window would be rejected by the anti-replay check.

    #[tokio::test]
    async fn regenerate_with_correct_password_returns_new_codes() {
        uptrakit_crypto::enable_plaintext_mode();

        let db = setup_test_db().await;
        let user_id = insert_test_user(&db).await;
        let state = test_state(db.clone()).await;

        enroll_and_confirm_totp(&db, user_id, Arc::clone(&state)).await;

        let resp = regenerate_recovery_codes(
            State(Arc::clone(&state)),
            FullSessionUser(auth_user(user_id)),
            Validated(RegenerateRecoveryCodesRequest {
                password: Some(uptrakit_shared_types::SecretString::new(
                    "correct-horse-battery-staple",
                )),
                totp_code: None,
            }),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body");
        let regen: RegenerateRecoveryCodesResponse = serde_json::from_slice(&body).expect("parse");
        assert_eq!(regen.recovery_codes.len(), 8);
    }

    // ── Helper: enroll and confirm TOTP for a user ──────────────────────────

    async fn enroll_and_confirm_totp(
        db: &DatabaseConnection,
        user_id: uuid::Uuid,
        state: Arc<AppState>,
    ) -> String {
        // Enroll.
        let enroll_resp = totp_enroll(
            State(Arc::clone(&state)),
            axum::Extension(auth_user(user_id)),
        )
        .await;
        assert_eq!(enroll_resp.status(), StatusCode::OK, "enroll must succeed");

        let body = axum::body::to_bytes(enroll_resp.into_body(), usize::MAX)
            .await
            .expect("body");
        let enroll: TotpEnrollResponse = serde_json::from_slice(&body).expect("parse enroll");

        let code = crate::auth::totp::generate_totp_code(&enroll.secret)
            .expect("should generate current TOTP code");

        let session_svc = crate::extract::SessionSvc::new(SessionService::new(db.clone()));

        let confirm_resp = totp_confirm(
            State(Arc::clone(&state)),
            session_svc,
            axum::Extension(auth_user(user_id)),
            axum::Extension(SetupRequired(false)),
            Validated(TotpConfirmRequest { code }),
        )
        .await;
        assert_eq!(
            confirm_resp.status(),
            StatusCode::OK,
            "confirm must succeed"
        );

        enroll.secret
    }
}
