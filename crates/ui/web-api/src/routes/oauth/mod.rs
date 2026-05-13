//! OAuth 2.0 endpoints (RFC 8628 device grant + RFC 8414 metadata).
//!
//! See `docs/superpowers/specs/2026-05-12-rfc8628-device-auth-design.md`.

pub mod authorize;
pub mod clients_api;
pub mod consent;
pub mod consents_api;
pub mod device_authorization;
mod helpers;
pub mod metadata;
pub mod register;
pub mod token;

use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};

/// Optional-auth middleware: if an `Authorization: Bearer <token>` header is
/// present and valid, inject `AuthenticatedUser` into request extensions.
/// Used by the authorization and consent endpoints which accept both
/// authenticated and unauthenticated callers.
pub(crate) async fn optional_oauth_auth(
    axum::extract::State(state): axum::extract::State<Arc<crate::AppState>>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use crate::middleware::require_auth::authenticate_jwt;

    let token = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_owned);

    if let Some(token) = token
        && let Ok((user, _setup_required)) = authenticate_jwt(&state, &token).await
    {
        req.extensions_mut().insert(user);
    }

    next.run(req).await
}

/// Assemble all public and optional-auth OAuth routes.
///
/// The authenticated operator/user routes (`/api/oauth/clients` and
/// `/api/oauth/consents`) are kept in `router.rs` inside `auth_routes` so
/// that `require_auth` middleware is applied consistently with the rest of
/// the authenticated API surface.
pub fn build_oauth_router(state: Arc<crate::AppState>) -> Router {
    // Public OAuth endpoints — no authentication required.
    let public = Router::new()
        .route(
            "/.well-known/oauth-authorization-server",
            get(metadata::get_as_metadata),
        )
        .route("/oauth/token", post(token::mcp_token))
        .route("/oauth/register", post(register::register))
        .route(
            "/oauth/register/{client_id}",
            get(register::get_client_registration)
                .put(register::update_client_registration)
                .delete(register::delete_client_registration),
        );

    // Endpoints that accept optional auth — inject `AuthenticatedUser` if a
    // valid Bearer token is present, but do not reject unauthenticated requests.
    let optional_auth = Router::new()
        .route("/oauth/authorize", get(authorize::authorize))
        .route("/oauth/consent/{request_id}", get(consent::consent_details))
        .route(
            "/oauth/consent/{request_id}/approve",
            post(consent::approve_consent),
        )
        .route(
            "/oauth/consent/{request_id}/deny",
            post(consent::deny_consent),
        )
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(&state),
            optional_oauth_auth,
        ));

    Router::new()
        .merge(public)
        .merge(optional_auth)
        .with_state(state)
}
