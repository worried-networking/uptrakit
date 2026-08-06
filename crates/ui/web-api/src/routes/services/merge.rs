//! Service merge route handler.

use super::audit::{AuditContext, emit_service_lifecycle_audit};
use super::{MergeAgentRequest, ServiceResponse};
use crate::AppState;
use crate::actions::services as svc_actions;
use crate::api_error::ApiError;
use crate::error_response::{error_response, error_response_with_code};
use crate::extract::Unvalidated;
use crate::middleware::action::CanUpdateServices;
use crate::middleware::require_auth::AuthenticatedApiTokenId;
use crate::queries::services as svc_queries;
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use std::sync::Arc;
use uuid::Uuid;

/// Merge a pending (source) agent into an existing approved (target) agent.
///
/// This operation is only valid for agent services. MQTT services cannot be merged.
#[utoipa::path(
    post,
    path = "/api/v1/services/{target_id}/merge",
    params(
        ("target_id" = Uuid, Path, description = "Target service UUID (approved agent)")
    ),
    request_body = MergeAgentRequest,
    responses(
        (status = 200, description = "Services merged", body = ServiceResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Service not found")
    ),
    tag = "Services",
    security(("oauth2" = ["services:update"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn merge_service(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateServices(user): CanUpdateServices,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(target_uuid): Path<Uuid>,
    body: Unvalidated<MergeAgentRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &state.audit_emitter,
        tenant_id: tenant_db.tenant_id(),
        user: &user,
        api_token_id,
    };

    let body = match body.require_valid() {
        Ok(body) => body,
        Err(e) => {
            emit_service_lifecycle_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::SERVICE_MERGE,
                target_uuid,
                None,
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                serde_json::json!({
                    "reason_code": "service.invalid_request",
                }),
            );
            return Ok(error_response(StatusCode::BAD_REQUEST, e.to_string()));
        }
    };
    let source_uuid = body.source_id;

    if target_uuid == source_uuid {
        emit_service_lifecycle_audit(
            &audit_ctx,
            uptrakit_audit_log::AuditActionType::SERVICE_MERGE,
            target_uuid,
            None,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            serde_json::json!({
                "source_service_id": source_uuid,
                "reason_code": "service.self_merge",
            }),
        );
        return Ok(error_response(
            StatusCode::BAD_REQUEST,
            "Cannot merge service into itself",
        ));
    }

    // Defence-in-depth: short-circuit with a clear 400 + audit before any
    // session/state interaction. The query layer also rejects this (see
    // ServiceQueryError::{Target,Source}Embedded) but the route layer owns
    // the audit emission for ValidationFailed outcomes.
    let (target_embedded, source_embedded) =
        match svc_queries::is_embedded_pair(&tenant_db, target_uuid, source_uuid).await {
            Ok(pair) => pair,
            Err(err) => return Err(ApiError::from(err)),
        };
    if target_embedded {
        emit_service_lifecycle_audit(
            &audit_ctx,
            uptrakit_audit_log::AuditActionType::SERVICE_MERGE,
            target_uuid,
            None,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            serde_json::json!({
                "source_service_id": source_uuid,
                "reason_code": "service.embedded_target",
            }),
        );
        return Ok(error_response_with_code(
            StatusCode::BAD_REQUEST,
            "Cannot merge into an embedded service.",
            "service.embedded_target",
        ));
    }
    if source_embedded {
        emit_service_lifecycle_audit(
            &audit_ctx,
            uptrakit_audit_log::AuditActionType::SERVICE_MERGE,
            target_uuid,
            None,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            serde_json::json!({
                "source_service_id": source_uuid,
                "reason_code": "service.embedded_source",
            }),
        );
        return Ok(error_response_with_code(
            StatusCode::BAD_REQUEST,
            "Cannot merge from an embedded service.",
            "service.embedded_source",
        ));
    }

    let target_connected = state.service_connections.is_connected(&target_uuid).await;
    let ctx = state.mutation_context();
    let resp = svc_actions::merge(
        &tenant_db,
        &ctx,
        svc_actions::MergeParams {
            target_id: target_uuid,
            source_id: source_uuid,
            target_connected,
            default_tenant_id: state.default_tenant_id,
        },
        &state.cert,
        &state.service_connections,
    )
    .await
    .map_err(|err| {
        let (outcome, reason_code) = err.current_context().audit_classification();
        emit_service_lifecycle_audit(
            &audit_ctx,
            uptrakit_audit_log::AuditActionType::SERVICE_MERGE,
            target_uuid,
            None,
            outcome,
            serde_json::json!({
                "source_service_id": source_uuid,
                "reason_code": reason_code,
            }),
        );
        ApiError::from(err)
    })?;

    tracing::info!(
        target_id = %target_uuid,
        source_id = %source_uuid,
        "services merged: source deactivated, target updated"
    );

    emit_service_lifecycle_audit(
        &audit_ctx,
        uptrakit_audit_log::AuditActionType::SERVICE_MERGE,
        target_uuid,
        Some(resp.friendly_name.clone()),
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "source_service_id": source_uuid,
        }),
    );

    Ok((StatusCode::OK, Json(resp)).into_response())
}
