use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use uptrakit_web_api_types::validation::Validate;

use crate::AppState;
use crate::api_error::ApiError;
use crate::auth::token::hash_token;
use crate::auth_audit_classification::DeviceFlowErrorAuditExt;
use crate::extract::Validated;
use crate::middleware::permission::CanViewServices;
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};

pub use uptrakit_web_api_types::device_auth::{
    DeviceAuthApproveRequest, DeviceAuthApproveResponse,
};
pub use uptrakit_web_api_types::oauth::{
    DeviceAuthDenyRequest, DeviceAuthDenyResponse, DeviceAuthLookupQuery, DeviceAuthLookupResponse,
};

fn emit_device_auth_decision_audit(
    state: &AppState,
    user: &AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
    device_flow_id: String,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(user, api_token_id);

    let entry = uptrakit_audit_log::AuditEntry::builder(action_type)
        .tenant_scope(state.default_tenant_id)
        .actor(actor_type, actor_id)
        .target("device_flow", device_flow_id, None)
        .outcome(outcome)
        .details(details)
        .build();

    if let Ok(entry) = entry {
        state.audit_emitter.emit_best_effort(entry);
    }
}

/// Approve a device authorization (authenticated)
#[utoipa::path(
    post,
    path = "/api/v1/auth/device/approve",
    request_body = DeviceAuthApproveRequest,
    responses(
        (status = 200, description = "Device authorized", body = DeviceAuthApproveResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Device flow not found"),
        (status = 409, description = "Already authorized")
    ),
    tag = "Authentication",
    extensions(("x-required-permission" = json!("view_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn device_auth_approve(
    State(state): State<Arc<AppState>>,
    CanViewServices(auth_user): CanViewServices,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Json(req): Json<DeviceAuthApproveRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let normalized = req.user_code.replace('-', "").to_uppercase();
    let device_flow_id = hash_token(&normalized);

    if let Err(error) = state
        .auth
        .device_flow_store
        .approve(&normalized, auth_user.user_id)
        .await
    {
        let (action_type, outcome, reason_code) = error.current_context().approval_classification();
        emit_device_auth_decision_audit(
            &state,
            &auth_user,
            api_token_id,
            action_type,
            device_flow_id,
            outcome,
            serde_json::json!({ "reason_code": reason_code }),
        );
        return Err(error.into());
    }

    emit_device_auth_decision_audit(
        &state,
        &auth_user,
        api_token_id,
        uptrakit_audit_log::AuditActionType::AUTH_DEVICE_APPROVE,
        device_flow_id,
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({}),
    );

    Ok((
        StatusCode::OK,
        Json(DeviceAuthApproveResponse {
            message: "Device authorized".into(),
        }),
    )
        .into_response())
}

/// Operator denies a pending device authorization.
#[utoipa::path(
    post,
    path = "/api/v1/auth/device/deny",
    request_body = DeviceAuthDenyRequest,
    responses(
        (status = 200, description = "Device denied", body = DeviceAuthDenyResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Device flow not found"),
        (status = 409, description = "Already authorized or denied"),
    ),
    tag = "Authentication",
    extensions(("x-required-permission" = json!("view_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn device_auth_deny(
    State(state): State<Arc<AppState>>,
    CanViewServices(auth_user): CanViewServices,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Validated(req): Validated<DeviceAuthDenyRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let normalized = req.user_code.replace('-', "").to_uppercase();
    let device_flow_id = hash_token(&normalized);

    if let Err(error) = state
        .auth
        .device_flow_store
        .deny(&normalized, auth_user.user_id)
        .await
    {
        let (outcome, reason_code) = error.current_context().denial_classification();
        emit_device_auth_decision_audit(
            &state,
            &auth_user,
            api_token_id,
            uptrakit_audit_log::AuditActionType::AUTH_DEVICE_DENY,
            device_flow_id,
            outcome,
            serde_json::json!({ "reason_code": reason_code }),
        );
        return Err(error.into());
    }

    emit_device_auth_decision_audit(
        &state,
        &auth_user,
        api_token_id,
        uptrakit_audit_log::AuditActionType::AUTH_DEVICE_DENY,
        device_flow_id,
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({}),
    );

    Ok((
        StatusCode::OK,
        Json(DeviceAuthDenyResponse {
            message: "Device authorization denied.".into(),
        }),
    )
        .into_response())
}

/// Look up client name and expiry for a pending device flow by user code.
#[utoipa::path(
    get,
    path = "/api/v1/auth/device/lookup",
    params(DeviceAuthLookupQuery),
    responses(
        (status = 200, description = "Lookup ok", body = DeviceAuthLookupResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Device flow not found"),
    ),
    tag = "Authentication",
    extensions(("x-required-permission" = json!("view_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn device_auth_lookup(
    State(state): State<Arc<AppState>>,
    _auth: CanViewServices,
    Query(query): Query<DeviceAuthLookupQuery>,
) -> Result<axum::Json<DeviceAuthLookupResponse>, ApiError> {
    if let Err(e) = query.validate() {
        return Err(ApiError::new(
            axum::http::StatusCode::BAD_REQUEST,
            e.to_string(),
            "validation_error",
            None,
        ));
    }

    let flow = state
        .auth
        .device_flow_store
        .lookup_by_user_code(&query.user_code)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| {
            ApiError::new(
                axum::http::StatusCode::NOT_FOUND,
                "Device flow not found.",
                "device_flow.not_found",
                None,
            )
        })?;

    Ok(axum::Json(DeviceAuthLookupResponse {
        client_name: flow.client_name,
        expires_at: flow.expires_at,
    }))
}
