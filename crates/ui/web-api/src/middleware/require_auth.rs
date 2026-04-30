use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use rootcause::prelude::*;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::{permission, role_permission, user_role};

use crate::AppState;
use crate::auth::api_token::ApiTokenService;
use crate::auth::permissions::Permission;
use crate::auth::{AuthError, AuthMethod};
use crate::error_response::error_response;

/// Extension type to carry the authenticated user ID, auth method, and permissions through the request.
#[derive(Clone, Debug)]
pub struct AuthenticatedUser {
    pub user_id: uuid::Uuid,
    pub auth_method: AuthMethod,
    pub permissions: Vec<Permission>,
    /// JTI of the JWT access token, if authenticated via JWT (None for API token auth).
    pub jti: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct AuthenticatedApiTokenId(pub uuid::Uuid);

impl AuthenticatedUser {
    pub fn has_permission(&self, perm: Permission) -> bool {
        self.permissions.contains(&perm)
    }

    pub fn audit_actor(
        &self,
        api_token_id: Option<AuthenticatedApiTokenId>,
    ) -> (uptrakit_audit_log::AuditActorType, Option<uuid::Uuid>) {
        match self.auth_method {
            AuthMethod::ApiToken => (
                uptrakit_audit_log::AuditActorType::ApiToken,
                api_token_id.map(|token| token.0),
            ),
            AuthMethod::Password | AuthMethod::Oidc { .. } => {
                (uptrakit_audit_log::AuditActorType::User, Some(self.user_id))
            }
        }
    }
}

pub fn authenticated_user_audit_actor(
    user: &AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
) -> (uptrakit_audit_log::AuditActorType, Option<uuid::Uuid>) {
    user.audit_actor(api_token_id)
}

pub(crate) fn emit_api_token_auth_audit(
    state: &AppState,
    request_id: Option<String>,
    outcome: uptrakit_audit_log::AuditOutcome,
    reason_code: &'static str,
) {
    let entry = uptrakit_audit_log::AuditEntry::builder(
        uptrakit_audit_log::AuditActionType::AUTH_API_TOKEN_AUTHENTICATE,
    )
    .tenant_scope(state.default_tenant_id)
    .actor(uptrakit_audit_log::AuditActorType::ApiToken, None)
    .outcome(outcome)
    .details(serde_json::json!({ "reason_code": reason_code }))
    .request_id_opt(request_id)
    .build();

    if let Ok(entry) = entry {
        state.audit_emitter.emit_best_effort(entry);
    }
}

fn emit_jwt_auth_audit(
    state: &AppState,
    request_id: Option<String>,
    actor_type: uptrakit_audit_log::AuditActorType,
    outcome: uptrakit_audit_log::AuditOutcome,
    reason_code: &'static str,
) {
    let entry = uptrakit_audit_log::AuditEntry::builder(
        uptrakit_audit_log::AuditActionType::AUTH_JWT_AUTHENTICATE,
    )
    .tenant_scope(state.default_tenant_id)
    .actor(actor_type, None)
    .outcome(outcome)
    .details(serde_json::json!({ "reason_code": reason_code }))
    .request_id_opt(request_id)
    .build();

    if let Ok(entry) = entry {
        state.audit_emitter.emit_best_effort(entry);
    }
}

fn classify_api_token_verify_error(error: &rootcause::Report<AuthError>) -> AuthFailure {
    match error.current_context() {
        AuthError::ApiTokenNotFound | AuthError::ApiTokenRevoked => AuthFailure::InvalidApiToken,
        _ => AuthFailure::InternalError,
    }
}

/// Middleware that requires authentication via Bearer token in Authorization header.
///
/// If the token starts with `upk_`, it is treated as an API token (DB lookup).
/// Otherwise, it is decoded as a JWT access token (stateless validation).
///
/// Returns 401 Unauthorized if the token is missing, invalid, or expired.
/// If authenticated, injects the user_id, auth method, and roles into request extensions.
pub async fn require_auth(
    State(state): State<Arc<AppState>>,
    mut req: Request,
    next: Next,
) -> Response {
    let request_id = req
        .extensions()
        .get::<crate::middleware::request_id::RequestId>()
        .map(|value| value.0.clone());

    // Extract bearer token from Authorization header
    let token = match extract_bearer_token(&req) {
        Some(token) => token,
        None => {
            emit_jwt_auth_audit(
                &state,
                request_id.clone(),
                uptrakit_audit_log::AuditActorType::User,
                uptrakit_audit_log::AuditOutcome::Denied,
                "missing_authorization_header",
            );
            return error_response(StatusCode::UNAUTHORIZED, "Authentication required");
        }
    };

    let (auth_user, api_token_id) = if token.starts_with("upk_") {
        // API token path: DB lookup
        match authenticate_api_token(&state, &token).await {
            Ok((user, token_id)) => (user, Some(AuthenticatedApiTokenId(token_id))),
            Err(e) => {
                let reason_code = e.api_token_reason_code();
                if let Some(reason_code) = reason_code {
                    emit_api_token_auth_audit(
                        &state,
                        request_id.clone(),
                        uptrakit_audit_log::AuditOutcome::Denied,
                        reason_code,
                    );
                }
                return e.into_response();
            }
        }
    } else {
        // JWT path: stateless validation + denylist check
        match authenticate_jwt(&state, &token).await {
            Ok(user) => (user, None),
            Err(e) => {
                if let Some((actor_type, outcome, reason_code)) = e.jwt_audit_attributes() {
                    emit_jwt_auth_audit(
                        &state,
                        request_id.clone(),
                        actor_type,
                        outcome,
                        reason_code,
                    );
                }
                return e.into_response();
            }
        }
    };

    if let Some(api_token_id) = api_token_id {
        req.extensions_mut().insert(api_token_id);
    }

    // Inject user into request extensions
    req.extensions_mut().insert(auth_user);

    next.run(req).await
}

/// Lightweight error type for authentication failures, replacing `Result<_, Response>`
/// to avoid the `clippy::result_large_err` lint.
#[derive(Debug)]
pub(crate) enum AuthFailure {
    InvalidApiToken,
    UserNotFound,
    UserDeactivated,
    InvalidOrExpiredToken,
    InvalidTokenSubject,
    TokenRevoked,
    InvalidOidcSessionMissingProvider,
    InternalError,
}

impl AuthFailure {
    pub(crate) fn api_token_reason_code(&self) -> Option<&'static str> {
        match self {
            Self::InvalidApiToken => Some("invalid_or_revoked_api_token"),
            Self::UserNotFound => Some("user_not_found"),
            Self::UserDeactivated => Some("user_deactivated"),
            Self::InternalError => None,
            _ => Some("authentication_denied"),
        }
    }

    fn jwt_audit_attributes(
        &self,
    ) -> Option<(
        uptrakit_audit_log::AuditActorType,
        uptrakit_audit_log::AuditOutcome,
        &'static str,
    )> {
        match self {
            Self::InvalidOrExpiredToken => Some((
                uptrakit_audit_log::AuditActorType::User,
                uptrakit_audit_log::AuditOutcome::Denied,
                "invalid_or_expired_token",
            )),
            Self::InvalidTokenSubject => Some((
                uptrakit_audit_log::AuditActorType::User,
                uptrakit_audit_log::AuditOutcome::Denied,
                "invalid_token_subject",
            )),
            Self::TokenRevoked => Some((
                uptrakit_audit_log::AuditActorType::User,
                uptrakit_audit_log::AuditOutcome::Denied,
                "token_revoked",
            )),
            Self::InvalidOidcSessionMissingProvider => Some((
                uptrakit_audit_log::AuditActorType::Oidc,
                uptrakit_audit_log::AuditOutcome::Denied,
                "invalid_oidc_session_missing_provider",
            )),
            Self::UserNotFound => Some((
                uptrakit_audit_log::AuditActorType::User,
                uptrakit_audit_log::AuditOutcome::Denied,
                "user_not_found",
            )),
            Self::UserDeactivated => Some((
                uptrakit_audit_log::AuditActorType::User,
                uptrakit_audit_log::AuditOutcome::Denied,
                "user_deactivated",
            )),
            Self::InternalError => Some((
                uptrakit_audit_log::AuditActorType::User,
                uptrakit_audit_log::AuditOutcome::Failed,
                "jwt_authenticate_failed",
            )),
            Self::InvalidApiToken => None,
        }
    }
}

impl IntoResponse for AuthFailure {
    fn into_response(self) -> Response {
        match self {
            Self::InvalidApiToken => {
                error_response(StatusCode::UNAUTHORIZED, "Invalid or revoked API token")
            }
            Self::UserNotFound => error_response(StatusCode::UNAUTHORIZED, "User not found"),
            Self::UserDeactivated => error_response(StatusCode::FORBIDDEN, "User is deactivated"),
            Self::InvalidOrExpiredToken => {
                error_response(StatusCode::UNAUTHORIZED, "Invalid or expired token")
            }
            Self::InvalidTokenSubject => {
                error_response(StatusCode::UNAUTHORIZED, "Invalid token subject")
            }
            Self::TokenRevoked => {
                error_response(StatusCode::UNAUTHORIZED, "Token has been revoked")
            }
            Self::InvalidOidcSessionMissingProvider => error_response(
                StatusCode::UNAUTHORIZED,
                "Invalid OIDC session: missing provider",
            ),
            Self::InternalError => {
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            }
        }
    }
}

/// Authenticate using a `upk_`-prefixed API token (requires DB lookup).
pub(crate) async fn authenticate_api_token(
    state: &AppState,
    token: &str,
) -> std::result::Result<(AuthenticatedUser, uuid::Uuid), AuthFailure> {
    let service = ApiTokenService::new(state.db().clone());

    let (user_id, token_id) = service
        .verify_token(token)
        .await
        .map_err(|error| classify_api_token_verify_error(&error))?;

    // Check user is active
    let user = User::find_by_id(user_id)
        .one(state.db())
        .await
        .map_err(|_| AuthFailure::InternalError)?
        .ok_or(AuthFailure::UserNotFound)?;

    if !user.is_active {
        return Err(AuthFailure::UserDeactivated);
    }

    // Fetch permissions from DB
    let permissions = get_user_permissions(state.db(), state.default_tenant_id, user_id)
        .await
        .map_err(|e| {
            tracing::error!(err = %e, user_id = %user_id, "Failed to load user permissions");
            AuthFailure::InternalError
        })?;

    Ok((
        AuthenticatedUser {
            user_id,
            auth_method: AuthMethod::ApiToken,
            permissions,
            jti: None,
        },
        token_id,
    ))
}

/// Authenticate using a JWT access token (stateless validation + denylist check).
pub(crate) async fn authenticate_jwt(
    state: &AppState,
    token: &str,
) -> std::result::Result<AuthenticatedUser, AuthFailure> {
    let claims = state
        .auth
        .jwt
        .decode_access_token(token)
        .map_err(|_| AuthFailure::InvalidOrExpiredToken)?;

    let user_id =
        uuid::Uuid::parse_str(&claims.sub).map_err(|_| AuthFailure::InvalidTokenSubject)?;

    // Check token denylist (immediate revocation on logout / deactivation)
    if state
        .auth
        .token_denylist
        .is_denied(&claims.jti, &user_id, claims.iat)
        .await
    {
        return Err(AuthFailure::TokenRevoked);
    }

    let auth_method = if claims.auth_method == "oidc" {
        let provider_id = claims
            .oidc_provider_id
            .as_deref()
            .and_then(|id| uuid::Uuid::parse_str(id).ok())
            .ok_or_else(|| {
                tracing::warn!(
                    user_id = %claims.sub,
                    "OIDC token missing valid provider ID; rejecting"
                );
                AuthFailure::InvalidOidcSessionMissingProvider
            })?;
        AuthMethod::Oidc { provider_id }
    } else {
        AuthMethod::Password
    };

    Ok(AuthenticatedUser {
        user_id,
        auth_method,
        permissions: claims.permissions,
        jti: Some(claims.jti.clone()),
    })
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

fn extract_bearer_token(req: &Request) -> Option<String> {
    req.headers()
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ServiceCredentialSources;
    use crate::auth::jwt::JwtManager;
    use crate::auth::permissions::Permission;
    use crate::auth::registration::{RegistrationMode, RegistrationSettings};
    use crate::auth::token::generate_uuid;
    use crate::settings::Settings;
    use axum::Router;
    use axum::body::Body;
    use axum::http::Request;
    use axum::middleware;
    use axum::routing::get;
    use http_body_util::BodyExt;
    use sea_orm::{ConnectOptions, Database, DatabaseConnection, EntityTrait, QueryOrder};
    use tower::ServiceExt;
    use uptrakit_shared_db::entity::{audit_log, tenant};

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        let db = Database::connect(opt).await.expect("test db");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("migrations");

        db
    }

    async fn test_state(db: DatabaseConnection) -> Arc<AppState> {
        use crate::cert_signer::{AgentCertSigner, CertSignerError, SignedCertBundle};
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
                jwt: Arc::new(JwtManager::from_secret(b"test-secret-for-middleware-tests")),
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

    async fn protected_handler(
        axum::Extension(user): axum::Extension<AuthenticatedUser>,
    ) -> impl IntoResponse {
        format!("user_id: {}", user.user_id)
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

    #[tokio::test]
    async fn test_require_auth_with_valid_jwt() {
        let db = test_db().await;
        let state = test_state(db).await;

        let user_id = generate_uuid();
        let permissions = vec![Permission::ViewServices];

        // Create a JWT access token
        let jwt_token = state
            .auth
            .jwt
            .create_access_token(user_id, &permissions, "password", None)
            .unwrap();

        // Build app with auth middleware
        let app = Router::new()
            .route("/protected", get(protected_handler))
            .layer(middleware::from_fn_with_state(
                Arc::clone(&state),
                require_auth,
            ))
            .with_state(state);

        // Make request with valid JWT bearer token
        let req = Request::builder()
            .uri("/protected")
            .header("authorization", format!("Bearer {}", jwt_token))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains(&user_id.to_string()));
    }

    #[tokio::test]
    async fn test_require_auth_without_token_emits_auth_jwt_authenticate_denied_audit_event() {
        let db = test_db().await;
        let state = test_state(db.clone()).await;

        let app = Router::new()
            .route("/protected", get(protected_handler))
            .layer(middleware::from_fn_with_state(
                Arc::clone(&state),
                require_auth,
            ))
            .with_state(state);

        // Make request without authorization header
        let req = Request::builder()
            .uri("/protected")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 401);

        let row = latest_tenant_audit_row(&db).await;
        assert_eq!(
            uptrakit_audit_log::AuditActionType::AUTH_JWT_AUTHENTICATE,
            row.action_type
        );
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
        assert_eq!(
            details["reason_code"],
            serde_json::json!("missing_authorization_header")
        );
    }

    #[tokio::test]
    async fn test_require_auth_with_invalid_token() {
        let db = test_db().await;
        let state = test_state(db).await;

        let app = Router::new()
            .route("/protected", get(protected_handler))
            .layer(middleware::from_fn_with_state(
                Arc::clone(&state),
                require_auth,
            ))
            .with_state(state);

        // Make request with invalid bearer token
        let req = Request::builder()
            .uri("/protected")
            .header("authorization", "Bearer invalid-token")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 401);
    }

    #[tokio::test]
    async fn test_require_auth_with_invalid_jwt_emits_auth_jwt_authenticate_denied_audit_event() {
        let db = test_db().await;
        let state = test_state(db.clone()).await;

        let app = Router::new()
            .route("/protected", get(protected_handler))
            .layer(middleware::from_fn_with_state(
                Arc::clone(&state),
                require_auth,
            ))
            .with_state(state);

        let req = Request::builder()
            .uri("/protected")
            .header("authorization", "Bearer definitely-not-a-jwt")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 401);

        let row = latest_tenant_audit_row(&db).await;
        assert_eq!(
            uptrakit_audit_log::AuditActionType::AUTH_JWT_AUTHENTICATE,
            row.action_type
        );
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
        assert_eq!(
            details["reason_code"],
            serde_json::json!("invalid_or_expired_token")
        );
    }

    #[tokio::test]
    async fn test_require_auth_with_wrong_secret() {
        let db = test_db().await;
        let state = test_state(db).await;

        // Create token with a different secret
        let other_jwt = JwtManager::from_secret(b"different-secret");
        let user_id = generate_uuid();
        let token = other_jwt
            .create_access_token(user_id, &[], "password", None)
            .unwrap();

        let app = Router::new()
            .route("/protected", get(protected_handler))
            .layer(middleware::from_fn_with_state(
                Arc::clone(&state),
                require_auth,
            ))
            .with_state(state);

        let req = Request::builder()
            .uri("/protected")
            .header("authorization", format!("Bearer {}", token))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 401);
    }

    #[tokio::test]
    async fn test_require_auth_with_invalid_api_token() {
        let db = test_db().await;
        let state = test_state(db.clone()).await;

        let app = Router::new()
            .route("/protected", get(protected_handler))
            .layer(middleware::from_fn_with_state(
                Arc::clone(&state),
                require_auth,
            ))
            .with_state(state);

        // Make request with invalid API token (upk_ prefix but not in DB)
        let req = Request::builder()
            .uri("/protected")
            .header("authorization", "Bearer upk_invalid_token_not_in_db")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 401);

        let row = latest_tenant_audit_row(&db).await;
        assert_eq!(
            uptrakit_audit_log::AuditActionType::AUTH_API_TOKEN_AUTHENTICATE,
            row.action_type
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::ApiToken.as_str()
        );
        assert!(row.actor_id.is_none());
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("invalid_or_revoked_api_token")
        );
    }

    #[test]
    fn classify_api_token_verify_error_treats_database_failures_as_internal() {
        let error = rootcause::report!(AuthError::Internal("boom".to_string()));
        assert!(matches!(
            classify_api_token_verify_error(&error),
            AuthFailure::InternalError
        ));
    }

    #[tokio::test]
    async fn authenticate_jwt_sets_jti_on_authenticated_user() {
        let db = test_db().await;
        let state = test_state(db).await;

        let user_id = generate_uuid();
        let permissions = vec![];
        let jwt_token = state
            .auth
            .jwt
            .create_access_token(user_id, &permissions, "password", None)
            .unwrap();

        let auth_user = authenticate_jwt(&state, &jwt_token).await.unwrap();

        assert!(auth_user.jti.is_some(), "jti must be set for JWT auth");
        assert!(!auth_user.jti.unwrap().is_empty());
    }
}
