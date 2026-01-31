use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use sea_orm::EntityTrait;
use uptrakit_shared_db::entity::prelude::*;

use crate::AppState;
use crate::auth::session::SessionService;

/// Extension type to carry the authenticated user ID and auth method through the request.
#[derive(Clone, Debug)]
pub struct AuthenticatedUser {
    pub user_id: uuid::Uuid,
    pub auth_method: AuthMethod,
}

/// Middleware that requires authentication via Bearer token in Authorization header.
/// Returns 401 Unauthorized if the token is missing, invalid, or expired.
/// If authenticated, injects the user_id and auth method into request extensions.
pub async fn require_auth(
    State(state): State<Arc<AppState>>,
    mut req: Request,
    next: Next,
) -> Response {
    // Extract bearer token from Authorization header
    let token = match extract_bearer_token(&req) {
        Some(token) => token,
        None => {
            return (StatusCode::UNAUTHORIZED, "Authentication required\n").into_response();
        }
    };

    // Verify session
    let session_service = SessionService::new(state.db.clone());
    let verified = match session_service.verify_session(&token).await {
        Ok(v) => v,
        Err(_) => {
            return (StatusCode::UNAUTHORIZED, "Invalid or expired session\n").into_response();
        }
    };

    // Check if user is active
    let user = match User::find_by_id(verified.user_id).one(&state.db).await {
        Ok(Some(user)) => user,
        _ => {
            return (StatusCode::UNAUTHORIZED, "User not found\n").into_response();
        }
    };

    if !user.is_active {
        return (StatusCode::FORBIDDEN, "User is deactivated\n").into_response();
    }

    // Inject user_id and auth_method into request extensions
    req.extensions_mut().insert(AuthenticatedUser {
        user_id: verified.user_id,
        auth_method: verified.auth_method,
    });

    next.run(req).await
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
    use crate::auth::registration::{RegistrationMode, RegistrationSettings};
    use crate::auth::session::SessionService;
    use crate::auth::token::generate_uuid;
    use crate::settings::Settings;
    use axum::Router;
    use axum::body::Body;
    use axum::http::Request;
    use axum::middleware;
    use axum::routing::get;
    use http_body_util::BodyExt;
    use sea_orm::{
        ActiveModelTrait, ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Set,
    };
    use time::OffsetDateTime;
    use tower::ServiceExt;
    use uptrakit_shared_db::entity::user;

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        Database::connect(opt).await.expect("test db")
    }

    async fn setup_test_db() -> DatabaseConnection {
        let db = test_db().await;

        // Create tables
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
                token_hash TEXT UNIQUE NOT NULL,
                auth_method TEXT NOT NULL,
                oidc_provider_id TEXT,
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                last_activity_at INTEGER NOT NULL,
                user_agent TEXT,
                ip_address TEXT,
                FOREIGN KEY (user_id) REFERENCES users(id)
            )",
        )
        .await
        .unwrap();

        db
    }

    async fn test_state(db: DatabaseConnection) -> Arc<AppState> {
        use crate::cert_signer::{AgentCertBundle, AgentCertSigner};
        struct NoopCertSigner;
        impl AgentCertSigner for NoopCertSigner {
            fn sign_agent_cert(
                &self,
                _: &uuid::Uuid,
                _: time::Duration,
            ) -> Result<AgentCertBundle, String> {
                unimplemented!()
            }
            fn active_ca_fingerprint(&self) -> String {
                "0".repeat(64)
            }
        }

        let ca_pem = "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n";
        let snapshot_data = crate::ca_snapshot::CaSnapshotData {
            active_cert_pem: ca_pem.to_string(),
            active_key_pem: String::new(),
            active_fingerprint: "0".repeat(64),
            previous_cert_pem: None,
            previous_key_pem: None,
            previous_fingerprint: None,
            bundle_pem: ca_pem.to_string(),
            bundle_hash: "0".repeat(64),
            managed: true,
            active_not_after: time::OffsetDateTime::now_utc() + time::Duration::days(365),
        };
        let (_ca_tx, ca_rx) = tokio::sync::watch::channel(snapshot_data);

        let rustls_cfg = {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
            let key_pair =
                rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
            let cert = rcgen::CertificateParams::new(vec!["localhost".into()])
                .unwrap()
                .self_signed(&key_pair)
                .unwrap();
            let server_config = rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(
                    vec![rustls::pki_types::CertificateDer::from(cert.der().to_vec())],
                    rustls::pki_types::PrivateKeyDer::try_from(key_pair.serialize_der())
                        .unwrap(),
                )
                .unwrap();
            axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(server_config))
        };

        Arc::new(AppState {
            ca_snapshot: ca_rx,
            trusted_proxies: vec![].into(),
            real_ip_header: "X-Forwarded-For".into(),
            db,
            settings: Settings::new(
                RegistrationSettings {
                    mode: RegistrationMode::Open,
                    token_hash: None,
                },
                7,
            ),
            cert_signer: Arc::new(NoopCertSigner),
            agent_connections: crate::agent_connections::AgentConnectionRegistry::new(),
            revocation_notify: Arc::new(tokio::sync::Notify::const_new()),
            oidc_flow_store: crate::auth::oidc_state::OidcFlowStore::new(),
            account_link_store: crate::auth::oidc_state::AccountLinkStore::new(),
            pki_path: std::path::PathBuf::from("/tmp/test-pki"),
            rustls_config: rustls_cfg,
            extra_sans: Arc::new([]),
        })
    }

    async fn protected_handler(
        axum::Extension(user): axum::Extension<AuthenticatedUser>,
    ) -> impl IntoResponse {
        format!("user_id: {}", user.user_id)
    }

    #[tokio::test]
    async fn test_require_auth_with_valid_session() {
        let db = setup_test_db().await;
        let state = test_state(db.clone()).await;

        // Create test user
        let user_id = generate_uuid();
        let now = OffsetDateTime::now_utc();
        let test_user = user::ActiveModel {
            id: Set(user_id),
            email: Set("test@example.com".to_string()),
            first_name: Set("Test".to_string()),
            last_name: Set("User".to_string()),
            password_hash: Set(None),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        };
        test_user.insert(&db).await.unwrap();

        // Create session
        let session_service = SessionService::new(db.clone());
        let token = session_service
            .create_session(user_id, AuthMethod::Password, None, None)
            .await
            .unwrap();

        // Build app with auth middleware
        let app = Router::new()
            .route("/protected", get(protected_handler))
            .layer(middleware::from_fn_with_state(
                Arc::clone(&state),
                require_auth,
            ))
            .with_state(state);

        // Make request with valid bearer token
        let req = Request::builder()
            .uri("/protected")
            .header("authorization", format!("Bearer {}", token))
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
        let db = setup_test_db().await;
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
        let db = setup_test_db().await;
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
    async fn test_require_auth_with_deactivated_user() {
        let db = setup_test_db().await;
        let state = test_state(db.clone()).await;

        // Create deactivated test user
        let user_id = generate_uuid();
        let now = OffsetDateTime::now_utc();
        let test_user = user::ActiveModel {
            id: Set(user_id),
            email: Set("test@example.com".to_string()),
            first_name: Set("Test".to_string()),
            last_name: Set("User".to_string()),
            password_hash: Set(None),
            is_active: Set(false),
            deactivated_at: Set(Some(now)),
            created_at: Set(now),
            updated_at: Set(now),
        };
        test_user.insert(&db).await.unwrap();

        // Create session
        let session_service = SessionService::new(db.clone());
        let token = session_service
            .create_session(user_id, AuthMethod::Password, None, None)
            .await
            .unwrap();

        // Build app with auth middleware
        let app = Router::new()
            .route("/protected", get(protected_handler))
            .layer(middleware::from_fn_with_state(
                Arc::clone(&state),
                require_auth,
            ))
            .with_state(state);

        // Make request with valid bearer token but deactivated user
        let req = Request::builder()
            .uri("/protected")
            .header("authorization", format!("Bearer {}", token))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 403);
    }
}
