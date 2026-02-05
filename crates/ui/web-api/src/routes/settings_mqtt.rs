use crate::AppState;
use crate::auth::permissions::Permission;
use crate::middleware::require_auth::AuthenticatedUser;
use crate::middleware::tenant_context::TenantContext;
use crate::mqtt_client_store;
use axum::{
    Extension, Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use uptrakit_shared_db::entity::mqtt_client;
use uptrakit_web_api_types::mqtt_transport::MqttTransport;
use uptrakit_web_api_types::mqtt_url::MqttUrl;

pub use uptrakit_web_api_types::settings_mqtt::{
    CreateMqttClientRequest, MqttClientResponse, UpdateMqttClientRequest,
};

fn model_to_response(model: &mqtt_client::Model) -> MqttClientResponse {
    let transport = MqttTransport::parse(&model.transport).unwrap_or_default();
    let port = u16::try_from(model.port).unwrap_or(transport.default_port());
    let url = uptrakit_web_api_types::mqtt_url::build_url(transport, &model.host, port);

    MqttClientResponse {
        id: model.id.to_string(),
        enabled: model.enabled,
        transport,
        host: model.host.clone(),
        port,
        url,
        client_id: model.client_id.clone(),
        username: model.username.clone(),
        has_password: model.password.is_some(),
        topic_prefix: model.topic_prefix.clone(),
    }
}

/// Get MQTT client configuration
#[utoipa::path(
    get,
    path = "/api/v1/settings/mqtt",
    responses(
        (status = 200, description = "MQTT client configuration", body = MqttClientResponse),
        (status = 404, description = "No MQTT client configured"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    security(("bearer_token" = []))
)]
pub async fn get_mqtt_settings(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    tenant: TenantContext,
) -> Response {
    if !user.has_permission(Permission::ViewSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let tenant_id = tenant.tenant_id;
    match mqtt_client_store::load_mqtt_client(&state.db, tenant_id).await {
        Ok(Some(model)) => (StatusCode::OK, Json(model_to_response(&model))).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!("Failed to load MQTT client: {e:?}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Create MQTT client configuration
#[utoipa::path(
    post,
    path = "/api/v1/settings/mqtt",
    request_body = CreateMqttClientRequest,
    responses(
        (status = 201, description = "MQTT client created", body = MqttClientResponse),
        (status = 400, description = "Invalid values"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 409, description = "MQTT client already exists")
    ),
    tag = "Settings",
    security(("bearer_token" = []))
)]
pub async fn create_mqtt_settings(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    tenant: TenantContext,
    Json(req): Json<CreateMqttClientRequest>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let tenant_id = tenant.tenant_id;

    // Resolve connection parameters from URL or individual fields
    let (transport, host, port) = if let Some(ref url_str) = req.url {
        match MqttUrl::parse(url_str) {
            Ok(parsed) => (parsed.transport, parsed.host, parsed.port),
            Err(e) => {
                return (StatusCode::BAD_REQUEST, format!("invalid url: {e}")).into_response();
            }
        }
    } else {
        let host = match req.host {
            Some(ref h) if !h.is_empty() => h.clone(),
            _ => {
                return (StatusCode::BAD_REQUEST, "host is required (or provide url)")
                    .into_response();
            }
        };
        let transport = req.transport.unwrap_or_default();
        let port = req.port.unwrap_or(transport.default_port());
        (transport, host, port)
    };

    let enabled = req.enabled.unwrap_or(true);
    let client_id = req.client_id.as_deref().unwrap_or("uptrakit-controller");
    let topic_prefix = req.topic_prefix.as_deref().unwrap_or("uptrakit");

    if client_id.is_empty() {
        return (StatusCode::BAD_REQUEST, "client_id must not be empty").into_response();
    }
    if topic_prefix.is_empty() {
        return (StatusCode::BAD_REQUEST, "topic_prefix must not be empty").into_response();
    }

    match mqtt_client_store::create_mqtt_client(
        &state.db,
        tenant_id,
        enabled,
        transport.as_str(),
        &host,
        port,
        client_id,
        req.username.as_deref(),
        req.password.as_deref(),
        topic_prefix,
    )
    .await
    {
        Ok(model) => (StatusCode::CREATED, Json(model_to_response(&model))).into_response(),
        Err(e) => {
            if let mqtt_client_store::MqttClientError::AlreadyExists = e.current_context() {
                return (
                    StatusCode::CONFLICT,
                    "MQTT client already exists for this tenant",
                )
                    .into_response();
            }
            tracing::error!("Failed to create MQTT client: {e:?}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Update MQTT client configuration
#[utoipa::path(
    put,
    path = "/api/v1/settings/mqtt",
    request_body = UpdateMqttClientRequest,
    responses(
        (status = 200, description = "Settings updated", body = MqttClientResponse),
        (status = 400, description = "Invalid values"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "No MQTT client configured")
    ),
    tag = "Settings",
    security(("bearer_token" = []))
)]
pub async fn update_mqtt_settings(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    tenant: TenantContext,
    Json(req): Json<UpdateMqttClientRequest>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let tenant_id = tenant.tenant_id;
    let existing = match mqtt_client_store::load_mqtt_client(&state.db, tenant_id).await {
        Ok(Some(model)) => model,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!("Failed to load MQTT client: {e:?}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Resolve URL-based overrides
    let (url_transport, url_host, url_port) = if let Some(ref url_str) = req.url {
        match MqttUrl::parse(url_str) {
            Ok(parsed) => (Some(parsed.transport), Some(parsed.host), Some(parsed.port)),
            Err(e) => {
                return (StatusCode::BAD_REQUEST, format!("invalid url: {e}")).into_response();
            }
        }
    } else {
        (None, None, None)
    };

    let transport = url_transport
        .or(req.transport)
        .map(|t| t.as_str().to_string());
    let host = url_host.or(req.host.clone());
    let port = url_port.or(req.port);

    // Username: JSON value can be string or null
    let username: Option<Option<&str>> = if let Some(ref username_val) = req.username {
        if username_val.is_null() {
            Some(None)
        } else if let Some(s) = username_val.as_str() {
            if s.is_empty() {
                Some(None)
            } else {
                Some(Some(s))
            }
        } else {
            return (StatusCode::BAD_REQUEST, "username must be a string or null").into_response();
        }
    } else {
        None
    };

    // Password: omitted = keep existing; empty string = clear; non-empty = set
    let password: Option<Option<&str>> = req
        .password
        .as_ref()
        .map(|p| if p.is_empty() { None } else { Some(p.as_str()) });

    if let Some(ref cid) = req.client_id
        && cid.is_empty()
    {
        return (StatusCode::BAD_REQUEST, "client_id must not be empty").into_response();
    }
    if let Some(ref tp) = req.topic_prefix
        && tp.is_empty()
    {
        return (StatusCode::BAD_REQUEST, "topic_prefix must not be empty").into_response();
    }

    match mqtt_client_store::update_mqtt_client(
        &state.db,
        existing,
        req.enabled,
        transport.as_deref(),
        host.as_deref(),
        port,
        req.client_id.as_deref(),
        username,
        password,
        req.topic_prefix.as_deref(),
    )
    .await
    {
        Ok(model) => (StatusCode::OK, Json(model_to_response(&model))).into_response(),
        Err(e) => {
            tracing::error!("Failed to update MQTT client: {e:?}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Delete MQTT client configuration
#[utoipa::path(
    delete,
    path = "/api/v1/settings/mqtt",
    responses(
        (status = 204, description = "MQTT client deleted"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "No MQTT client configured")
    ),
    tag = "Settings",
    security(("bearer_token" = []))
)]
pub async fn delete_mqtt_settings(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    tenant: TenantContext,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let tenant_id = tenant.tenant_id;
    match mqtt_client_store::delete_mqtt_client(&state.db, tenant_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            if let mqtt_client_store::MqttClientError::NotFound = e.current_context() {
                return StatusCode::NOT_FOUND.into_response();
            }
            tracing::error!("Failed to delete MQTT client: {e:?}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
