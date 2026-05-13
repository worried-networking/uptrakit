//! RFC 8628 §3.1 device-authorization endpoint.

use std::sync::Arc;

use axum::Extension;
use axum::Form;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use uptrakit_web_api_auth::auth::device_flow::validate_client_id;
use uptrakit_web_api_types::oauth::{
    DeviceAuthorizationRequest, DeviceAuthorizationResponse, OAuthErrorCode,
};
use uptrakit_web_api_types::validation::Validate;

use crate::AppState;
use crate::error_response::oauth_error_response;
use crate::extract::ExternalBaseUrl;
use crate::routes::oauth::helpers::resolve_external_base_url;

/// RFC 8628 §3.1 device-authorization request.
#[utoipa::path(
    post,
    path = "/api/v1/oauth/device_authorization",
    request_body(
        content = DeviceAuthorizationRequest,
        content_type = "application/x-www-form-urlencoded"
    ),
    responses(
        (status = 200, description = "Device authorization started", body = DeviceAuthorizationResponse),
        (status = 400, description = "Invalid request or invalid_client")
    ),
    tag = "OAuth"
)]
#[tracing::instrument(skip_all)]
pub async fn device_authorization(
    State(state): State<Arc<AppState>>,
    external_base_url: Option<Extension<ExternalBaseUrl>>,
    headers: HeaderMap,
    Form(req): Form<DeviceAuthorizationRequest>,
) -> Response {
    if let Err(e) = req.validate() {
        return oauth_error_response(
            StatusCode::BAD_REQUEST,
            OAuthErrorCode::InvalidRequest,
            Some(e.to_string()),
            None,
        );
    }

    if let Err(code) = validate_client_id(&req.client_id) {
        return oauth_error_response(StatusCode::BAD_REQUEST, code, None, None);
    }

    let (device_code, user_code) = match state
        .auth
        .device_flow_store
        .create(req.client_name.clone(), req.scope.clone())
        .await
    {
        Ok(pair) => pair,
        Err(e) => {
            tracing::error!("device flow create failed: {e}");
            emit_device_start_audit(
                &state,
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({ "reason_code": "device_flow_create_failed" }),
            );
            return oauth_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                OAuthErrorCode::ServerError,
                Some("internal error".into()),
                None,
            );
        }
    };

    let device_code_hash = crate::auth::token::hash_token(&device_code);

    emit_device_start_audit(
        &state,
        Some(device_code_hash),
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "has_client_name": req.client_name.is_some(),
            "scope": req.scope,
        }),
    );

    let base = resolve_external_base_url(external_base_url, &headers);
    let verification_uri = format!("{base}/device");
    let verification_uri_complete = format!("{base}/device?user_code={user_code}");

    let interval = uptrakit_web_api_auth::auth::device_flow::POLL_INTERVAL_SECONDS;
    let body = DeviceAuthorizationResponse::new(
        device_code,
        user_code,
        verification_uri,
        verification_uri_complete,
        // 10-minute TTL matching the device_flow_store's DEVICE_CODE_TTL_SECONDS.
        600,
        interval,
    );
    (StatusCode::OK, axum::Json(body)).into_response()
}

fn emit_device_start_audit(
    state: &AppState,
    device_flow_id: Option<String>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let mut builder = uptrakit_audit_log::AuditEntry::<uptrakit_audit_log::Event>::builder_event(
        uptrakit_audit_log::AuditActionType::AUTH_DEVICE_START,
    )
    .tenant_scope(state.default_tenant_id)
    .actor_system()
    .outcome(outcome)
    .details(details);

    if let Some(id) = device_flow_id {
        builder = builder.target("device_flow", id, None);
    }

    if let Ok(entry) = builder.build() {
        state.audit_emitter.emit_event(entry);
    }
}
