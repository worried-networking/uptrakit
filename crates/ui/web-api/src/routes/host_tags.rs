use crate::AppState;
use crate::actions::host_tags as tag_actions;
use crate::error_response::error_response;
use crate::middleware::permission::{CanUpdateHosts, CanViewHosts};
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
use crate::queries::host_tags as tag_queries;
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use std::sync::Arc;
use uptrakit_shared_db::entity::{host, host_tag};
use uptrakit_web_api_types::validation::Validate;
use uuid::Uuid;

pub use uptrakit_web_api_types::batch_actions::{
    BatchActionFailure, BatchActionRequest, BatchActionResponse, BatchActionSuccess,
};
pub use uptrakit_web_api_types::host_tags::{
    CreateHostTagRequest, HostTagResponse, HostTagSummary, ListHostTagsQuery, SetHostTagsRequest,
    UpdateHostTagRequest,
};
pub use uptrakit_web_api_types::pagination::PaginatedResponse;

struct AuditContext<'a> {
    state: &'a AppState,
    tenant_id: Uuid,
    user: &'a AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
}

fn emit_host_tag_audit(
    ctx: &AuditContext<'_>,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
    target_type: Option<&'static str>,
    target_id: Option<String>,
    target_display: Option<String>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(ctx.user, ctx.api_token_id);
    let mut builder = uptrakit_audit_log::AuditEntry::builder(action_type)
        .tenant_scope(ctx.tenant_id)
        .actor(actor_type, actor_id)
        .outcome(outcome)
        .details(details);

    if let (Some(target_type), Some(target_id)) = (target_type, target_id) {
        builder = builder.target(target_type, target_id, target_display);
    }

    if let Ok(entry) = builder.build() {
        ctx.state.audit_emitter.emit_best_effort(entry);
    }
}

// --- Endpoints ---

/// List all active host tags
#[utoipa::path(
    get,
    path = "/api/v1/host-tags",
    params(
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)"),
        ("search" = Option<String>, Query, description = "Filter by name (contains)")
    ),
    responses(
        (status = 200, description = "Paginated list of host tags", body = PaginatedResponse<HostTagResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Host Tags",
    extensions(("x-required-permission" = json!("view_hosts"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_host_tags(
    tenant_db: TenantDb,
    CanViewHosts(_user): CanViewHosts,
    Query(params): Query<ListHostTagsQuery>,
) -> Response {
    match tag_queries::list_host_tags(&tenant_db, &params).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!("Failed to list host tags: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Get a single host tag by ID
#[utoipa::path(
    get,
    path = "/api/v1/host-tags/{id}",
    params(
        ("id" = Uuid, Path, description = "Host tag UUID")
    ),
    responses(
        (status = 200, description = "Host tag details", body = HostTagResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Host tag not found")
    ),
    tag = "Host Tags",
    extensions(("x-required-permission" = json!("view_hosts"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_host_tag(
    tenant_db: TenantDb,
    CanViewHosts(_user): CanViewHosts,
    Path(tag_id): Path<Uuid>,
) -> Response {
    match tag_queries::get_host_tag(&tenant_db, tag_id).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Host tag not found"),
        Err(e) => {
            tracing::error!("DB error: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Create a new host tag
#[utoipa::path(
    post,
    path = "/api/v1/host-tags",
    request_body = CreateHostTagRequest,
    responses(
        (status = 201, description = "Host tag created", body = HostTagResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 409, description = "Tag with this name already exists")
    ),
    tag = "Host Tags",
    extensions(("x-required-permission" = json!("update_hosts"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn create_host_tag(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateHosts(caller): CanUpdateHosts,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Json(body): Json<CreateHostTagRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        state: &state,
        tenant_id: tenant_db.tenant_id,
        user: &caller,
        api_token_id,
    };
    if let Err(e) = body.validate() {
        emit_host_tag_audit(
            &audit_ctx,
            uptrakit_audit_log::AuditActionType::HOST_TAG_CREATE,
            None,
            None,
            None,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            serde_json::json!({
                "reason_code": "invalid_request",
            }),
        );
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    let ctx = state.mutation_context();
    match tag_actions::create(&tenant_db, &ctx, &body).await {
        Ok(resp) => {
            emit_host_tag_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::HOST_TAG_CREATE,
                Some("host_tag"),
                Some(resp.id.to_string()),
                Some(resp.name.clone()),
                uptrakit_audit_log::AuditOutcome::Success,
                serde_json::json!({
                    "name": resp.name,
                    "tag_name": resp.name,
                    "color": resp.color,
                    "description_present": resp.description.is_some(),
                }),
            );
            (StatusCode::CREATED, Json(resp)).into_response()
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("UNIQUE") || msg.contains("unique") || msg.contains("duplicate") {
                emit_host_tag_audit(
                    &audit_ctx,
                    uptrakit_audit_log::AuditActionType::HOST_TAG_CREATE,
                    None,
                    None,
                    None,
                    uptrakit_audit_log::AuditOutcome::ValidationFailed,
                    serde_json::json!({
                        "reason_code": "duplicate_tag_name",
                        "name": body.name,
                    }),
                );
                error_response(StatusCode::CONFLICT, "A tag with this name already exists")
            } else {
                tracing::error!("Failed to create host tag: {e}");
                emit_host_tag_audit(
                    &audit_ctx,
                    uptrakit_audit_log::AuditActionType::HOST_TAG_CREATE,
                    None,
                    None,
                    None,
                    uptrakit_audit_log::AuditOutcome::Failed,
                    serde_json::json!({
                        "reason_code": "host_tag_create_failed",
                        "name": body.name,
                    }),
                );
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            }
        }
    }
}

/// Update an existing host tag
#[utoipa::path(
    put,
    path = "/api/v1/host-tags/{id}",
    params(
        ("id" = Uuid, Path, description = "Host tag UUID")
    ),
    request_body = UpdateHostTagRequest,
    responses(
        (status = 200, description = "Host tag updated", body = HostTagResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Host tag not found"),
        (status = 409, description = "Tag with this name already exists")
    ),
    tag = "Host Tags",
    extensions(("x-required-permission" = json!("update_hosts"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_host_tag(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateHosts(caller): CanUpdateHosts,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(tag_id): Path<Uuid>,
    Json(body): Json<UpdateHostTagRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        state: &state,
        tenant_id: tenant_db.tenant_id,
        user: &caller,
        api_token_id,
    };
    if let Err(e) = body.validate() {
        emit_host_tag_audit(
            &audit_ctx,
            uptrakit_audit_log::AuditActionType::HOST_TAG_UPDATE,
            Some("host_tag"),
            Some(tag_id.to_string()),
            None,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            serde_json::json!({
                "reason_code": "invalid_request",
            }),
        );
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    let ctx = state.mutation_context();
    match tag_actions::update(&tenant_db, &ctx, tag_id, &body).await {
        Ok(Some(resp)) => {
            let mut changed_fields = Vec::new();
            if body.name.is_some() {
                changed_fields.push("name");
            }
            if body.color.is_some() {
                changed_fields.push("color");
            }
            if body.description.is_some() {
                changed_fields.push("description");
            }

            emit_host_tag_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::HOST_TAG_UPDATE,
                Some("host_tag"),
                Some(resp.id.to_string()),
                Some(resp.name.clone()),
                uptrakit_audit_log::AuditOutcome::Success,
                serde_json::json!({
                    "name": resp.name,
                    "color": resp.color,
                    "description_present": resp.description.is_some(),
                    "changed_fields": changed_fields,
                }),
            );
            (StatusCode::OK, Json(resp)).into_response()
        }
        Ok(None) => {
            emit_host_tag_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::HOST_TAG_UPDATE,
                Some("host_tag"),
                Some(tag_id.to_string()),
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "reason_code": "tag_not_found",
                }),
            );
            error_response(StatusCode::NOT_FOUND, "Host tag not found")
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("UNIQUE") || msg.contains("unique") || msg.contains("duplicate") {
                emit_host_tag_audit(
                    &audit_ctx,
                    uptrakit_audit_log::AuditActionType::HOST_TAG_UPDATE,
                    Some("host_tag"),
                    Some(tag_id.to_string()),
                    None,
                    uptrakit_audit_log::AuditOutcome::ValidationFailed,
                    serde_json::json!({
                        "reason_code": "duplicate_tag_name",
                    }),
                );
                error_response(StatusCode::CONFLICT, "A tag with this name already exists")
            } else {
                tracing::error!("Failed to update host tag: {e}");
                emit_host_tag_audit(
                    &audit_ctx,
                    uptrakit_audit_log::AuditActionType::HOST_TAG_UPDATE,
                    Some("host_tag"),
                    Some(tag_id.to_string()),
                    None,
                    uptrakit_audit_log::AuditOutcome::Failed,
                    serde_json::json!({
                        "reason_code": "host_tag_update_failed",
                    }),
                );
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            }
        }
    }
}

/// Delete a host tag (soft-delete)
#[utoipa::path(
    delete,
    path = "/api/v1/host-tags/{id}",
    params(
        ("id" = Uuid, Path, description = "Host tag UUID")
    ),
    responses(
        (status = 204, description = "Host tag deleted"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Host tag not found")
    ),
    tag = "Host Tags",
    extensions(("x-required-permission" = json!("update_hosts"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn delete_host_tag(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateHosts(caller): CanUpdateHosts,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(tag_id): Path<Uuid>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        state: &state,
        tenant_id: tenant_db.tenant_id,
        user: &caller,
        api_token_id,
    };
    let existing_tag = host_tag::Entity::find_by_id(tag_id)
        .filter(host_tag::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(host_tag::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
        .ok()
        .flatten();

    let ctx = state.mutation_context();
    match tag_actions::delete(&tenant_db, &ctx, tag_id).await {
        Ok(true) => {
            emit_host_tag_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::HOST_TAG_DELETE,
                Some("host_tag"),
                Some(tag_id.to_string()),
                existing_tag.as_ref().map(|tag| tag.name.clone()),
                uptrakit_audit_log::AuditOutcome::Success,
                serde_json::json!({
                    "name": existing_tag.as_ref().map(|tag| tag.name.clone()),
                }),
            );
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => {
            emit_host_tag_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::HOST_TAG_DELETE,
                Some("host_tag"),
                Some(tag_id.to_string()),
                existing_tag.as_ref().map(|tag| tag.name.clone()),
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "reason_code": "tag_not_found",
                }),
            );
            error_response(StatusCode::NOT_FOUND, "Host tag not found")
        }
        Err(e) => {
            tracing::error!("Failed to delete host tag: {e}");
            emit_host_tag_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::HOST_TAG_DELETE,
                Some("host_tag"),
                Some(tag_id.to_string()),
                existing_tag.as_ref().map(|tag| tag.name.clone()),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "host_tag_delete_failed",
                }),
            );
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Perform a batch action on multiple host tags.
///
/// Supported actions: `delete`.
#[utoipa::path(
    post,
    path = "/api/v1/host-tags/batch",
    request_body = BatchActionRequest,
    responses(
        (status = 200, description = "Batch action results", body = BatchActionResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Host Tags",
    extensions(("x-required-permission" = json!("update_hosts"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn batch_host_tags(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateHosts(caller): CanUpdateHosts,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Json(body): Json<BatchActionRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        state: &state,
        tenant_id: tenant_db.tenant_id,
        user: &caller,
        api_token_id,
    };
    if let Err(e) = body.validate() {
        emit_host_tag_audit(
            &audit_ctx,
            uptrakit_audit_log::AuditActionType::HOST_TAG_DELETE,
            None,
            None,
            None,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            serde_json::json!({
                "update_kind": "batch_delete",
                "reason_code": "invalid_request",
            }),
        );
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    let ctx = state.mutation_context();
    let (succeeded_ids, failed) = match body.action.as_str() {
        "delete" => match tag_actions::batch_delete(&tenant_db, &ctx, &body.ids).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("batch delete host tags failed: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        },
        unknown => {
            emit_host_tag_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::HOST_TAG_DELETE,
                None,
                None,
                None,
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                serde_json::json!({
                    "update_kind": "batch_delete",
                    "reason_code": "unknown_action",
                    "action": unknown,
                }),
            );
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("unknown action: {unknown}. Supported: delete"),
            );
        }
    };

    emit_host_tag_audit(
        &audit_ctx,
        uptrakit_audit_log::AuditActionType::HOST_TAG_DELETE,
        None,
        None,
        None,
        if failed.is_empty() {
            uptrakit_audit_log::AuditOutcome::Success
        } else if succeeded_ids.is_empty() {
            uptrakit_audit_log::AuditOutcome::Denied
        } else {
            uptrakit_audit_log::AuditOutcome::Partial
        },
        serde_json::json!({
            "update_kind": "batch_delete",
            "requested_count": body.ids.len(),
            "deleted_count": succeeded_ids.len(),
            "failed_count": failed.len(),
        }),
    );

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

/// Set (replace-all) tags on a host
#[utoipa::path(
    put,
    path = "/api/v1/hosts/{id}/tags",
    params(
        ("id" = Uuid, Path, description = "Host UUID")
    ),
    request_body = SetHostTagsRequest,
    responses(
        (status = 200, description = "Tags assigned", body = Vec<HostTagSummary>),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Host not found")
    ),
    tag = "Host Tags",
    extensions(("x-required-permission" = json!("update_hosts"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn set_host_tags(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateHosts(caller): CanUpdateHosts,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(host_id): Path<Uuid>,
    Json(body): Json<SetHostTagsRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        state: &state,
        tenant_id: tenant_db.tenant_id,
        user: &caller,
        api_token_id,
    };
    if let Err(e) = body.validate() {
        emit_host_tag_audit(
            &audit_ctx,
            uptrakit_audit_log::AuditActionType::HOST_TAG_ASSIGN,
            Some("host"),
            Some(host_id.to_string()),
            None,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            serde_json::json!({
                "reason_code": "invalid_request",
            }),
        );
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    let host_record = match host::Entity::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(host)) => host,
        Ok(None) => {
            emit_host_tag_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::HOST_TAG_ASSIGN,
                Some("host"),
                Some(host_id.to_string()),
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "reason_code": "host_not_found",
                    "requested_tag_count": body.tag_ids.len(),
                }),
            );
            return error_response(StatusCode::NOT_FOUND, "Host not found");
        }
        Err(e) => {
            tracing::error!("DB error: {e}");
            emit_host_tag_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::HOST_TAG_ASSIGN,
                Some("host"),
                Some(host_id.to_string()),
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "host_lookup_failed",
                    "requested_tag_count": body.tag_ids.len(),
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let ctx = state.mutation_context();
    // Refresh MQTT `{prefix}/hosts/{h}/tags` for all connected MQTT services.
    match tag_actions::set(&tenant_db, &ctx, host_id, &body.tag_ids).await {
        Ok(tags) => {
            emit_host_tag_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::HOST_TAG_ASSIGN,
                Some("host"),
                Some(host_id.to_string()),
                Some(host_record.friendly_name),
                if tags.len() == body.tag_ids.len() {
                    uptrakit_audit_log::AuditOutcome::Success
                } else {
                    uptrakit_audit_log::AuditOutcome::Partial
                },
                serde_json::json!({
                    "requested_tag_count": body.tag_ids.len(),
                    "assigned_tag_count": tags.len(),
                    "assigned_tag_ids": tags.iter().map(|tag| tag.id).collect::<Vec<_>>(),
                }),
            );
            (StatusCode::OK, Json(tags)).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to set host tags: {e}");
            emit_host_tag_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::HOST_TAG_ASSIGN,
                Some("host"),
                Some(host_id.to_string()),
                Some(host_record.friendly_name),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "host_tag_assignment_failed",
                    "requested_tag_count": body.tag_ids.len(),
                }),
            );
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    #![expect(
        clippy::expect_used,
        reason = "test code: panics on failure are acceptable"
    )]
    #![expect(clippy::panic, reason = "test code: panics on failure are acceptable")]

    use super::*;
    use crate::test_harness::TestApp;
    use crate::test_harness::fixtures;
    use crate::test_harness::fixtures::insert_host;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
    use uptrakit_shared_db::entity::audit_log;

    async fn latest_tenant_audit_row_for_action(
        db: &sea_orm::DatabaseConnection,
        action_type: &str,
    ) -> audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = audit_log::Entity::find()
                .filter(audit_log::Column::TenantId.is_not_null())
                .filter(audit_log::Column::ActionType.eq(action_type))
                .order_by_desc(audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected tenant audit row for action {action_type}");
    }

    #[tokio::test]
    async fn create_host_tag_writes_host_tag_create_success_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = fixtures::register_and_get_token(&client).await;

        let req = CreateHostTagRequest {
            name: "production".to_string(),
            color: Some("#3B82F6".to_string()),
            description: Some("Production hosts".to_string()),
        };

        let status = client
            .post_json("/api/v1/host-tags", &req)
            .bearer(&access_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::CREATED);

        let row = latest_tenant_audit_row_for_action(&app.db, "host_tag.create").await;
        assert_eq!(row.action_type, "host_tag.create");
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::User.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("host_tag"));
        let details = row.details_json.expect("details");
        assert_eq!(details["tag_name"], serde_json::json!("production"));
    }

    #[tokio::test]
    async fn update_host_tag_not_found_writes_host_tag_update_denied_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = fixtures::register_and_get_token(&client).await;
        let missing_tag_id = Uuid::now_v7();

        let req = UpdateHostTagRequest {
            name: Some("renamed-tag".to_string()),
            color: None,
            description: None,
        };

        let status = client
            .put_json(&format!("/api/v1/host-tags/{missing_tag_id}"), &req)
            .bearer(&access_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let row = latest_tenant_audit_row_for_action(&app.db, "host_tag.update").await;
        assert_eq!(row.action_type, "host_tag.update");
        assert_eq!(row.target_type.as_deref(), Some("host_tag"));
        assert_eq!(
            row.target_id.as_deref(),
            Some(missing_tag_id.to_string().as_str())
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["reason_code"], serde_json::json!("tag_not_found"));
    }

    #[tokio::test]
    async fn delete_host_tag_not_found_writes_host_tag_delete_denied_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = fixtures::register_and_get_token(&client).await;
        let missing_tag_id = Uuid::now_v7();

        let status = client
            .delete(&format!("/api/v1/host-tags/{missing_tag_id}"))
            .bearer(&access_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let row = latest_tenant_audit_row_for_action(&app.db, "host_tag.delete").await;
        assert_eq!(row.action_type, "host_tag.delete");
        assert_eq!(row.target_type.as_deref(), Some("host_tag"));
        assert_eq!(
            row.target_id.as_deref(),
            Some(missing_tag_id.to_string().as_str())
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["reason_code"], serde_json::json!("tag_not_found"));
    }

    #[tokio::test]
    async fn set_host_tags_invalid_request_writes_host_tag_assign_validation_failed_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = fixtures::register_and_get_token(&client).await;
        let host_id = Uuid::now_v7();
        let tag_ids = (0..51).map(|_| Uuid::now_v7()).collect::<Vec<_>>();

        let status = client
            .put_json(
                &format!("/api/v1/hosts/{host_id}/tags"),
                &SetHostTagsRequest { tag_ids },
            )
            .bearer(&access_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let row = latest_tenant_audit_row_for_action(&app.db, "host_tag.assign").await;
        assert_eq!(row.action_type, "host_tag.assign");
        assert_eq!(row.target_type.as_deref(), Some("host"));
        assert_eq!(row.target_id.as_deref(), Some(host_id.to_string().as_str()));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["reason_code"], serde_json::json!("invalid_request"));
    }

    #[tokio::test]
    async fn set_host_tags_missing_host_writes_host_tag_assign_denied_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = fixtures::register_and_get_token(&client).await;
        let host_id = Uuid::now_v7();

        let status = client
            .put_json(
                &format!("/api/v1/hosts/{host_id}/tags"),
                &SetHostTagsRequest { tag_ids: vec![] },
            )
            .bearer(&access_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let row = latest_tenant_audit_row_for_action(&app.db, "host_tag.assign").await;
        assert_eq!(row.action_type, "host_tag.assign");
        assert_eq!(row.target_type.as_deref(), Some("host"));
        assert_eq!(row.target_id.as_deref(), Some(host_id.to_string().as_str()));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["reason_code"], serde_json::json!("host_not_found"));
    }

    #[tokio::test]
    async fn set_host_tags_success_writes_host_tag_assign_success_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = fixtures::register_and_get_token(&client).await;
        let host = insert_host(&app.db, app.tenant_id).await;

        let (create_status, created_tag): (StatusCode, serde_json::Value) = client
            .post_json(
                "/api/v1/host-tags",
                &CreateHostTagRequest {
                    name: "blue".to_string(),
                    color: Some("#3B82F6".to_string()),
                    description: None,
                },
            )
            .bearer(&access_token)
            .send_json()
            .await;
        assert_eq!(create_status, StatusCode::CREATED);
        let tag_id = created_tag["id"].as_str().expect("tag id");

        let status = client
            .put_json(
                &format!("/api/v1/hosts/{}/tags", host.id),
                &serde_json::json!({ "tag_ids": [tag_id] }),
            )
            .bearer(&access_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::OK);

        let row = latest_tenant_audit_row_for_action(&app.db, "host_tag.assign").await;
        assert_eq!(row.action_type, "host_tag.assign");
        assert_eq!(row.target_type.as_deref(), Some("host"));
        assert_eq!(row.target_id.as_deref(), Some(host.id.to_string().as_str()));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["requested_tag_count"], serde_json::json!(1));
        assert_eq!(details["assigned_tag_count"], serde_json::json!(1));
    }
}
