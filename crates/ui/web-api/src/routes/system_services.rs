use crate::AppState;
use crate::actions::system_services as ss_actions;
use crate::api_error::ApiError;
use crate::auth::permissions::Permission;
use crate::error_response::error_response;
use crate::middleware::permission::{
    CanApproveSystemServices, CanRejectSystemServices, CanRemoveSystemServices,
    CanUpdateSystemServices, CanViewSystemServices,
};
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
use crate::queries::system_services as ss_queries;
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use uptrakit_web_api_types::validation::Validate;
use uuid::Uuid;

pub use uptrakit_web_api_types::batch_actions::{
    BatchActionFailure, BatchActionRequest, BatchActionResponse, BatchActionSuccess,
};
pub use uptrakit_web_api_types::pagination::PaginatedResponse;
pub use uptrakit_web_api_types::system_services::{
    ListSystemServicesQuery, SystemServiceResponse, UpdateSystemServiceRequest,
};

fn emit_system_service_audit(
    state: &AppState,
    user: &AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
    target: Option<(Uuid, Option<String>)>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(user, api_token_id);
    let mut builder = uptrakit_audit_log::AuditEntry::builder(action_type)
        .system_scope()
        .actor(actor_type, actor_id)
        .outcome(outcome)
        .details(details);

    if let Some((service_id, service_name)) = target {
        builder = builder.target("service", service_id.to_string(), service_name);
    }

    if let Ok(entry) = builder.build() {
        state.audit_emitter.emit_best_effort(entry);
    }
}

fn batch_action_to_audit_action(action: &str) -> Option<uptrakit_audit_log::RegisteredAuditAction> {
    match action {
        "approve" => Some(uptrakit_audit_log::AuditActionType::SERVICE_APPROVE),
        "reject" => Some(uptrakit_audit_log::AuditActionType::SERVICE_REJECT),
        "deactivate" => Some(uptrakit_audit_log::AuditActionType::SERVICE_DEACTIVATE),
        _ => None,
    }
}

fn classify_system_service_query_audit_failure(
    err: &rootcause::Report<crate::queries::system_services::SystemServiceQueryError>,
) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
    use crate::queries::system_services::SystemServiceQueryError;

    match err.current_context() {
        SystemServiceQueryError::NotFound => (
            uptrakit_audit_log::AuditOutcome::Denied,
            "system_service.not_found",
        ),
        SystemServiceQueryError::NotPending => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "system_service.not_pending",
        ),
        SystemServiceQueryError::NotApproved => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "system_service.not_approved",
        ),
        SystemServiceQueryError::EmbeddedService => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "system_service.embedded_service",
        ),
        SystemServiceQueryError::Db(_) => (
            uptrakit_audit_log::AuditOutcome::Failed,
            "system_service.database_error",
        ),
    }
}

// ---------------------------------------------------------------------------
// Endpoints
// ---------------------------------------------------------------------------

/// List all system services
#[utoipa::path(
    get,
    path = "/api/v1/system-services",
    params(
        ("capability" = Option<String>, Query, description = "Filter by capability (update_tracking, scheduler)"),
        ("status" = Option<String>, Query, description = "Filter by status (pending, approved, rejected, deactivated)"),
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)")
    ),
    responses(
        (status = 200, description = "Paginated list of system services", body = PaginatedResponse<SystemServiceResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "System Services",
    extensions(("x-required-permission" = json!("view_system_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_system_services(
    State(state): State<Arc<AppState>>,
    CanViewSystemServices(_user): CanViewSystemServices,
    Query(query): Query<ListSystemServicesQuery>,
) -> Response {
    match ss_queries::list_system_services(state.db(), &query).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!("Failed to list system services: {}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Get a single system service by ID
#[utoipa::path(
    get,
    path = "/api/v1/system-services/{id}",
    params(
        ("id" = Uuid, Path, description = "System service UUID")
    ),
    responses(
        (status = 200, description = "System service details", body = SystemServiceResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "System service not found")
    ),
    tag = "System Services",
    extensions(("x-required-permission" = json!("view_system_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_system_service(
    State(state): State<Arc<AppState>>,
    CanViewSystemServices(_user): CanViewSystemServices,
    Path(service_id): Path<Uuid>,
) -> Response {
    match ss_queries::get_active_system_service(state.db(), service_id).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "System service not found"),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Update a system service's configurable settings (e.g. ping interval)
#[utoipa::path(
    put,
    path = "/api/v1/system-services/{id}",
    params(
        ("id" = Uuid, Path, description = "System service UUID")
    ),
    request_body = UpdateSystemServiceRequest,
    responses(
        (status = 200, description = "System service updated", body = SystemServiceResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "System service not found")
    ),
    tag = "System Services",
    extensions(("x-required-permission" = json!("update_system_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_system_service(
    State(state): State<Arc<AppState>>,
    CanUpdateSystemServices(user): CanUpdateSystemServices,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(service_id): Path<Uuid>,
    Json(body): Json<UpdateSystemServiceRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);

    if let Err(e) = body.validate() {
        emit_system_service_audit(
            &state,
            &user,
            api_token_id,
            uptrakit_audit_log::AuditActionType::SERVICE_UPDATE,
            Some((service_id, None)),
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            serde_json::json!({
                "reason_code": "invalid_request",
            }),
        );
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    match ss_queries::update_system_service_settings(
        state.db(),
        service_id,
        body.ping_interval_seconds,
        body.cert_lifetime_hours,
    )
    .await
    {
        Ok(Some(resp)) => {
            emit_system_service_audit(
                &state,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::SERVICE_UPDATE,
                Some((resp.id, Some(resp.friendly_name.clone()))),
                uptrakit_audit_log::AuditOutcome::Success,
                serde_json::json!({
                    "ping_interval_seconds": body.ping_interval_seconds,
                    "cert_lifetime_hours": body.cert_lifetime_hours,
                }),
            );
            (StatusCode::OK, Json(resp)).into_response()
        }
        Ok(None) => {
            emit_system_service_audit(
                &state,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::SERVICE_UPDATE,
                Some((service_id, None)),
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "reason_code": "system_service_not_found",
                }),
            );
            error_response(StatusCode::NOT_FOUND, "System service not found")
        }
        Err(e) => {
            tracing::error!("Failed to update system service: {}", e);
            emit_system_service_audit(
                &state,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::SERVICE_UPDATE,
                Some((service_id, None)),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "system_service_update_failed",
                }),
            );
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Approve a pending system service
#[utoipa::path(
    post,
    path = "/api/v1/system-services/{id}/approve",
    params(
        ("id" = Uuid, Path, description = "System service UUID")
    ),
    responses(
        (status = 200, description = "System service approved", body = SystemServiceResponse),
        (status = 400, description = "Service is not in pending status"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "System service not found")
    ),
    tag = "System Services",
    extensions(("x-required-permission" = json!("approve_system_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn approve_system_service(
    State(state): State<Arc<AppState>>,
    CanApproveSystemServices(user): CanApproveSystemServices,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(service_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let ctx = state.mutation_context();
    let resp = match ss_actions::approve(state.db(), &ctx, service_id).await {
        Ok(resp) => resp,
        Err(err) => {
            let (outcome, reason_code) = classify_system_service_query_audit_failure(&err);
            emit_system_service_audit(
                &state,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::SERVICE_APPROVE,
                Some((service_id, None)),
                outcome,
                serde_json::json!({
                    "reason_code": reason_code,
                }),
            );
            return Err(err.into());
        }
    };
    emit_system_service_audit(
        &state,
        &user,
        api_token_id,
        uptrakit_audit_log::AuditActionType::SERVICE_APPROVE,
        Some((resp.id, Some(resp.friendly_name.clone()))),
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "status": resp.status,
        }),
    );
    Ok((StatusCode::OK, Json(resp)).into_response())
}

/// Reject a pending system service
#[utoipa::path(
    post,
    path = "/api/v1/system-services/{id}/reject",
    params(
        ("id" = Uuid, Path, description = "System service UUID")
    ),
    responses(
        (status = 200, description = "System service rejected", body = SystemServiceResponse),
        (status = 400, description = "Service is not in pending status"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "System service not found")
    ),
    tag = "System Services",
    extensions(("x-required-permission" = json!("reject_system_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn reject_system_service(
    State(state): State<Arc<AppState>>,
    CanRejectSystemServices(user): CanRejectSystemServices,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(service_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let ctx = state.mutation_context();
    let resp =
        match ss_actions::reject(state.db(), &ctx, service_id, &state.service_connections).await {
            Ok(resp) => resp,
            Err(err) => {
                let (outcome, reason_code) = classify_system_service_query_audit_failure(&err);
                emit_system_service_audit(
                    &state,
                    &user,
                    api_token_id,
                    uptrakit_audit_log::AuditActionType::SERVICE_REJECT,
                    Some((service_id, None)),
                    outcome,
                    serde_json::json!({
                        "reason_code": reason_code,
                    }),
                );
                return Err(err.into());
            }
        };
    emit_system_service_audit(
        &state,
        &user,
        api_token_id,
        uptrakit_audit_log::AuditActionType::SERVICE_REJECT,
        Some((resp.id, Some(resp.friendly_name.clone()))),
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "status": resp.status,
        }),
    );
    Ok((StatusCode::OK, Json(resp)).into_response())
}

/// Deactivate a system service (soft-delete)
#[utoipa::path(
    delete,
    path = "/api/v1/system-services/{id}",
    params(
        ("id" = Uuid, Path, description = "System service UUID")
    ),
    responses(
        (status = 204, description = "System service deactivated"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "System service not found")
    ),
    tag = "System Services",
    extensions(("x-required-permission" = json!("remove_system_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn deactivate_system_service(
    State(state): State<Arc<AppState>>,
    CanRemoveSystemServices(user): CanRemoveSystemServices,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(service_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let ctx = state.mutation_context();
    let found = ss_actions::deactivate(
        state.db(),
        &ctx,
        service_id,
        &state.cert,
        &state.service_connections,
    )
    .await
    .map_err(|err| {
        let (outcome, reason_code) = classify_system_service_query_audit_failure(&err);
        emit_system_service_audit(
            &state,
            &user,
            api_token_id,
            uptrakit_audit_log::AuditActionType::SERVICE_DEACTIVATE,
            Some((service_id, None)),
            outcome,
            serde_json::json!({
                "reason_code": reason_code,
            }),
        );
        ApiError::from(err)
    })?;
    if found {
        emit_system_service_audit(
            &state,
            &user,
            api_token_id,
            uptrakit_audit_log::AuditActionType::SERVICE_DEACTIVATE,
            Some((service_id, None)),
            uptrakit_audit_log::AuditOutcome::Success,
            serde_json::json!({}),
        );
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        emit_system_service_audit(
            &state,
            &user,
            api_token_id,
            uptrakit_audit_log::AuditActionType::SERVICE_DEACTIVATE,
            Some((service_id, None)),
            uptrakit_audit_log::AuditOutcome::Denied,
            serde_json::json!({
                "reason_code": "system_service_not_found",
            }),
        );
        Ok(error_response(
            StatusCode::NOT_FOUND,
            "System service not found",
        ))
    }
}

/// Perform a batch action on multiple system services.
///
/// Supported actions: `approve`, `reject`, `deactivate`.
/// Returns per-item success/failure results (partial success is possible).
#[utoipa::path(
    post,
    path = "/api/v1/system-services/batch",
    request_body = BatchActionRequest,
    responses(
        (status = 200, description = "Batch action results", body = BatchActionResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "System Services",
    extensions(("x-required-permission" = json!("approve_system_services, reject_system_services, or remove_system_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn batch_system_services(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthenticatedUser>,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Json(body): Json<BatchActionRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let action_type = batch_action_to_audit_action(&body.action);

    if let Err(e) = body.validate() {
        if let Some(action_type) = action_type {
            emit_system_service_audit(
                &state,
                &auth_user,
                api_token_id,
                action_type,
                None,
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                serde_json::json!({
                    "reason_code": "invalid_request",
                    "batch": true,
                }),
            );
        }
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    let required = match body.action.as_str() {
        "approve" => Permission::ApproveSystemServices,
        "reject" => Permission::RejectSystemServices,
        "deactivate" => Permission::RemoveSystemServices,
        _ => return error_response(StatusCode::BAD_REQUEST, "Unknown batch action"),
    };
    if !auth_user.has_permission(required) {
        if let Some(action_type) = action_type {
            emit_system_service_audit(
                &state,
                &auth_user,
                api_token_id,
                action_type,
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "reason_code": "insufficient_permissions",
                    "batch": true,
                    "requested_count": body.ids.len(),
                }),
            );
        }
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let ctx = state.mutation_context();
    let (succeeded_ids, failed) = match body.action.as_str() {
        "approve" => match ss_actions::batch_approve(state.db(), &ctx, &body.ids).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("batch approve failed: {e}");
                emit_system_service_audit(
                    &state,
                    &auth_user,
                    api_token_id,
                    uptrakit_audit_log::AuditActionType::SERVICE_APPROVE,
                    None,
                    uptrakit_audit_log::AuditOutcome::Failed,
                    serde_json::json!({
                        "reason_code": "batch_approve_failed",
                        "batch": true,
                        "requested_count": body.ids.len(),
                    }),
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        },
        "reject" => {
            match ss_actions::batch_reject(state.db(), &ctx, &body.ids, &state.service_connections)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("batch reject failed: {e}");
                    emit_system_service_audit(
                        &state,
                        &auth_user,
                        api_token_id,
                        uptrakit_audit_log::AuditActionType::SERVICE_REJECT,
                        None,
                        uptrakit_audit_log::AuditOutcome::Failed,
                        serde_json::json!({
                            "reason_code": "batch_reject_failed",
                            "batch": true,
                            "requested_count": body.ids.len(),
                        }),
                    );
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            }
        }
        "deactivate" => {
            match ss_actions::batch_deactivate(
                state.db(),
                &ctx,
                &body.ids,
                &state.cert,
                &state.service_connections,
            )
            .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("batch deactivate failed: {e}");
                    emit_system_service_audit(
                        &state,
                        &auth_user,
                        api_token_id,
                        uptrakit_audit_log::AuditActionType::SERVICE_DEACTIVATE,
                        None,
                        uptrakit_audit_log::AuditOutcome::Failed,
                        serde_json::json!({
                            "reason_code": "batch_deactivate_failed",
                            "batch": true,
                            "requested_count": body.ids.len(),
                        }),
                    );
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            }
        }
        unknown => {
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("unknown action: {unknown}. Supported: approve, reject, deactivate"),
            );
        }
    };

    if let Some(action_type) = action_type {
        let outcome = if succeeded_ids.is_empty() {
            uptrakit_audit_log::AuditOutcome::Failed
        } else if failed.is_empty() {
            uptrakit_audit_log::AuditOutcome::Success
        } else {
            uptrakit_audit_log::AuditOutcome::Partial
        };

        emit_system_service_audit(
            &state,
            &auth_user,
            api_token_id,
            action_type,
            None,
            outcome,
            serde_json::json!({
                "batch": true,
                "requested_count": body.ids.len(),
                "succeeded_count": succeeded_ids.len(),
                "failed_count": failed.len(),
            }),
        );
    }

    let response = BatchActionResponse {
        succeeded: succeeded_ids
            .into_iter()
            .map(|id| BatchActionSuccess { id })
            .collect(),
        failed: failed
            .into_iter()
            .map(|(id, error)| BatchActionFailure { id, error })
            .collect(),
    };

    (StatusCode::OK, Json(response)).into_response()
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;
    use crate::auth::AuthMethod;
    use sea_orm::{
        ActiveModelTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryOrder, QuerySelect,
        Set,
    };
    use uptrakit_shared_db::entity::{system_audit_log, system_service};

    async fn latest_system_audit_row(db: &DatabaseConnection) -> system_audit_log::Model {
        for _ in 0..40 {
            if let Some(row) = system_audit_log::Entity::find()
                .order_by_desc(system_audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query system audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("expected at least one system audit row");
    }

    async fn count_system_audit_rows(db: &DatabaseConnection) -> u64 {
        system_audit_log::Entity::find()
            .select_only()
            .column(system_audit_log::Column::Id)
            .count(db)
            .await
            .expect("count system audit rows")
    }

    async fn wait_for_system_audit_rows(db: &DatabaseConnection, expected: u64) {
        for _ in 0..40 {
            if count_system_audit_rows(db).await == expected {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("expected {expected} system audit rows");
    }

    async fn insert_pending_system_service(db: &DatabaseConnection) -> system_service::Model {
        insert_system_service_with_status(db, system_service::SystemServiceStatus::Pending).await
    }

    async fn insert_system_service_with_status(
        db: &DatabaseConnection,
        status: system_service::SystemServiceStatus,
    ) -> system_service::Model {
        let id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        system_service::ActiveModel {
            id: Set(id),
            capabilities: Set("[]".to_string()),
            hostname: Set(format!("sys-host-{}", &id.to_string()[..8])),
            friendly_name: Set(format!("System Service {}", &id.to_string()[..8])),
            ip_address: Set(None),
            status: Set(status),
            enrollment_secret_hash: Set(format!("system-secret-{id}")),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            cert_lifetime_hours: Set(None),
            system_enrollment_token_id: Set(None),
            service_app_name: Set(None),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        }
        .insert(db)
        .await
        .expect("insert system service")
    }

    async fn set_system_service_embedded(
        db: &DatabaseConnection,
        service: system_service::Model,
        is_embedded: bool,
    ) -> system_service::Model {
        let mut active: system_service::ActiveModel = service.into();
        active.is_embedded = Set(is_embedded);
        active.updated_at = Set(time::OffsetDateTime::now_utc());
        active
            .update(db)
            .await
            .expect("update system service embedded flag")
    }

    #[tokio::test]
    async fn update_system_service_writes_service_update_audit_event() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service = insert_pending_system_service(&db).await;

        let auth_user = AuthenticatedUser {
            user_id: Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::UpdateSystemServices],
        };

        let response = update_system_service(
            State(Arc::clone(&state)),
            CanUpdateSystemServices::new(auth_user),
            None,
            Path(service.id),
            Json(UpdateSystemServiceRequest {
                ping_interval_seconds: Some(30),
                cert_lifetime_hours: Some(48),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        wait_for_system_audit_rows(&db, 1).await;
        let row = latest_system_audit_row(&db).await;
        let expected_target_id = service.id.to_string();

        assert_eq!(
            row.action_type,
            uptrakit_audit_log::AuditActionType::SERVICE_UPDATE
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("service"));
        assert_eq!(row.target_id.as_deref(), Some(expected_target_id.as_str()));
    }

    #[tokio::test]
    async fn batch_system_services_permission_denied_writes_denied_audit_event() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service = insert_pending_system_service(&db).await;

        let auth_user = AuthenticatedUser {
            user_id: Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::ViewSystemServices],
        };

        let response = batch_system_services(
            State(Arc::clone(&state)),
            Extension(auth_user),
            None,
            Json(BatchActionRequest {
                action: "approve".to_string(),
                ids: vec![service.id],
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        wait_for_system_audit_rows(&db, 1).await;
        let row = latest_system_audit_row(&db).await;

        assert_eq!(
            row.action_type,
            uptrakit_audit_log::AuditActionType::SERVICE_APPROVE
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
    }

    #[tokio::test]
    async fn approve_system_service_missing_service_writes_denied_audit_event() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let missing_service_id = Uuid::now_v7();

        let auth_user = AuthenticatedUser {
            user_id: Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::ApproveSystemServices],
        };

        let response = approve_system_service(
            State(Arc::clone(&state)),
            CanApproveSystemServices::new(auth_user),
            None,
            Path(missing_service_id),
        )
        .await;

        let status = match response {
            Err(e) => e.into_response().status(),
            Ok(_) => panic!("expected Err(ApiError) but got Ok"),
        };
        assert_eq!(status, StatusCode::NOT_FOUND);

        wait_for_system_audit_rows(&db, 1).await;
        let row = latest_system_audit_row(&db).await;

        assert_eq!(
            row.action_type,
            uptrakit_audit_log::AuditActionType::SERVICE_APPROVE
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(
            row.target_id.as_deref(),
            Some(missing_service_id.to_string().as_str())
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("system_service.not_found")
        );
    }

    #[tokio::test]
    async fn approve_system_service_non_pending_writes_validation_failed_audit_event() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service =
            insert_system_service_with_status(&db, system_service::SystemServiceStatus::Approved)
                .await;

        let auth_user = AuthenticatedUser {
            user_id: Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::ApproveSystemServices],
        };

        let response = approve_system_service(
            State(Arc::clone(&state)),
            CanApproveSystemServices::new(auth_user),
            None,
            Path(service.id),
        )
        .await;

        let status = match response {
            Err(e) => e.into_response().status(),
            Ok(_) => panic!("expected Err(ApiError) but got Ok"),
        };
        assert_eq!(status, StatusCode::BAD_REQUEST);

        wait_for_system_audit_rows(&db, 1).await;
        let row = latest_system_audit_row(&db).await;

        assert_eq!(
            row.action_type,
            uptrakit_audit_log::AuditActionType::SERVICE_APPROVE
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        assert_eq!(
            row.target_id.as_deref(),
            Some(service.id.to_string().as_str())
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("system_service.not_pending")
        );
    }

    #[tokio::test]
    async fn reject_system_service_missing_service_writes_denied_audit_event() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let missing_service_id = Uuid::now_v7();

        let auth_user = AuthenticatedUser {
            user_id: Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::RejectSystemServices],
        };

        let response = reject_system_service(
            State(Arc::clone(&state)),
            CanRejectSystemServices::new(auth_user),
            None,
            Path(missing_service_id),
        )
        .await;

        let status = match response {
            Err(e) => e.into_response().status(),
            Ok(_) => panic!("expected Err(ApiError) but got Ok"),
        };
        assert_eq!(status, StatusCode::NOT_FOUND);

        wait_for_system_audit_rows(&db, 1).await;
        let row = latest_system_audit_row(&db).await;

        assert_eq!(
            row.action_type,
            uptrakit_audit_log::AuditActionType::SERVICE_REJECT
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(
            row.target_id.as_deref(),
            Some(missing_service_id.to_string().as_str())
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("system_service.not_found")
        );
    }

    #[tokio::test]
    async fn deactivate_system_service_not_approved_writes_validation_failed_audit_event() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service = insert_pending_system_service(&db).await;
        let service = set_system_service_embedded(&db, service, true).await;

        let auth_user = AuthenticatedUser {
            user_id: Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::RemoveSystemServices],
        };

        let response = deactivate_system_service(
            State(Arc::clone(&state)),
            CanRemoveSystemServices::new(auth_user),
            None,
            Path(service.id),
        )
        .await;

        let status = match response {
            Err(e) => e.into_response().status(),
            Ok(_) => panic!("expected Err(ApiError) but got Ok"),
        };
        assert_eq!(status, StatusCode::BAD_REQUEST);

        wait_for_system_audit_rows(&db, 1).await;
        let row = latest_system_audit_row(&db).await;

        assert_eq!(
            row.action_type,
            uptrakit_audit_log::AuditActionType::SERVICE_DEACTIVATE
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        assert_eq!(
            row.target_id.as_deref(),
            Some(service.id.to_string().as_str())
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("system_service.embedded_service")
        );
    }
}
