//! Test-utilities endpoints — only compiled with the `test-utils` Cargo feature.
//!
//! All handlers check `UPTRAKIT_TEST_UTILS_ENABLED=true` at runtime before acting.
//! If the env var is absent or not `"true"`, every handler returns 404, making
//! the endpoints invisible to clients not in a test context.
#![cfg(feature = "test-utils")]

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use uptrakit_wire::{ControllerMessage, RequestCertRenewalPayload};
use uuid::Uuid;

use crate::AppState;
use crate::oauth::services::authorization_code::{
    MintAuthorizationCode, OAuthAuthorizationCodeService,
};
use crate::oauth::services::authorization_request::OAuthAuthorizationRequestService;
use crate::oauth::services::consent::OAuthConsentService;

fn test_utils_allowed() -> bool {
    std::env::var("UPTRAKIT_TEST_UTILS_ENABLED").as_deref() == Ok("true")
}

/// Send `RequestCertRenewal` to a specific connected service.
///
/// Returns 200 if the message was sent, 404 if the service is not connected
/// or if `UPTRAKIT_TEST_UTILS_ENABLED` is not `"true"`.
pub(crate) async fn request_service_renewal(
    State(state): State<Arc<AppState>>,
    Path(service_id): Path<Uuid>,
) -> Response {
    if !test_utils_allowed() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let msg = ControllerMessage::RequestCertRenewal(RequestCertRenewalPayload {
        reason: "test-utils: forced renewal".to_string(),
    });
    if state.service_connections.send(&service_id, msg).await {
        StatusCode::OK.into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

/// Close the WebSocket for a specific service, triggering its reconnect loop.
///
/// Returns 200 unconditionally (disconnect is a no-op if already disconnected).
/// Returns 404 if `UPTRAKIT_TEST_UTILS_ENABLED` is not `"true"`.
pub(crate) async fn disconnect_service(
    State(state): State<Arc<AppState>>,
    Path(service_id): Path<Uuid>,
) -> Response {
    if !test_utils_allowed() {
        return StatusCode::NOT_FOUND.into_response();
    }
    state
        .service_connections
        .force_disconnect(&service_id)
        .await;
    StatusCode::OK.into_response()
}

/// Trigger an unconditional reexec without going through config-triage.
///
/// Returns 202 immediately. The reexec fires asynchronously from a background
/// task; the HTTP connection will be closed when exec() replaces the process
/// image. The caller must poll GET /healthz (checking X-Reexec-Generation) to
/// know when the new generation is ready.
///
/// Returns 404 if UPTRAKIT_TEST_UTILS_ENABLED is not "true".
/// Returns 503 if the notify handle is not installed (env var not set at startup).
pub(crate) async fn force_reexec(State(state): State<Arc<AppState>>) -> Response {
    if !test_utils_allowed() {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Some(notify) = &state.test_reexec_notify {
        notify.notify_one();
        StatusCode::ACCEPTED.into_response()
    } else {
        StatusCode::SERVICE_UNAVAILABLE.into_response()
    }
}

/// Approve an OAuth consent request without going through the browser UI.
///
/// Looks up the pending authorization request by `request_id`, grants consent on behalf
/// of whichever user initiated the authorization (their `user_id` is on the request row),
/// issues an authorization code, and returns JSON:
/// `{ "redirect_uri": "https://...?code=<code>&state=<state>" }`.
///
/// No ownership check — this endpoint is already double-gated by the compile-time
/// `#[cfg(feature = "test-utils")]` file attribute and the `test_utils_allowed()` runtime check.
///
/// Returns 404 when `UPTRAKIT_TEST_UTILS_ENABLED != "true"`.
/// Returns 404 when OAuth is disabled or the authorization request does not exist / already consumed.
pub(crate) async fn oauth_auto_approve_consent(
    State(state): State<Arc<AppState>>,
    Path(request_id): Path<Uuid>,
) -> Response {
    if !test_utils_allowed() {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !state.oauth.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }

    let ar_svc =
        OAuthAuthorizationRequestService::new(state.db().clone(), Arc::clone(&state.oauth.clock));
    let row = match ar_svc.consume(request_id).await {
        Ok(Some(r)) => r,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "auto-approve: failed to consume authorization request");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let consent_svc = OAuthConsentService::new(state.db().clone(), Arc::clone(&state.oauth.clock));
    if let Err(e) = consent_svc
        .grant(row.user_id, &row.client_id, &row.scope, None)
        .await
    {
        tracing::error!(error = %e, "auto-approve: failed to grant consent");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    let code_svc =
        OAuthAuthorizationCodeService::new(state.db().clone(), Arc::clone(&state.oauth.clock));
    let code = match code_svc
        .mint(MintAuthorizationCode {
            request_id: row.request_id,
            client_id: row.client_id.clone(),
            user_id: row.user_id,
            redirect_uri: row.redirect_uri.clone(),
            scope: row.scope.clone(),
            code_challenge: row.code_challenge.clone(),
            code_challenge_method: row.code_challenge_method.clone(),
            resource: row.resource.clone(),
        })
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "auto-approve: failed to mint authorization code");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let sep = if row.redirect_uri.contains('?') {
        '&'
    } else {
        '?'
    };
    let redirect_uri = format!(
        "{}{}code={}&state={}",
        row.redirect_uri,
        sep,
        utf8_percent_encode(code.as_str(), NON_ALPHANUMERIC),
        utf8_percent_encode(&row.state, NON_ALPHANUMERIC),
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({ "redirect_uri": redirect_uri })),
    )
        .into_response()
}
