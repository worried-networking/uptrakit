use crate::AppState;
use crate::api_error::ApiError;
use crate::auth::refresh_cookie::{
    clear_refresh_token_cookie, extract_refresh_token_from_cookie, set_refresh_token_cookie,
};
use crate::auth::{AuthError, password, token::generate_uuid};
use crate::auth_audit_classification::AuthErrorAuditExt;
use crate::error_response::error_response;
use crate::extract::SessionSvc;
use crate::middleware::require_auth::{AuthenticatedUser, get_user_permissions};
use axum::{
    Json,
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, Set,
    TransactionTrait,
};
use std::sync::Arc;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::{role, user, user_role};
use uptrakit_shared_types::MaskedEmail;

use crate::auth::AuthMethod;
use crate::extract::Validated;
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
    actor_display: Option<String>,
    outcome: uptrakit_audit_log::AuditOutcome,
    reason_code: Option<&str>,
) {
    let mut details =
        serde_json::Map::from_iter([("auth_method".to_string(), serde_json::json!("password"))]);
    if let Some(reason_code) = reason_code {
        details.insert("reason_code".to_string(), serde_json::json!(reason_code));
    }

    let mut builder =
        uptrakit_audit_log::AuditEntry::builder(uptrakit_audit_log::AuditActionType::AUTH_LOGIN)
            .tenant_scope(state.default_tenant_id)
            .actor(uptrakit_audit_log::AuditActorType::User, actor_id)
            .actor_display_opt(actor_display.clone())
            .outcome(outcome)
            .details(serde_json::Value::Object(details));

    if let Some(actor_id) = actor_id {
        builder = builder.target("user", actor_id.to_string(), actor_display);
    }

    if let Ok(entry) = builder.build() {
        state.audit_emitter.emit_best_effort(entry);
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

    let mut builder = uptrakit_audit_log::AuditEntry::builder(
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
        state.audit_emitter.emit_best_effort(entry);
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

    let mut builder =
        uptrakit_audit_log::AuditEntry::builder(uptrakit_audit_log::AuditActionType::AUTH_LOGOUT)
            .tenant_scope(state.default_tenant_id)
            .actor(uptrakit_audit_log::AuditActorType::User, Some(actor_id))
            .outcome(outcome)
            .details(serde_json::Value::Object(details));

    if let Some(target_user_id) = target_user_id {
        builder = builder.target("user", target_user_id.to_string(), None);
    }

    if let Ok(entry) = builder.build() {
        state.audit_emitter.emit_best_effort(entry);
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

    let mut builder =
        uptrakit_audit_log::AuditEntry::builder(uptrakit_audit_log::AuditActionType::USER_CREATE)
            .tenant_scope(state.default_tenant_id)
            .actor(uptrakit_audit_log::AuditActorType::User, user_id)
            .outcome(outcome)
            .details(serde_json::Value::Object(details));

    if let Some(user_id) = user_id {
        builder = builder.target("user", user_id.to_string(), None);
    }

    if let Ok(entry) = builder.build() {
        state.audit_emitter.emit_best_effort(entry);
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
    let txn = match state.db().begin().await {
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

    if let Err(e) = new_user.insert(&txn).await {
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

    // Atomically check if this is the first user (threshold 1 because we just inserted)
    // and assign owner role + complete initial setup inside the same transaction.
    let is_first_user =
        match handle_first_user_setup(&txn, &state.settings, state.default_tenant_id, user_id, 1)
            .await
        {
            Ok(is_first) => is_first,
            Err(e) => {
                tracing::error!("Failed to handle first-user setup: {e:?}");
                false
            }
        };

    if !is_first_user
        && let Err(e) = assign_viewer_role(&txn, state.default_tenant_id, user_id).await
    {
        tracing::error!("Failed to assign user role: {e:?}");
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

    emit_user_register_audit(
        &state,
        Some(user_id),
        uptrakit_audit_log::AuditOutcome::Success,
        None,
        Some(is_first_user),
    );

    // Get user permissions
    let permissions = match get_user_permissions(state.db(), state.default_tenant_id, user_id).await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("Failed to get user permissions: {:?}", e);
            vec![]
        }
    };

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
    let access_token =
        match state
            .auth
            .jwt
            .create_access_token(user_id, &permissions, "password", None)
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
            permissions,
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
            Some(req.email.clone()),
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
            Some(req.email.clone()),
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
                Some(req.email.clone()),
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
                Some(req.email.clone()),
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
            Some(user.email.expose_email().to_string()),
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
                Some(user.email.expose_email().to_string()),
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
                Some(user.email.expose_email().to_string()),
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
            Some(user.email.expose_email().to_string()),
            uptrakit_audit_log::AuditOutcome::Denied,
            Some("invalid_credentials"),
        );
        return error_response(StatusCode::UNAUTHORIZED, "Invalid credentials");
    }

    // Get user permissions
    let permissions = match get_user_permissions(state.db(), state.default_tenant_id, user.id).await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("Failed to get user permissions: {:?}", e);
            vec![]
        }
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
                Some(user.email.expose_email().to_string()),
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
            .create_access_token(user.id, &permissions, "password", None)
        {
            Ok(token) => token,
            Err(e) => {
                tracing::error!("Failed to create access token: {:?}", e);
                emit_auth_login_audit(
                    &state,
                    Some(user.id),
                    Some(user.email.expose_email().to_string()),
                    uptrakit_audit_log::AuditOutcome::Failed,
                    Some("access_token_create_failed"),
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    emit_auth_login_audit(
        &state,
        Some(user.id),
        Some(user.email.expose_email().to_string()),
        uptrakit_audit_log::AuditOutcome::Success,
        None,
    );

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
            permissions,
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
    extensions(("x-required-permission" = json!("self"))),
    security(("bearer_token" = []))
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
                    .publish_controller_event(
                        uptrakit_internal_wire::ControllerMessage::TokenRevoked(
                            uptrakit_internal_wire::TokenRevokedPayload {
                                jti: None,
                                exp: None,
                                user_id: Some(verified.user_id),
                                iat_cutoff: Some(now),
                                purge_after: Some(purge_after),
                            },
                        ),
                    )
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
    use super::*;
    use crate::ServiceCredentialSources;
    use crate::auth::permissions::Permission;
    use crate::auth::session::SessionService;
    use axum::body::Body;
    use axum::http::Request;
    use sea_orm::{
        ConnectOptions, ConnectionTrait, Database, DatabaseConnection, EntityTrait, QueryFilter,
        QueryOrder,
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

        Arc::new(AppState {
            db: crate::app_state::DbState::new(db.clone()),
            cert: crate::app_state::CertState {
                ca_snapshot: ca_rx,
                ca_key_store,
                revocation_notify: Arc::new(tokio::sync::Notify::const_new()),
                crl_pem_cache: Arc::new(tokio::sync::RwLock::new(String::new())),
                ca_rotation_trigger: Arc::new(tokio::sync::Notify::const_new()),
            },
            auth: crate::app_state::AuthState {
                jwt: Arc::new(crate::auth::jwt::JwtManager::from_secret(
                    b"test-secret-for-logout-tests",
                )),
                device_flow_store: crate::auth::device_flow::DeviceFlowStore::new(db.clone()),
                rate_limit_store: crate::auth::rate_limit::RateLimitStore::new(db.clone()),
                token_denylist: Arc::new(crate::auth::token_denylist::TokenDenylist::new()),
            },
            notification: crate::app_state::NotificationState {
                notification_service,
                notification_dispatcher,
                event_broadcaster: crate::event_broadcaster::EventBroadcaster::new(),
            },
            broadcast: crate::app_state::BroadcastState {
                device_flow_broadcaster: crate::device_flow_broadcaster::DeviceFlowBroadcaster::new(
                ),
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
            plugin_ops,
            global_providers: Arc::new(crate::global_providers::GlobalProviders::new(db.clone())),
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
            surface_registry: Arc::new(crate::surface_registry::SurfaceRegistry::new(
                crate::surface_registry::SurfaceRegistryConfig::default(),
            )),
            surface_proxy: Arc::new(crate::surface_proxy::SurfaceProxy::new()),
            config_test_proxy: Arc::new(crate::config_test_proxy::ConfigTestProxy::new()),
            workload_claim_registry: Arc::new(crate::workload_claims::WorkloadClaimRegistry::new()),
            pki_path: std::path::PathBuf::from("/tmp/test-pki"),
            rustls_config: rustls_cfg,
            reject_dangerous_commands: false,
            #[cfg(feature = "interactive")]
            interactive_sessions: crate::interactive_sessions::InteractiveSessionRegistry::new(),
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

        let auth_user = AuthenticatedUser {
            user_id,
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::ViewServices],
        };

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

        let auth_user = AuthenticatedUser {
            user_id: User::find().one(&db).await.unwrap().unwrap().id,
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::ViewServices],
        };

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

        let auth_user = AuthenticatedUser {
            user_id,
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::ViewServices],
        };

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

        db.execute_unprepared("DROP TABLE sessions")
            .await
            .expect("drop session table");

        let auth_user = AuthenticatedUser {
            user_id,
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::ViewServices],
        };

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

        let auth_user = AuthenticatedUser {
            user_id,
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::ViewServices],
        };

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
    extensions(("x-required-permission" = json!("self"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn me(
    State(state): State<Arc<AppState>>,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
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

    // Get fresh user permissions from DB
    let permissions = match get_user_permissions(state.db(), state.default_tenant_id, user.id).await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("Failed to get user permissions: {:?}", e);
            vec![]
        }
    };

    let response = UserResponse {
        id: user.id,
        email: user.email.expose_email().to_string(),
        first_name: user.first_name,
        last_name: user.last_name,
        permissions,
        has_pending_email_change: false,
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

    // Rotate refresh token: revoke old, create new
    let (verified, new_refresh_token) = match session_svc.rotate_refresh_token(&refresh_token).await
    {
        Ok(v) => v,
        Err(error) => {
            let (status, outcome, reason_code) =
                error.current_context().refresh_rotation_classification();
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
        // Revoke the newly issued refresh token since user is deactivated
        let _ = session_svc.revoke_refresh_token(&new_refresh_token).await;
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

    // Get fresh permissions from DB
    let permissions = match get_user_permissions(state.db(), state.default_tenant_id, user.id).await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("Failed to get user permissions: {:?}", e);
            vec![]
        }
    };

    // Issue new JWT access token
    let auth_method = verified.auth_method.kind();
    let oidc_provider_id = verified.auth_method.oidc_provider_id();

    let access_token = match state.auth.jwt.create_access_token(
        user.id,
        &permissions,
        auth_method,
        oidc_provider_id,
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

/// Atomically handle first-user registration within a transaction.
///
/// Counts users, and if `count <= threshold`, assigns all roles (owner preset)
/// and completes initial setup (closes registration, clears token).
/// Returns `Ok(true)` if this was the first user.
pub async fn handle_first_user_setup(
    txn: &impl ConnectionTrait,
    settings: &crate::settings::Settings,
    tenant_id: uuid::Uuid,
    user_id: uuid::Uuid,
    threshold: u64,
) -> crate::auth::Result<bool> {
    let user_count = User::find().count(txn).await.context_to()?;
    if user_count > threshold {
        return Ok(false);
    }

    assign_owner_roles(txn, tenant_id, user_id).await?;

    let mut reg = settings.registration();
    reg.complete_initial_setup(txn, tenant_id).await?;
    settings.set_registration(reg).await;

    Ok(true)
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
