use crate::AppState;
use crate::SettingKey;
use crate::auth::permissions::Permission;
use crate::middleware::require_auth::AuthenticatedUser;
use crate::settings_store::upsert_setting;
use axum::{
    Extension, Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct MqttSettingsResponse {
    pub host: Option<String>,
    pub port: u16,
    pub client_id: String,
    pub username: Option<String>,
    pub has_password: bool,
    pub topic_prefix: String,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateMqttSettingsRequest {
    pub host: Option<serde_json::Value>,
    pub port: Option<u16>,
    pub client_id: Option<String>,
    pub username: Option<serde_json::Value>,
    pub password: Option<String>,
    pub topic_prefix: Option<String>,
}

/// Get MQTT settings
#[utoipa::path(
    get,
    path = "/api/v1/settings/mqtt",
    responses(
        (status = 200, description = "MQTT settings", body = MqttSettingsResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    security(("bearer_token" = []))
)]
pub async fn get_mqtt_settings(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
) -> Response {
    if !user.has_permission(Permission::ViewSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let mqtt = state.settings.mqtt().await;
    let response = MqttSettingsResponse {
        host: mqtt.host,
        port: mqtt.port,
        client_id: mqtt.client_id,
        username: mqtt.username,
        has_password: mqtt.password.is_some(),
        topic_prefix: mqtt.topic_prefix,
    };
    (StatusCode::OK, Json(response)).into_response()
}

/// Update MQTT settings
#[utoipa::path(
    put,
    path = "/api/v1/settings/mqtt",
    request_body = UpdateMqttSettingsRequest,
    responses(
        (status = 200, description = "Settings updated", body = MqttSettingsResponse),
        (status = 400, description = "Invalid values"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    security(("bearer_token" = []))
)]
pub async fn update_mqtt_settings(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    Json(req): Json<UpdateMqttSettingsRequest>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let mut mqtt = state.settings.mqtt().await;

    // host: JSON value can be string or null
    if let Some(ref host_val) = req.host {
        if host_val.is_null() {
            mqtt.host = None;
            if let Err(e) =
                upsert_setting(&state.db, SettingKey::MqttHost, serde_json::Value::Null).await
            {
                tracing::error!("Failed to save mqtt.host: {e:?}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        } else if let Some(s) = host_val.as_str() {
            if s.is_empty() {
                mqtt.host = None;
                if let Err(e) =
                    upsert_setting(&state.db, SettingKey::MqttHost, serde_json::Value::Null).await
                {
                    tracing::error!("Failed to save mqtt.host: {e:?}");
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            } else {
                mqtt.host = Some(s.to_string());
                if let Err(e) =
                    upsert_setting(&state.db, SettingKey::MqttHost, serde_json::json!(s)).await
                {
                    tracing::error!("Failed to save mqtt.host: {e:?}");
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            }
        } else {
            return (StatusCode::BAD_REQUEST, "host must be a string or null").into_response();
        }
    }

    if let Some(port) = req.port {
        mqtt.port = port;
        if let Err(e) =
            upsert_setting(&state.db, SettingKey::MqttPort, serde_json::json!(port)).await
        {
            tracing::error!("Failed to save mqtt.port: {e:?}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    if let Some(ref client_id) = req.client_id {
        if client_id.is_empty() {
            return (StatusCode::BAD_REQUEST, "client_id must not be empty").into_response();
        }
        mqtt.client_id = client_id.clone();
        if let Err(e) = upsert_setting(
            &state.db,
            SettingKey::MqttClientId,
            serde_json::json!(client_id),
        )
        .await
        {
            tracing::error!("Failed to save mqtt.client_id: {e:?}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    // username: JSON value can be string or null
    if let Some(ref username_val) = req.username {
        if username_val.is_null() {
            mqtt.username = None;
            if let Err(e) =
                upsert_setting(&state.db, SettingKey::MqttUsername, serde_json::Value::Null).await
            {
                tracing::error!("Failed to save mqtt.username: {e:?}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        } else if let Some(s) = username_val.as_str() {
            if s.is_empty() {
                mqtt.username = None;
                if let Err(e) =
                    upsert_setting(&state.db, SettingKey::MqttUsername, serde_json::Value::Null)
                        .await
                {
                    tracing::error!("Failed to save mqtt.username: {e:?}");
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            } else {
                mqtt.username = Some(s.to_string());
                if let Err(e) =
                    upsert_setting(&state.db, SettingKey::MqttUsername, serde_json::json!(s)).await
                {
                    tracing::error!("Failed to save mqtt.username: {e:?}");
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            }
        } else {
            return (StatusCode::BAD_REQUEST, "username must be a string or null").into_response();
        }
    }

    // password: omitted = keep existing; empty string = clear; non-empty = set
    if let Some(ref password) = req.password {
        if password.is_empty() {
            mqtt.password = None;
            if let Err(e) =
                upsert_setting(&state.db, SettingKey::MqttPassword, serde_json::Value::Null).await
            {
                tracing::error!("Failed to save mqtt.password: {e:?}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        } else {
            mqtt.password = Some(password.clone());
            if let Err(e) = upsert_setting(
                &state.db,
                SettingKey::MqttPassword,
                serde_json::json!(password),
            )
            .await
            {
                tracing::error!("Failed to save mqtt.password: {e:?}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        }
    }

    if let Some(ref topic_prefix) = req.topic_prefix {
        if topic_prefix.is_empty() {
            return (StatusCode::BAD_REQUEST, "topic_prefix must not be empty").into_response();
        }
        mqtt.topic_prefix = topic_prefix.clone();
        if let Err(e) = upsert_setting(
            &state.db,
            SettingKey::MqttTopicPrefix,
            serde_json::json!(topic_prefix),
        )
        .await
        {
            tracing::error!("Failed to save mqtt.topic_prefix: {e:?}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    state.settings.set_mqtt(mqtt.clone()).await;

    let response = MqttSettingsResponse {
        host: mqtt.host,
        port: mqtt.port,
        client_id: mqtt.client_id,
        username: mqtt.username,
        has_password: mqtt.password.is_some(),
        topic_prefix: mqtt.topic_prefix,
    };
    (StatusCode::OK, Json(response)).into_response()
}
