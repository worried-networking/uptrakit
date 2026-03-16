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
use crate::auth::AuthMethod;
use crate::auth::api_token::ApiTokenService;
use crate::auth::permissions::Permission;
use crate::error_response::error_response;

/// Extension type to carry the authenticated user ID, auth method, and permissions through the request.
#[derive(Clone, Debug)]
pub struct AuthenticatedUser {
    pub user_id: uuid::Uuid,
    pub auth_method: AuthMethod,
    pub permissions: Vec<Permission>,
}

impl AuthenticatedUser {
    pub fn has_permission(&self, perm: Permission) -> bool {
        self.permissions.contains(&perm)
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
    // Extract bearer token from Authorization header
    let token = match extract_bearer_token(&req) {
        Some(token) => token,
        None => {
            return error_response(StatusCode::UNAUTHORIZED, "Authentication required");
        }
    };

    let auth_user = if token.starts_with("upk_") {
        // API token path: DB lookup
        match authenticate_api_token(&state, &token).await {
            Ok(user) => user,
            Err(e) => return e.into_response(),
        }
    } else {
        // JWT path: stateless validation + denylist check
        match authenticate_jwt(&state, &token).await {
            Ok(user) => user,
            Err(e) => return e.into_response(),
        }
    };

    // Inject user into request extensions
    req.extensions_mut().insert(auth_user);

    next.run(req).await
}

/// Lightweight error type for authentication failures, replacing `Result<_, Response>`
/// to avoid the `clippy::result_large_err` lint.
pub(crate) enum AuthFailure {
    Unauthorized(&'static str),
    Forbidden(&'static str),
    InternalError,
}

impl IntoResponse for AuthFailure {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthorized(msg) => error_response(StatusCode::UNAUTHORIZED, msg),
            Self::Forbidden(msg) => error_response(StatusCode::FORBIDDEN, msg),
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
) -> std::result::Result<AuthenticatedUser, AuthFailure> {
    let service = ApiTokenService::new(state.db().clone());

    let (user_id, _token_id) = service
        .verify_token(token)
        .await
        .map_err(|_| AuthFailure::Unauthorized("Invalid or revoked API token"))?;

    // Check user is active
    let user = User::find_by_id(user_id)
        .one(state.db())
        .await
        .map_err(|_| AuthFailure::InternalError)?
        .ok_or(AuthFailure::Unauthorized("User not found"))?;

    if !user.is_active {
        return Err(AuthFailure::Forbidden("User is deactivated"));
    }

    // Fetch permissions from DB
    let permissions = get_user_permissions(state.db(), state.default_tenant_id, user_id)
        .await
        .map_err(|e| {
            tracing::error!(err = %e, user_id = %user_id, "Failed to load user permissions");
            AuthFailure::InternalError
        })?;

    Ok(AuthenticatedUser {
        user_id,
        auth_method: AuthMethod::ApiToken,
        permissions,
    })
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
        .map_err(|_| AuthFailure::Unauthorized("Invalid or expired token"))?;

    let user_id = uuid::Uuid::parse_str(&claims.sub)
        .map_err(|_| AuthFailure::Unauthorized("Invalid token subject"))?;

    // Check token denylist (immediate revocation on logout / deactivation)
    if state
        .auth
        .token_denylist
        .is_denied(&claims.jti, &user_id, claims.iat)
        .await
    {
        return Err(AuthFailure::Unauthorized("Token has been revoked"));
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
                AuthFailure::Unauthorized("Invalid OIDC session: missing provider")
            })?;
        AuthMethod::Oidc { provider_id }
    } else {
        AuthMethod::Password
    };

    Ok(AuthenticatedUser {
        user_id,
        auth_method,
        permissions: claims.permissions,
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
    use sea_orm::{ConnectOptions, Database, DatabaseConnection};
    use tower::ServiceExt;

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        Database::connect(opt).await.expect("test db")
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

        let plugin_ops: Arc<dyn uptrakit_plugin_infrastructure_registry::PluginOps> =
            Arc::new(uptrakit_plugin_infrastructure_registry::PluginRegistry::new());

        let notification_dispatcher = crate::notifications::dispatcher::NotificationDispatcher::new(
            db.clone(),
            Arc::clone(&plugin_ops),
            "https://localhost".to_string(),
        );

        Arc::new(AppState {
            db: db.clone(),
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
            broadcast: crate::app_state::BroadcastState {
                event_broadcaster: crate::event_broadcaster::EventBroadcaster::new(),
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
            default_tenant_id: uuid::Uuid::nil(),
            settings,
            cert_signer: Arc::new(NoopCertSigner),
            service_connections: crate::service_connections::ServiceConnectionRegistry::new(),
            controller_id: uuid::Uuid::nil(),
            notification_service,
            notification_dispatcher,
            plugin_ops,
            credential_sources: ServiceCredentialSources::default(),
            shutdown_token: Default::default(),
            embedded_service_notifier: None,
            audit_log_filter: uptrakit_audit_log::AuditFilter::default(),
            audit_log_dispatcher: uptrakit_audit_log::AuditLogDispatcher::new(Arc::new(
                uptrakit_audit_log::NoopBackend,
            )),
            extension_registry: Arc::new(crate::extension_registry::ExtensionRegistry::new(vec![])),
            extension_proxy: Arc::new(crate::extension_proxy::ExtensionProxy::new()),
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
    async fn test_require_auth_without_token() {
        let db = test_db().await;
        let state = test_state(db).await;

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
        let state = test_state(db).await;

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
    }
}
