//! Generic notification extension data action handler + SMTP settings handler.
//!
//! This module handles extension actions dispatched by the
//! [`ExtensionOwner::Notification`] variant. It extracts the channel type
//! from the extension ID (`notifications.<type>` → `<type>`) and performs
//! generic operations without transport-specific knowledge.

use std::sync::Arc;

use axum::{Json, http::StatusCode, response::IntoResponse, response::Response};
use sea_orm::{ColumnTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};

use uptrakit_shared_db::entity::notification_channel;

use crate::AppState;
use crate::error_response::{error_response, error_response_with_code};
use crate::middleware::tenant_context::TenantContext;

/// Dispatch a notification extension action.
pub async fn handle(
    state: &Arc<AppState>,
    tenant_ctx: &TenantContext,
    extension_id: &str,
    action_id: &str,
    params: serde_json::Value,
) -> Response {
    // Extract channel_type: "notifications.webhook" → "webhook"
    let channel_type = extension_id
        .strip_prefix("notifications.")
        .unwrap_or(extension_id);

    match action_id {
        "list" => list_channels(state, tenant_ctx, channel_type, &params).await,
        "get_smtp" => get_smtp_settings(state).await,
        "save_smtp" => save_smtp_settings(state, &params).await,
        _ => error_response_with_code(StatusCode::NOT_FOUND, "Unknown action", "not_found"),
    }
}

/// List notification channels of a given type with flattened config fields.
async fn list_channels(
    state: &Arc<AppState>,
    tenant_ctx: &TenantContext,
    channel_type: &str,
    params: &serde_json::Value,
) -> Response {
    let page = params
        .get("page")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .max(1);
    let per_page = params
        .get("per_page")
        .and_then(|v| v.as_u64())
        .unwrap_or(50)
        .clamp(1, 100);

    let tenant_db =
        uptrakit_web_api_queries::TenantDb::new(state.db().clone(), tenant_ctx.tenant_id);
    let offset = (page - 1) * per_page;

    // Count total channels of this type for this tenant.
    let total = match tenant_db
        .find::<notification_channel::Entity>()
        .filter(notification_channel::Column::ChannelType.eq(channel_type))
        .count(tenant_db.db())
        .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to count notification channels: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let total_pages = if total == 0 {
        1
    } else {
        total.div_ceil(per_page)
    };

    // Fetch channels of this type.
    let channels = match tenant_db
        .find::<notification_channel::Entity>()
        .filter(notification_channel::Column::ChannelType.eq(channel_type))
        .order_by_desc(notification_channel::Column::CreatedAt)
        .offset(Some(offset))
        .limit(Some(per_page))
        .all(tenant_db.db())
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to list notification channels: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Build rows with flattened config.
    let mut items = Vec::with_capacity(channels.len());
    for ch in channels {
        let config: serde_json::Value =
            serde_json::from_str(ch.config.expose_secret()).unwrap_or_default();
        let masked_config = state
            .notification_ops
            .mask_config_secrets(&ch.channel_type, &config);

        let mut row = serde_json::json!({
            "id": ch.id,
            "name": ch.name,
            "enabled": ch.enabled,
            "created_at": ch.created_at,
        });

        // Generic flatten: merge all top-level config keys into the row.
        if let (Some(obj), Some(row_obj)) = (masked_config.as_object(), row.as_object_mut()) {
            for (key, val) in obj {
                row_obj.insert(key.clone(), val.clone());
            }
        }

        items.push(row);
    }

    let response = serde_json::json!({
        "items": items,
        "total": total,
        "page": page,
        "per_page": per_page,
        "total_pages": total_pages,
    });

    (StatusCode::OK, Json(response)).into_response()
}

/// Get SMTP settings as a flat JSON object for extension pre-load.
async fn get_smtp_settings(state: &Arc<AppState>) -> Response {
    let smtp = state.settings.smtp();
    let response = serde_json::json!({
        "host": smtp.host.as_deref().unwrap_or(""),
        "port": smtp.port.unwrap_or(587),
        "username": smtp.username.as_deref().unwrap_or(""),
        "has_password": smtp.password.is_some(),
        "from_address": smtp.from_address.as_deref().unwrap_or(""),
        "from_name": smtp.from_name.as_deref().unwrap_or(""),
        "tls_mode": smtp.tls_mode,
    });
    (StatusCode::OK, Json(response)).into_response()
}

/// Save SMTP settings with patch semantics (absent keys = keep existing).
async fn save_smtp_settings(state: &Arc<AppState>, params: &serde_json::Value) -> Response {
    use crate::SettingKey;
    use crate::settings_store::upsert_setting;

    let mut smtp = state.settings.smtp();

    if let Some(host) = params.get("host").and_then(|v| v.as_str()) {
        if let Err(e) = upsert_setting(
            state.db(),
            state.default_tenant_id,
            SettingKey::SmtpHost,
            serde_json::json!(host),
        )
        .await
        {
            tracing::error!("Failed to save smtp.host: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        smtp.host = Some(host.to_string()).filter(|h| !h.is_empty());
    }

    if let Some(port) = params.get("port").and_then(|v| {
        v.as_u64()
            .map(|n| n as u16)
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    }) {
        if let Err(e) = upsert_setting(
            state.db(),
            state.default_tenant_id,
            SettingKey::SmtpPort,
            serde_json::json!(port),
        )
        .await
        {
            tracing::error!("Failed to save smtp.port: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        smtp.port = Some(port);
    }

    if let Some(username) = params.get("username").and_then(|v| v.as_str()) {
        let new_username = if username.is_empty() {
            None
        } else {
            Some(username.to_string())
        };
        if let Err(e) = upsert_setting(
            state.db(),
            state.default_tenant_id,
            SettingKey::SmtpUsername,
            serde_json::json!(new_username.as_deref().unwrap_or("")),
        )
        .await
        {
            tracing::error!("Failed to save smtp.username: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        smtp.username = new_username;
    }

    if let Some(password) = params
        .get("password")
        .and_then(|v| v.as_str())
        .filter(|p| !p.is_empty())
    {
        let stored_value =
            match uptrakit_crypto::encrypt_str(password, "uptrakit:settings:smtp_password") {
                Ok(encrypted) => serde_json::json!(encrypted),
                Err(e) => {
                    tracing::error!("Failed to encrypt SMTP password: {e:?}");
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            };
        if let Err(e) = upsert_setting(
            state.db(),
            state.default_tenant_id,
            SettingKey::SmtpPassword,
            stored_value,
        )
        .await
        {
            tracing::error!("Failed to save smtp.password: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        smtp.password = Some(password.to_string());
    }

    if let Some(from_address) = params.get("from_address").and_then(|v| v.as_str()) {
        if let Err(e) = upsert_setting(
            state.db(),
            state.default_tenant_id,
            SettingKey::SmtpFromAddress,
            serde_json::json!(from_address),
        )
        .await
        {
            tracing::error!("Failed to save smtp.from_address: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        smtp.from_address = Some(from_address.to_string()).filter(|a| !a.is_empty());
    }

    if let Some(from_name) = params.get("from_name").and_then(|v| v.as_str()) {
        if let Err(e) = upsert_setting(
            state.db(),
            state.default_tenant_id,
            SettingKey::SmtpFromName,
            serde_json::json!(from_name),
        )
        .await
        {
            tracing::error!("Failed to save smtp.from_name: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        smtp.from_name = Some(from_name.to_string()).filter(|n| !n.is_empty());
    }

    if let Some(tls_mode) = params.get("tls_mode").and_then(|v| v.as_str()) {
        if let Err(e) = upsert_setting(
            state.db(),
            state.default_tenant_id,
            SettingKey::SmtpTlsMode,
            serde_json::json!(tls_mode),
        )
        .await
        {
            tracing::error!("Failed to save smtp.tls_mode: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        smtp.tls_mode = tls_mode.to_string();
    }

    state.settings.set_smtp(smtp.clone()).await;

    let response = serde_json::json!({
        "host": smtp.host.as_deref().unwrap_or(""),
        "port": smtp.port.unwrap_or(587),
        "username": smtp.username.as_deref().unwrap_or(""),
        "has_password": smtp.password.is_some(),
        "from_address": smtp.from_address.as_deref().unwrap_or(""),
        "from_name": smtp.from_name.as_deref().unwrap_or(""),
        "tls_mode": smtp.tls_mode,
    });

    (StatusCode::OK, Json(response)).into_response()
}
