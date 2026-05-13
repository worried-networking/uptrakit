use crate::AppState;
use crate::actions::hosts as host_actions;
use crate::error_response::error_response;
use crate::middleware::permission::{
    CanDeactivateHosts, CanTriggerChecks, CanUpdateHosts, CanViewHosts,
};
use crate::middleware::require_auth::{AuthenticatedApiTokenId, authenticated_user_audit_actor};
use crate::queries::hosts as host_queries;
use crate::routes::service_ws::trigger_discovery_for_agent_host;
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{
    ColumnTrait, EntityTrait, QueryFilter, RelationTrait, SqliteTransactionMode,
    TransactionOptions, TransactionTrait,
};
use std::sync::Arc;
use uptrakit_audit_log::{AbsentView, AuditEntry, AuditOutcome, Stateful};
use uptrakit_shared_db::entity::{host, prelude::*, service, service_host};
use uptrakit_web_api_queries::queries::hosts::{
    HostView, deactivate_host_in_tx, update_host_in_tx,
};
use uptrakit_web_api_types::validation::Validate;
use uuid::Uuid;

pub use uptrakit_web_api_types::autodiscovery::TriggerDiscoveryResponse;
pub use uptrakit_web_api_types::batch_actions::{
    BatchActionFailure, BatchActionRequest, BatchActionResponse, BatchActionSuccess,
};
pub use uptrakit_web_api_types::hosts::{
    HostAgentSummary, HostMessageResponse, HostResponse, UpdateHostRequest,
};
pub use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};

// --- Endpoints ---

/// List all non-deactivated hosts
#[utoipa::path(
    get,
    path = "/api/v1/hosts",
    params(
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)")
    ),
    responses(
        (status = 200, description = "Paginated list of hosts", body = PaginatedResponse<HostResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Hosts",
    extensions(("x-required-permission" = json!("view_hosts"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_hosts(
    tenant_db: TenantDb,
    CanViewHosts(_user): CanViewHosts,
    Query(params): Query<PaginationParams>,
) -> Response {
    match host_queries::list_hosts(&tenant_db, &params).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!("Failed to list hosts: {}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Get a single host by ID
#[utoipa::path(
    get,
    path = "/api/v1/hosts/{id}",
    params(
        ("id" = Uuid, Path, description = "Host UUID")
    ),
    responses(
        (status = 200, description = "Host details", body = HostResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Host not found")
    ),
    tag = "Hosts",
    extensions(("x-required-permission" = json!("view_hosts"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_host(
    tenant_db: TenantDb,
    CanViewHosts(_user): CanViewHosts,
    Path(host_id): Path<Uuid>,
) -> Response {
    match host_queries::get_active_host(&tenant_db, host_id).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Host not found"),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Update a host's friendly name
#[utoipa::path(
    put,
    path = "/api/v1/hosts/{id}",
    params(
        ("id" = Uuid, Path, description = "Host UUID")
    ),
    request_body = UpdateHostRequest,
    responses(
        (status = 200, description = "Host updated", body = HostResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Host not found")
    ),
    tag = "Hosts",
    extensions(("x-required-permission" = json!("update_hosts"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_host(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateHosts(caller): CanUpdateHosts,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(host_id): Path<Uuid>,
    Json(body): Json<UpdateHostRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|v| v.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&caller, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    let tx = match state
        .db()
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
    {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Failed to begin transaction for host update: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let pair = match update_host_in_tx(&tx, tenant_id, host_id, &body).await {
        Ok(p) => p,
        Err(e) => {
            drop(tx);
            tracing::error!("Failed to update host: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let Some((before_model, after_model)) = pair else {
        drop(tx);
        if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
            uptrakit_audit_log::AuditActionType::HOST_UPDATE,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .target("host", host_id.to_string(), None)
        .outcome(AuditOutcome::Denied)
        .details(serde_json::json!({ "reason_code": "host_not_found" }))
        .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        return error_response(StatusCode::NOT_FOUND, "Host not found");
    };

    let before_view = HostView::from(&before_model);
    let after_view = HostView::from(&after_model);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::host_update(&before_view, &after_view)
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Success)
        .details(serde_json::json!({
            "changed_fields": if body.friendly_name.is_some() { vec!["friendly_name"] } else { Vec::<&str>::new() },
        }))
        .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!("Failed to build audit entry for host update: {e}");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!("Failed to emit audit entry for host update: {e}");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit host update: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    let resp = match host_queries::get_active_host(&tenant_db, host_id).await {
        Ok(Some(r)) => r,
        Ok(None) | Err(_) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };
    state
        .notification
        .event_broadcaster
        .send(
            tenant_id,
            uptrakit_web_api_types::events::AdminEvent::HostUpdated { id: host_id },
        )
        .await;
    (StatusCode::OK, Json(resp)).into_response()
}

/// Deactivate a host (soft-delete)
#[utoipa::path(
    delete,
    path = "/api/v1/hosts/{id}",
    params(
        ("id" = Uuid, Path, description = "Host UUID")
    ),
    responses(
        (status = 204, description = "Host deactivated"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Host not found")
    ),
    tag = "Hosts",
    extensions(("x-required-permission" = json!("deactivate_hosts"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn deactivate_host(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanDeactivateHosts(caller): CanDeactivateHosts,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(host_id): Path<Uuid>,
) -> Response {
    let api_token_id = api_token_id.map(|v| v.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&caller, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    let tx = match state
        .db()
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
    {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Failed to begin transaction for host deactivate: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let before_model = match deactivate_host_in_tx(&tx, tenant_id, host_id).await {
        Ok(m) => m,
        Err(e) => {
            drop(tx);
            tracing::error!("Failed to deactivate host: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let Some(before_model) = before_model else {
        drop(tx);
        if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
            uptrakit_audit_log::AuditActionType::HOST_DEACTIVATE,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .target("host", host_id.to_string(), None)
        .outcome(AuditOutcome::Denied)
        .details(serde_json::json!({ "reason_code": "host_not_found" }))
        .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        return error_response(StatusCode::NOT_FOUND, "Host not found");
    };

    let before_view = HostView::from(&before_model);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry =
        match AuditEntry::<Stateful>::host_deactivate(&before_view, &AbsentView(&before_view))
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Success)
            .details(serde_json::json!({}))
            .build()
        {
            Ok(entry) => entry,
            Err(e) => {
                tracing::error!("Failed to build audit entry for host deactivate: {e}");
                drop(tx);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!("Failed to emit audit entry for host deactivate: {e}");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit host deactivate: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    state
        .notification
        .event_broadcaster
        .send(
            tenant_id,
            uptrakit_web_api_types::events::AdminEvent::HostDeleted { id: host_id },
        )
        .await;
    StatusCode::NO_CONTENT.into_response()
}

// ── Autodiscovery endpoints ───────────────────────────────────────────────────

/// Trigger autodiscovery on a specific host.
///
/// Sends `DiscoverSoftware` to all agents that have this host linked.
#[utoipa::path(
    post,
    path = "/api/v1/hosts/{id}/discover",
    params(("id" = Uuid, Path, description = "Host UUID")),
    extensions(("x-required-permission" = json!("trigger_checks"))),
    responses(
        (status = 200, description = "Discovery triggered", body = TriggerDiscoveryResponse),
        (status = 404, description = "Host not found")
    ),
    tag = "Hosts",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn discover_host(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanTriggerChecks(caller): CanTriggerChecks,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(host_id): Path<Uuid>,
) -> Response {
    let api_token_id = api_token_id.map(|v| v.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&caller, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    // Verify host belongs to tenant.
    let host_record = match Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(h)) => h,
        Ok(None) => {
            if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                uptrakit_audit_log::AuditActionType::HOST_DISCOVER,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("host", host_id.to_string(), None)
            .outcome(AuditOutcome::Denied)
            .details(serde_json::json!({ "reason_code": "host_not_found" }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::NOT_FOUND, "Host not found");
        }
        Err(e) => {
            tracing::error!("DB error: {e}");
            if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                uptrakit_audit_log::AuditActionType::HOST_DISCOVER,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("host", host_id.to_string(), None)
            .outcome(AuditOutcome::Failed)
            .details(serde_json::json!({ "reason_code": "host_lookup_failed" }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Find all agents linked to this host (tenant-scoped via join on service).
    let links = match tenant_db
        .find_via_tenant_join::<service_host::Entity, service::Entity>(
            service_host::Relation::Service.def(),
        )
        .filter(service_host::Column::HostId.eq(host_id))
        .filter(service::Column::DeactivatedAt.is_null())
        .filter(service::Column::Status.eq(service::ServiceStatus::Approved))
        .all(tenant_db.db())
        .await
    {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Failed to query service-host links: {e}");
            if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                uptrakit_audit_log::AuditActionType::HOST_DISCOVER,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target(
                "host",
                host_id.to_string(),
                Some(host_record.friendly_name.clone()),
            )
            .outcome(AuditOutcome::Failed)
            .details(serde_json::json!({ "reason_code": "host_service_link_lookup_failed" }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let agents_notified = links.len() as u32;
    for link in &links {
        trigger_discovery_for_agent_host(
            &state,
            link.service_id,
            tenant_id,
            host_id,
            &host_record.machine_id,
        )
        .await;
    }

    if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
        uptrakit_audit_log::AuditActionType::HOST_DISCOVER,
    )
    .tenant_scope(tenant_id)
    .actor(actor_type, actor_id)
    .target(
        "host",
        host_id.to_string(),
        Some(host_record.friendly_name.clone()),
    )
    .outcome(AuditOutcome::Success)
    .details(serde_json::json!({ "agents_notified": agents_notified }))
    .build()
    {
        state.audit_emitter.emit_event(entry);
    }

    (
        StatusCode::OK,
        Json(TriggerDiscoveryResponse {
            plugins_queued: agents_notified,
            message: format!(
                "Discovery triggered on {} agent(s) for host '{}'",
                agents_notified, host_record.hostname
            ),
        }),
    )
        .into_response()
}

/// Perform a batch action on multiple hosts.
///
/// Supported actions: `deactivate`.
/// Returns per-item success/failure results (partial success is possible).
#[utoipa::path(
    post,
    path = "/api/v1/hosts/batch",
    request_body = BatchActionRequest,
    responses(
        (status = 200, description = "Batch action results", body = BatchActionResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Hosts",
    extensions(("x-required-permission" = json!("deactivate_hosts"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn batch_hosts(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanDeactivateHosts(caller): CanDeactivateHosts,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Json(body): Json<BatchActionRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|v| v.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&caller, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    if let Err(e) = body.validate() {
        if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
            uptrakit_audit_log::AuditActionType::HOST_DEACTIVATE,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::ValidationFailed)
        .details(serde_json::json!({
            "update_kind": "batch_deactivate",
            "reason_code": "invalid_request",
        }))
        .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    let ctx = state.mutation_context();
    let (succeeded_ids, failed) = match body.action.as_str() {
        "deactivate" => match host_actions::batch_deactivate(&tenant_db, &ctx, &body.ids).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("batch deactivate failed: {e}");
                if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                    uptrakit_audit_log::AuditActionType::HOST_DEACTIVATE,
                )
                .tenant_scope(tenant_id)
                .actor(actor_type, actor_id)
                .outcome(AuditOutcome::Failed)
                .details(serde_json::json!({
                    "update_kind": "batch_deactivate",
                    "reason_code": "host_batch_deactivate_failed",
                }))
                .build()
                {
                    state.audit_emitter.emit_event(entry);
                }
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        },
        unknown => {
            if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                uptrakit_audit_log::AuditActionType::HOST_DEACTIVATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::ValidationFailed)
            .details(serde_json::json!({
                "update_kind": "batch_deactivate",
                "reason_code": "unknown_action",
                "action": unknown,
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("unknown action: {unknown}. Supported: deactivate"),
            );
        }
    };

    if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
        uptrakit_audit_log::AuditActionType::HOST_DEACTIVATE,
    )
    .tenant_scope(tenant_id)
    .actor(actor_type, actor_id)
    .outcome(if failed.is_empty() {
        AuditOutcome::Success
    } else if succeeded_ids.is_empty() {
        AuditOutcome::Denied
    } else {
        AuditOutcome::Partial
    })
    .details(serde_json::json!({
        "update_kind": "batch_deactivate",
        "requested_count": body.ids.len(),
        "deactivated_count": succeeded_ids.len(),
        "failed_count": failed.len(),
    }))
    .build()
    {
        state.audit_emitter.emit_event(entry);
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
mod route_tests {
    #![expect(
        clippy::expect_used,
        reason = "test code: panics on failure are acceptable"
    )]
    #![expect(clippy::panic, reason = "test code: panics on failure are acceptable")]

    use super::*;
    use crate::test_harness::TestApp;
    use crate::test_harness::fixtures::{insert_host, register_and_get_token};
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
    use uptrakit_shared_db::entity::audit_log;

    async fn latest_host_audit_row(
        db: &sea_orm::DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
        target_id: Option<&str>,
    ) -> audit_log::Model {
        for _ in 0..50 {
            let mut query = audit_log::Entity::find()
                .filter(audit_log::Column::TenantId.is_not_null())
                .filter(audit_log::Column::ActionType.eq(action_type))
                .order_by_desc(audit_log::Column::OccurredAt);
            if let Some(target_id) = target_id {
                query = query.filter(audit_log::Column::TargetId.eq(target_id));
            }
            if let Some(row) = query.one(db).await.expect("query audit rows") {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected host audit row");
    }

    #[tokio::test]
    async fn update_host_writes_host_update_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = register_and_get_token(&client).await;
        let host = insert_host(&app.db, app.tenant_id).await;

        let status = client
            .put_json(
                &format!("/api/v1/hosts/{}", host.id),
                &UpdateHostRequest {
                    friendly_name: Some("Renamed Host".to_string()),
                },
            )
            .bearer(&access_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::OK);

        let row = latest_host_audit_row(
            &app.db,
            uptrakit_audit_log::AuditActionType::HOST_UPDATE,
            Some(host.id.to_string().as_str()),
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.target_id.as_deref(), Some(host.id.to_string().as_str()));

        // V2 stateful: before and after snapshots must be present
        let before = row.before_snapshot.expect("before_snapshot");
        assert_eq!(
            before["friendly_name"],
            serde_json::json!(host.friendly_name)
        );

        let after = row.after_snapshot.expect("after_snapshot");
        assert_eq!(after["friendly_name"], serde_json::json!("Renamed Host"));

        let details = row.details_json.expect("details");
        assert_eq!(
            details["changed_fields"],
            serde_json::json!(["friendly_name"])
        );
    }

    #[tokio::test]
    async fn deactivate_missing_host_writes_host_deactivate_denied_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = register_and_get_token(&client).await;
        let missing_id = Uuid::now_v7();

        let status = client
            .delete(&format!("/api/v1/hosts/{missing_id}"))
            .bearer(&access_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let missing_id_string = missing_id.to_string();
        let row = latest_host_audit_row(
            &app.db,
            uptrakit_audit_log::AuditActionType::HOST_DEACTIVATE,
            Some(missing_id_string.as_str()),
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["reason_code"], serde_json::json!("host_not_found"));
    }

    #[tokio::test]
    async fn deactivate_host_writes_host_deactivate_stateful_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = register_and_get_token(&client).await;
        let host = insert_host(&app.db, app.tenant_id).await;

        let status = client
            .delete(&format!("/api/v1/hosts/{}", host.id))
            .bearer(&access_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let row = latest_host_audit_row(
            &app.db,
            uptrakit_audit_log::AuditActionType::HOST_DEACTIVATE,
            Some(host.id.to_string().as_str()),
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.target_id.as_deref(), Some(host.id.to_string().as_str()));

        // V2 stateful: before snapshot has the host's friendly_name, after is {}
        let before = row.before_snapshot.expect("before_snapshot");
        assert_eq!(
            before["friendly_name"],
            serde_json::json!(host.friendly_name)
        );

        let after = row.after_snapshot.expect("after_snapshot");
        assert_eq!(after, serde_json::json!({}));
    }

    #[tokio::test]
    async fn discover_missing_host_writes_host_discover_denied_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = register_and_get_token(&client).await;
        let missing_id = Uuid::now_v7();

        let status = client
            .post_empty(&format!("/api/v1/hosts/{missing_id}/discover"))
            .bearer(&access_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let missing_id_string = missing_id.to_string();
        let row = latest_host_audit_row(
            &app.db,
            uptrakit_audit_log::AuditActionType::HOST_DISCOVER,
            Some(missing_id_string.as_str()),
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["reason_code"], serde_json::json!("host_not_found"));
    }

    #[tokio::test]
    async fn batch_hosts_invalid_request_writes_host_deactivate_validation_failed_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = register_and_get_token(&client).await;

        let status = client
            .post_json(
                "/api/v1/hosts/batch",
                &serde_json::json!({
                    "action": "deactivate",
                    "ids": [],
                }),
            )
            .bearer(&access_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let row = latest_host_audit_row(
            &app.db,
            uptrakit_audit_log::AuditActionType::HOST_DEACTIVATE,
            None,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["update_kind"],
            serde_json::json!("batch_deactivate")
        );
        assert_eq!(details["reason_code"], serde_json::json!("invalid_request"));
    }
}
