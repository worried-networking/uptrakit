use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use sea_orm::EntityTrait;
use uptrakit_shared_db::entity::prelude::*;

use crate::AppState;
use crate::auth::session::SessionService;

/// Extension type to carry the authenticated user ID through the request
#[derive(Clone, Debug)]
pub struct AuthenticatedUser {
    pub user_id: uuid::Uuid,
}

/// Middleware that requires authentication via Bearer token in Authorization header.
/// Returns 401 Unauthorized if the token is missing, invalid, or expired.
/// If authenticated, injects the user_id into request extensions.
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
    let user_id = match session_service.verify_session(&token).await {
        Ok(id) => id,
        Err(_) => {
            return (StatusCode::UNAUTHORIZED, "Invalid or expired session\n").into_response();
        }
    };

    // Check if user is active
    let user = match User::find_by_id(user_id).one(&state.db).await {
        Ok(Some(user)) => user,
        _ => {
            return (StatusCode::UNAUTHORIZED, "User not found\n").into_response();
        }
    };

    if !user.is_active {
        return (StatusCode::FORBIDDEN, "User is deactivated\n").into_response();
    }

    // Inject user_id into request extensions
    req.extensions_mut().insert(AuthenticatedUser { user_id });

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
            fn sign_agent_cert(&self, _: &uuid::Uuid, _: u16) -> Result<AgentCertBundle, String> {
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
            .create_session(user_id, "password".to_string(), None, None)
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
            .create_session(user_id, "password".to_string(), None, None)
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
