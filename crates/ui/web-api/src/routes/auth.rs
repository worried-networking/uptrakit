#![expect(
    clippy::let_underscore_must_use,
    reason = "fire-and-forget cleanup sends on error paths intentionally drop results"
)]

use uptrakit_shared_db::begin_immediate;

use crate::AppState;
use crate::api_error::ApiError;
use crate::auth::mfa_challenge::create_mfa_challenge;
use crate::auth::refresh_cookie::{
    clear_refresh_token_cookie, extract_refresh_token_from_cookie, set_refresh_token_cookie,
};
use crate::auth::{AuthError, password, token::generate_uuid};
use crate::auth_audit_classification::AuthErrorAuditExt;
use crate::error_response::error_response;
use crate::extract::SessionSvc;
use crate::middleware::require_auth::AuthenticatedUser;
use axum::{
    Json,
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, Set,
};
use std::sync::Arc;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::{role, user, user_role, user_totp};
use uptrakit_shared_types::MaskedEmail;
use uptrakit_web_api_types::mfa::{MfaChallengeResponse, MfaMethod};

use crate::auth::AuthMethod;
use crate::auth::registration::RegistrationSettings;
use crate::extract::Validated;
use uptrakit_audit_log::{AbsentView, AuditEntry, AuditOutcome, Stateful};
use uptrakit_web_api_queries::queries::users::UserView;
use uptrakit_web_api_types::SecretString;

pub use uptrakit_web_api_types::auth::{
    AuthResponse, LoginRequest, LogoutRequest, RefreshRequest, RefreshResponse, RegisterRequest,
    UserResponse,
};

fn audit_actor_type_for_auth_method(
    auth_method: &AuthMethod,
) -> uptrakit_audit_log::AuditActorType {
    match auth_method {
        AuthMethod::Password | AuthMethod::Oidc { .. } => uptrakit_audit_log::AuditActorType::User,
        AuthMethod::ApiToken => uptrakit_audit_log::AuditActorType::ApiToken,
    }
}

fn emit_auth_login_audit(
    state: &AppState,
    actor_id: Option<uuid::Uuid>,
    outcome: uptrakit_audit_log::AuditOutcome,
    reason_code: Option<&str>,
) {
    let mut details =
        serde_json::Map::from_iter([("auth_method".to_string(), serde_json::json!("password"))]);
    if let Some(reason_code) = reason_code {
        details.insert("reason_code".to_string(), serde_json::json!(reason_code));
    }

    let mut builder = uptrakit_audit_log::AuditEntry::<uptrakit_audit_log::Event>::builder_event(
        uptrakit_audit_log::AuditActionType::AUTH_LOGIN,
    )
    .tenant_scope(state.default_tenant_id)
    .actor(uptrakit_audit_log::AuditActorType::User, actor_id)
    .outcome(outcome)
    .details(serde_json::Value::Object(details));

    if let Some(actor_id) = actor_id {
        builder = builder.target("user", actor_id.to_string(), None);
    }

    if let Ok(entry) = builder.build() {
        state.audit_emitter.emit_event(entry);
    }
}

fn emit_auth_token_refresh_audit(
    state: &AppState,
    actor_type: uptrakit_audit_log::AuditActorType,
    auth_method: Option<&str>,
    actor_id: Option<uuid::Uuid>,
    outcome: uptrakit_audit_log::AuditOutcome,
    reason_code: Option<&str>,
    request_id: Option<String>,
) {
    let mut details = serde_json::Map::new();
    if let Some(auth_method) = auth_method {
        details.insert("auth_method".to_string(), serde_json::json!(auth_method));
    }
    if let Some(reason_code) = reason_code {
        details.insert("reason_code".to_string(), serde_json::json!(reason_code));
    }

    let mut builder = uptrakit_audit_log::AuditEntry::<uptrakit_audit_log::Event>::builder_event(
        uptrakit_audit_log::AuditActionType::AUTH_TOKEN_REFRESH,
    )
    .tenant_scope(state.default_tenant_id)
    .actor(actor_type, actor_id)
    .outcome(outcome)
    .details(serde_json::Value::Object(details))
    .request_id_opt(request_id);

    if let Some(actor_id) = actor_id {
        builder = builder.target("user", actor_id.to_string(), None);
    }

    if let Ok(entry) = builder.build() {
        state.audit_emitter.emit_event(entry);
    }
}

fn emit_auth_logout_audit(
    state: &AppState,
    actor_id: uuid::Uuid,
    target_user_id: Option<uuid::Uuid>,
    outcome: uptrakit_audit_log::AuditOutcome,
    reason_code: Option<&str>,
) {
    let mut details = serde_json::Map::new();
    details.insert(
        "auth_method".to_string(),
        serde_json::json!("refresh_token"),
    );
    if let Some(reason_code) = reason_code {
        details.insert("reason_code".to_string(), serde_json::json!(reason_code));
    }

    let mut builder = uptrakit_audit_log::AuditEntry::<uptrakit_audit_log::Event>::builder_event(
        uptrakit_audit_log::AuditActionType::AUTH_LOGOUT,
    )
    .tenant_scope(state.default_tenant_id)
    .actor(uptrakit_audit_log::AuditActorType::User, Some(actor_id))
    .outcome(outcome)
    .details(serde_json::Value::Object(details));

    if let Some(target_user_id) = target_user_id {
        builder = builder.target("user", target_user_id.to_string(), None);
    }

    if let Ok(entry) = builder.build() {
        state.audit_emitter.emit_event(entry);
    }
}

fn emit_user_register_audit(
    state: &AppState,
    user_id: Option<uuid::Uuid>,
    outcome: uptrakit_audit_log::AuditOutcome,
    reason_code: Option<&str>,
    is_first_user: Option<bool>,
) {
    let mut details =
        serde_json::Map::from_iter([("auth_method".to_string(), serde_json::json!("password"))]);
    if let Some(reason_code) = reason_code {
        details.insert("reason_code".to_string(), serde_json::json!(reason_code));
    }
    if let Some(is_first_user) = is_first_user {
        details.insert(
            "is_first_user".to_string(),
            serde_json::json!(is_first_user),
        );
    }

    let mut builder = uptrakit_audit_log::AuditEntry::<uptrakit_audit_log::Event>::builder_event(
        uptrakit_audit_log::AuditActionType::USER_CREATE,
    )
    .tenant_scope(state.default_tenant_id)
    .actor(uptrakit_audit_log::AuditActorType::User, user_id)
    .outcome(outcome)
    .details(serde_json::Value::Object(details));

    if let Some(user_id) = user_id {
        builder = builder.target("user", user_id.to_string(), None);
    }

    if let Ok(entry) = builder.build() {
        state.audit_emitter.emit_event(entry);
    }
}

/// Effective-action view for embedding a [`UserResponse`] in an auth
/// response. Engine failure degrades to `Unavailable` + an empty list;
/// the auth flow itself proceeds — same carve-out as `me` (spec §3).
pub(crate) async fn effective_actions(
    engine: &uptrakit_controller_core::access::AccessEngine,
    tenant_id: uuid::Uuid,
    user_id: uuid::Uuid,
) -> (Vec<String>, uptrakit_web_api_types::auth::AuthorityStatus) {
    match engine.context(tenant_id, user_id, None).await {
        Ok(ctx) => (
            engine
                .allowed_actions(&ctx)
                .iter()
                .map(ToString::to_string)
                .collect(),
            uptrakit_web_api_types::auth::AuthorityStatus::Ok,
        ),
        Err(e) => {
            tracing::warn!(%user_id, "access engine unavailable while building auth response: {e:?}");
            (
                Vec::new(),
                uptrakit_web_api_types::auth::AuthorityStatus::Unavailable,
            )
        }
    }
}

/// Register a new user
#[utoipa::path(
    post,
    path = "/api/v1/auth/register",
    request_body = RegisterRequest,
    responses(
        (status = 201, description = "User registered successfully", body = AuthResponse),
        (status = 400, description = "Invalid input"),
        (status = 409, description = "Email already exists")
    ),
    tag = "Authentication"
)]
#[tracing::instrument(skip_all)]
pub async fn register(
    State(state): State<Arc<AppState>>,
    session_svc: SessionSvc,
    Validated(req): Validated<RegisterRequest>,
) -> Result<impl IntoResponse, ApiError> {
    // Check if password auth is enabled
    if !state.settings.authentication().password_auth_enabled {
        emit_user_register_audit(
            &state,
            None,
            uptrakit_audit_log::AuditOutcome::Denied,
            Some("password_auth_disabled"),
            None,
        );
        return Ok(error_response(
            StatusCode::FORBIDDEN,
            "Password authentication is disabled",
        ));
    }

    // Validate password length
    if let Some(message) = password::validate_password_length(req.password.expose_secret()) {
        emit_user_register_audit(
            &state,
            None,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            Some("invalid_password_length"),
            None,
        );
        return Ok(error_response(StatusCode::BAD_REQUEST, message));
    }

    // Validate registration is allowed
    if let Err(err) = state
        .settings
        .registration()
        .validate(req.registration_token.as_ref().map(|t| t.expose_secret()))
    {
        emit_user_register_audit(
            &state,
            None,
            uptrakit_audit_log::AuditOutcome::Denied,
            Some("registration_not_allowed"),
            None,
        );
        return Err(err.into());
    }

    // Hash password
    let password_hash = match password::hash_password(req.password.expose_secret()) {
        Ok(hash) => hash,
        Err(e) => {
            tracing::error!("Password hashing failed: {:?}", e);
            emit_user_register_audit(
                &state,
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                Some("password_hash_failed"),
                None,
            );
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    // Run user creation + first-user check + role assignment inside a transaction
    // to prevent the race where two concurrent registrations both see count == 0.
    let txn = match begin_immediate(state.db()).await {
        Ok(txn) => txn,
        Err(e) => {
            tracing::error!("Failed to start transaction: {e}");
            emit_user_register_audit(
                &state,
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                Some("registration_transaction_start_failed"),
                None,
            );
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    // Check if user already exists
    let existing = User::find()
        .filter(user::Column::Email.eq(&req.email))
        .one(&txn)
        .await;

    if let Ok(Some(_)) = existing {
        emit_user_register_audit(
            &state,
            None,
            uptrakit_audit_log::AuditOutcome::Denied,
            Some("email_already_exists"),
            None,
        );
        return Ok(error_response(StatusCode::CONFLICT, "Email already exists"));
    }

    // Create user
    let user_id = generate_uuid();
    let now = OffsetDateTime::now_utc();

    let new_user = user::ActiveModel {
        id: Set(user_id),
        email: Set(MaskedEmail::new(req.email.clone())),
        first_name: Set(req.first_name.clone()),
        last_name: Set(req.last_name.clone()),
        password_hash: Set(Some(password_hash)),
        is_active: Set(true),
        deactivated_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };

    let inserted_user = match new_user.insert(&txn).await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("Failed to create user: {e}");
            emit_user_register_audit(
                &state,
                Some(user_id),
                uptrakit_audit_log::AuditOutcome::Failed,
                Some("user_insert_failed"),
                None,
            );
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    // Atomically check if this is the first user (threshold 1 because we just inserted)
    // and assign owner role + complete initial setup inside the same transaction.
    // A failure here must abort the whole registration (`?` → 500 via the
    // Report<AuthError> → ApiError mapping); the dropped txn rolls back.
    let first_user_registration = handle_first_user_setup(
        &txn,
        &state.settings,
        state.default_tenant_id,
        user_id,
        1,
        ClearDefaultRoles::Keep,
    )
    .await?;
    let is_first_user = first_user_registration.is_some();

    if !is_first_user
        && let Err(e) = assign_viewer_role(&txn, state.default_tenant_id, user_id).await
    {
        tracing::error!("Failed to assign user role: {e:?}");
    }

    let after_view = UserView::from(&inserted_user);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry = AuditEntry::<Stateful>::user_create(&AbsentView(&after_view), &after_view)
        .tenant_scope(state.default_tenant_id)
        .actor(uptrakit_audit_log::AuditActorType::User, Some(user_id))
        .outcome(AuditOutcome::Success)
        .details(serde_json::json!({
            "auth_method": "password",
            "is_first_user": is_first_user,
        }))
        .build();
    match audit_entry {
        Ok(entry) => {
            if let Err(e) = state.audit_emitter.emit_stateful(&txn, &hook, entry).await {
                tracing::error!("Failed to write registration audit log: {e:?}");
                return Ok(error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error",
                ));
            }
        }
        Err(e) => {
            tracing::error!("Failed to build registration audit entry: {e:?}");
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    }

    if let Err(e) = txn.commit().await {
        tracing::error!("Failed to commit registration transaction: {e}");
        emit_user_register_audit(
            &state,
            Some(user_id),
            uptrakit_audit_log::AuditOutcome::Failed,
            Some("registration_commit_failed"),
            Some(is_first_user),
        );
        return Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error",
        ));
    }
    hook.flush_after_commit().await;
    if let Some(reg) = first_user_registration {
        state.settings.set_registration(reg).await;
    }

    // Create refresh token
    let refresh_token = match session_svc
        .create_refresh_token(user_id, AuthMethod::Password, None, None)
        .await
    {
        Ok(token) => token,
        Err(e) => {
            tracing::error!("Failed to create refresh token: {:?}", e);
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    // Create JWT access token
    let access_token = match state
        .auth
        .jwt
        .create_access_token(user_id, "password", None, None)
    {
        Ok(token) => token,
        Err(e) => {
            tracing::error!("Failed to create access token: {:?}", e);
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    let (actions, authority) =
        effective_actions(&state.access_engine, state.default_tenant_id, user_id).await;

    let cookie = set_refresh_token_cookie(&refresh_token);
    let response = AuthResponse {
        access_token: SecretString::new(access_token),
        refresh_token: SecretString::new(refresh_token),
        expires_in: state.auth.jwt.expires_in(),
        token_type: "Bearer".to_string(),
        user: UserResponse {
            id: user_id,
            email: req.email,
            first_name: req.first_name,
            last_name: req.last_name,
            actions,
            authority,
            has_pending_email_change: false,
        },
    };

    Ok((
        StatusCode::CREATED,
        [(header::SET_COOKIE, cookie)],
        Json(response),
    )
        .into_response())
}

/// Login with email and password
#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Login successful", body = AuthResponse),
        (status = 401, description = "Invalid credentials or account deactivated")
    ),
    tag = "Authentication"
)]
#[tracing::instrument(skip_all)]
pub async fn login(
    State(state): State<Arc<AppState>>,
    session_svc: SessionSvc,
    Validated(req): Validated<LoginRequest>,
) -> Response {
    // Check if password auth is enabled
    if !state.settings.authentication().password_auth_enabled {
        emit_auth_login_audit(
            &state,
            None,
            uptrakit_audit_log::AuditOutcome::Denied,
            Some("password_auth_disabled"),
        );
        return error_response(StatusCode::FORBIDDEN, "Password authentication is disabled");
    }

    // Validate password length early to avoid expensive hashing on absurd inputs
    if let Some(message) = password::validate_password_length(req.password.expose_secret()) {
        emit_auth_login_audit(
            &state,
            None,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            Some("invalid_password_length"),
        );
        return error_response(StatusCode::BAD_REQUEST, message);
    }

    // Find user by email
    let user = match User::find()
        .filter(user::Column::Email.eq(&req.email))
        .one(state.db())
        .await
    {
        Ok(Some(user)) => user,
        Ok(None) => {
            emit_auth_login_audit(
                &state,
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                Some("invalid_credentials"),
            );
            return error_response(StatusCode::UNAUTHORIZED, "Invalid credentials");
        }
        Err(e) => {
            tracing::error!("Failed to load user during login: {e}");
            emit_auth_login_audit(
                &state,
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                Some("user_lookup_failed"),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Check if user is active.
    // Return 401 with the same generic message to avoid leaking whether an
    // account exists at all (user-enumeration prevention).
    if !user.is_active {
        emit_auth_login_audit(
            &state,
            Some(user.id),
            uptrakit_audit_log::AuditOutcome::Denied,
            Some("invalid_credentials"),
        );
        return error_response(StatusCode::UNAUTHORIZED, "Invalid credentials");
    }

    // Verify password
    let hash = match user.password_hash.as_ref() {
        Some(h) => h,
        None => {
            emit_auth_login_audit(
                &state,
                Some(user.id),
                uptrakit_audit_log::AuditOutcome::Denied,
                Some("invalid_credentials"),
            );
            return error_response(StatusCode::UNAUTHORIZED, "Invalid credentials");
        }
    };

    let valid = match password::verify_password(req.password.expose_secret(), hash.expose_secret())
    {
        Ok(valid) => valid,
        Err(e) => {
            tracing::error!("Password verification error: {:?}", e);
            emit_auth_login_audit(
                &state,
                Some(user.id),
                uptrakit_audit_log::AuditOutcome::Failed,
                Some("password_verification_error"),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if !valid {
        emit_auth_login_audit(
            &state,
            Some(user.id),
            uptrakit_audit_log::AuditOutcome::Denied,
            Some("invalid_credentials"),
        );
        return error_response(StatusCode::UNAUTHORIZED, "Invalid credentials");
    }

    // Check if user has active TOTP enrolled — if so, issue an MFA challenge.
    let active_totp = match UserTotp::find()
        .filter(user_totp::Column::UserId.eq(user.id))
        .filter(user_totp::Column::IsActive.eq(true))
        .one(state.db())
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to check TOTP status during login: {e}");
            emit_auth_login_audit(
                &state,
                Some(user.id),
                uptrakit_audit_log::AuditOutcome::Failed,
                Some("totp_check_failed"),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if active_totp.is_some() {
        let challenge_token = match create_mfa_challenge(state.db(), user.id).await {
            Ok(t) => t,
            Err(e) => {
                tracing::error!("Failed to create MFA challenge: {:?}", e);
                emit_auth_login_audit(
                    &state,
                    Some(user.id),
                    uptrakit_audit_log::AuditOutcome::Failed,
                    Some("mfa_challenge_create_failed"),
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

        if let Ok(entry) = uptrakit_audit_log::AuditEntry::builder(
            uptrakit_audit_log::AuditActionType::AUTH_MFA_CHALLENGE_ISSUED,
        )
        .tenant_scope(state.default_tenant_id)
        .actor(uptrakit_audit_log::AuditActorType::User, Some(user.id))
        .target("user", user.id.to_string(), None)
        .outcome(uptrakit_audit_log::AuditOutcome::Success)
        .build()
        {
            state.audit_emitter.emit_event(entry);
        }

        return (
            StatusCode::ACCEPTED,
            Json(MfaChallengeResponse::new(
                challenge_token,
                vec![MfaMethod::Totp, MfaMethod::Email],
            )),
        )
            .into_response();
    }

    // No active TOTP: determine whether this is a setup-required session.
    let two_factor_required = state.settings.authentication().two_factor_required;
    let setup_required_claim: Option<bool> = if two_factor_required {
        Some(true)
    } else {
        None
    };

    // Create refresh token
    let refresh_token = match session_svc
        .create_refresh_token(user.id, AuthMethod::Password, None, None)
        .await
    {
        Ok(token) => token,
        Err(e) => {
            tracing::error!("Failed to create refresh token: {:?}", e);
            emit_auth_login_audit(
                &state,
                Some(user.id),
                uptrakit_audit_log::AuditOutcome::Failed,
                Some("refresh_token_create_failed"),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Create JWT access token
    let access_token =
        match state
            .auth
            .jwt
            .create_access_token(user.id, "password", None, setup_required_claim)
        {
            Ok(token) => token,
            Err(e) => {
                tracing::error!("Failed to create access token: {:?}", e);
                emit_auth_login_audit(
                    &state,
                    Some(user.id),
                    uptrakit_audit_log::AuditOutcome::Failed,
                    Some("access_token_create_failed"),
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    emit_auth_login_audit(
        &state,
        Some(user.id),
        uptrakit_audit_log::AuditOutcome::Success,
        None,
    );

    let (actions, authority) =
        effective_actions(&state.access_engine, state.default_tenant_id, user.id).await;

    let cookie = set_refresh_token_cookie(&refresh_token);
    let response = AuthResponse {
        access_token: SecretString::new(access_token),
        refresh_token: SecretString::new(refresh_token),
        expires_in: state.auth.jwt.expires_in(),
        token_type: "Bearer".to_string(),
        user: UserResponse {
            id: user.id,
            email: user.email.expose_email().to_string(),
            first_name: user.first_name,
            last_name: user.last_name,
            actions,
            authority,
            has_pending_email_change: false,
        },
    };

    (
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(response),
    )
        .into_response()
}

/// Logout and revoke refresh token
#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    request_body = LogoutRequest,
    responses(
        (status = 204, description = "Logout successful"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Token does not belong to this user")
    ),
    tag = "Authentication",
    security(("oauth2" = []), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn logout(
    State(state): State<Arc<AppState>>,
    session_svc: SessionSvc,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
    req: axum::extract::Request,
) -> Response {
    // Extract refresh token: prefer cookie, fall back to JSON body
    let cookie_token = extract_refresh_token_from_cookie(&req);
    let body_token = {
        let body_bytes = match axum::body::to_bytes(req.into_body(), 1024 * 16).await {
            Ok(b) => b,
            Err(_) => axum::body::Bytes::new(),
        };
        serde_json::from_slice::<LogoutRequest>(&body_bytes)
            .ok()
            .and_then(|r| {
                r.refresh_token
                    .filter(|t| !t.expose_secret().is_empty())
                    .map(|t| t.expose_secret().to_string())
            })
    };

    let refresh_token = cookie_token.or(body_token);

    if let Some(token) = &refresh_token {
        let mut verified_current_user = false;

        // Verify the token to get the user_id before revoking
        match session_svc.verify_refresh_token(token).await {
            Ok(verified) => {
                if verified.user_id != auth_user.user_id {
                    emit_auth_logout_audit(
                        &state,
                        auth_user.user_id,
                        Some(verified.user_id),
                        uptrakit_audit_log::AuditOutcome::Denied,
                        Some("token_not_owned"),
                    );
                    return error_response(
                        StatusCode::FORBIDDEN,
                        "Token does not belong to this user",
                    );
                }

                verified_current_user = true;

                // Deny all current access tokens for this user.
                //
                // `iat_cutoff = now` ensures only tokens issued before this logout
                // are blocked; tokens from a subsequent login (iat >= now) are
                // allowed immediately. `purge_after = now + ACCESS_TOKEN_EXPIRY_SECS`
                // keeps the entry alive long enough for pre-logout tokens to expire
                // naturally (max lifetime = 15 minutes).
                let now = time::OffsetDateTime::now_utc().unix_timestamp();
                let purge_after = now + crate::auth::jwt::ACCESS_TOKEN_EXPIRY_SECS;
                state
                    .auth
                    .token_denylist
                    .deny_user(verified.user_id, now, purge_after)
                    .await;

                // Propagate to other controller instances via NATS (best-effort).
                // Failure is logged but does not abort the logout — the local
                // denylist and DB write already took effect.
                state
                    .notification
                    .notification_service
                    .publish_controller_event(uptrakit_wire::ControllerMessage::TokenRevoked(
                        uptrakit_wire::TokenRevokedPayload {
                            jti: None,
                            exp: None,
                            user_id: Some(verified.user_id),
                            iat_cutoff: Some(now),
                            purge_after: Some(purge_after),
                        },
                    ))
                    .await;
            }
            Err(error) => {
                let (outcome, reason_code) = error.current_context().logout_verify_classification();
                emit_auth_logout_audit(&state, auth_user.user_id, None, outcome, Some(reason_code));
            }
        }

        if let Err(e) = session_svc.revoke_refresh_token(token).await {
            tracing::error!("Failed to revoke refresh token: {:?}", e);
            if verified_current_user {
                emit_auth_logout_audit(
                    &state,
                    auth_user.user_id,
                    Some(auth_user.user_id),
                    uptrakit_audit_log::AuditOutcome::Failed,
                    Some("refresh_token_revoke_failed"),
                );
            }
        } else if verified_current_user {
            emit_auth_logout_audit(
                &state,
                auth_user.user_id,
                Some(auth_user.user_id),
                uptrakit_audit_log::AuditOutcome::Success,
                None,
            );
        }
    }

    let cookie = clear_refresh_token_cookie();
    (StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]).into_response()
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions use assert!(result.is_ok()) pattern"
    )]

    use super::*;
    use crate::ServiceCredentialSources;
    use crate::auth::registration::RegistrationMode;
    use crate::auth::session::SessionService;
    use axum::body::Body;
    use axum::http::Request;
    use sea_orm::{
        ColumnTrait, ConnectOptions, ConnectionTrait, Database, DatabaseConnection, EntityTrait,
        QueryFilter, QueryOrder,
    };
    use uptrakit_shared_db::entity::{audit_log, tenant};

    async fn setup_test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        let db = Database::connect(opt).await.expect("test db");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("migrations");

        let now = OffsetDateTime::now_utc();
        let password_hash =
            password::hash_password("correct-horse-battery-staple").expect("password hash");
        let user = user::ActiveModel {
            id: Set(generate_uuid()),
            email: Set(MaskedEmail::new("test@example.com")),
            first_name: Set("Test".to_string()),
            last_name: Set("User".to_string()),
            password_hash: Set(Some(password_hash)),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        };
        user.insert(&db).await.unwrap();

        db
    }

    async fn test_state(db: DatabaseConnection) -> Arc<AppState> {
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
                    "noop signer".to_string(),
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
            let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).unwrap();
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
            .expect("default tenant")
            .id;

        let (_, config_rx_for_auth) = uptrakit_config_reload::RuntimeConfigChannels::from_runtime(
            &uptrakit_config_reload::RuntimeConfig::default(),
        );

        Arc::new(AppState {
            db: crate::app_state::DbState::new(db.clone()),
            access_engine: Arc::new(uptrakit_controller_core::access::AccessEngine::new(
                db.clone(),
            )),
            cert: crate::app_state::CertState {
                ca_snapshot: ca_rx,
                ca_key_store,
                revocation_notify: Arc::new(tokio::sync::Notify::const_new()),
                crl_pem_cache: Arc::new(parking_lot::RwLock::new(String::new())),
                ca_rotation_trigger: Arc::new(tokio::sync::Notify::const_new()),
            },
            auth: crate::app_state::AuthState::new(
                Arc::new(crate::auth::jwt::JwtManager::from_secret(
                    b"test-secret-for-logout-tests",
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
            audit_log_filter_rx: tokio::sync::watch::channel(std::sync::Arc::new(
                uptrakit_config_reload::config::AuditConfig::default(),
            ))
            .1,
            audit_log_dispatcher: uptrakit_audit_log::AuditLogDispatcher::new(Arc::new(
                uptrakit_audit_log::DatabaseBackend::new(db.clone()),
            )),
            audit_emitter: uptrakit_audit_log::AuditEmitter::with_backends(
                uptrakit_audit_log::AuditLogDispatcher::new(Arc::new(
                    uptrakit_audit_log::DatabaseBackend::new(db.clone()),
                )),
                Arc::new(uptrakit_audit_log::DatabaseBackend::new(db.clone())),
                Arc::new(uptrakit_audit_log::NoopBackend),
            ),
            surface_proxy_deps: crate::app_state::SurfaceProxyDeps::new(
                Arc::new(crate::surface_registry::SurfaceRegistry::new(
                    crate::surface_registry::SurfaceRegistryConfig::default(),
                )),
                Arc::new(crate::surface_proxy::SurfaceProxy::new()),
                Arc::new(crate::surface_proxy::AllProvidersVisible),
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
            #[cfg(feature = "test-utils")]
            test_reexec_notify: None,
            update_dispatcher: Arc::new(uptrakit_controller_core::update::NoopUpdateDispatcher),
            instance_plugin_snapshot: Arc::new(arc_swap::ArcSwap::from_pointee(
                uptrakit_web_api_queries::instance_plugin_settings::InstancePluginSnapshot::empty(),
            )),
            coordinator_handle: {
                let (tx, _) = tokio::sync::mpsc::unbounded_channel();
                uptrakit_config_reload::ReloadCoordinator::new(
                    vec![],
                    tx,
                    std::sync::Arc::new(uptrakit_config_reload::NoopAlertWriter),
                )
                .1
            },
            settings_version_cache: uptrakit_config_reload::SettingsVersionCache::new(),
            db_config_rx: config_rx_for_auth.db,
            network_config_rx: config_rx_for_auth.network,
            nats_config_rx: config_rx_for_auth.nats,
            tls_config_rx: config_rx_for_auth.tls,
            audit_config_rx: config_rx_for_auth.audit,
            log_config_rx: config_rx_for_auth.log,
            master_key_config_rx: config_rx_for_auth.master_key,
            embedded_services_config_rx: config_rx_for_auth.embedded_services,
            zeroconf_config_rx: config_rx_for_auth.zeroconf,
            oauth: crate::oauth::OAuthState::disabled(),
            config_file_state: tokio::sync::watch::channel(
                uptrakit_config_reload::ConfigFileState::default(),
            )
            .1,
            last_reload: tokio::sync::watch::channel(None).1,
            recent_reload_events: tokio::sync::watch::channel(Vec::new()).1,
        })
    }

    async fn latest_tenant_audit_row(db: &DatabaseConnection) -> audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = audit_log::Entity::find()
                .order_by_desc(audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected tenant audit row");
    }

    async fn latest_tenant_audit_row_for_action(
        db: &DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
    ) -> audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = audit_log::Entity::find()
                .filter(audit_log::Column::ActionType.eq(action_type))
                .order_by_desc(audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query audit rows by action")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected tenant audit row for action {action_type}");
    }

    #[tokio::test]
    async fn logout_revokes_own_token() {
        let db = setup_test_db().await;
        let state = test_state(db.clone()).await;
        let user_id = User::find().one(&db).await.unwrap().unwrap().id;
        let session_service = SessionService::new(db.clone());
        let token = session_service
            .create_refresh_token(user_id, AuthMethod::Password, None, None)
            .await
            .unwrap();

        let auth_user = AuthenticatedUser::new(user_id, AuthMethod::Password, None);

        let req = Request::builder()
            .uri("/api/v1/auth/logout")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "refresh_token": token }).to_string(),
            ))
            .unwrap();

        let response = logout(
            State(state),
            crate::extract::SessionSvc::new(SessionService::new(db.clone())),
            axum::Extension(auth_user),
            req,
        )
        .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let verified = session_service.verify_refresh_token(&token).await;
        assert!(verified.is_err());

        let row = latest_tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::AUTH_LOGOUT,
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
        assert_eq!(row.actor_id, Some(user_id));
        let details = row.details_json.expect("details");
        assert!(details.get("reason_code").is_none());
    }

    #[tokio::test]
    async fn logout_rejects_other_user_token() {
        let db = setup_test_db().await;
        let state = test_state(db.clone()).await;

        let now = OffsetDateTime::now_utc();
        let other_user = user::ActiveModel {
            id: Set(generate_uuid()),
            email: Set(MaskedEmail::new("other@example.com")),
            first_name: Set("Other".to_string()),
            last_name: Set("User".to_string()),
            password_hash: Set(None),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        };
        let other_user = other_user.insert(&db).await.unwrap();

        let session_service = SessionService::new(db.clone());
        let token = session_service
            .create_refresh_token(other_user.id, AuthMethod::Password, None, None)
            .await
            .unwrap();

        let auth_user = AuthenticatedUser::new(
            User::find().one(&db).await.unwrap().unwrap().id,
            AuthMethod::Password,
            None,
        );

        let req = Request::builder()
            .uri("/api/v1/auth/logout")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "refresh_token": token }).to_string(),
            ))
            .unwrap();

        let response = logout(
            State(state),
            crate::extract::SessionSvc::new(SessionService::new(db.clone())),
            axum::Extension(auth_user),
            req,
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let verified = session_service.verify_refresh_token(&token).await;
        assert!(verified.is_ok());

        let row = latest_tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::AUTH_LOGOUT,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["reason_code"], serde_json::json!("token_not_owned"));
    }

    #[tokio::test]
    async fn logout_invalid_refresh_token_writes_auth_logout_denied_audit_event() {
        let db = setup_test_db().await;
        let state = test_state(db.clone()).await;
        let user_id = User::find().one(&db).await.unwrap().unwrap().id;

        let auth_user = AuthenticatedUser::new(user_id, AuthMethod::Password, None);

        let req = Request::builder()
            .uri("/api/v1/auth/logout")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "refresh_token": "invalid-refresh-token" }).to_string(),
            ))
            .unwrap();

        let response = logout(
            State(state),
            crate::extract::SessionSvc::new(SessionService::new(db.clone())),
            axum::Extension(auth_user),
            req,
        )
        .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let row = latest_tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::AUTH_LOGOUT,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(row.actor_id, Some(user_id));
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("invalid_or_expired_refresh_token")
        );
    }

    #[tokio::test]
    async fn logout_verify_persistence_failure_writes_auth_logout_failed_audit_event() {
        let db = setup_test_db().await;
        let state = test_state(db.clone()).await;
        let user_id = User::find().one(&db).await.unwrap().unwrap().id;

        crate::test_harness::fixtures::drop_table(&db, "sessions").await;

        let auth_user = AuthenticatedUser::new(user_id, AuthMethod::Password, None);

        let req = Request::builder()
            .uri("/api/v1/auth/logout")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "refresh_token": "any-token" }).to_string(),
            ))
            .unwrap();

        let response = logout(
            State(state),
            crate::extract::SessionSvc::new(SessionService::new(db.clone())),
            axum::Extension(auth_user),
            req,
        )
        .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let row = latest_tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::AUTH_LOGOUT,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("refresh_token_verify_failed")
        );
    }

    #[tokio::test]
    async fn logout_revoke_persistence_failure_writes_auth_logout_failed_audit_event() {
        let db = setup_test_db().await;
        let state = test_state(db.clone()).await;
        let user_id = User::find().one(&db).await.unwrap().unwrap().id;
        let session_service = SessionService::new(db.clone());
        let token = session_service
            .create_refresh_token(user_id, AuthMethod::Password, None, None)
            .await
            .unwrap();

        db.execute_unprepared(
            "CREATE TRIGGER fail_revoke BEFORE UPDATE OF revoked_at ON sessions BEGIN SELECT RAISE(FAIL, 'forced revoke failure'); END;",
        )
        .await
        .expect("install revoke failure trigger");

        let auth_user = AuthenticatedUser::new(user_id, AuthMethod::Password, None);

        let req = Request::builder()
            .uri("/api/v1/auth/logout")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "refresh_token": token }).to_string(),
            ))
            .unwrap();

        let response = logout(
            State(state),
            crate::extract::SessionSvc::new(SessionService::new(db.clone())),
            axum::Extension(auth_user),
            req,
        )
        .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let row = latest_tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::AUTH_LOGOUT,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("refresh_token_revoke_failed")
        );
    }

    #[tokio::test]
    async fn login_success_writes_auth_login_audit_event() {
        let db = setup_test_db().await;
        let state = test_state(db.clone()).await;

        let response = login(
            State(state),
            crate::extract::SessionSvc::new(SessionService::new(db.clone())),
            crate::extract::Validated(LoginRequest {
                email: "test@example.com".to_string(),
                password: SecretString::new("correct-horse-battery-staple".to_string()),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);

        let row = latest_tenant_audit_row(&db).await;
        assert_eq!(
            uptrakit_audit_log::AuditActionType::AUTH_LOGIN,
            row.action_type
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::User.as_str()
        );
    }

    #[tokio::test]
    async fn login_validation_failure_writes_auth_login_audit_event() {
        let db = setup_test_db().await;
        let state = test_state(db.clone()).await;

        let response = login(
            State(state),
            crate::extract::SessionSvc::new(SessionService::new(db.clone())),
            crate::extract::Validated(LoginRequest {
                email: "test@example.com".to_string(),
                password: SecretString::new("short".to_string()),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let row = latest_tenant_audit_row(&db).await;
        assert_eq!(
            uptrakit_audit_log::AuditActionType::AUTH_LOGIN,
            row.action_type
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("invalid_password_length")
        );
    }

    /// D13: engine unavailable degrades `me` to HTTP 200 with an explicit
    /// `unavailable` authority marker and an empty action list — never a
    /// non-2xx (the SPA logs the user out on any non-2xx `me` response).
    #[tokio::test]
    async fn me_engine_unavailable_is_200_with_unavailable_authority() {
        let db = setup_test_db().await;
        let state = test_state(db.clone()).await;
        let user_id = User::find().one(&db).await.unwrap().unwrap().id;

        let auth_user = AuthenticatedUser::new(user_id, AuthMethod::Password, None);

        let response = me(
            State(state),
            axum::Extension(auth_user),
            axum::Extension(crate::middleware::action::AccessAuthority::Unavailable),
        )
        .await;
        let (parts, body) = response.into_parts();
        assert_eq!(parts.status, StatusCode::OK);
        let bytes = axum::body::to_bytes(body, usize::MAX).await.expect("body");
        let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(json["authority"], "unavailable");
        assert_eq!(json["actions"].as_array().expect("array").len(), 0);
    }

    /// D13: `effective_actions` (the `me`-adjacent helper embedding actions
    /// into login/register responses) degrades the same way when the
    /// engine's own DB access fails — mirrors the engine's own
    /// `context_propagates_db_errors_never_empty_authority` test, which
    /// proves `context()` itself errors against a schema-less connection.
    #[tokio::test]
    async fn effective_actions_degrades_to_unavailable_on_engine_failure() {
        let db = Database::connect("sqlite::memory:").await.expect("connect");
        let engine = uptrakit_controller_core::access::AccessEngine::new(db);
        let (actions, authority) =
            effective_actions(&engine, uuid::Uuid::now_v7(), uuid::Uuid::now_v7()).await;
        assert!(actions.is_empty());
        assert_eq!(
            authority,
            uptrakit_web_api_types::auth::AuthorityStatus::Unavailable
        );
    }

    #[tokio::test]
    async fn register_conflict_writes_user_create_denied_audit_event() {
        let db = setup_test_db().await;
        let state = test_state(db.clone()).await;

        let response = match register(
            State(state),
            crate::extract::SessionSvc::new(SessionService::new(db.clone())),
            crate::extract::Validated(RegisterRequest {
                email: "test@example.com".to_string(),
                first_name: "Test".to_string(),
                last_name: "User".to_string(),
                password: SecretString::new("correct-horse-battery-staple".to_string()),
                registration_token: None,
            }),
        )
        .await
        {
            Ok(response) => response.into_response(),
            Err(_) => panic!("register response"),
        };

        assert_eq!(response.status(), StatusCode::CONFLICT);

        let row = latest_tenant_audit_row(&db).await;
        assert_eq!(
            uptrakit_audit_log::AuditActionType::USER_CREATE,
            row.action_type
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("email_already_exists")
        );
    }

    #[tokio::test]
    async fn token_refresh_failure_writes_auth_token_refresh_audit_event() {
        let db = setup_test_db().await;
        let state = test_state(db.clone()).await;

        let response = refresh(
            State(state),
            crate::extract::SessionSvc::new(SessionService::new(db.clone())),
            Request::builder()
                .uri("/api/v1/auth/refresh")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "refresh_token": "invalid-refresh-token" }).to_string(),
                ))
                .unwrap(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let row = latest_tenant_audit_row(&db).await;
        assert_eq!(row.action_type, "auth.token_refresh");
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::User.as_str()
        );
        assert!(row.actor_id.is_none());
        let details = row.details_json.expect("details");
        assert!(details.get("auth_method").is_none());
        assert_eq!(
            details["reason_code"],
            serde_json::json!("invalid_or_expired_refresh_token")
        );
    }

    #[test]
    fn token_refresh_oidc_sessions_are_audited_as_user_actors() {
        assert_eq!(
            audit_actor_type_for_auth_method(&AuthMethod::Oidc {
                provider_id: uuid::Uuid::now_v7(),
            }),
            uptrakit_audit_log::AuditActorType::User
        );
    }

    #[tokio::test]
    async fn register_success_writes_user_create_audit_event() {
        let db = setup_test_db().await;
        let state = test_state(db.clone()).await;

        let response = match register(
            State(state),
            crate::extract::SessionSvc::new(SessionService::new(db.clone())),
            crate::extract::Validated(RegisterRequest {
                email: "new-user@example.com".to_string(),
                first_name: "New".to_string(),
                last_name: "User".to_string(),
                password: SecretString::new("correct-horse-battery-staple".to_string()),
                registration_token: None,
            }),
        )
        .await
        {
            Ok(response) => response,
            Err(_) => panic!("register response"),
        }
        .into_response();

        assert_eq!(response.status(), StatusCode::CREATED);

        let created_user = User::find()
            .filter(user::Column::Email.eq("new-user@example.com"))
            .one(&db)
            .await
            .expect("query created user")
            .expect("created user row");

        let row = latest_tenant_audit_row(&db).await;
        assert_eq!(
            uptrakit_audit_log::AuditActionType::USER_CREATE,
            row.action_type
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::User.as_str()
        );
        assert_eq!(row.actor_id, Some(created_user.id));
        assert_eq!(row.target_type.as_deref(), Some("user"));
        let expected_target_id = created_user.id.to_string();
        assert_eq!(row.target_id.as_deref(), Some(expected_target_id.as_str()));

        let details = row.details_json.expect("details");
        assert_eq!(details["auth_method"], serde_json::json!("password"));
        assert_eq!(details["is_first_user"], serde_json::json!(false));
    }

    #[test]
    fn classify_refresh_rotation_error_treats_internal_errors_as_failed() {
        use crate::auth_audit_classification::AuthErrorAuditExt;
        let error = rootcause::report!(AuthError::Internal("boom".to_string()));
        let (status, outcome, reason_code) =
            error.current_context().refresh_rotation_classification();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(outcome, uptrakit_audit_log::AuditOutcome::Failed);
        assert_eq!(reason_code, "refresh_rotation_failed");
    }

    async fn insert_active_totp(db: &DatabaseConnection, user_id: uuid::Uuid) {
        use uptrakit_shared_db::entity::user_totp;
        uptrakit_crypto::enable_plaintext_mode();
        let secret = crate::auth::totp::generate_totp_secret();
        let enc = uptrakit_crypto::EncryptedString::new(secret, "uptrakit:user_totp:secret")
            .expect("encrypt");
        user_totp::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            user_id: Set(user_id),
            secret: Set(enc),
            is_active: Set(true),
            enrolled_at: Set(Some(OffsetDateTime::now_utc())),
            last_used_step: Set(None),
            created_at: Set(OffsetDateTime::now_utc()),
        }
        .insert(db)
        .await
        .expect("insert active totp");
    }

    #[tokio::test]
    async fn login_returns_202_when_totp_active() {
        uptrakit_crypto::enable_plaintext_mode();

        let db = setup_test_db().await;
        let state = test_state(db.clone()).await;
        let user_id = User::find().one(&db).await.unwrap().unwrap().id;

        insert_active_totp(&db, user_id).await;

        let response = login(
            State(state),
            crate::extract::SessionSvc::new(crate::auth::session::SessionService::new(db.clone())),
            crate::extract::Validated(LoginRequest {
                email: "test@example.com".to_string(),
                password: SecretString::new("correct-horse-battery-staple".to_string()),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::ACCEPTED);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert!(
            parsed["mfa_token"].is_string(),
            "mfa_token should be present"
        );
        assert!(
            parsed["mfa_methods"].is_array(),
            "mfa_methods should be present"
        );
    }

    #[tokio::test]
    async fn login_returns_setup_required_jwt_when_enforcement_on_and_unenrolled() {
        uptrakit_crypto::enable_plaintext_mode();

        let db = setup_test_db().await;
        let state = test_state(db.clone()).await;

        // Enable two_factor_required in settings.
        let mut auth_settings = state.settings.authentication();
        auth_settings.two_factor_required = true;
        state.settings.set_authentication(auth_settings).await;

        let response = login(
            State(Arc::clone(&state)),
            crate::extract::SessionSvc::new(crate::auth::session::SessionService::new(db.clone())),
            crate::extract::Validated(LoginRequest {
                email: "test@example.com".to_string(),
                password: SecretString::new("correct-horse-battery-staple".to_string()),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json");
        let token_str = parsed["access_token"].as_str().expect("access_token");

        // Decode (without verification) to inspect claims.
        let parts: Vec<&str> = token_str.split('.').collect();
        assert_eq!(parts.len(), 3, "JWT must have 3 parts");
        let payload =
            base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, parts[1])
                .expect("base64 decode claims");
        let claims: serde_json::Value = serde_json::from_slice(&payload).expect("claims json");
        assert_eq!(
            claims["setup_required"],
            serde_json::json!(true),
            "setup_required claim must be true"
        );
    }

    /// A fresh migrated DB with no pre-seeded user, so a subsequently registered user
    /// becomes the first user (threshold 1) and gets roles assigned by
    /// `handle_first_user_setup`.
    async fn setup_empty_test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        let db = Database::connect(opt).await.expect("test db");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("migrations");
        db
    }

    /// Sorted role names assigned to a user. Mirrors assign_owner_roles' list on
    /// purpose — drift in that list must fail these tests loudly.
    fn owner_role_names() -> Vec<String> {
        let mut names: Vec<String> = [
            "viewer",
            "operator",
            "service_manager",
            "software_manager",
            "host_manager",
            "settings_manager",
            "command_manager",
            "system_administrator",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
        names.sort();
        names
    }

    async fn role_names_for_user(
        db: &DatabaseConnection,
        tenant_id: uuid::Uuid,
        user_id: uuid::Uuid,
    ) -> Vec<String> {
        let role_ids: Vec<uuid::Uuid> = user_role::Entity::find()
            .filter(user_role::Column::TenantId.eq(tenant_id))
            .filter(user_role::Column::UserId.eq(user_id))
            .all(db)
            .await
            .expect("user_role rows")
            .into_iter()
            .map(|r| r.role_id)
            .collect();
        let mut names: Vec<String> = Role::find()
            .filter(role::Column::Id.is_in(role_ids))
            .all(db)
            .await
            .expect("role rows")
            .into_iter()
            .map(|r| r.name)
            .collect();
        names.sort();
        names
    }

    /// The helper must return the mutated snapshot WITHOUT publishing it —
    /// publishing is the commit owner's post-commit responsibility. On the old
    /// code this fails: the helper called settings.set_registration() itself.
    #[tokio::test]
    async fn first_user_setup_returns_snapshot_without_publishing() {
        let db = setup_empty_test_db().await;
        let state = test_state(db.clone()).await;
        let mode_before = state.settings.registration().mode;

        let now = OffsetDateTime::now_utc();
        let user = user::ActiveModel {
            id: Set(generate_uuid()),
            email: Set(MaskedEmail::new("first@example.com")),
            first_name: Set("First".to_string()),
            last_name: Set("User".to_string()),
            password_hash: Set(None),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        };
        let inserted = user.insert(&db).await.expect("insert user");

        let txn = begin_immediate(&db).await.expect("begin");
        let reg = handle_first_user_setup(
            &txn,
            &state.settings,
            state.default_tenant_id,
            inserted.id,
            1,
            ClearDefaultRoles::Keep,
        )
        .await
        .expect("first-user setup")
        .expect("must detect the first user");
        txn.commit().await.expect("commit");

        assert_eq!(
            reg.mode,
            RegistrationMode::Closed,
            "returned snapshot must be Closed"
        );
        assert_eq!(
            state.settings.registration().mode,
            mode_before,
            "helper must NOT publish the snapshot"
        );
    }

    /// Above-threshold count → Ok(None), no roles assigned by the helper.
    #[tokio::test]
    async fn first_user_setup_skips_when_not_first() {
        let db = setup_test_db().await; // seeds one user already
        let state = test_state(db.clone()).await;

        let now = OffsetDateTime::now_utc();
        let second = user::ActiveModel {
            id: Set(generate_uuid()),
            email: Set(MaskedEmail::new("second@example.com")),
            first_name: Set("Second".to_string()),
            last_name: Set("User".to_string()),
            password_hash: Set(None),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        };
        let inserted = second.insert(&db).await.expect("insert user");

        let txn = begin_immediate(&db).await.expect("begin");
        let reg = handle_first_user_setup(
            &txn,
            &state.settings,
            state.default_tenant_id,
            inserted.id,
            1,
            ClearDefaultRoles::Keep,
        )
        .await
        .expect("setup call");
        txn.commit().await.expect("commit");

        assert!(reg.is_none(), "second user must not be treated as first");
    }

    /// ClearDefaultRoles::Clear deletes pre-assigned rows before owner assignment
    /// (also proves no PK conflict when a pre-assigned role overlaps the owner set).
    #[tokio::test]
    async fn first_user_setup_clear_removes_preassigned_default_role() {
        let db = setup_empty_test_db().await;
        let state = test_state(db.clone()).await;

        let now = OffsetDateTime::now_utc();
        let user = user::ActiveModel {
            id: Set(generate_uuid()),
            email: Set(MaskedEmail::new("oidc-first@example.com")),
            first_name: Set("Oidc".to_string()),
            last_name: Set("First".to_string()),
            password_hash: Set(None),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        };
        let inserted = user.insert(&db).await.expect("insert user");

        let txn = begin_immediate(&db).await.expect("begin");
        // Stand-in for resolve_oidc_user's best-effort default role (the legacy
        // "user" role no longer exists post-m20260310; viewer overlaps the owner
        // set, so Keep would PK-conflict — Clear must make this succeed).
        assign_viewer_role(&txn, state.default_tenant_id, inserted.id)
            .await
            .expect("pre-assign default role");
        let reg = handle_first_user_setup(
            &txn,
            &state.settings,
            state.default_tenant_id,
            inserted.id,
            1,
            ClearDefaultRoles::Clear,
        )
        .await
        .expect("first-user setup")
        .expect("must detect the first user");
        txn.commit().await.expect("commit");
        drop(reg);

        assert_eq!(
            role_names_for_user(&db, state.default_tenant_id, inserted.id).await,
            owner_role_names(),
            "exactly the owner role set, each exactly once"
        );
    }

    /// The pre-rotation ordering invariant survives independently of the
    /// removed permission load: `refresh` must verify the refresh token AND
    /// check `user.is_active` before calling `rotate_refresh_token`, so a
    /// deactivated-user denial never revokes or replaces the caller's
    /// still-valid old refresh token.
    #[tokio::test]
    async fn refresh_denies_deactivated_user_without_rotating_old_token() {
        let db = setup_test_db().await;
        let state = test_state(db.clone()).await;
        let user_id = User::find().one(&db).await.unwrap().unwrap().id;

        let session_service = SessionService::new(db.clone());
        let refresh_token = session_service
            .create_refresh_token(user_id, AuthMethod::Password, None, None)
            .await
            .expect("create refresh token");

        let mut active: user::ActiveModel = User::find_by_id(user_id)
            .one(&db)
            .await
            .expect("query user")
            .expect("user exists")
            .into();
        active.is_active = Set(false);
        active.deactivated_at = Set(Some(OffsetDateTime::now_utc()));
        active.update(&db).await.expect("deactivate user");

        let response = refresh(
            State(state),
            crate::extract::SessionSvc::new(SessionService::new(db.clone())),
            Request::builder()
                .uri("/api/v1/auth/refresh")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "refresh_token": refresh_token }).to_string(),
                ))
                .unwrap(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        // The deactivation check runs before `rotate_refresh_token`: the
        // caller's original refresh token's session row must be untouched
        // (not revoked, not replaced) so reactivating the user lets the same
        // token succeed on retry.
        let original = uptrakit_shared_db::entity::session::Entity::find()
            .filter(
                uptrakit_shared_db::entity::session::Column::RefreshTokenHash
                    .eq(crate::auth::token::hash_token(&refresh_token)),
            )
            .one(&db)
            .await
            .expect("query")
            .expect("original session row still exists");
        assert!(
            original.revoked_at.is_none(),
            "the pre-existing token must not be revoked on a pre-rotation denial"
        );

        // Exactly one session row exists for the user — no new row was minted.
        let all_sessions = uptrakit_shared_db::entity::session::Entity::find()
            .all(&db)
            .await
            .expect("query");
        assert_eq!(
            all_sessions.len(),
            1,
            "no new session row should be minted before the is_active check passes"
        );
    }

    /// Renames one required role so assign_owner_roles fails mid-bootstrap.
    async fn break_owner_role_assignment(db: &DatabaseConnection) {
        use sea_orm::sea_query::Expr;
        Role::update_many()
            .col_expr(
                role::Column::Name,
                Expr::value("system_administrator_renamed"),
            )
            .filter(role::Column::Name.eq("system_administrator"))
            .exec(db)
            .await
            .expect("rename role");
    }

    fn first_user_request() -> crate::extract::Validated<RegisterRequest> {
        crate::extract::Validated(RegisterRequest {
            email: "first-user@example.com".to_string(),
            first_name: "First".to_string(),
            last_name: "User".to_string(),
            password: SecretString::new("correct-horse-battery-staple".to_string()),
            registration_token: None,
        })
    }

    /// Spec test 1: owner-role failure → 500, NO user committed, snapshot untouched.
    /// RED pre-fix: the old code swallowed the error and committed a viewer-only user.
    #[tokio::test]
    async fn register_first_user_rolls_back_atomically_on_owner_role_failure() {
        let db = setup_empty_test_db().await;
        let state = test_state(db.clone()).await;
        let mode_before = state.settings.registration().mode;

        break_owner_role_assignment(&db).await;

        let result = register(
            State(state.clone()),
            crate::extract::SessionSvc::new(SessionService::new(db.clone())),
            first_user_request(),
        )
        .await;

        let err = match result {
            Ok(_) => panic!("register must fail when owner-role assignment fails"),
            Err(e) => e,
        };
        assert_eq!(
            err.into_response().status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            User::find().count(&db).await.expect("count"),
            0,
            "transaction must roll back: no user row committed"
        );
        assert_eq!(
            state.settings.registration().mode,
            mode_before,
            "registration snapshot must be untouched"
        );
        // Spec test 1 letter: the DB settings rows rolled back too.
        // Verified paths: `crate::settings_store` re-exported at web-api/src/lib.rs:47,
        // `crate::SettingKey` at web-api/src/lib.rs:65; load_setting(db, tenant_id, key)
        // -> Result<Option<serde_json::Value>> (web-api-auth/src/settings_store.rs:143).
        let mode_in_db = crate::settings_store::load_setting(
            &db,
            state.default_tenant_id,
            crate::SettingKey::RegistrationMode,
        )
        .await
        .expect("load registration-mode setting");
        assert_ne!(
            mode_in_db,
            Some(serde_json::Value::String(
                crate::auth::registration::RegistrationMode::Closed
                    .as_str()
                    .to_string()
            )),
            "DB must not record Closed after rollback"
        );
    }

    /// Spec test 3 (publish-deferred integrity): a pre-commit failure AFTER a
    /// successful first-user setup must not leave the snapshot Closed. Injected
    /// via the in-transaction stateful audit emit (drop audit_logs → emit fails
    /// → handler 500s before commit). RED pre-fix: the helper published Closed
    /// before the failure.
    ///
    /// ORDERING DEPENDENCY: this test's power rests on the audit emit sitting
    /// AFTER handle_first_user_setup and BEFORE txn.commit() in register(). If
    /// that ordering ever changes, this test can pass while proving nothing —
    /// re-point the injection at whatever failure site remains between setup
    /// and commit.
    #[tokio::test]
    async fn register_pre_commit_failure_does_not_publish_closed_snapshot() {
        let db = setup_empty_test_db().await;
        let state = test_state(db.clone()).await;
        let mode_before = state.settings.registration().mode;

        crate::test_harness::fixtures::drop_table(&db, "audit_logs").await;

        let response = match register(
            State(state.clone()),
            crate::extract::SessionSvc::new(SessionService::new(db.clone())),
            first_user_request(),
        )
        .await
        {
            Ok(r) => r.into_response(),
            Err(e) => e.into_response(),
        };
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            User::find().count(&db).await.expect("count"),
            0,
            "transaction must roll back"
        );
        assert_eq!(
            state.settings.registration().mode,
            mode_before,
            "snapshot must not be published before commit"
        );
    }

    /// Spec test 4: happy path — exactly the owner role set, snapshot Closed
    /// after commit; a second user (snapshot re-opened) gets viewer only.
    #[tokio::test]
    async fn register_first_user_gets_owner_roles_then_second_gets_viewer() {
        use crate::auth::registration::RegistrationMode;

        let db = setup_empty_test_db().await;
        let state = test_state(db.clone()).await;

        let response = match register(
            State(state.clone()),
            crate::extract::SessionSvc::new(SessionService::new(db.clone())),
            first_user_request(),
        )
        .await
        {
            Ok(r) => r.into_response(),
            Err(_) => panic!("first registration must succeed"),
        };
        // register()'s success path returns 201 CREATED (auth.rs utoipa doc + handler tail).
        assert_eq!(response.status(), StatusCode::CREATED);

        let first = User::find().one(&db).await.expect("query").expect("user");
        assert_eq!(
            role_names_for_user(&db, state.default_tenant_id, first.id).await,
            owner_role_names(),
            "first user gets exactly the owner role set"
        );
        assert_eq!(
            state.settings.registration().mode,
            RegistrationMode::Closed,
            "snapshot published Closed after commit"
        );

        // Re-open registration in the snapshot so a second user can register.
        let mut reg = state.settings.registration();
        reg.mode = RegistrationMode::Open;
        state.settings.set_registration(reg).await;

        let response2 = match register(
            State(state.clone()),
            crate::extract::SessionSvc::new(SessionService::new(db.clone())),
            crate::extract::Validated(RegisterRequest {
                email: "second-user@example.com".to_string(),
                first_name: "Second".to_string(),
                last_name: "User".to_string(),
                password: SecretString::new("correct-horse-battery-staple".to_string()),
                registration_token: None,
            }),
        )
        .await
        {
            Ok(r) => r.into_response(),
            Err(_) => panic!("second registration must succeed"),
        };
        assert_eq!(response2.status(), StatusCode::CREATED);

        let second = User::find()
            .filter(user::Column::Email.eq("second-user@example.com"))
            .one(&db)
            .await
            .expect("query")
            .expect("second user");
        assert_eq!(
            role_names_for_user(&db, state.default_tenant_id, second.id).await,
            vec!["viewer".to_string()],
            "second user gets the default viewer role only"
        );
    }

    /// M16a-plan3 Task 2: a tenant-scoped role named "viewer" must never
    /// shadow the global built-in — `assign_viewer_role`'s by-name lookup
    /// must stay scoped to `tenant_id IS NULL` rows.
    ///
    /// RED staging (per the M16a-plan3 spec): `.one()` with no `ORDER BY` is
    /// nondeterministic once both the global row and the shadow exist (a
    /// naive "insert shadow, drive, assert global wins" setup can pass
    /// pre-fix by luck — confirmed empirically here). Phase 1 makes the
    /// shadow the ONLY name match by renaming the global row away first,
    /// which is deterministic pre-fix (assigns the shadow) and post-fix
    /// (the scoped query matches nothing, so `assign_viewer_role` errors —
    /// swallowed by `register()`'s best-effort call site, leaving the user
    /// role-less). Phase 2 restores the global row and re-drives with both
    /// rows present as the durable regression pin.
    #[tokio::test]
    async fn assign_viewer_role_ignores_tenant_shadow() {
        use sea_orm::sea_query::Expr;

        let db = setup_empty_test_db().await;
        let state = test_state(db.clone()).await;

        // First user registers to open the tenant up and take the owner
        // role set; subsequent registrations are the ones that drive
        // assign_viewer_role.
        let response = match register(
            State(state.clone()),
            crate::extract::SessionSvc::new(SessionService::new(db.clone())),
            first_user_request(),
        )
        .await
        {
            Ok(r) => r.into_response(),
            Err(_) => panic!("first registration must succeed"),
        };
        assert_eq!(response.status(), StatusCode::CREATED);

        async fn reopen_registration(state: &Arc<AppState>) {
            let mut reg = state.settings.registration();
            reg.mode = RegistrationMode::Open;
            state.settings.set_registration(reg).await;
        }

        async fn register_user(
            state: &Arc<AppState>,
            db: &DatabaseConnection,
            email: &str,
        ) -> user::Model {
            reopen_registration(state).await;
            let response = match register(
                State(state.clone()),
                crate::extract::SessionSvc::new(SessionService::new(db.clone())),
                crate::extract::Validated(RegisterRequest {
                    email: email.to_string(),
                    first_name: "Second".to_string(),
                    last_name: "User".to_string(),
                    password: SecretString::new("correct-horse-battery-staple".to_string()),
                    registration_token: None,
                }),
            )
            .await
            {
                Ok(r) => r.into_response(),
                Err(_) => panic!("registration must succeed"),
            };
            assert_eq!(response.status(), StatusCode::CREATED);
            User::find()
                .filter(user::Column::Email.eq(email))
                .one(db)
                .await
                .expect("query")
                .expect("registered user")
        }

        async fn assigned_role_ids(
            db: &DatabaseConnection,
            tenant_id: uuid::Uuid,
            user_id: uuid::Uuid,
        ) -> Vec<uuid::Uuid> {
            user_role::Entity::find()
                .filter(user_role::Column::TenantId.eq(tenant_id))
                .filter(user_role::Column::UserId.eq(user_id))
                .all(db)
                .await
                .expect("query user_role")
                .into_iter()
                .map(|r| r.role_id)
                .collect()
        }

        // Phase 1: only the shadow matches the name "viewer".
        Role::update_many()
            .col_expr(role::Column::Name, Expr::value("viewer_renamed"))
            .filter(role::Column::Name.eq("viewer"))
            .filter(role::Column::TenantId.is_null())
            .exec(&db)
            .await
            .expect("rename global viewer role away");

        let shadow_role_id = crate::test_harness::fixtures::insert_shadow_role(
            &db,
            state.default_tenant_id,
            "viewer",
        )
        .await;

        let phase1_user = register_user(&state, &db, "viewer-shadow-phase1@example.com").await;
        let phase1_roles = assigned_role_ids(&db, state.default_tenant_id, phase1_user.id).await;
        // `is_empty`, not `!contains(shadow)`: with the global row renamed
        // away the scoped lookup finds nothing and assigns nothing, so the
        // weaker form would pass vacuously and stay green if the filter were
        // reverted to also match tenant rows on an empty result.
        assert!(
            phase1_roles.is_empty(),
            "assign_viewer_role must assign no role at all when only the tenant \
             shadow matches the name; got {phase1_roles:?} (shadow: {shadow_role_id})"
        );

        // Phase 2: restore the global row and re-drive with both rows
        // present — the durable regression pin.
        Role::update_many()
            .col_expr(role::Column::Name, Expr::value("viewer"))
            .filter(role::Column::Name.eq("viewer_renamed"))
            .exec(&db)
            .await
            .expect("restore global viewer role");

        let global_viewer_id = Role::find()
            .filter(role::Column::Name.eq("viewer"))
            .filter(role::Column::TenantId.is_null())
            .one(&db)
            .await
            .expect("query")
            .expect("restored global viewer role")
            .id;

        let phase2_user = register_user(&state, &db, "viewer-shadow-phase2@example.com").await;
        let phase2_roles = assigned_role_ids(&db, state.default_tenant_id, phase2_user.id).await;
        assert_eq!(
            phase2_roles,
            vec![global_viewer_id],
            "assign_viewer_role must pick the global viewer row, not the tenant shadow"
        );
    }

    /// M16a-plan3 Task 2: a tenant-scoped role named "system_administrator"
    /// must never shadow the global built-in — `assign_owner_roles`' by-name
    /// lookup must stay scoped to `tenant_id IS NULL` rows.
    ///
    /// RED staging (per the M16a-plan3 spec): `.one()` with no `ORDER BY` is
    /// nondeterministic once both the global row and the shadow exist, so a
    /// naive "insert shadow, drive, assert global wins" setup could pass
    /// pre-fix by luck. Phase 1 makes the shadow the ONLY name match by
    /// renaming the global row away first, which is deterministic pre-fix
    /// (assigns the shadow) and post-fix (the scoped query matches nothing,
    /// so the lookup errors instead). Phase 2 restores the global row and
    /// re-drives with both rows present as the durable regression pin.
    #[tokio::test]
    async fn assign_owner_roles_ignores_tenant_shadow() {
        use sea_orm::sea_query::Expr;

        let db = setup_empty_test_db().await;
        let state = test_state(db.clone()).await;

        Role::update_many()
            .col_expr(
                role::Column::Name,
                Expr::value("system_administrator_renamed"),
            )
            .filter(role::Column::Name.eq("system_administrator"))
            .exec(&db)
            .await
            .expect("rename global system_administrator role away");

        let shadow_role_id = crate::test_harness::fixtures::insert_shadow_role(
            &db,
            state.default_tenant_id,
            "system_administrator",
        )
        .await;

        let now = OffsetDateTime::now_utc();
        let user_id = crate::auth::token::generate_uuid();
        user::ActiveModel {
            id: Set(user_id),
            email: Set(MaskedEmail::new("owner-shadow-target@example.com")),
            first_name: Set("Owner".to_string()),
            last_name: Set("Shadow".to_string()),
            password_hash: Set(None),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert user");

        // Phase 1: only the shadow matches the name "system_administrator".
        let txn1 = begin_immediate(&db).await.expect("begin phase-1 txn");
        match assign_owner_roles(&txn1, state.default_tenant_id, user_id).await {
            Ok(()) => {
                // Pre-fix: the unscoped lookup found and assigned the
                // tenant shadow instead of erroring — this is the RED case.
                txn1.commit().await.expect("commit phase 1");
            }
            Err(_) => {
                // Post-fix: the scoped query finds no global row (it was
                // renamed away) and errors instead of falling back to the
                // shadow. Roll back EXPLICITLY so the partial writes for the
                // other roles are gone before phase 2 reads: `drop` alone only
                // QUEUES the rollback (see the savepoint note on
                // `sync_oidc_roles`), which is not a contract to assert on.
                txn1.rollback().await.expect("rollback phase 1");
            }
        }

        // Asserted outside the match so it pins BOTH outcomes: whether the
        // call errors or succeeds, the tenant shadow must never end up
        // assigned, even when it is the only name match.
        let shadow_assigned = user_role::Entity::find()
            .filter(user_role::Column::TenantId.eq(state.default_tenant_id))
            .filter(user_role::Column::UserId.eq(user_id))
            .filter(user_role::Column::RoleId.eq(shadow_role_id))
            .one(&db)
            .await
            .expect("query user_role");
        assert!(
            shadow_assigned.is_none(),
            "assign_owner_roles must never assign the tenant shadow role, \
             even when it is the only name match"
        );

        // Phase 2: restore the global row and re-drive with both rows
        // present — the durable regression pin.
        Role::update_many()
            .col_expr(role::Column::Name, Expr::value("system_administrator"))
            .filter(role::Column::Name.eq("system_administrator_renamed"))
            .exec(&db)
            .await
            .expect("restore global system_administrator role");

        let global_owner_role_ids: std::collections::BTreeSet<uuid::Uuid> = Role::find()
            .filter(role::Column::Name.is_in(owner_role_names()))
            .filter(role::Column::TenantId.is_null())
            .all(&db)
            .await
            .expect("query global owner roles")
            .into_iter()
            .map(|r| r.id)
            .collect();

        let txn2 = begin_immediate(&db).await.expect("begin phase-2 txn");
        assign_owner_roles(&txn2, state.default_tenant_id, user_id)
            .await
            .expect("assign_owner_roles must succeed once the global row is restored");
        txn2.commit().await.expect("commit phase 2");

        let assigned_role_ids: std::collections::BTreeSet<uuid::Uuid> = user_role::Entity::find()
            .filter(user_role::Column::TenantId.eq(state.default_tenant_id))
            .filter(user_role::Column::UserId.eq(user_id))
            .all(&db)
            .await
            .expect("query user_role")
            .into_iter()
            .map(|r| r.role_id)
            .collect();

        assert_eq!(
            assigned_role_ids, global_owner_role_ids,
            "assign_owner_roles must assign exactly the global role ids, never the tenant shadow"
        );
        assert!(
            !assigned_role_ids.contains(&shadow_role_id),
            "the tenant shadow id must never appear among the assigned roles"
        );
    }
}

/// Query parameters for the email-change confirmation endpoint.
#[derive(serde::Deserialize, utoipa::IntoParams)]
pub struct ConfirmEmailChangeQuery {
    /// One-time email change confirmation token.
    pub token: String,
}

/// Confirm an email change via a one-time token.
///
/// `GET /api/v1/auth/email-change/confirm?token=<token>` — public, no auth required.
#[utoipa::path(
    get,
    path = "/api/v1/auth/email-change/confirm",
    params(ConfirmEmailChangeQuery),
    responses(
        (status = 200, description = "Email changed; all sessions invalidated. Sign in again.", body = uptrakit_web_api_types::agents::MessageResponse),
        (status = 400, description = "Missing token parameter"),
        (status = 404, description = "Invalid or expired token, or user no longer exists"),
        (status = 409, description = "Email already in use by another account"),
        (status = 410, description = "Token has expired")
    ),
    tag = "Authentication"
)]
#[tracing::instrument(skip_all)]
pub async fn confirm_email_change(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<ConfirmEmailChangeQuery>,
) -> Response {
    use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait as _, QueryFilter, Set};
    use uptrakit_shared_db::entity::{email_change_request, prelude::*};

    let raw_token = params.token;

    let token_hash = crate::auth::token::hash_token(&raw_token);
    let now = time::OffsetDateTime::now_utc();

    let txn = match begin_immediate(state.db()).await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "failed to begin transaction");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let request_row = match EmailChangeRequest::find()
        .filter(email_change_request::Column::TokenHash.eq(&token_hash))
        .one(&txn)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Invalid or expired token"),
        Err(e) => {
            tracing::error!(error = %e, "failed to look up email change request");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if now >= request_row.expires_at {
        let _ = EmailChangeRequest::delete_by_id(request_row.id)
            .exec(&txn)
            .await;
        let _ = txn.commit().await;
        return error_response(StatusCode::GONE, "Token has expired");
    }

    let user_id = request_row.user_id;
    let new_email_plain = request_row.new_email.expose_secret().to_string();

    // Race condition check: new email not taken by another user
    let conflict = User::find()
        .filter(
            uptrakit_shared_db::entity::user::Column::Email
                .eq(uptrakit_shared_types::MaskedEmail::new(&new_email_plain)),
        )
        .filter(uptrakit_shared_db::entity::user::Column::Id.ne(user_id))
        .one(&txn)
        .await;
    if let Ok(Some(_)) = conflict {
        let _ = EmailChangeRequest::delete_by_id(request_row.id)
            .exec(&txn)
            .await;
        let _ = txn.commit().await;
        return error_response(StatusCode::CONFLICT, "Email address is already in use");
    }

    let user = match User::find_by_id(user_id).one(&txn).await {
        Ok(Some(u)) => u,
        _ => return error_response(StatusCode::NOT_FOUND, "User not found"),
    };

    let mut active: uptrakit_shared_db::entity::user::ActiveModel = user.into();
    active.email = Set(uptrakit_shared_types::MaskedEmail::new(&new_email_plain));
    active.updated_at = Set(now);
    if let Err(e) = active.update(&txn).await {
        tracing::error!(error = %e, "failed to update user email");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = EmailChangeRequest::delete_by_id(request_row.id)
        .exec(&txn)
        .await
    {
        tracing::error!(error = %e, "failed to delete email change request");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = txn.commit().await {
        tracing::error!(error = %e, "failed to commit email change");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    // Invalidate all sessions and access tokens
    let session_service = crate::auth::session::SessionService::new(state.db().clone());
    if let Err(e) = session_service.delete_user_sessions(user_id).await {
        tracing::warn!(error = %e, "failed to delete sessions after email change");
    }

    let now_ts = now.unix_timestamp();
    let expiry_secs = crate::auth::jwt::ACCESS_TOKEN_EXPIRY_SECS;
    state
        .auth
        .token_denylist
        .deny_user(user_id, now_ts, now_ts + expiry_secs)
        .await;

    // Propagate token revocation to other controller instances (best-effort).
    state
        .notification
        .notification_service
        .publish_controller_event(uptrakit_wire::ControllerMessage::TokenRevoked(
            uptrakit_wire::TokenRevokedPayload {
                jti: None,
                exp: None,
                user_id: Some(user_id),
                iat_cutoff: Some(now_ts),
                purge_after: Some(now_ts + expiry_secs),
            },
        ))
        .await;

    axum::Json(serde_json::json!({ "message": "Email updated. Please sign in again." }))
        .into_response()
}

/// Get current user information
#[utoipa::path(
    get,
    path = "/api/v1/auth/me",
    responses(
        (status = 200, description = "Current user information", body = UserResponse),
        (status = 401, description = "Not authenticated")
    ),
    tag = "Authentication",
    security(("oauth2" = []), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn me(
    State(state): State<Arc<AppState>>,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
    axum::Extension(access): axum::Extension<crate::middleware::action::AccessAuthority>,
) -> Response {
    // Get user info from DB (fresh data)
    let user = match User::find_by_id(auth_user.user_id).one(state.db()).await {
        Ok(Some(user)) => user,
        _ => {
            return error_response(StatusCode::UNAUTHORIZED, "User not found");
        }
    };

    if !user.is_active {
        return error_response(StatusCode::FORBIDDEN, "User is deactivated");
    }

    let (actions, authority) = match access.ready() {
        Some(ctx) => (
            state
                .access_engine
                .allowed_actions(ctx)
                .iter()
                .map(ToString::to_string)
                .collect(),
            uptrakit_web_api_types::auth::AuthorityStatus::Ok,
        ),
        // Engine unavailable: HTTP 200 with an explicit degraded marker —
        // the SPA logs out on any non-2xx from `me`, so a 500 here would
        // eject a logged-in user on a transient DB blip (spec §3; resolved
        // carve-out 09-resolved-questions.md §4).
        None => (
            Vec::new(),
            uptrakit_web_api_types::auth::AuthorityStatus::Unavailable,
        ),
    };

    let has_pending_email_change = EmailChangeRequest::find()
        .filter(uptrakit_shared_db::entity::email_change_request::Column::UserId.eq(user.id))
        .filter(
            uptrakit_shared_db::entity::email_change_request::Column::ExpiresAt
                .gt(OffsetDateTime::now_utc()),
        )
        .one(state.db())
        .await
        .unwrap_or(None)
        .is_some();

    let response = UserResponse {
        id: user.id,
        email: user.email.expose_email().to_string(),
        first_name: user.first_name,
        last_name: user.last_name,
        actions,
        authority,
        has_pending_email_change,
    };

    (StatusCode::OK, Json(response)).into_response()
}

/// Refresh an access token using a refresh token
#[utoipa::path(
    post,
    path = "/api/v1/auth/refresh",
    request_body = RefreshRequest,
    responses(
        (status = 200, description = "Token refreshed", body = RefreshResponse),
        (status = 401, description = "Invalid or expired refresh token")
    ),
    tag = "Authentication"
)]
#[tracing::instrument(skip_all)]
pub async fn refresh(
    State(state): State<Arc<AppState>>,
    session_svc: SessionSvc,
    req: axum::extract::Request,
) -> Response {
    let request_id = req
        .extensions()
        .get::<crate::middleware::request_id::RequestId>()
        .map(|value| value.0.clone());

    // Extract refresh token: prefer cookie, fall back to JSON body
    let cookie_token = extract_refresh_token_from_cookie(&req);
    let body_token = {
        let body_bytes = match axum::body::to_bytes(req.into_body(), 1024 * 16).await {
            Ok(b) => b,
            Err(_) => axum::body::Bytes::new(),
        };
        serde_json::from_slice::<RefreshRequest>(&body_bytes)
            .ok()
            .and_then(|r| {
                r.refresh_token
                    .filter(|t| !t.expose_secret().is_empty())
                    .map(|t| t.expose_secret().to_string())
            })
    };

    let refresh_token = match cookie_token.or(body_token) {
        Some(t) => t,
        None => {
            emit_auth_token_refresh_audit(
                &state,
                uptrakit_audit_log::AuditActorType::User,
                None,
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                Some("missing_refresh_token"),
                request_id.clone(),
            );
            return error_response(StatusCode::UNAUTHORIZED, "No refresh token provided");
        }
    };

    // PRIMARY: verify (non-mutating) before rotating, so a downstream failure never
    // strands the caller's still-valid old token.
    let verified = match session_svc.verify_refresh_token(&refresh_token).await {
        Ok(v) => v,
        Err(error) => {
            let (status, outcome, reason_code) =
                error.current_context().refresh_rotation_classification();
            if status == StatusCode::INTERNAL_SERVER_ERROR {
                tracing::error!("Failed to verify refresh token: {:?}", error);
            }
            emit_auth_token_refresh_audit(
                &state,
                uptrakit_audit_log::AuditActorType::User,
                None,
                None,
                outcome,
                Some(reason_code),
                request_id.clone(),
            );
            return error_response(
                status,
                if status == StatusCode::UNAUTHORIZED {
                    "Invalid or expired refresh token"
                } else {
                    "Internal server error"
                },
            );
        }
    };

    // Check user is active
    let user = match User::find_by_id(verified.user_id).one(state.db()).await {
        Ok(Some(user)) => user,
        Ok(None) => {
            emit_auth_token_refresh_audit(
                &state,
                audit_actor_type_for_auth_method(&verified.auth_method),
                Some(verified.auth_method.kind()),
                Some(verified.user_id),
                uptrakit_audit_log::AuditOutcome::Denied,
                Some("user_not_found"),
                request_id.clone(),
            );
            return error_response(StatusCode::UNAUTHORIZED, "User not found");
        }
        Err(e) => {
            tracing::error!("Failed to load user during token refresh: {e}");
            emit_auth_token_refresh_audit(
                &state,
                audit_actor_type_for_auth_method(&verified.auth_method),
                Some(verified.auth_method.kind()),
                Some(verified.user_id),
                uptrakit_audit_log::AuditOutcome::Failed,
                Some("user_lookup_failed"),
                request_id.clone(),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if !user.is_active {
        // No token has been minted yet at this point (rotation has not happened) — nothing to revoke.
        emit_auth_token_refresh_audit(
            &state,
            audit_actor_type_for_auth_method(&verified.auth_method),
            Some(verified.auth_method.kind()),
            Some(user.id),
            uptrakit_audit_log::AuditOutcome::Denied,
            Some("user_deactivated"),
            request_id.clone(),
        );
        return error_response(StatusCode::FORBIDDEN, "User is deactivated");
    }

    // Only now rotate: revoke old, mint new. All validation has passed.
    let (verified, new_refresh_token) = match session_svc.rotate_refresh_token(&refresh_token).await
    {
        Ok(v) => v,
        Err(error) => {
            let (status, outcome, reason_code) =
                error.current_context().refresh_rotation_classification();
            if status == StatusCode::INTERNAL_SERVER_ERROR {
                tracing::error!("Failed to rotate refresh token: {:?}", error);
            }
            emit_auth_token_refresh_audit(
                &state,
                uptrakit_audit_log::AuditActorType::User,
                None,
                None,
                outcome,
                Some(reason_code),
                request_id.clone(),
            );
            return error_response(
                status,
                if status == StatusCode::UNAUTHORIZED {
                    "Invalid or expired refresh token"
                } else {
                    "Internal server error"
                },
            );
        }
    };

    // Determine setup_required: if 2FA is enforced and user has no active TOTP, carry it forward.
    let active_totp_on_refresh = UserTotp::find()
        .filter(user_totp::Column::UserId.eq(user.id))
        .filter(user_totp::Column::IsActive.eq(true))
        .one(state.db())
        .await
        .ok()
        .flatten();
    let refresh_two_factor_required = state.settings.authentication().two_factor_required;
    let setup_required_claim: Option<bool> =
        if refresh_two_factor_required && active_totp_on_refresh.is_none() {
            Some(true)
        } else {
            None
        };

    // Issue new JWT access token
    let auth_method = verified.auth_method.kind();
    let oidc_provider_id = verified.auth_method.oidc_provider_id();

    let access_token = match state.auth.jwt.create_access_token(
        user.id,
        auth_method,
        oidc_provider_id,
        setup_required_claim,
    ) {
        Ok(token) => token,
        Err(e) => {
            tracing::error!("Failed to create access token: {:?}", e);
            emit_auth_token_refresh_audit(
                &state,
                audit_actor_type_for_auth_method(&verified.auth_method),
                Some(verified.auth_method.kind()),
                Some(user.id),
                uptrakit_audit_log::AuditOutcome::Failed,
                Some("access_token_create_failed"),
                request_id.clone(),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    emit_auth_token_refresh_audit(
        &state,
        audit_actor_type_for_auth_method(&verified.auth_method),
        Some(verified.auth_method.kind()),
        Some(user.id),
        uptrakit_audit_log::AuditOutcome::Success,
        None,
        request_id,
    );

    let cookie = set_refresh_token_cookie(&new_refresh_token);
    let response = RefreshResponse {
        access_token: SecretString::new(access_token),
        refresh_token: SecretString::new(new_refresh_token),
        expires_in: state.auth.jwt.expires_in(),
        token_type: "Bearer".to_string(),
    };

    (
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(response),
    )
        .into_response()
}

/// Boolean-like flag: whether the bootstrap should delete roles pre-assigned
/// to the user (OIDC's `resolve_oidc_user` best-effort default) before
/// assigning the owner set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive] // matches the SchedulerStopMode precedent for pub 2-variant discriminators
pub enum ClearDefaultRoles {
    /// Delete existing `user_role` rows for the user first (OIDC paths).
    Clear,
    /// Leave pre-assigned roles untouched (password path — none exist).
    Keep,
}

/// Atomically handle first-user registration within a transaction.
///
/// Counts users; if `count <= threshold`, optionally clears pre-assigned
/// default roles, assigns all owner roles, and completes initial setup in
/// the DB (closes registration, clears token). Returns `Ok(Some(reg))` with
/// the mutated registration snapshot for the FIRST user, `Ok(None)` otherwise.
///
/// # Contract: publish only after commit
///
/// This function never touches the process-wide settings snapshot. The
/// caller that owns `txn.commit()` MUST call
/// `settings.set_registration(reg)` only AFTER the commit succeeds —
/// publishing earlier re-introduces the closed-registration brick: a failed
/// commit rolls the DB back while the in-memory snapshot stays `Closed`,
/// rejecting every registration on a zero-user instance.
///
/// Deferring is safe: on restart `RegistrationSettings` is re-derived from
/// the DB (zero users → fresh invite token), so a lost post-commit publish
/// self-corrects. Do not "fix" a lost publish by persisting the in-memory
/// snapshot.
///
/// # Errors
///
/// Propagates every DB/setup error so the caller can roll back atomically —
/// callers must NOT demote a failure to "not the first user".
pub async fn handle_first_user_setup(
    txn: &impl ConnectionTrait,
    settings: &crate::settings::Settings,
    tenant_id: uuid::Uuid,
    user_id: uuid::Uuid,
    threshold: u64,
    clear_default_roles: ClearDefaultRoles,
) -> crate::auth::Result<Option<RegistrationSettings>> {
    let user_count = User::find().count(txn).await.context_to()?;
    if user_count > threshold {
        return Ok(None);
    }

    if clear_default_roles == ClearDefaultRoles::Clear {
        user_role::Entity::delete_many()
            .filter(user_role::Column::TenantId.eq(tenant_id))
            .filter(user_role::Column::UserId.eq(user_id))
            .exec(txn)
            .await
            .context_to()?;
    }

    assign_owner_roles(txn, tenant_id, user_id).await?;

    let mut reg = settings.registration();
    reg.complete_initial_setup(txn, tenant_id).await?;
    Ok(Some(reg))
}

// Helper functions

pub async fn assign_owner_roles(
    db: &impl ConnectionTrait,
    tenant_id: uuid::Uuid,
    user_id: uuid::Uuid,
) -> crate::auth::Result<()> {
    let now = OffsetDateTime::now_utc();
    let all_roles = [
        "viewer",
        "operator",
        "service_manager",
        "software_manager",
        "host_manager",
        "settings_manager",
        "command_manager",
        "system_administrator",
    ];
    for role_name in all_roles {
        let role_entity = Role::find()
            .filter(role::Column::Name.eq(role_name))
            .filter(role::Column::TenantId.is_null())
            .one(db)
            .await
            .context_to()?
            .ok_or_else(|| report!(AuthError::Internal(format!("{role_name} role not found"))))?;

        let user_role_model = user_role::ActiveModel {
            tenant_id: Set(tenant_id),
            user_id: Set(user_id),
            role_id: Set(role_entity.id),
            assigned_at: Set(now),
        };
        user_role_model.insert(db).await.context_to()?;
    }
    Ok(())
}

pub async fn assign_viewer_role(
    db: &impl ConnectionTrait,
    tenant_id: uuid::Uuid,
    user_id: uuid::Uuid,
) -> crate::auth::Result<()> {
    let viewer_role = Role::find()
        .filter(role::Column::Name.eq("viewer"))
        .filter(role::Column::TenantId.is_null())
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(AuthError::Internal("viewer role not found".to_string())))?;

    let now = OffsetDateTime::now_utc();
    let user_role_model = user_role::ActiveModel {
        tenant_id: Set(tenant_id),
        user_id: Set(user_id),
        role_id: Set(viewer_role.id),
        assigned_at: Set(now),
    };
    user_role_model.insert(db).await.context_to()?;
    Ok(())
}
