//! RFC 6749 §3.2 / RFC 8628 §3.4 token endpoint.

use std::sync::Arc;

use axum::Form;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use time::OffsetDateTime;
use uptrakit_web_api_auth::auth::device_flow::{PollOutcome, validate_client_id};
use uptrakit_web_api_types::oauth::{OAuthErrorCode, OAuthTokenRequest, OAuthTokenResponse};
use uptrakit_web_api_types::validation::Validate;

use crate::AppState;
use crate::error_response::oauth_error_response;

const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// RFC 6749 §3.2 / RFC 8628 §3.4 token endpoint.
#[utoipa::path(
    post,
    path = "/api/v1/oauth/token",
    request_body(
        content = OAuthTokenRequest,
        content_type = "application/x-www-form-urlencoded"
    ),
    responses(
        (status = 200, description = "Token granted", body = OAuthTokenResponse),
        (status = 400, description = "OAuth error per RFC 6749 §5.2 / RFC 8628 §3.5")
    ),
    tag = "OAuth"
)]
#[tracing::instrument(skip_all)]
pub async fn token(
    State(state): State<Arc<AppState>>,
    Form(req): Form<OAuthTokenRequest>,
) -> Response {
    if let Err(e) = req.validate() {
        return oauth_error_response(
            StatusCode::BAD_REQUEST,
            OAuthErrorCode::InvalidRequest,
            Some(e.to_string()),
            None,
        );
    }

    match req.grant_type.as_str() {
        DEVICE_CODE_GRANT => device_code_grant(state, req).await,
        _ => oauth_error_response(
            StatusCode::BAD_REQUEST,
            OAuthErrorCode::UnsupportedGrantType,
            None,
            None,
        ),
    }
}

async fn device_code_grant(state: Arc<AppState>, req: OAuthTokenRequest) -> Response {
    let device_code = match req.device_code.as_deref() {
        Some(s) if !s.trim().is_empty() => s.to_string(),
        _ => {
            return oauth_error_response(
                StatusCode::BAD_REQUEST,
                OAuthErrorCode::InvalidRequest,
                Some("device_code is required".into()),
                None,
            );
        }
    };

    if let Some(client_id) = req.client_id.as_deref() {
        if let Err(code) = validate_client_id(client_id) {
            return oauth_error_response(StatusCode::BAD_REQUEST, code, None, None);
        }
    }

    let outcome = match state
        .auth
        .device_flow_store
        .poll(&device_code, OffsetDateTime::now_utc())
        .await
    {
        Ok(o) => o,
        Err(e) => {
            tracing::error!("device flow poll failed: {e}");
            return oauth_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                OAuthErrorCode::ServerError,
                Some("internal error".into()),
                None,
            );
        }
    };

    emit_poll_audit(&state, &device_code, &outcome);

    match outcome {
        PollOutcome::Authorized { token, .. } => {
            let body = OAuthTokenResponse {
                access_token: token.expose_secret().to_string(),
                token_type: "Bearer".into(),
                expires_in: None,
                refresh_token: None,
                scope: None,
            };
            (StatusCode::OK, Json(body)).into_response()
        }
        PollOutcome::Pending => oauth_error_response(
            StatusCode::BAD_REQUEST,
            OAuthErrorCode::AuthorizationPending,
            None,
            None,
        ),
        PollOutcome::SlowDown { bumped_interval } => oauth_error_response(
            StatusCode::BAD_REQUEST,
            OAuthErrorCode::SlowDown,
            None,
            Some(bumped_interval),
        ),
        PollOutcome::Denied => oauth_error_response(
            StatusCode::BAD_REQUEST,
            OAuthErrorCode::AccessDenied,
            None,
            None,
        ),
        PollOutcome::Expired | PollOutcome::Unknown => oauth_error_response(
            StatusCode::BAD_REQUEST,
            OAuthErrorCode::ExpiredToken,
            None,
            None,
        ),
        PollOutcome::MalformedDeviceCode => oauth_error_response(
            StatusCode::BAD_REQUEST,
            OAuthErrorCode::InvalidGrant,
            None,
            None,
        ),
        _ => {
            tracing::warn!(
                "unhandled PollOutcome variant returned by device_flow_store.poll(); \
                 treating as server_error"
            );
            oauth_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                OAuthErrorCode::ServerError,
                None,
                None,
            )
        }
    }
}

fn emit_poll_audit(state: &AppState, device_code: &str, outcome: &PollOutcome) {
    use uptrakit_audit_log::AuditOutcome as Outcome;

    let device_flow_id = crate::auth::token::hash_token(device_code);

    let (audit_outcome, details) = match outcome {
        PollOutcome::Authorized { .. } => (Outcome::Success, serde_json::json!({})),
        PollOutcome::SlowDown { bumped_interval } => (
            Outcome::Failed,
            serde_json::json!({
                "reason_code": "slow_down",
                "bumped_interval": bumped_interval,
            }),
        ),
        PollOutcome::Denied => (
            Outcome::Failed,
            serde_json::json!({ "reason_code": "access_denied" }),
        ),
        PollOutcome::Expired | PollOutcome::Unknown => (
            Outcome::Failed,
            serde_json::json!({ "reason_code": "expired_token" }),
        ),
        PollOutcome::MalformedDeviceCode => (
            Outcome::Failed,
            serde_json::json!({ "reason_code": "invalid_grant" }),
        ),
        _ => (Outcome::Failed, serde_json::json!({})),
    };

    let builder = uptrakit_audit_log::AuditEntry::builder(
        uptrakit_audit_log::AuditActionType::AUTH_DEVICE_POLL,
    )
    .tenant_scope(state.default_tenant_id)
    .actor_system()
    .target("device_flow", device_flow_id, None)
    .outcome(audit_outcome)
    .details(details);

    if let Ok(entry) = builder.build() {
        state.audit_emitter.emit_best_effort(entry);
    }
}
