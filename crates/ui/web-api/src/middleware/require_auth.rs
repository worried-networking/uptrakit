use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use uptrakit_shared_db::entity::prelude::*;

use crate::AppState;
use crate::auth::permissions::Permission;

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

/// Middleware that requires authentication via JWT Bearer token in Authorization header.
/// Returns 401 Unauthorized if the token is missing, invalid, or expired.
/// If authenticated, injects the user_id, auth method, and roles into request extensions.
///
/// No DB call is made — user deactivation/role changes take effect at refresh (~15 min max delay).
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

    // Decode JWT access token (stateless validation)
    let claims = match state.jwt.decode_access_token(&token) {
        Ok(c) => c,
        Err(_) => {
            return (StatusCode::UNAUTHORIZED, "Invalid or expired token\n").into_response();
        }
    };

    // Parse user_id from claims.sub
    let user_id = match uuid::Uuid::parse_str(&claims.sub) {
        Ok(id) => id,
        Err(_) => {
            return (StatusCode::UNAUTHORIZED, "Invalid token subject\n").into_response();
        }
    };

    // Reconstruct auth_method from claims
    let auth_method = if claims.auth_method == "oidc" {
        let provider_id = claims
            .oidc_provider_id
            .as_deref()
            .and_then(|id| uuid::Uuid::parse_str(id).ok());
        match provider_id {
            Some(pid) => AuthMethod::Oidc { provider_id: pid },
            None => AuthMethod::Password,
        }
    } else {
        AuthMethod::Password
    };

    // Inject user_id, auth_method, and permissions into request extensions
    req.extensions_mut().insert(AuthenticatedUser {
        user_id,
        auth_method,
        permissions: claims.permissions,
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
        }
        Arc::new(AppState {
            ca_pem: "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n".into(),
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
            jwt: Arc::new(JwtManager::from_secret(b"test-secret-for-middleware-tests")),
            oidc_token_exchange_store: crate::auth::oidc_state::OidcTokenExchangeStore::new(),
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
        let permissions = vec![Permission::ViewAgents];

        // Create a JWT access token
        let jwt_token = state
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
}
