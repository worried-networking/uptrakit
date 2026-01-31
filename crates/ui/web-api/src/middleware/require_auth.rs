use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use sea_orm::EntityTrait;
use uptrakit_shared_db::entity::prelude::*;

use crate::AppState;
use crate::auth::api_token::ApiTokenService;
use crate::routes::auth::get_user_roles;

/// Extension type to carry the authenticated user ID, auth method, and roles through the request.
#[derive(Clone, Debug)]
pub struct AuthenticatedUser {
    pub user_id: uuid::Uuid,
    pub auth_method: AuthMethod,
    pub roles: Vec<String>,
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
            return (StatusCode::UNAUTHORIZED, "Authentication required\n").into_response();
        }
    };

    let auth_user = if token.starts_with("upk_") {
        // API token path: DB lookup
        match authenticate_api_token(&state, &token).await {
            Ok(user) => user,
            Err(resp) => return resp,
        }
    } else {
        // JWT path: stateless validation
        match authenticate_jwt(&state, &token) {
            Ok(user) => user,
            Err(resp) => return resp,
        }
    };

    // Inject user into request extensions
    req.extensions_mut().insert(auth_user);

    next.run(req).await
}

/// Authenticate using a `upk_`-prefixed API token (requires DB lookup).
#[allow(clippy::result_large_err)]
async fn authenticate_api_token(
    state: &AppState,
    token: &str,
) -> std::result::Result<AuthenticatedUser, Response> {
    let service = ApiTokenService::new(state.db.clone());

    let (user_id, _token_id) = service.verify_token(token).await.map_err(|_| {
        (StatusCode::UNAUTHORIZED, "Invalid or revoked API token\n").into_response()
    })?;

    // Check user is active
    let user = User::find_by_id(user_id)
        .one(&state.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())?
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "User not found\n").into_response())?;

    if !user.is_active {
        return Err((StatusCode::FORBIDDEN, "User is deactivated\n").into_response());
    }

    // Fetch roles from DB
    let roles = get_user_roles(&state.db, user_id).await.unwrap_or_default();

    Ok(AuthenticatedUser {
        user_id,
        auth_method: AuthMethod::ApiToken,
        roles,
    })
}

/// Authenticate using a JWT access token (stateless, no DB call).
#[allow(clippy::result_large_err)]
fn authenticate_jwt(
    state: &AppState,
    token: &str,
) -> std::result::Result<AuthenticatedUser, Response> {
    let claims = state
        .jwt
        .decode_access_token(token)
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid or expired token\n").into_response())?;

    let user_id = uuid::Uuid::parse_str(&claims.sub)
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid token subject\n").into_response())?;

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

    Ok(AuthenticatedUser {
        user_id,
        auth_method,
        roles: claims.roles,
    })
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
        let roles = vec!["admin".to_string()];

        // Create a JWT access token
        let jwt_token = state
            .jwt
            .create_access_token(user_id, &roles, "password", None)
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
