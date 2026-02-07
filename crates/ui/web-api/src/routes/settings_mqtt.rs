use crate::AppState;
use crate::auth::permissions::Permission;
use crate::error_response::error_response;
use crate::middleware::require_auth::AuthenticatedUser;
use crate::middleware::tenant_context::TenantContext;
use crate::mqtt_client_store;
use crate::mqtt_lease_coordinator::MqttLeaseCoordinator;
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use uptrakit_shared_db::entity::mqtt_client;
use uptrakit_web_api_types::mqtt_transport::MqttTransport;
use uptrakit_web_api_types::mqtt_url::MqttUrl;

pub use uptrakit_web_api_types::settings_mqtt::{
    CreateMqttClientRequest, MqttClientResponse, MqttLimitResponse, UpdateMqttClientRequest,
    UpdateMqttLimitRequest,
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

/// List all MQTT client configurations
#[utoipa::path(
    get,
    path = "/api/v1/settings/mqtt",
    responses(
        (status = 200, description = "MQTT client configurations", body = Vec<MqttClientResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    security(("bearer_token" = []))
)]
pub async fn list_mqtt_settings(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    tenant: TenantContext,
) -> Response {
    if !user.has_permission(Permission::ViewSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let tenant_id = tenant.tenant_id;
    match mqtt_client_store::load_mqtt_clients(&state.db, tenant_id).await {
        Ok(models) => {
            let responses: Vec<MqttClientResponse> = models.iter().map(model_to_response).collect();
            (StatusCode::OK, Json(responses)).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to load MQTT clients: {e:?}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
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
        (status = 409, description = "MQTT client limit reached")
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
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let tenant_id = tenant.tenant_id;
    let max_clients = state.settings.mqtt_max_clients_per_tenant().await;

    // Resolve connection parameters from URL or individual fields
    let (transport, host, port) = if let Some(ref url_str) = req.url {
        match MqttUrl::parse(url_str) {
            Ok(parsed) => (parsed.transport, parsed.host, parsed.port),
            Err(e) => {
                return error_response(StatusCode::BAD_REQUEST, format!("invalid url: {e}"));
            }
        }
    } else {
        let host = match req.host {
            Some(ref h) if !h.is_empty() => h.clone(),
            _ => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "host is required (or provide url)",
                );
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
        return error_response(StatusCode::BAD_REQUEST, "client_id must not be empty");
    }
    if topic_prefix.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "topic_prefix must not be empty");
    }

    match mqtt_client_store::create_mqtt_client(mqtt_client_store::CreateMqttClientParams {
        db: &state.db,
        tenant_id,
        max_clients,
        enabled,
        transport: transport.as_str(),
        host: &host,
        port,
        client_id,
        username: req.username.as_deref(),
        password: req.password.as_deref(),
        topic_prefix,
    })
    .await
    {
        Ok(model) => (StatusCode::CREATED, Json(model_to_response(&model))).into_response(),
        Err(e) => {
            if let mqtt_client_store::MqttClientError::LimitReached(max) = e.current_context() {
                return error_response(
                    StatusCode::CONFLICT,
                    format!("MQTT client limit reached: maximum {max} per tenant"),
                );
            }
            tracing::error!("Failed to create MQTT client: {e:?}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Get MQTT client limit
#[utoipa::path(
    get,
    path = "/api/v1/settings/mqtt/limit",
    responses(
        (status = 200, description = "MQTT client limit", body = MqttLimitResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    security(("bearer_token" = []))
)]
pub async fn get_mqtt_limit(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
) -> Response {
    if !user.has_permission(Permission::ViewSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let max = state.settings.mqtt_max_clients_per_tenant().await;
    (
        StatusCode::OK,
        Json(MqttLimitResponse {
            max_clients_per_tenant: max,
        }),
    )
        .into_response()
}

/// Update MQTT client limit
#[utoipa::path(
    put,
    path = "/api/v1/settings/mqtt/limit",
    request_body = UpdateMqttLimitRequest,
    responses(
        (status = 200, description = "MQTT client limit updated", body = MqttLimitResponse),
        (status = 400, description = "Invalid value"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    security(("bearer_token" = []))
)]
pub async fn update_mqtt_limit(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    Json(req): Json<UpdateMqttLimitRequest>,
) -> Response {
    if !user.has_permission(Permission::ManageGlobalSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    if req.max_clients_per_tenant < 1 {
        return error_response(
            StatusCode::BAD_REQUEST,
            "max_clients_per_tenant must be at least 1",
        );
    }

    // Persist to DB
    if let Err(e) = crate::settings_store::upsert_setting(
        &state.db,
        state.default_tenant_id,
        crate::SettingKey::MqttMaxClientsPerTenant,
        serde_json::Value::Number(serde_json::Number::from(req.max_clients_per_tenant)),
    )
    .await
    {
        tracing::error!("Failed to persist MQTT limit: {e:?}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    state
        .settings
        .set_mqtt_max_clients_per_tenant(req.max_clients_per_tenant)
        .await;

    (
        StatusCode::OK,
        Json(MqttLimitResponse {
            max_clients_per_tenant: req.max_clients_per_tenant,
        }),
    )
        .into_response()
}

/// Get a specific MQTT client configuration
#[utoipa::path(
    get,
    path = "/api/v1/settings/mqtt/{id}",
    params(
        ("id" = String, Path, description = "MQTT client ID")
    ),
    responses(
        (status = 200, description = "MQTT client configuration", body = MqttClientResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "MQTT client not found")
    ),
    tag = "Settings",
    security(("bearer_token" = []))
)]
pub async fn get_mqtt_settings(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Response {
    if !user.has_permission(Permission::ViewSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let mqtt_client_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid id"),
    };

    match mqtt_client_store::load_mqtt_client_by_id(&state.db, mqtt_client_id).await {
        Ok(Some(model)) if model.tenant_id == tenant.tenant_id => {
            (StatusCode::OK, Json(model_to_response(&model))).into_response()
        }
        Ok(Some(_)) => error_response(StatusCode::NOT_FOUND, "Not found"),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Not found"),
        Err(e) => {
            tracing::error!("Failed to load MQTT client: {e:?}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Update a specific MQTT client configuration
#[utoipa::path(
    put,
    path = "/api/v1/settings/mqtt/{id}",
    params(
        ("id" = String, Path, description = "MQTT client ID")
    ),
    request_body = UpdateMqttClientRequest,
    responses(
        (status = 200, description = "Settings updated", body = MqttClientResponse),
        (status = 400, description = "Invalid values"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "MQTT client not found")
    ),
    tag = "Settings",
    security(("bearer_token" = []))
)]
pub async fn update_mqtt_settings(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    tenant: TenantContext,
    Path(id): Path<String>,
    Json(req): Json<UpdateMqttClientRequest>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let mqtt_client_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid id"),
    };

    let existing = match mqtt_client_store::load_mqtt_client_by_id(&state.db, mqtt_client_id).await
    {
        Ok(Some(model)) if model.tenant_id == tenant.tenant_id => model,
        Ok(Some(_)) | Ok(None) => return error_response(StatusCode::NOT_FOUND, "Not found"),
        Err(e) => {
            tracing::error!("Failed to load MQTT client: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Resolve URL-based overrides
    let (url_transport, url_host, url_port) = if let Some(ref url_str) = req.url {
        match MqttUrl::parse(url_str) {
            Ok(parsed) => (Some(parsed.transport), Some(parsed.host), Some(parsed.port)),
            Err(e) => {
                return error_response(StatusCode::BAD_REQUEST, format!("invalid url: {e}"));
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
            return error_response(StatusCode::BAD_REQUEST, "username must be a string or null");
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
        return error_response(StatusCode::BAD_REQUEST, "client_id must not be empty");
    }
    if let Some(ref tp) = req.topic_prefix
        && tp.is_empty()
    {
        return error_response(StatusCode::BAD_REQUEST, "topic_prefix must not be empty");
    }

    match mqtt_client_store::update_mqtt_client(mqtt_client_store::UpdateMqttClientParams {
        db: &state.db,
        existing,
        enabled: req.enabled,
        transport: transport.as_deref(),
        host: host.as_deref(),
        port,
        client_id: req.client_id.as_deref(),
        username,
        password,
        topic_prefix: req.topic_prefix.as_deref(),
    })
    .await
    {
        Ok(model) => {
            // Push config update to MQTT service if assigned
            let lease_coordinator = MqttLeaseCoordinator::new(
                state.db.clone(),
                state.service_connections.clone(),
                state.notification_service.clone(),
            );
            if let Err(e) = lease_coordinator
                .push_mqtt_client_config_update(mqtt_client_id)
                .await
            {
                tracing::warn!("Failed to push MQTT config update: {e:?}");
            }

            (StatusCode::OK, Json(model_to_response(&model))).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to update MQTT client: {e:?}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Delete a specific MQTT client configuration
#[utoipa::path(
    delete,
    path = "/api/v1/settings/mqtt/{id}",
    params(
        ("id" = String, Path, description = "MQTT client ID")
    ),
    responses(
        (status = 204, description = "MQTT client deleted"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "MQTT client not found")
    ),
    tag = "Settings",
    security(("bearer_token" = []))
)]
pub async fn delete_mqtt_settings(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    tenant: TenantContext,
    Path(id): Path<String>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let mqtt_client_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid id"),
    };

    // Verify tenant ownership
    match mqtt_client_store::load_mqtt_client_by_id(&state.db, mqtt_client_id).await {
        Ok(Some(model)) if model.tenant_id == tenant.tenant_id => {}
        Ok(Some(_)) | Ok(None) => return error_response(StatusCode::NOT_FOUND, "Not found"),
        Err(e) => {
            tracing::error!("Failed to load MQTT client: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    }

    // Revoke lease first
    let lease_coordinator = MqttLeaseCoordinator::new(
        state.db.clone(),
        state.service_connections.clone(),
        state.notification_service.clone(),
    );
    if let Err(e) = lease_coordinator
        .revoke_mqtt_client(mqtt_client_id, "mqtt client deleted")
        .await
    {
        tracing::warn!("Failed to revoke MQTT client lease: {e:?}");
    }

    match mqtt_client_store::delete_mqtt_client(&state.db, mqtt_client_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            if let mqtt_client_store::MqttClientError::NotFound = e.current_context() {
                return error_response(StatusCode::NOT_FOUND, "Not found");
            }
            tracing::error!("Failed to delete MQTT client: {e:?}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}
