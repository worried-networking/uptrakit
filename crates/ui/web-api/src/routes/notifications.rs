use crate::AppState;
use crate::api_error::ApiError;
use crate::error_response::error_response;
use crate::middleware::permission::{CanManageNotifications, CanViewNotifications};
use crate::queries::notifications as notif_queries;
use crate::tenant_db::TenantDb;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use std::sync::Arc;
use uptrakit_plugin_infrastructure_registry::DeliveryMessage;
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
    CanManageNotifications(_user): CanManageNotifications,
    Json(body): Json<CreateNotificationChannelRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if let Err(e) = body.validate() {
        return Ok(error_response(StatusCode::BAD_REQUEST, e.to_string()));
    }

    let resp = notif_queries::create_channel(&tenant_db, &body, &*state.plugin_ops).await?;
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
    match notif_queries::list_channels(&tenant_db, &params, &*state.plugin_ops).await {
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
    match notif_queries::get_channel(&tenant_db, channel_id, &*state.plugin_ops).await {
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
    CanManageNotifications(_user): CanManageNotifications,
    Path(channel_id): Path<Uuid>,
    Json(body): Json<UpdateNotificationChannelRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if let Err(e) = body.validate() {
        return Ok(error_response(StatusCode::BAD_REQUEST, e.to_string()));
    }

    match notif_queries::update_channel(&tenant_db, channel_id, &body, &*state.plugin_ops).await? {
        Some(resp) => Ok((StatusCode::OK, Json(resp)).into_response()),
        None => Ok(error_response(StatusCode::NOT_FOUND, "Channel not found")),
    }
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
    tenant_db: TenantDb,
    CanManageNotifications(_user): CanManageNotifications,
    Path(channel_id): Path<Uuid>,
) -> Response {
    match notif_queries::delete_channel(&tenant_db, channel_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "Channel not found"),
        Err(e) => {
            tracing::error!(error = ?e, "failed to delete notification channel");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
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
    CanManageNotifications(_user): CanManageNotifications,
    Path(channel_id): Path<Uuid>,
) -> Response {
    // Load channel from DB
    let channel_model = match tenant_db
        .find_by_id::<uptrakit_shared_db::entity::notification_channel::Entity, _>(channel_id)
        .one(tenant_db.db())
        .await
    {
        Ok(Some(ch)) => ch,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Channel not found"),
        Err(e) => {
            tracing::error!(error = ?e, "failed to load channel for test");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Parse encrypted config
    let config_json: serde_json::Value =
        match serde_json::from_str(channel_model.config.expose_secret()) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(error = ?e, "failed to parse channel config");
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to parse channel config",
                );
            }
        };

    // Look up channel implementation
    let channel_type_id = uptrakit_shared_types::PluginTypeId::new(&channel_model.channel_type);
    let channel_transport = match state.plugin_ops.transport(&channel_type_id) {
        Some(c) => c,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("Unsupported channel type: {}", channel_model.channel_type),
            );
        }
    };

    // Build settings bag from database
    let settings_bag =
        crate::notifications::dispatcher::build_settings_bag(state.db(), tenant_db.tenant_id).await;

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
            let resp = TestNotificationResponse {
                success: true,
                message: "Test notification delivered successfully".to_string(),
            };
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            tracing::warn!(error = ?e, "test channel notification delivery failed");
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
    tenant_db: TenantDb,
    CanManageNotifications(_user): CanManageNotifications,
    Json(body): Json<CreateNotificationRuleRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if let Err(e) = body.validate() {
        return Ok(error_response(StatusCode::BAD_REQUEST, e.to_string()));
    }

    let resp = notif_queries::create_rule(&tenant_db, &body).await?;
    Ok((StatusCode::CREATED, Json(resp)).into_response())
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
    tenant_db: TenantDb,
    CanManageNotifications(_user): CanManageNotifications,
    Path(rule_id): Path<Uuid>,
    Json(body): Json<UpdateNotificationRuleRequest>,
) -> Response {
    if let Err(e) = body.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    match notif_queries::update_rule(&tenant_db, rule_id, &body).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Rule not found"),
        Err(e) => {
            tracing::error!(error = ?e, "failed to update notification rule");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
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
    tenant_db: TenantDb,
    CanManageNotifications(_user): CanManageNotifications,
    Path(rule_id): Path<Uuid>,
) -> Response {
    match notif_queries::delete_rule(&tenant_db, rule_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "Rule not found"),
        Err(e) => {
            tracing::error!(error = ?e, "failed to delete notification rule");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
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
    let ctx = uptrakit_plugin_infrastructure_registry::SurfaceActionContext {
        db: state.db(),
        tenant_id: Some(channel_model.tenant_id),
        caller_user_id: None,
    };

    match state
        .plugin_ops
        .handle_surface_action(&ctx, &surface_id, "handle_callback", params)
        .await
    {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => {
            if e.contains("Unauthorized") || e.contains("Invalid secret") {
                error_response(StatusCode::UNAUTHORIZED, e)
            } else if e.contains("Bad request") || e.contains("Invalid request") {
                error_response(StatusCode::BAD_REQUEST, e)
            } else {
                tracing::error!(error = %e, "notification callback failed");
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            }
        }
    }
}
