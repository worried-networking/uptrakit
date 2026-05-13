use crate::AppState;
use crate::api_error::ApiError;
use crate::error_response::error_response;
use crate::middleware::permission::{CanManageNotifications, CanViewNotifications};
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
use crate::queries::notifications as notif_queries;
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{SqliteTransactionMode, TransactionOptions, TransactionTrait};
use serde::Deserialize;
use std::sync::Arc;
use uptrakit_audit_log::{AbsentView, AuditEntry, AuditOutcome, Stateful};
use uptrakit_plugin_infrastructure_registry::{DeliveryMessage, SurfaceActionContext};
use uptrakit_web_api_types::pagination::PaginationParams;
use uptrakit_web_api_types::validation::Validate;
use uuid::Uuid;

pub use uptrakit_web_api_types::notifications::{
    CreateNotificationChannelRequest, CreateNotificationRuleRequest, NotificationChannelResponse,
    NotificationDeliveryStatus, NotificationEventType, NotificationLogResponse,
    NotificationRuleResponse, TestNotificationResponse, UpdateNotificationChannelRequest,
    UpdateNotificationRuleRequest,
};
pub use uptrakit_web_api_types::pagination::PaginatedResponse;

// ---------------------------------------------------------------------------
// Query params
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ListRulesQuery {
    pub channel_id: Option<Uuid>,
    pub event_type: Option<String>,
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

struct AuditContext<'a> {
    audit_emitter: &'a uptrakit_audit_log::AuditEmitter,
    tenant_id: Uuid,
    user: &'a AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
}

fn emit_notification_audit(
    ctx: &AuditContext<'_>,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
    target_type: &'static str,
    target_id: String,
    target_display: Option<String>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(ctx.user, ctx.api_token_id);

    let entry =
        uptrakit_audit_log::AuditEntry::<uptrakit_audit_log::Event>::builder_event(action_type)
            .tenant_scope(ctx.tenant_id)
            .actor(actor_type, actor_id)
            .target(target_type, target_id, target_display)
            .outcome(outcome)
            .details(details)
            .build();

    if let Ok(entry) = entry {
        ctx.audit_emitter.emit_event(entry);
    }
}

fn emit_notification_callback_audit(
    audit_emitter: &uptrakit_audit_log::AuditEmitter,
    tenant_id: Uuid,
    channel_id: Uuid,
    channel_name: &str,
    channel_type: &str,
    outcome: uptrakit_audit_log::AuditOutcome,
    reason_code: Option<&str>,
) {
    let mut details =
        serde_json::Map::from_iter([("channel_type".to_string(), serde_json::json!(channel_type))]);
    if let Some(reason_code) = reason_code {
        details.insert("reason_code".to_string(), serde_json::json!(reason_code));
    }

    let entry = uptrakit_audit_log::AuditEntry::<uptrakit_audit_log::Event>::builder_event(
        uptrakit_audit_log::AuditActionType::NOTIFICATION_CALLBACK,
    )
    .tenant_scope(tenant_id)
    .actor_system()
    .target(
        "notification_channel",
        channel_id.to_string(),
        Some(channel_name.to_string()),
    )
    .outcome(outcome)
    .details(serde_json::Value::Object(details))
    .build();

    if let Ok(entry) = entry {
        audit_emitter.emit_event(entry);
    }
}

fn classify_notification_callback_error(
    err: &uptrakit_plugin_infrastructure_registry::SurfaceActionError,
) -> (StatusCode, uptrakit_audit_log::AuditOutcome, &'static str) {
    match err {
        uptrakit_plugin_infrastructure_registry::SurfaceActionError::InvalidInput(message)
            if message.contains("Unauthorized: invalid_secret") =>
        {
            (
                StatusCode::UNAUTHORIZED,
                uptrakit_audit_log::AuditOutcome::Denied,
                "invalid_secret",
            )
        }
        uptrakit_plugin_infrastructure_registry::SurfaceActionError::InvalidInput(message)
            if message.contains("Bad request: missing_action_token") =>
        {
            (
                StatusCode::BAD_REQUEST,
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "missing_action_token",
            )
        }
        uptrakit_plugin_infrastructure_registry::SurfaceActionError::InvalidInput(message)
            if message.contains("Bad request: invalid_action_token") =>
        {
            (
                StatusCode::BAD_REQUEST,
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "invalid_action_token",
            )
        }
        uptrakit_plugin_infrastructure_registry::SurfaceActionError::ControllerIntegration(
            message,
        ) if message.contains("Internal server error: notification_log_lookup_failed") => (
            StatusCode::INTERNAL_SERVER_ERROR,
            uptrakit_audit_log::AuditOutcome::Failed,
            "notification_log_lookup_failed",
        ),
        uptrakit_plugin_infrastructure_registry::SurfaceActionError::ControllerIntegration(
            message,
        ) if message.contains("Internal server error: notification_log_update_failed") => (
            StatusCode::INTERNAL_SERVER_ERROR,
            uptrakit_audit_log::AuditOutcome::Failed,
            "notification_log_update_failed",
        ),
        uptrakit_plugin_infrastructure_registry::SurfaceActionError::InvalidInput(_) => (
            StatusCode::BAD_REQUEST,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "notification_callback_invalid_input",
        ),
        uptrakit_plugin_infrastructure_registry::SurfaceActionError::ControllerIntegration(_)
        | uptrakit_plugin_infrastructure_registry::SurfaceActionError::PluginInternal(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            uptrakit_audit_log::AuditOutcome::Failed,
            "notification_callback_failed",
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            uptrakit_audit_log::AuditOutcome::Failed,
            "notification_callback_failed",
        ),
    }
}

// ---------------------------------------------------------------------------
// Channel endpoints
// ---------------------------------------------------------------------------

/// Create a notification channel
#[utoipa::path(
    post,
    path = "/api/v1/notifications/channels",
    request_body = CreateNotificationChannelRequest,
    responses(
        (status = 201, description = "Channel created", body = NotificationChannelResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Notifications",
    extensions(("x-required-permission" = json!("manage_notifications"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn create_channel(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageNotifications(user): CanManageNotifications,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Json(body): Json<CreateNotificationChannelRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    if let Err(e) = body.validate() {
        let audit_ctx = AuditContext {
            audit_emitter: &state.audit_emitter,
            tenant_id,
            user: &user,
            api_token_id,
        };
        emit_notification_audit(
            &audit_ctx,
            uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_CREATE,
            "notification_channel",
            "pending".to_string(),
            body.name.clone().into(),
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            serde_json::json!({ "reason_code": "invalid_request" }),
        );
        return Ok(error_response(StatusCode::BAD_REQUEST, e.to_string()));
    }

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
            tracing::error!("Failed to begin transaction for notification channel create: {e}");
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    let model =
        match notif_queries::create_channel_in_tx(&tx, tenant_id, &body, &*state.plugin.plugin_ops)
            .await
        {
            Ok(m) => m,
            Err(err) => {
                drop(tx);
                let (outcome, reason_code) = err.current_context().audit_classification();
                let audit_ctx = AuditContext {
                    audit_emitter: &state.audit_emitter,
                    tenant_id,
                    user: &user,
                    api_token_id,
                };
                emit_notification_audit(
                    &audit_ctx,
                    uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_CREATE,
                    "notification_channel",
                    "pending".to_string(),
                    body.name.clone().into(),
                    outcome,
                    serde_json::json!({ "reason_code": reason_code }),
                );
                return Err(err.into());
            }
        };

    let after_view = notif_queries::NotificationChannelView::from(&model);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::notification_channel_create(
        &AbsentView(&after_view),
        &after_view,
    )
    .tenant_scope(tenant_id)
    .actor(actor_type, actor_id)
    .outcome(AuditOutcome::Success)
    .details(serde_json::json!({
        "channel_type": model.channel_type,
        "enabled": model.enabled,
    }))
    .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!("Failed to build audit entry for notification channel create: {e}");
            drop(tx);
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!("Failed to emit audit entry for notification channel create: {e}");
        drop(tx);
        return Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error",
        ));
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit notification channel create: {e}");
        return Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error",
        ));
    }
    hook.flush_after_commit().await;

    let masked_config = notif_queries::mask_channel_config(&model, &*state.plugin.plugin_ops);
    let resp = notif_queries::channel_to_response(model, masked_config);
    Ok((StatusCode::CREATED, Json(resp)).into_response())
}

/// List all notification channels
#[utoipa::path(
    get,
    path = "/api/v1/notifications/channels",
    params(
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)")
    ),
    responses(
        (status = 200, description = "Paginated list of channels", body = PaginatedResponse<NotificationChannelResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Notifications",
    extensions(("x-required-permission" = json!("view_notifications"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_channels(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanViewNotifications(_user): CanViewNotifications,
    Query(params): Query<PaginationParams>,
) -> Response {
    match notif_queries::list_channels(&tenant_db, &params, &*state.plugin.plugin_ops).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!(error = ?e, "failed to list notification channels");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Get a notification channel by ID
#[utoipa::path(
    get,
    path = "/api/v1/notifications/channels/{id}",
    params(
        ("id" = Uuid, Path, description = "Channel UUID")
    ),
    responses(
        (status = 200, description = "Channel details", body = NotificationChannelResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Channel not found")
    ),
    tag = "Notifications",
    extensions(("x-required-permission" = json!("view_notifications"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_channel(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanViewNotifications(_user): CanViewNotifications,
    Path(channel_id): Path<Uuid>,
) -> Response {
    match notif_queries::get_channel(&tenant_db, channel_id, &*state.plugin.plugin_ops).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Channel not found"),
        Err(e) => {
            tracing::error!(error = ?e, "failed to get notification channel");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Update a notification channel
#[utoipa::path(
    put,
    path = "/api/v1/notifications/channels/{id}",
    params(
        ("id" = Uuid, Path, description = "Channel UUID")
    ),
    request_body = UpdateNotificationChannelRequest,
    responses(
        (status = 200, description = "Channel updated", body = NotificationChannelResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Channel not found")
    ),
    tag = "Notifications",
    extensions(("x-required-permission" = json!("manage_notifications"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_channel(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageNotifications(user): CanManageNotifications,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(channel_id): Path<Uuid>,
    Json(body): Json<UpdateNotificationChannelRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    if let Err(e) = body.validate() {
        let audit_ctx = AuditContext {
            audit_emitter: &state.audit_emitter,
            tenant_id,
            user: &user,
            api_token_id,
        };
        emit_notification_audit(
            &audit_ctx,
            uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_UPDATE,
            "notification_channel",
            channel_id.to_string(),
            None,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            serde_json::json!({ "reason_code": "invalid_request" }),
        );
        return Ok(error_response(StatusCode::BAD_REQUEST, e.to_string()));
    }

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
            tracing::error!("Failed to begin transaction for notification channel update: {e}");
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    let pair = match notif_queries::update_channel_in_tx(
        &tx,
        tenant_id,
        channel_id,
        &body,
        &*state.plugin.plugin_ops,
    )
    .await
    {
        Ok(p) => p,
        Err(err) => {
            drop(tx);
            let (outcome, reason_code) = err.current_context().audit_classification();
            let audit_ctx = AuditContext {
                audit_emitter: &state.audit_emitter,
                tenant_id,
                user: &user,
                api_token_id,
            };
            emit_notification_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_UPDATE,
                "notification_channel",
                channel_id.to_string(),
                None,
                outcome,
                serde_json::json!({ "reason_code": reason_code }),
            );
            return Err(err.into());
        }
    };

    let Some((before_model, after_model)) = pair else {
        drop(tx);
        let audit_ctx = AuditContext {
            audit_emitter: &state.audit_emitter,
            tenant_id,
            user: &user,
            api_token_id,
        };
        emit_notification_audit(
            &audit_ctx,
            uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_UPDATE,
            "notification_channel",
            channel_id.to_string(),
            None,
            uptrakit_audit_log::AuditOutcome::Denied,
            serde_json::json!({ "reason_code": "channel_not_found" }),
        );
        return Ok(error_response(StatusCode::NOT_FOUND, "Channel not found"));
    };

    let before_view = notif_queries::NotificationChannelView::from(&before_model);
    let after_view = notif_queries::NotificationChannelView::from(&after_model);

    let hook = state.audit_emitter.commit_hook();
    let audit_entry =
        match AuditEntry::<Stateful>::notification_channel_update(&before_view, &after_view)
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Success)
            .details(serde_json::json!({
                "channel_type": after_model.channel_type,
                "enabled": after_model.enabled,
                "name_changed": body.name.is_some(),
                "config_changed": body.config.is_some(),
                "enabled_changed": body.enabled.is_some(),
            }))
            .build()
        {
            Ok(entry) => entry,
            Err(e) => {
                tracing::error!("Failed to build audit entry for notification channel update: {e}");
                drop(tx);
                return Ok(error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error",
                ));
            }
        };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!("Failed to emit audit entry for notification channel update: {e}");
        drop(tx);
        return Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error",
        ));
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit notification channel update: {e}");
        return Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error",
        ));
    }
    hook.flush_after_commit().await;

    let masked_config = notif_queries::mask_channel_config(&after_model, &*state.plugin.plugin_ops);
    let resp = notif_queries::channel_to_response(after_model, masked_config);
    Ok((StatusCode::OK, Json(resp)).into_response())
}

/// Delete a notification channel
#[utoipa::path(
    delete,
    path = "/api/v1/notifications/channels/{id}",
    params(
        ("id" = Uuid, Path, description = "Channel UUID")
    ),
    responses(
        (status = 204, description = "Channel deleted"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Channel not found")
    ),
    tag = "Notifications",
    extensions(("x-required-permission" = json!("manage_notifications"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn delete_channel(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageNotifications(user): CanManageNotifications,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(channel_id): Path<Uuid>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
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
            tracing::error!("Failed to begin transaction for notification channel delete: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let before_model = match notif_queries::delete_channel_in_tx(&tx, tenant_id, channel_id).await {
        Ok(Some(m)) => m,
        Ok(None) => {
            drop(tx);
            let audit_ctx = AuditContext {
                audit_emitter: &state.audit_emitter,
                tenant_id,
                user: &user,
                api_token_id,
            };
            emit_notification_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_DELETE,
                "notification_channel",
                channel_id.to_string(),
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({ "reason_code": "channel_not_found" }),
            );
            return error_response(StatusCode::NOT_FOUND, "Channel not found");
        }
        Err(e) => {
            drop(tx);
            tracing::error!(error = ?e, "failed to delete notification channel");
            let audit_ctx = AuditContext {
                audit_emitter: &state.audit_emitter,
                tenant_id,
                user: &user,
                api_token_id,
            };
            emit_notification_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_DELETE,
                "notification_channel",
                channel_id.to_string(),
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({ "reason_code": "channel_delete_failed" }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let before_view = notif_queries::NotificationChannelView::from(&before_model);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::notification_channel_delete(
        &before_view,
        &AbsentView(&before_view),
    )
    .tenant_scope(tenant_id)
    .actor(actor_type, actor_id)
    .outcome(AuditOutcome::Success)
    .details(serde_json::json!({}))
    .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!("Failed to build audit entry for notification channel delete: {e}");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!("Failed to emit audit entry for notification channel delete: {e}");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit notification channel delete: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    StatusCode::NO_CONTENT.into_response()
}

/// Send a test notification through a channel
#[utoipa::path(
    post,
    path = "/api/v1/notifications/channels/{id}/test",
    params(
        ("id" = Uuid, Path, description = "Channel UUID")
    ),
    responses(
        (status = 200, description = "Test result", body = TestNotificationResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Channel not found")
    ),
    tag = "Notifications",
    extensions(("x-required-permission" = json!("manage_notifications"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn test_channel(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageNotifications(user): CanManageNotifications,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(channel_id): Path<Uuid>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &state.audit_emitter,
        tenant_id: tenant_db.tenant_id(),
        user: &user,
        api_token_id,
    };

    // Load channel from DB
    let channel_model = match tenant_db
        .find_by_id::<uptrakit_shared_db::entity::notification_channel::Entity, _>(channel_id)
        .one(tenant_db.db())
        .await
    {
        Ok(Some(ch)) => ch,
        Ok(None) => {
            emit_notification_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_TEST,
                "notification_channel",
                channel_id.to_string(),
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "channel_not_found",
                }),
            );
            return error_response(StatusCode::NOT_FOUND, "Channel not found");
        }
        Err(e) => {
            tracing::error!(error = ?e, "failed to load channel for test");
            emit_notification_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_TEST,
                "notification_channel",
                channel_id.to_string(),
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "channel_load_failed",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Parse encrypted config
    let config_json: serde_json::Value =
        match serde_json::from_str(channel_model.config.expose_secret()) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(error = ?e, "failed to parse channel config");
                emit_notification_audit(
                    &audit_ctx,
                    uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_TEST,
                    "notification_channel",
                    channel_model.id.to_string(),
                    Some(channel_model.name.clone()),
                    uptrakit_audit_log::AuditOutcome::Failed,
                    serde_json::json!({
                        "channel_type": channel_model.channel_type,
                        "reason_code": "channel_config_parse_failed",
                    }),
                );
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to parse channel config",
                );
            }
        };

    // Look up channel implementation
    let channel_type_id = uptrakit_shared_types::PluginTypeId::new(&channel_model.channel_type);
    let channel_transport = match state.plugin.plugin_ops.transport(&channel_type_id) {
        Some(c) => c,
        None => {
            emit_notification_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_TEST,
                "notification_channel",
                channel_model.id.to_string(),
                Some(channel_model.name.clone()),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "channel_type": channel_model.channel_type,
                    "reason_code": "unsupported_channel_type",
                }),
            );
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("Unsupported channel type: {}", channel_model.channel_type),
            );
        }
    };

    // Build settings bag from database
    let settings_bag = uptrakit_web_api_queries::notification_settings::build_settings_bag(
        state.db(),
        tenant_db.tenant_id(),
    )
    .await;

    // Build test message
    let test_msg = DeliveryMessage::new(
        "Test Notification",
        "This is a test notification from Uptrakit.",
        None,
        serde_json::json!({"test": true}),
        vec![],
    );

    // Deliver
    match channel_transport
        .deliver(&config_json, &settings_bag, &test_msg)
        .await
    {
        Ok(()) => {
            emit_notification_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_TEST,
                "notification_channel",
                channel_model.id.to_string(),
                Some(channel_model.name.clone()),
                uptrakit_audit_log::AuditOutcome::Success,
                serde_json::json!({
                    "channel_type": channel_model.channel_type,
                }),
            );
            let resp = TestNotificationResponse {
                success: true,
                message: "Test notification delivered successfully".to_string(),
            };
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            tracing::warn!(error = ?e, "test channel notification delivery failed");
            emit_notification_audit(
                &audit_ctx,
                uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_TEST,
                "notification_channel",
                channel_model.id.to_string(),
                Some(channel_model.name.clone()),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "channel_type": channel_model.channel_type,
                    "reason_code": "channel_delivery_failed",
                }),
            );
            error_response(StatusCode::UNPROCESSABLE_ENTITY, e.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// Rule endpoints
// ---------------------------------------------------------------------------

/// Create a notification rule
#[utoipa::path(
    post,
    path = "/api/v1/notifications/rules",
    request_body = CreateNotificationRuleRequest,
    responses(
        (status = 201, description = "Rule created", body = NotificationRuleResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Channel not found")
    ),
    tag = "Notifications",
    extensions(("x-required-permission" = json!("manage_notifications"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn create_rule(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageNotifications(user): CanManageNotifications,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Json(body): Json<CreateNotificationRuleRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    if let Err(e) = body.validate() {
        if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
            uptrakit_audit_log::AuditActionType::NOTIFICATION_RULE_CREATE,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .target("notification_rule", "pending".to_string(), None)
        .outcome(AuditOutcome::ValidationFailed)
        .details(serde_json::json!({ "reason_code": "invalid_request" }))
        .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

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
            tracing::error!("Failed to begin transaction for notification rule create: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let model = match notif_queries::create_rule_in_tx(&tx, tenant_id, &body).await {
        Ok(m) => m,
        Err(err) => {
            drop(tx);
            let (outcome, reason_code) = err.current_context().audit_classification();
            if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                uptrakit_audit_log::AuditActionType::NOTIFICATION_RULE_CREATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("notification_rule", "pending".to_string(), None)
            .outcome(outcome)
            .details(serde_json::json!({ "reason_code": reason_code }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            let ctx = err.current_context();
            let status = match ctx {
                notif_queries::RuleQueryError::ChannelNotFound => StatusCode::NOT_FOUND,
                notif_queries::RuleQueryError::InvalidField(_) => StatusCode::BAD_REQUEST,
                notif_queries::RuleQueryError::Db(_) => StatusCode::INTERNAL_SERVER_ERROR,
            };
            let msg = match status {
                StatusCode::NOT_FOUND => "Channel not found",
                StatusCode::BAD_REQUEST => "Invalid request",
                _ => "Internal server error",
            };
            return error_response(status, msg);
        }
    };

    let after_view = notif_queries::NotificationRuleView::from(&model);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::notification_rule_create(
        &AbsentView(&after_view),
        &after_view,
    )
    .tenant_scope(tenant_id)
    .actor(actor_type, actor_id)
    .outcome(AuditOutcome::Success)
    .details(serde_json::json!({}))
    .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!("Failed to build audit entry for notification rule create: {e}");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!("Failed to emit audit entry for notification rule create: {e}");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit notification rule create: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    let resp = notif_queries::rule_to_response(model);
    (StatusCode::CREATED, Json(resp)).into_response()
}

/// List notification rules
#[utoipa::path(
    get,
    path = "/api/v1/notifications/rules",
    params(
        ("channel_id" = Option<Uuid>, Query, description = "Filter by channel ID"),
        ("event_type" = Option<String>, Query, description = "Filter by event type"),
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)")
    ),
    responses(
        (status = 200, description = "Paginated list of rules", body = PaginatedResponse<NotificationRuleResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Notifications",
    extensions(("x-required-permission" = json!("view_notifications"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_rules(
    tenant_db: TenantDb,
    CanViewNotifications(_user): CanViewNotifications,
    Query(query): Query<ListRulesQuery>,
) -> Response {
    let params = PaginationParams {
        page: query.page,
        per_page: query.per_page,
    };

    match notif_queries::list_rules(
        &tenant_db,
        &params,
        query.channel_id,
        query.event_type.as_deref(),
    )
    .await
    {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!(error = ?e, "failed to list notification rules");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Get a notification rule by ID
#[utoipa::path(
    get,
    path = "/api/v1/notifications/rules/{id}",
    params(
        ("id" = Uuid, Path, description = "Rule UUID")
    ),
    responses(
        (status = 200, description = "Rule details", body = NotificationRuleResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Rule not found")
    ),
    tag = "Notifications",
    extensions(("x-required-permission" = json!("view_notifications"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_rule(
    tenant_db: TenantDb,
    CanViewNotifications(_user): CanViewNotifications,
    Path(rule_id): Path<Uuid>,
) -> Response {
    match notif_queries::get_rule(&tenant_db, rule_id).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Rule not found"),
        Err(e) => {
            tracing::error!(error = ?e, "failed to get notification rule");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Update a notification rule
#[utoipa::path(
    put,
    path = "/api/v1/notifications/rules/{id}",
    params(
        ("id" = Uuid, Path, description = "Rule UUID")
    ),
    request_body = UpdateNotificationRuleRequest,
    responses(
        (status = 200, description = "Rule updated", body = NotificationRuleResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Rule not found")
    ),
    tag = "Notifications",
    extensions(("x-required-permission" = json!("manage_notifications"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_rule(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageNotifications(user): CanManageNotifications,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(rule_id): Path<Uuid>,
    Json(body): Json<UpdateNotificationRuleRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    if let Err(e) = body.validate() {
        if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
            uptrakit_audit_log::AuditActionType::NOTIFICATION_RULE_UPDATE,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .target("notification_rule", rule_id.to_string(), None)
        .outcome(AuditOutcome::ValidationFailed)
        .details(serde_json::json!({ "reason_code": "invalid_request" }))
        .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

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
            tracing::error!("Failed to begin transaction for notification rule update: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let pair = match notif_queries::update_rule_in_tx(&tx, tenant_id, rule_id, &body).await {
        Ok(p) => p,
        Err(err) => {
            drop(tx);
            let (outcome, reason_code) = err.current_context().audit_classification();
            if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                uptrakit_audit_log::AuditActionType::NOTIFICATION_RULE_UPDATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("notification_rule", rule_id.to_string(), None)
            .outcome(outcome)
            .details(serde_json::json!({ "reason_code": reason_code }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            let ctx = err.current_context();
            let status = match ctx {
                notif_queries::RuleQueryError::InvalidField(_) => StatusCode::BAD_REQUEST,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            let msg = if status == StatusCode::BAD_REQUEST {
                "Invalid request"
            } else {
                "Internal server error"
            };
            return error_response(status, msg);
        }
    };

    let Some((before_model, after_model)) = pair else {
        drop(tx);
        if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
            uptrakit_audit_log::AuditActionType::NOTIFICATION_RULE_UPDATE,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .target("notification_rule", rule_id.to_string(), None)
        .outcome(AuditOutcome::Denied)
        .details(serde_json::json!({ "reason_code": "rule_not_found" }))
        .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        return error_response(StatusCode::NOT_FOUND, "Rule not found");
    };

    let before_view = notif_queries::NotificationRuleView::from(&before_model);
    let after_view = notif_queries::NotificationRuleView::from(&after_model);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry =
        match AuditEntry::<Stateful>::notification_rule_update(&before_view, &after_view)
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Success)
            .details(serde_json::json!({}))
            .build()
        {
            Ok(entry) => entry,
            Err(e) => {
                tracing::error!("Failed to build audit entry for notification rule update: {e}");
                drop(tx);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!("Failed to emit audit entry for notification rule update: {e}");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit notification rule update: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    let resp = notif_queries::rule_to_response(after_model);
    (StatusCode::OK, Json(resp)).into_response()
}

/// Delete a notification rule
#[utoipa::path(
    delete,
    path = "/api/v1/notifications/rules/{id}",
    params(
        ("id" = Uuid, Path, description = "Rule UUID")
    ),
    responses(
        (status = 204, description = "Rule deleted"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Rule not found")
    ),
    tag = "Notifications",
    extensions(("x-required-permission" = json!("manage_notifications"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn delete_rule(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageNotifications(user): CanManageNotifications,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(rule_id): Path<Uuid>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
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
            tracing::error!("Failed to begin transaction for notification rule delete: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let before_model = match notif_queries::delete_rule_in_tx(&tx, tenant_id, rule_id).await {
        Ok(Some(m)) => m,
        Ok(None) => {
            drop(tx);
            if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                uptrakit_audit_log::AuditActionType::NOTIFICATION_RULE_DELETE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("notification_rule", rule_id.to_string(), None)
            .outcome(AuditOutcome::Denied)
            .details(serde_json::json!({ "reason_code": "rule_not_found" }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::NOT_FOUND, "Rule not found");
        }
        Err(e) => {
            drop(tx);
            tracing::error!(error = ?e, "failed to delete notification rule");
            if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                uptrakit_audit_log::AuditActionType::NOTIFICATION_RULE_DELETE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("notification_rule", rule_id.to_string(), None)
            .outcome(AuditOutcome::Failed)
            .details(serde_json::json!({ "reason_code": "rule_delete_failed" }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let before_view = notif_queries::NotificationRuleView::from(&before_model);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::notification_rule_delete(
        &before_view,
        &AbsentView(&before_view),
    )
    .tenant_scope(tenant_id)
    .actor(actor_type, actor_id)
    .outcome(AuditOutcome::Success)
    .details(serde_json::json!({}))
    .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!("Failed to build audit entry for notification rule delete: {e}");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!("Failed to emit audit entry for notification rule delete: {e}");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit notification rule delete: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    StatusCode::NO_CONTENT.into_response()
}

// ---------------------------------------------------------------------------
// Log endpoint
// ---------------------------------------------------------------------------

/// List notification delivery log entries
#[utoipa::path(
    get,
    path = "/api/v1/notifications/log",
    params(
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)")
    ),
    responses(
        (status = 200, description = "Paginated list of log entries", body = PaginatedResponse<NotificationLogResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Notifications",
    extensions(("x-required-permission" = json!("view_notifications"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_log(
    tenant_db: TenantDb,
    CanViewNotifications(_user): CanViewNotifications,
    Query(params): Query<PaginationParams>,
) -> Response {
    match notif_queries::list_log(&tenant_db, &params).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!(error = ?e, "failed to list notification log");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

// ---------------------------------------------------------------------------
// Generic notification callback (public endpoint)
// ---------------------------------------------------------------------------

/// Generic notification callback for interactive notification actions.
///
/// This endpoint is called by external services (e.g., Telegram Bot API)
/// when a user interacts with a notification. Not authenticated via JWT;
/// verification is handled by each plugin's `handle_callback` action.
#[tracing::instrument(skip_all)]
pub async fn notification_callback(
    State(state): State<Arc<AppState>>,
    Path((channel_type, channel_id)): Path<(String, Uuid)>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    use sea_orm::EntityTrait;
    use uptrakit_shared_db::entity::notification_channel;

    // Load channel directly from DB (no TenantDb — this is a public endpoint)
    let channel_model = match notification_channel::Entity::find_by_id(channel_id)
        .one(state.db())
        .await
    {
        Ok(Some(ch)) => ch,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Channel not found"),
        Err(e) => {
            tracing::error!(error = ?e, "failed to load channel for notification callback");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Verify channel type matches the URL path
    if channel_model.channel_type != channel_type {
        return error_response(StatusCode::NOT_FOUND, "Channel not found");
    }

    // Parse channel config
    let config_json: serde_json::Value =
        match serde_json::from_str(channel_model.config.expose_secret()) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(error = ?e, "failed to parse channel config for callback");
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to parse channel config",
                );
            }
        };

    // Serialize headers into a JSON map
    let mut headers_map = serde_json::Map::new();
    for (name, value) in &headers {
        if let Ok(v) = value.to_str() {
            headers_map.insert(name.as_str().to_string(), serde_json::json!(v));
        }
    }

    // Parse body as JSON if possible, otherwise pass as string
    let body_value: serde_json::Value = serde_json::from_slice(&body)
        .unwrap_or_else(|_| serde_json::json!(String::from_utf8_lossy(&body).to_string()));

    let params = serde_json::json!({
        "channel_config": config_json,
        "headers": headers_map,
        "body": body_value,
    });

    // Delegate to the plugin's surface action handler.
    let surface_id = format!("notifications.{channel_type}");
    let controller =
        uptrakit_surface_proxy::AppStateSurfaceActionController::from_database_connection(
            state.db(),
            channel_model.tenant_id,
            None,
        );
    let ctx = SurfaceActionContext {
        controller: &controller,
    };

    match state
        .plugin
        .plugin_ops
        .handle_surface_action(&ctx, &surface_id, "handle_callback", params)
        .await
    {
        Ok(result) => {
            emit_notification_callback_audit(
                &state.audit_emitter,
                channel_model.tenant_id,
                channel_model.id,
                &channel_model.name,
                &channel_model.channel_type,
                uptrakit_audit_log::AuditOutcome::Success,
                None,
            );
            (StatusCode::OK, Json(result)).into_response()
        }
        Err(e) => {
            let (status, outcome, reason_code) = classify_notification_callback_error(&e);
            emit_notification_callback_audit(
                &state.audit_emitter,
                channel_model.tenant_id,
                channel_model.id,
                &channel_model.name,
                &channel_model.channel_type,
                outcome,
                Some(reason_code),
            );
            if status == StatusCode::INTERNAL_SERVER_ERROR {
                tracing::error!(error = %e, "notification callback failed");
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            } else {
                error_response(status, e.to_string())
            }
        }
    }
}

#[cfg(all(test, feature = "db-sqlite", feature = "notifications-telegram"))]
mod tests {
    #![expect(
        clippy::expect_used,
        reason = "test code: panics on failure are acceptable"
    )]
    #![expect(clippy::panic, reason = "test code: panics on failure are acceptable")]

    use super::*;

    use crate::test_harness::TestApp;
    use sea_orm::{
        ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
    };
    use uptrakit_shared_db::entity::{
        audit_log, notification_channel, notification_log, notification_rule,
    };

    async fn latest_notification_callback_audit_row(
        db: &sea_orm::DatabaseConnection,
    ) -> audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = audit_log::Entity::find()
                .filter(
                    audit_log::Column::ActionType
                        .eq(uptrakit_audit_log::AuditActionType::NOTIFICATION_CALLBACK),
                )
                .order_by_desc(audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query callback audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected notification callback audit row");
    }

    async fn insert_telegram_callback_fixture(
        app: &TestApp,
        webhook_secret: &str,
    ) -> (Uuid, Uuid, Uuid) {
        let now = time::OffsetDateTime::now_utc();
        let channel_id = Uuid::now_v7();
        let rule_id = Uuid::now_v7();
        let log_id = Uuid::now_v7();
        let action_token = Uuid::now_v7();
        let channel_config = serde_json::json!({
            "bot_token": "telegram-bot-token",
            "chat_id": "12345",
            "webhook_secret": webhook_secret,
        });

        notification_channel::ActiveModel {
            id: Set(channel_id),
            tenant_id: Set(app.tenant_id),
            name: Set("Telegram Callback Channel".to_string()),
            channel_type: Set("telegram".to_string()),
            config: Set(uptrakit_crypto::EncryptedString::new(
                channel_config.to_string(),
                "notification_channels.config",
            )
            .expect("encrypt test channel config")),
            enabled: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&app.db)
        .await
        .expect("insert callback channel");

        notification_rule::ActiveModel {
            id: Set(rule_id),
            tenant_id: Set(app.tenant_id),
            channel_id: Set(channel_id),
            event_type: Set("update_available".to_string()),
            host_id: Set(None),
            software_item_id: Set(None),
            plugin_type: Set(None),
            enabled: Set(true),
            created_at: Set(now),
        }
        .insert(&app.db)
        .await
        .expect("insert callback rule");

        notification_log::ActiveModel {
            id: Set(log_id),
            tenant_id: Set(app.tenant_id),
            channel_id: Set(channel_id),
            rule_id: Set(rule_id),
            event_type: Set("update_available".to_string()),
            event_payload: Set(serde_json::json!({ "software_item": "nginx" })),
            status: Set("delivered".to_string()),
            error_message: Set(None),
            action_token: Set(Some(action_token)),
            action_taken: Set(None),
            created_at: Set(now),
            delivered_at: Set(Some(now)),
        }
        .insert(&app.db)
        .await
        .expect("insert callback log");

        (channel_id, log_id, action_token)
    }

    fn callback_body(action_token: Option<&str>) -> serde_json::Value {
        match action_token {
            Some(action_token) => serde_json::json!({
                "callback_query": {
                    "data": action_token,
                }
            }),
            None => serde_json::json!({ "update_id": 1 }),
        }
    }

    #[tokio::test]
    async fn notification_callback_success_writes_audit_and_updates_log() {
        let app = TestApp::new().await;
        let client = app.client();
        let webhook_secret = "expected-secret";
        let (channel_id, log_id, action_token) =
            insert_telegram_callback_fixture(&app, webhook_secret).await;

        let status = client
            .post_json(
                &format!("/api/v1/notifications/callback/telegram/{channel_id}"),
                &callback_body(Some(action_token.to_string().as_str())),
            )
            .header("x-telegram-bot-api-secret-token", webhook_secret)
            .send_status()
            .await;

        assert_eq!(status, StatusCode::OK);

        let row = latest_notification_callback_audit_row(&app.db).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::System.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("notification_channel"));
        assert_eq!(
            row.target_id.as_deref(),
            Some(channel_id.to_string().as_str())
        );
        let details = row.details_json.expect("callback audit details");
        assert_eq!(details["channel_type"], serde_json::json!("telegram"));
        assert!(details.get("reason_code").is_none());
        let serialized_details =
            serde_json::to_string(&details).expect("serialize callback details");
        assert!(!serialized_details.contains(webhook_secret));
        assert!(!serialized_details.contains(action_token.to_string().as_str()));

        let log_row = notification_log::Entity::find_by_id(log_id)
            .one(&app.db)
            .await
            .expect("query callback log")
            .expect("callback log should exist");
        assert_eq!(log_row.action_taken.as_deref(), Some("triggered"));
    }

    #[tokio::test]
    async fn notification_callback_invalid_secret_writes_denied_audit() {
        let app = TestApp::new().await;
        let client = app.client();
        let webhook_secret = "expected-secret";
        let wrong_secret = "wrong-secret";
        let (channel_id, _log_id, action_token) =
            insert_telegram_callback_fixture(&app, webhook_secret).await;

        let status = client
            .post_json(
                &format!("/api/v1/notifications/callback/telegram/{channel_id}"),
                &callback_body(Some(action_token.to_string().as_str())),
            )
            .header("x-telegram-bot-api-secret-token", wrong_secret)
            .send_status()
            .await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let row = latest_notification_callback_audit_row(&app.db).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("callback audit details");
        assert_eq!(details["reason_code"], serde_json::json!("invalid_secret"));
        let serialized_details =
            serde_json::to_string(&details).expect("serialize callback details");
        assert!(!serialized_details.contains(webhook_secret));
        assert!(!serialized_details.contains(wrong_secret));
    }

    #[tokio::test]
    async fn notification_callback_missing_token_writes_validation_failed_audit() {
        let app = TestApp::new().await;
        let client = app.client();
        let webhook_secret = "expected-secret";
        let (channel_id, _log_id, _action_token) =
            insert_telegram_callback_fixture(&app, webhook_secret).await;

        let status = client
            .post_json(
                &format!("/api/v1/notifications/callback/telegram/{channel_id}"),
                &callback_body(None),
            )
            .header("x-telegram-bot-api-secret-token", webhook_secret)
            .send_status()
            .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);

        let row = latest_notification_callback_audit_row(&app.db).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let details = row.details_json.expect("callback audit details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("missing_action_token")
        );
    }

    #[tokio::test]
    async fn notification_callback_bad_token_writes_validation_failed_audit() {
        let app = TestApp::new().await;
        let client = app.client();
        let webhook_secret = "expected-secret";
        let (channel_id, _log_id, _action_token) =
            insert_telegram_callback_fixture(&app, webhook_secret).await;

        let status = client
            .post_json(
                &format!("/api/v1/notifications/callback/telegram/{channel_id}"),
                &callback_body(Some("not-a-uuid")),
            )
            .header("x-telegram-bot-api-secret-token", webhook_secret)
            .send_status()
            .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);

        let row = latest_notification_callback_audit_row(&app.db).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let details = row.details_json.expect("callback audit details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("invalid_action_token")
        );
    }

    #[tokio::test]
    async fn notification_callback_db_failure_writes_failed_audit() {
        let app = TestApp::new().await;
        let client = app.client();
        let webhook_secret = "expected-secret";
        let (channel_id, _log_id, action_token) =
            insert_telegram_callback_fixture(&app, webhook_secret).await;

        app.db
            .execute_unprepared("DROP TABLE notification_log")
            .await
            .expect("drop notification_log table");

        let status = client
            .post_json(
                &format!("/api/v1/notifications/callback/telegram/{channel_id}"),
                &callback_body(Some(action_token.to_string().as_str())),
            )
            .header("x-telegram-bot-api-secret-token", webhook_secret)
            .send_status()
            .await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

        let row = latest_notification_callback_audit_row(&app.db).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = row.details_json.expect("callback audit details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("notification_log_lookup_failed")
        );
    }
}

#[cfg(all(test, feature = "db-sqlite"))]
mod snapshot_tests {
    #![expect(
        clippy::expect_used,
        reason = "test code: panics on failure are acceptable"
    )]
    #![expect(clippy::panic, reason = "test code: panics on failure are acceptable")]

    use crate::test_harness::TestApp;
    use crate::test_harness::fixtures;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
    use uptrakit_shared_db::entity::audit_log;

    async fn latest_notification_channel_audit_row(
        db: &sea_orm::DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
    ) -> audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = audit_log::Entity::find()
                .filter(audit_log::Column::ActionType.eq(action_type))
                .order_by_desc(audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query notification channel audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("expected notification channel audit row for {action_type}");
    }

    #[tokio::test]
    async fn create_channel_config_absent_from_audit_snapshots() {
        let app = TestApp::new().await;
        let client = app.client();
        let token = fixtures::register_and_get_token(&client).await;
        let secret_signing_key = "super-secret-webhook-signing-key";

        let (status, _): (http::StatusCode, serde_json::Value) = client
            .post_json(
                "/api/v1/notifications/channels",
                &serde_json::json!({
                    "name": "Snapshot Test Channel",
                    "channel_type": "webhook",
                    "config": {
                        "url": "https://hooks.example.com/notify",
                        "secret": secret_signing_key
                    },
                    "enabled": true
                }),
            )
            .bearer(&token)
            .send_json()
            .await;

        assert_eq!(status, http::StatusCode::CREATED);

        let row = latest_notification_channel_audit_row(
            &app.db,
            uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_CREATE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );

        let before = row.before_snapshot.expect("before_snapshot");
        let after = row.after_snapshot.expect("after_snapshot");

        assert!(
            before.get("config").is_none(),
            "config key must not appear in before_snapshot"
        );
        assert!(
            after.get("config").is_none(),
            "config key must not appear in after_snapshot"
        );
        assert!(
            !before.to_string().contains(secret_signing_key),
            "webhook secret must not appear in before_snapshot JSON"
        );
        assert!(
            !after.to_string().contains(secret_signing_key),
            "webhook secret must not appear in after_snapshot JSON"
        );
    }
}
