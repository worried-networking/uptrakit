use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use uptrakit_audit_log::AuditActorType;

use crate::AppState;
use crate::extract::ClientIp;
use crate::middleware::request_id::RequestId;

/// Request-scoped context available to semantic audit producers.
#[derive(Clone, Debug)]
pub struct AuditRequestContext {
    pub request_id: Option<String>,
    pub client_ip: Option<String>,
    pub actor_type: AuditActorType,
    pub actor_id: Option<uuid::Uuid>,
}

/// Build [`AuditRequestContext`] from request parts and actor metadata.
pub fn audit_context_from_parts(
    parts: &http::request::Parts,
    actor_type: AuditActorType,
    actor_id: Option<uuid::Uuid>,
) -> AuditRequestContext {
    AuditRequestContext {
        request_id: parts.extensions.get::<RequestId>().map(|v| v.0.clone()),
        client_ip: parts.extensions.get::<ClientIp>().map(|v| v.0.to_string()),
        actor_type,
        actor_id,
    }
}

/// Legacy middleware kept as a no-op after semantic audit cutover.
///
pub async fn audit_log(State(_state): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    next.run(req).await
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    #![expect(
        clippy::expect_used,
        reason = "test code: panics on failure are acceptable"
    )]

    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::Request;
    use axum::middleware as axum_mw;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use axum::{Router, extract::Request as AxumRequest};
    use sea_orm::{EntityTrait, PaginatorTrait};
    use tower::ServiceExt;
    use uptrakit_shared_db::entity::audit_log;

    use super::audit_log;
    use crate::AppState;
    use crate::auth::AuthMethod;
    use crate::middleware::require_auth::AuthenticatedUser;

    async fn inject_authenticated_user(
        mut req: AxumRequest,
        next: axum::middleware::Next,
    ) -> axum::response::Response {
        req.extensions_mut().insert(AuthenticatedUser {
            user_id: uuid::Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: Vec::new(),
            jti: None,
        });
        next.run(req).await
    }

    async fn build_authenticated_test_app() -> (Router, sea_orm::DatabaseConnection) {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

        let dispatcher = uptrakit_audit_log::AuditLogDispatcher::new(Arc::new(
            uptrakit_audit_log::DatabaseBackend::new(db.clone()),
        ));
        let emitter = uptrakit_audit_log::AuditEmitter::new(dispatcher.clone());
        let state = Arc::new(AppState {
            audit_log_dispatcher: dispatcher,
            audit_emitter: emitter,
            ..(*state).clone()
        });

        let app = Router::new()
            .route(
                "/api/v1/plugin-configs",
                get(|| async { "ok".into_response() }),
            )
            .layer(axum_mw::from_fn_with_state(Arc::clone(&state), audit_log))
            .layer(axum_mw::from_fn(inject_authenticated_user))
            .with_state(state);

        (app, db)
    }

    async fn count_audit_rows(db: &sea_orm::DatabaseConnection) -> u64 {
        audit_log::Entity::find()
            .count(db)
            .await
            .expect("count audit rows")
    }

    #[tokio::test]
    async fn request_middleware_does_not_persist_audit_rows_by_itself() {
        let (app, db) = build_authenticated_test_app().await;

        assert_eq!(count_audit_rows(&db).await, 0);

        let _ = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/plugin-configs")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(count_audit_rows(&db).await, 0);
    }
}
