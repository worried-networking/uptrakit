use crate::AppState;
use crate::auth::permissions::Permission;
use crate::auth::refresh_cookie::{
    clear_refresh_token_cookie, extract_refresh_token_from_cookie, set_refresh_token_cookie,
};
use crate::auth::{AuthError, password, session::SessionService, token::generate_uuid};
use crate::error_response::error_response;
use crate::middleware::require_auth::AuthenticatedUser;
use axum::{
    Json,
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    PaginatorTrait, QueryFilter, Set, TransactionTrait,
};
use std::sync::Arc;
use time::OffsetDateTime;
use uptrakit_shared_db::MaskedEmail;
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::{permission, role, role_permission, user, user_role};
use uptrakit_web_api_types::SecretString;
use uptrakit_web_api_types::validation::Validate;

pub use uptrakit_web_api_types::auth::{
    AuthResponse, LoginRequest, LogoutRequest, RefreshRequest, RefreshResponse, RegisterRequest,
    UserResponse,
};

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
pub async fn register(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RegisterRequest>,
) -> Response {
    // Check if password auth is enabled
    if !state.settings.authentication().password_auth_enabled {
        return error_response(StatusCode::FORBIDDEN, "Password authentication is disabled");
    }

    if let Err(e) = req.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    // Validate password length
    if let Some(message) = password::validate_password_length(req.password.expose_secret()) {
        return error_response(StatusCode::BAD_REQUEST, message);
    }

    // Validate registration is allowed
    if let Err(e) = state
        .settings
        .registration()
        .validate(req.registration_token.as_ref().map(|t| t.expose_secret()))
    {
        return e.into_response();
    }

    // Hash password
    let password_hash = match password::hash_password(req.password.expose_secret()) {
        Ok(hash) => hash,
        Err(e) => {
            tracing::error!("Password hashing failed: {:?}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Run user creation + first-user check + role assignment inside a transaction
    // to prevent the race where two concurrent registrations both see count == 0.
    let txn = match state.db().begin().await {
        Ok(txn) => txn,
        Err(e) => {
            tracing::error!("Failed to start transaction: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Check if user already exists
    let existing = User::find()
        .filter(user::Column::Email.eq(&req.email))
        .one(&txn)
        .await;

    if let Ok(Some(_)) = existing {
        return error_response(StatusCode::CONFLICT, "Email already exists");
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
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
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

    if !is_first_user && let Err(e) = assign_user_role(&txn, state.default_tenant_id, user_id).await
    {
        tracing::error!("Failed to assign user role: {e:?}");
    }

    if let Err(e) = txn.commit().await {
        tracing::error!("Failed to commit registration transaction: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

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
    let session_service = SessionService::new(state.db().clone());
    let refresh_token = match session_service
        .create_refresh_token(user_id, AuthMethod::Password, None, None)
        .await
    {
        Ok(token) => token,
        Err(e) => {
            tracing::error!("Failed to create refresh token: {:?}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Create JWT access token
    let access_token = match state
        .jwt
        .create_access_token(user_id, &permissions, "password", None)
    {
        Ok(token) => token,
        Err(e) => {
            tracing::error!("Failed to create access token: {:?}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let cookie = set_refresh_token_cookie(&refresh_token);
    let response = AuthResponse {
        access_token: SecretString::new(access_token),
        refresh_token: SecretString::new(refresh_token),
        expires_in: state.jwt.expires_in(),
        token_type: "Bearer".to_string(),
        user: UserResponse {
            id: user_id,
            email: req.email,
            first_name: req.first_name,
            last_name: req.last_name,
            permissions,
        },
    };

    (
        StatusCode::CREATED,
        [(header::SET_COOKIE, cookie)],
        Json(response),
    )
        .into_response()
}

/// Login with email and password
#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Login successful", body = AuthResponse),
        (status = 401, description = "Invalid credentials"),
        (status = 403, description = "User is deactivated")
    ),
    tag = "Authentication"
)]
pub async fn login(State(state): State<Arc<AppState>>, Json(req): Json<LoginRequest>) -> Response {
    // Check if password auth is enabled
    if !state.settings.authentication().password_auth_enabled {
        return error_response(StatusCode::FORBIDDEN, "Password authentication is disabled");
    }

    if let Err(e) = req.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    // Validate password length early to avoid expensive hashing on absurd inputs
    if let Some(message) = password::validate_password_length(req.password.expose_secret()) {
        return error_response(StatusCode::BAD_REQUEST, message);
    }

    // Find user by email
    let user = match User::find()
        .filter(user::Column::Email.eq(&req.email))
        .one(state.db())
        .await
    {
        Ok(Some(user)) => user,
        _ => {
            return error_response(StatusCode::UNAUTHORIZED, "Invalid credentials");
        }
    };

    // Check if user is active
    if !user.is_active {
        return error_response(StatusCode::FORBIDDEN, "User is deactivated");
    }

    // Verify password
    let hash = match user.password_hash.as_ref() {
        Some(h) => h,
        None => {
            return error_response(StatusCode::UNAUTHORIZED, "Invalid credentials");
        }
    };

    let valid = match password::verify_password(req.password.expose_secret(), hash.expose_secret())
    {
        Ok(valid) => valid,
        Err(e) => {
            tracing::error!("Password verification error: {:?}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if !valid {
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
    let session_service = SessionService::new(state.db().clone());
    let refresh_token = match session_service
        .create_refresh_token(user.id, AuthMethod::Password, None, None)
        .await
    {
        Ok(token) => token,
        Err(e) => {
            tracing::error!("Failed to create refresh token: {:?}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Create JWT access token
    let access_token = match state
        .jwt
        .create_access_token(user.id, &permissions, "password", None)
    {
        Ok(token) => token,
        Err(e) => {
            tracing::error!("Failed to create access token: {:?}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let cookie = set_refresh_token_cookie(&refresh_token);
    let response = AuthResponse {
        access_token: SecretString::new(access_token),
        refresh_token: SecretString::new(refresh_token),
        expires_in: state.jwt.expires_in(),
        token_type: "Bearer".to_string(),
        user: UserResponse {
            id: user.id,
            email: user.email.expose_email().to_string(),
            first_name: user.first_name,
            last_name: user.last_name,
            permissions,
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
    security(("bearer_token" = []))
)]
pub async fn logout(
    State(state): State<Arc<AppState>>,
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
        let session_service = SessionService::new(state.db().clone());

        // Verify the token to get the user_id before revoking
        if let Ok(verified) = session_service.verify_refresh_token(token).await {
            if verified.user_id != auth_user.user_id {
                return error_response(StatusCode::FORBIDDEN, "Token does not belong to this user");
            }

            // Deny all current access tokens for this user
            let now = time::OffsetDateTime::now_utc().unix_timestamp();
            state
                .token_denylist
                .deny_user(
                    verified.user_id,
                    now + crate::auth::jwt::ACCESS_TOKEN_EXPIRY_SECS,
                )
                .await;
        }

        if let Err(e) = session_service.revoke_refresh_token(token).await {
            tracing::error!("Failed to revoke refresh token: {:?}", e);
        }
    }

    let cookie = clear_refresh_token_cookie();
    (StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        Database::connect(opt).await.expect("test db")
    }

    async fn setup_test_db() -> DatabaseConnection {
        let db = test_db().await;

        db.execute_unprepared(
            "CREATE TABLE users (
                id TEXT PRIMARY KEY,
                email TEXT UNIQUE NOT NULL,
                first_name TEXT NOT NULL,
                last_name TEXT NOT NULL,
                password_hash TEXT,
                is_active INTEGER NOT NULL DEFAULT 1,
                deactivated_at INTEGER,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            )",
        )
        .await
        .unwrap();

        db.execute_unprepared(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                refresh_token_hash TEXT UNIQUE NOT NULL,
                auth_method TEXT NOT NULL,
                oidc_provider_id TEXT,
                token_type TEXT NOT NULL DEFAULT 'refresh_token',
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                revoked_at INTEGER,
                user_agent TEXT,
                ip_address TEXT,
                FOREIGN KEY (user_id) REFERENCES users(id)
            )",
        )
        .await
        .unwrap();

        let now = OffsetDateTime::now_utc();
        let user = user::ActiveModel {
            id: Set(generate_uuid()),
            email: Set(MaskedEmail::new("test@example.com".to_string())),
            first_name: Set("Test".to_string()),
            last_name: Set("User".to_string()),
            password_hash: Set(None),
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
                Err(rootcause::Report::new(CertSignerError::Signing(
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
            db.clone(),
            crate::service_connections::ServiceConnectionRegistry::new(),
            uuid::Uuid::nil(),
        );

        Arc::new(AppState {
            ca_snapshot: ca_rx,
            ca_key_store,
            #[cfg(feature = "oidc")]
            oidc_flow_store: crate::auth::oidc_state::OidcFlowStore::new(db.clone()),
            #[cfg(feature = "oidc")]
            account_link_store: crate::auth::oidc_state::AccountLinkStore::new(db.clone()),
            #[cfg(feature = "oidc")]
            oidc_token_exchange_store: crate::auth::oidc_state::OidcTokenExchangeStore::new(
                db.clone(),
            ),
            #[cfg(feature = "oidc")]
            oidc_registration_store: crate::auth::oidc_state::OidcRegistrationStore::new(
                db.clone(),
            ),
            device_flow_store: crate::auth::device_flow::DeviceFlowStore::new(db.clone()),
            rate_limit_store: crate::auth::rate_limit::RateLimitStore::new(db.clone()),
            db,
            default_tenant_id: uuid::Uuid::nil(),
            settings: Settings::new(
                RegistrationSettings {
                    mode: RegistrationMode::Open,
                    token_hash: None,
                    require_token_for_oidc: false,
                },
                7,
            ),
            cert_signer: Arc::new(NoopCertSigner),
            service_connections: crate::service_connections::ServiceConnectionRegistry::new(),
            revocation_notify: Arc::new(tokio::sync::Notify::const_new()),
            jwt: Arc::new(crate::auth::jwt::JwtManager::from_secret(
                b"test-secret-for-logout-tests",
            )),
            pki_path: std::path::PathBuf::from("/tmp/test-pki"),
            rustls_config: rustls_cfg,
            crl_pem_cache: Arc::new(tokio::sync::RwLock::new(String::new())),
            ca_rotation_trigger: Arc::new(tokio::sync::Notify::const_new()),
            controller_id: uuid::Uuid::nil(),
            notification_service,
            token_denylist: Arc::new(crate::auth::token_denylist::TokenDenylist::new()),
            provider_ops: Arc::new(uptrakit_plugin_registry::PluginRegistry),
        })
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
            permissions: vec![Permission::ViewAgents],
        };

        let req = Request::builder()
            .uri("/api/v1/auth/logout")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "refresh_token": token }).to_string(),
            ))
            .unwrap();

        let response = logout(State(state), axum::Extension(auth_user), req).await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let verified = session_service.verify_refresh_token(&token).await;
        assert!(verified.is_err());
    }

    #[tokio::test]
    async fn logout_rejects_other_user_token() {
        let db = setup_test_db().await;
        let state = test_state(db.clone()).await;

        let now = OffsetDateTime::now_utc();
        let other_user = user::ActiveModel {
            id: Set(generate_uuid()),
            email: Set(MaskedEmail::new("other@example.com".to_string())),
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
            permissions: vec![Permission::ViewAgents],
        };

        let req = Request::builder()
            .uri("/api/v1/auth/logout")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "refresh_token": token }).to_string(),
            ))
            .unwrap();

        let response = logout(State(state), axum::Extension(auth_user), req).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let verified = session_service.verify_refresh_token(&token).await;
        assert!(verified.is_ok());
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
    security(("bearer_token" = []))
)]
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
pub async fn refresh(State(state): State<Arc<AppState>>, req: axum::extract::Request) -> Response {
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
            return error_response(StatusCode::UNAUTHORIZED, "No refresh token provided");
        }
    };

    // Rotate refresh token: revoke old, create new
    let session_service = SessionService::new(state.db().clone());
    let (verified, new_refresh_token) = match session_service
        .rotate_refresh_token(&refresh_token)
        .await
    {
        Ok(v) => v,
        Err(_) => {
            return error_response(StatusCode::UNAUTHORIZED, "Invalid or expired refresh token");
        }
    };

    // Check user is active
    let user = match User::find_by_id(verified.user_id).one(state.db()).await {
        Ok(Some(user)) => user,
        _ => {
            return error_response(StatusCode::UNAUTHORIZED, "User not found");
        }
    };

    if !user.is_active {
        // Revoke the newly issued refresh token since user is deactivated
        let _ = session_service
            .revoke_refresh_token(&new_refresh_token)
            .await;
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

    let access_token =
        match state
            .jwt
            .create_access_token(user.id, &permissions, auth_method, oidc_provider_id)
        {
            Ok(token) => token,
            Err(e) => {
                tracing::error!("Failed to create access token: {:?}", e);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    let cookie = set_refresh_token_cookie(&new_refresh_token);
    let response = RefreshResponse {
        access_token: SecretString::new(access_token),
        refresh_token: SecretString::new(new_refresh_token),
        expires_in: state.jwt.expires_in(),
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
/// Counts users, and if `count <= threshold`, assigns the owner role and
/// completes initial setup (closes registration, clears token).
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

    assign_owner_role(txn, tenant_id, user_id).await?;

    let mut reg = settings.registration();
    reg.complete_initial_setup(txn, tenant_id).await?;
    settings.set_registration(reg).await;

    Ok(true)
}

// Helper functions

pub async fn assign_owner_role(
    db: &impl ConnectionTrait,
    tenant_id: uuid::Uuid,
    user_id: uuid::Uuid,
) -> crate::auth::Result<()> {
    let owner_role = Role::find()
        .filter(role::Column::Name.eq("owner"))
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(AuthError::Internal("owner role not found".to_string())))?;

    let now = OffsetDateTime::now_utc();

    let user_role_model = user_role::ActiveModel {
        tenant_id: Set(tenant_id),
        user_id: Set(user_id),
        role_id: Set(owner_role.id),
        assigned_at: Set(now),
    };

    user_role_model.insert(db).await.context_to()?;

    Ok(())
}

pub async fn assign_user_role(
    db: &impl ConnectionTrait,
    tenant_id: uuid::Uuid,
    user_id: uuid::Uuid,
) -> crate::auth::Result<()> {
    let user_role_entity = Role::find()
        .filter(role::Column::Name.eq("user"))
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(AuthError::Internal("user role not found".to_string())))?;

    let now = OffsetDateTime::now_utc();

    let user_role_model = user_role::ActiveModel {
        tenant_id: Set(tenant_id),
        user_id: Set(user_id),
        role_id: Set(user_role_entity.id),
        assigned_at: Set(now),
    };

    user_role_model.insert(db).await.context_to()?;

    Ok(())
}

/// Resolve the deduplicated set of permissions for a user via user_roles -> role_permissions -> permissions.
pub async fn get_user_permissions(
    db: &DatabaseConnection,
    tenant_id: uuid::Uuid,
    user_id: uuid::Uuid,
) -> crate::auth::Result<Vec<Permission>> {
    // Get user's role IDs
    let user_roles = UserRole::find()
        .filter(user_role::Column::TenantId.eq(tenant_id))
        .filter(user_role::Column::UserId.eq(user_id))
        .all(db)
        .await
        .context_to()?;

    let role_ids: Vec<uuid::Uuid> = user_roles.iter().map(|ur| ur.role_id).collect();

    if role_ids.is_empty() {
        return Ok(vec![]);
    }

    // Get permission IDs for those roles
    let role_perms = RolePermission::find()
        .filter(role_permission::Column::RoleId.is_in(role_ids))
        .all(db)
        .await
        .context_to()?;

    let perm_ids: Vec<uuid::Uuid> = role_perms.iter().map(|rp| rp.permission_id).collect();

    if perm_ids.is_empty() {
        return Ok(vec![]);
    }

    // Get permission names
    let perm_models = uptrakit_shared_db::entity::prelude::Permission::find()
        .filter(permission::Column::Id.is_in(perm_ids))
        .all(db)
        .await
        .context_to()?;

    // Deduplicate and convert to enum
    let mut seen = std::collections::HashSet::new();
    let permissions: Vec<Permission> = perm_models
        .into_iter()
        .filter_map(|p| p.name.parse::<Permission>().ok())
        .filter(|p| seen.insert(p.clone()))
        .collect();

    Ok(permissions)
}
