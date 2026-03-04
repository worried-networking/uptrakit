use std::sync::Arc;
use std::time::Instant;

use axum::extract::{MatchedPath, Request, State};
use axum::middleware::Next;
use axum::response::Response;
use uptrakit_audit_log::{AuditActorType, AuditEntry};

use crate::AppState;
use crate::extract::ClientIp;
use crate::middleware::require_auth::AuthenticatedUser;

/// Audit log middleware that captures authenticated HTTP requests.
///
/// Must be placed **inside** `require_auth` (runs after auth sets
/// `AuthenticatedUser` in extensions) and **after** `resolve_ip`
/// (which sets `ClientIp`).
///
/// Reads `MatchedPath`, `AuthenticatedUser`, `ClientIp`, and `User-Agent`
/// from the request, calls `next.run()`, then dispatches an `AuditEntry`
/// through the fire-and-forget dispatcher.
pub async fn audit_log(State(state): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let start = Instant::now();

    // Capture request metadata before passing to the handler.
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let route_pattern = req
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_string());
    let client_ip = req.extensions().get::<ClientIp>().map(|c| c.0.to_string());
    let user_agent = req
        .headers()
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // Extract auth info (set by require_auth middleware).
    let auth_user = req.extensions().get::<AuthenticatedUser>().cloned();

    let response = next.run(req).await;

    // Only dispatch if we have an authenticated user.
    if let Some(user) = auth_user {
        let (actor_type, auth_method) = map_auth_method(&user.auth_method);

        // Check filter: should we log this request?
        // Per-tenant override is loaded from settings when available.
        // For now, we always use the global filter since per-tenant
        // override requires a DB read which is deferred to a future iteration.
        if state.audit_log_filter.should_log(&method, None) {
            // Routes under `/api/v1/global-settings/` and `/api/v1/system-services`
            // represent infrastructure-scoped operations and are written to
            // `system_audit_logs` (tenant_id = None).  All other authenticated
            // routes are written to the per-tenant `audit_logs` table.
            let is_system_route = route_pattern.as_deref().is_some_and(|p| {
                p.starts_with("/api/v1/global-settings/")
                    || p == "/api/v1/global-settings"
                    || p.starts_with("/api/v1/system-services/")
                    || p == "/api/v1/system-services"
            });
            let tenant_id = if is_system_route {
                None
            } else {
                Some(state.default_tenant_id)
            };

            let duration_ms = start.elapsed().as_millis() as u64;
            let entry = AuditEntry {
                id: uuid::Uuid::now_v7(),
                tenant_id,
                actor_id: user.user_id,
                actor_type,
                auth_method,
                http_method: method,
                http_path: path,
                route_pattern,
                http_status: response.status().as_u16(),
                client_ip,
                user_agent,
                duration_ms,
                occurred_at: time::OffsetDateTime::now_utc(),
            };

            state.audit_log_dispatcher.dispatch(entry);
        }
    }

    response
}

/// Map the web-api `AuthMethod` to the audit log's `AuditActorType` and auth method string.
fn map_auth_method(auth_method: &crate::auth::AuthMethod) -> (AuditActorType, String) {
    match auth_method {
        crate::auth::AuthMethod::Password => (AuditActorType::User, "password".to_string()),
        crate::auth::AuthMethod::Oidc { .. } => (AuditActorType::Oidc, "oidc".to_string()),
        crate::auth::AuthMethod::ApiToken => (AuditActorType::ApiToken, "api_token".to_string()),
    }
}
