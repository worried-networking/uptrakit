use crate::AppState;
use crate::error_response::error_response;
use crate::middleware::permission::{CanManageNotifications, CanViewNotifications};
use crate::queries::notifications::{self as notif_queries, ChannelQueryError, RuleQueryError};
use crate::tenant_db::TenantDb;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use std::sync::Arc;
use uptrakit_notification_channels::DeliveryMessage;
use uptrakit_web_api_types::pagination::PaginationParams;
use uptrakit_web_api_types::validation::Validate;
use uuid::Uuid;

pub use uptrakit_web_api_types::notifications::{
    CreateNotificationChannelRequest, CreateNotificationRuleRequest, NotificationChannelResponse,
    NotificationChannelType, NotificationDeliveryStatus, NotificationEventType,
    NotificationLogResponse, NotificationRuleResponse, TestNotificationResponse,
    UpdateNotificationChannelRequest, UpdateNotificationRuleRequest,
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
) -> Response {
    if let Err(e) = body.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    match notif_queries::create_channel(&tenant_db, &body, &state.channel_registry).await {
        Ok(resp) => (StatusCode::CREATED, Json(resp)).into_response(),
        Err(report) => match report.current_context() {
            ChannelQueryError::UnsupportedType(t) => error_response(
                StatusCode::BAD_REQUEST,
                format!("Unsupported channel type: {t}"),
            ),
            ChannelQueryError::InvalidConfig(msg) => {
                error_response(StatusCode::BAD_REQUEST, format!("Invalid config: {msg}"))
            }
            ChannelQueryError::Db(_) => {
                tracing::error!(error = ?report, "failed to create notification channel");
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            }
        },
    }
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
    match notif_queries::list_channels(&tenant_db, &params, &state.channel_registry).await {
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
    match notif_queries::get_channel(&tenant_db, channel_id, &state.channel_registry).await {
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
) -> Response {
    if let Err(e) = body.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    match notif_queries::update_channel(&tenant_db, channel_id, &body, &state.channel_registry)
        .await
    {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Channel not found"),
        Err(report) => match report.current_context() {
            ChannelQueryError::UnsupportedType(t) => error_response(
                StatusCode::BAD_REQUEST,
                format!("Unsupported channel type: {t}"),
            ),
            ChannelQueryError::InvalidConfig(msg) => {
                error_response(StatusCode::BAD_REQUEST, format!("Invalid config: {msg}"))
            }
            ChannelQueryError::Db(_) => {
                tracing::error!(error = ?report, "failed to update notification channel");
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            }
        },
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
    let channel_impl = match state.channel_registry.get(&channel_model.channel_type) {
        Some(c) => c,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("Unsupported channel type: {}", channel_model.channel_type),
            );
        }
    };

    // For email channels, merge global SMTP settings into the per-channel config before delivery.
    let config_json = if channel_model.channel_type == "email" {
        let smtp = state.settings.smtp();
        if !smtp.is_configured() {
            return error_response(
                StatusCode::BAD_REQUEST,
                "SMTP is not configured. Set SMTP settings before testing an email channel.",
            );
        }
        crate::notifications::dispatcher::merge_smtp_into_config_pub(&smtp, config_json)
    } else {
        config_json
    };

    // Build test message
    let test_msg = DeliveryMessage {
        title: "Test Notification".to_string(),
        body: "This is a test notification from Uptrakit.".to_string(),
        body_html: None,
        event_payload: serde_json::json!({"test": true}),
        actions: vec![],
    };

    // Deliver
    match channel_impl.deliver(&config_json, &test_msg).await {
        Ok(()) => {
            let resp = TestNotificationResponse {
                success: true,
                message: "Test notification delivered successfully".to_string(),
            };
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            let resp = TestNotificationResponse {
                success: false,
                message: e.to_string(),
            };
            (StatusCode::OK, Json(resp)).into_response()
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
) -> Response {
    if let Err(e) = body.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    match notif_queries::create_rule(&tenant_db, &body).await {
        Ok(resp) => (StatusCode::CREATED, Json(resp)).into_response(),
        Err(report) => match report.current_context() {
            RuleQueryError::ChannelNotFound => {
                error_response(StatusCode::NOT_FOUND, "Channel not found")
            }
            RuleQueryError::Db(_) => {
                tracing::error!(error = ?report, "failed to create notification rule");
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            }
            RuleQueryError::InvalidField(field) => error_response(
                StatusCode::BAD_REQUEST,
                format!("Invalid value for field '{field}'"),
            ),
        },
    }
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
// Telegram callback (public endpoint)
// ---------------------------------------------------------------------------

/// Telegram bot callback for interactive notification actions.
///
/// This endpoint is called by Telegram's Bot API when a user presses an
/// inline keyboard button. It is not authenticated via JWT but verified
/// via the `X-Telegram-Bot-Api-Secret-Token` header against the channel's
/// `webhook_secret` config field.
#[tracing::instrument(skip_all)]
pub async fn telegram_callback(
    State(state): State<Arc<AppState>>,
    Path(channel_id): Path<Uuid>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait};
    use uptrakit_shared_db::entity::{notification_channel, notification_log};

    // Load channel directly from DB (no TenantDb — this is a public endpoint)
    let channel_model = match notification_channel::Entity::find_by_id(channel_id)
        .one(state.db())
        .await
    {
        Ok(Some(ch)) => ch,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Channel not found"),
        Err(e) => {
            tracing::error!(error = ?e, "failed to load channel for telegram callback");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Parse channel config
    let config_json: serde_json::Value =
        match serde_json::from_str(channel_model.config.expose_secret()) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(error = ?e, "failed to parse channel config for telegram callback");
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to parse channel config",
                );
            }
        };

    // Verify secret token using constant-time comparison to prevent timing attacks.
    // Both secrets are hashed to SHA-256 so ct_eq always compares equal-length arrays,
    // eliminating any length-based information leak.
    use sha2::{Digest, Sha256};
    use subtle::ConstantTimeEq;

    let expected_secret = config_json
        .get("webhook_secret")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");

    let provided_secret = headers
        .get("x-telegram-bot-api-secret-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let expected_hash: [u8; 32] = Sha256::digest(expected_secret.as_bytes()).into();
    let provided_hash: [u8; 32] = Sha256::digest(provided_secret.as_bytes()).into();
    let secrets_match: bool = expected_hash.ct_eq(&provided_hash).into();

    if expected_secret.is_empty() || !secrets_match {
        return error_response(StatusCode::UNAUTHORIZED, "Invalid secret token");
    }

    // Parse Telegram Update body
    let update: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = ?e, "failed to parse Telegram update body");
            return error_response(StatusCode::BAD_REQUEST, "Invalid request body");
        }
    };

    // Extract callback_query.data (action token UUID)
    let action_token_str = match update
        .get("callback_query")
        .and_then(|cq| cq.get("data"))
        .and_then(serde_json::Value::as_str)
    {
        Some(s) => s,
        None => {
            // Not a callback query we care about — acknowledge silently
            return (StatusCode::OK, Json(serde_json::json!({}))).into_response();
        }
    };

    let action_token: Uuid = match action_token_str.parse() {
        Ok(id) => id,
        Err(_) => {
            tracing::warn!(action_token = %action_token_str, "invalid action token UUID in Telegram callback");
            return (StatusCode::OK, Json(serde_json::json!({}))).into_response();
        }
    };

    // Look up notification log by action token
    let log_entry = match notif_queries::find_log_by_action_token(state.db(), action_token).await {
        Ok(Some(entry)) => entry,
        Ok(None) => {
            tracing::warn!(%action_token, "no notification log found for action token");
            return (StatusCode::OK, Json(serde_json::json!({}))).into_response();
        }
        Err(e) => {
            tracing::error!(error = ?e, %action_token, "failed to look up action token");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // If already actioned, return 200 with empty JSON
    if log_entry.action_taken.is_some() {
        return (StatusCode::OK, Json(serde_json::json!({}))).into_response();
    }

    // Update action_taken
    let mut active: notification_log::ActiveModel = log_entry.into();
    active.action_taken = Set(Some("triggered".to_string()));

    if let Err(e) = active.update(state.db()).await {
        tracing::error!(error = ?e, "failed to update notification log action_taken");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    (StatusCode::OK, Json(serde_json::json!({}))).into_response()
}
