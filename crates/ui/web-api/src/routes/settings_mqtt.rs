use crate::AppState;
use crate::error_response::error_response;
use crate::middleware::permission::{CanManageGlobalSettings, CanManageSettings, CanViewSettings};
use crate::mqtt_client_store;
use crate::mqtt_lease_coordinator::{LeaseOutcome, MQTT_LEASE_STALE_AFTER, MqttLeaseCoordinator};
use crate::tenant_db::TenantDb;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use std::collections::HashMap;
use std::sync::Arc;
use time::OffsetDateTime;
use uptrakit_internal_wire::{ControllerMessage, MqttClientCreatedPayload};
use uptrakit_shared_db::entity::{mqtt_client, mqtt_lease};
use uptrakit_web_api_types::mqtt_url::MqttUrl;
use uptrakit_web_api_types::validation::Validate;
use uuid::Uuid;

pub use uptrakit_web_api_types::settings_mqtt::{
    CreateMqttClientRequest, MqttClientConnectionStatus, MqttClientResponse, MqttLimitResponse,
    UpdateMqttClientRequest, UpdateMqttLimitRequest,
};

fn model_to_response(
    model: &mqtt_client::Model,
    connection_status: MqttClientConnectionStatus,
) -> MqttClientResponse {
    let transport = model.transport;
    let port = u16::try_from(model.port).unwrap_or(transport.default_port());
    let url = uptrakit_web_api_types::mqtt_url::build_url(transport, &model.host, port);

    MqttClientResponse {
        id: model.id,
        enabled: model.enabled,
        transport,
        host: model.host.clone(),
        port,
        url,
        client_id: model.client_id.clone(),
        username: model.username.clone(),
        has_password: model.password.is_some(),
        has_ca_cert: model.ca_cert_pem.is_some(),
        topic_prefix: model.topic_prefix.clone(),
        ha_discovery: model.ha_discovery,
        ha_discovery_prefix: model.ha_discovery_prefix.clone(),
        connection_status,
    }
}

fn parse_connection_status(model: &mqtt_client::Model) -> MqttClientConnectionStatus {
    model.connection_status
}

fn resolve_connection_status(
    model: &mqtt_client::Model,
    heartbeat_at: Option<OffsetDateTime>,
    now: OffsetDateTime,
) -> MqttClientConnectionStatus {
    if !model.enabled {
        return MqttClientConnectionStatus::Offline;
    }

    let persisted = parse_connection_status(model);
    let Some(heartbeat_at) = heartbeat_at else {
        return MqttClientConnectionStatus::Offline;
    };

    let age = now - heartbeat_at;
    let stale_after = time::Duration::seconds(MQTT_LEASE_STALE_AFTER.as_secs() as i64);
    if age > stale_after {
        MqttClientConnectionStatus::Offline
    } else {
        persisted
    }
}

/// List all MQTT client configurations
#[utoipa::path(
    get,
    path = "/api/v1/settings/mqtt",
    extensions(("x-required-permission" = json!("view_settings"))),
    responses(
        (status = 200, description = "MQTT client configurations", body = Vec<MqttClientResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_mqtt_settings(
    State(state): State<Arc<AppState>>,
    CanViewSettings(_user): CanViewSettings,
    tenant_db: TenantDb,
) -> Response {
    match mqtt_client_store::load_mqtt_clients(tenant_db.db(), tenant_db.tenant_id).await {
        Ok(models) => {
            let mqtt_client_ids: Vec<uuid::Uuid> = models.iter().map(|m| m.id).collect();
            let leases: Vec<mqtt_lease::Model> = if mqtt_client_ids.is_empty() {
                Vec::new()
            } else {
                match mqtt_lease::Entity::find()
                    .filter(mqtt_lease::Column::MqttClientId.is_in(mqtt_client_ids))
                    .all(state.db())
                    .await
                {
                    Ok(values) => values,
                    Err(e) => {
                        tracing::error!("Failed to load MQTT leases: {e:?}");
                        return error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "Internal server error",
                        );
                    }
                }
            };

            let mut lease_map = HashMap::new();
            for lease in leases {
                lease_map.insert(lease.mqtt_client_id, lease.heartbeat_at);
            }

            let now = OffsetDateTime::now_utc();
            let responses: Vec<MqttClientResponse> = models
                .iter()
                .map(|model| {
                    let heartbeat_at = lease_map.get(&model.id).copied();
                    let status = resolve_connection_status(model, heartbeat_at, now);
                    model_to_response(model, status)
                })
                .collect();
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
    extensions(("x-required-permission" = json!("manage_settings"))),
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
#[tracing::instrument(skip_all)]
pub async fn create_mqtt_settings(
    State(state): State<Arc<AppState>>,
    CanManageSettings(_user): CanManageSettings,
    tenant_db: TenantDb,
    Json(req): Json<CreateMqttClientRequest>,
) -> Response {
    if let Err(e) = req.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    let tenant_id = tenant_db.tenant_id;
    let max_clients = state.settings.mqtt_max_clients_per_tenant();

    // Resolve connection parameters from URL or individual fields
    let (transport, host, port) = if let Some(ref url_str) = req.url {
        match url_str.parse::<MqttUrl>() {
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
        db: tenant_db.db(),
        tenant_id,
        max_clients,
        enabled,
        transport,
        host: &host,
        port,
        client_id,
        username: req.username.as_deref(),
        password: req.password.as_ref().map(|p| p.expose_secret()),
        ca_cert_pem: req.ca_pem.as_ref().map(|c| c.expose_secret()),
        topic_prefix,
        ha_discovery: req.ha_discovery.unwrap_or(false),
        ha_discovery_prefix: req
            .ha_discovery_prefix
            .as_deref()
            .unwrap_or("homeassistant"),
    })
    .await
    {
        Ok(model) => {
            let status = resolve_connection_status(&model, None, OffsetDateTime::now_utc());
            let lease_coordinator =
                MqttLeaseCoordinator::new(state.db().clone(), state.service_connections.clone());
            match lease_coordinator
                .lease_new_client_to_least_busy(&model)
                .await
            {
                Ok(LeaseOutcome::Leased { service_id }) => {
                    tracing::info!(
                        mqtt_client_id = %model.id,
                        %service_id,
                        "leased new MQTT client to local service"
                    );
                }
                Ok(LeaseOutcome::NoLocalService) => {
                    let msg = ControllerMessage::MqttClientCreated(MqttClientCreatedPayload {
                        mqtt_client_id: model.id,
                    });
                    state
                        .notification_service
                        .publish_controller_event(msg)
                        .await;
                    tracing::info!(
                        mqtt_client_id = %model.id,
                        "no local MQTT service available; published cross-controller lease event"
                    );
                }
                Ok(LeaseOutcome::AlreadyLeased) => {
                    tracing::debug!(
                        mqtt_client_id = %model.id,
                        "MQTT client already leased; skipping immediate assignment"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        mqtt_client_id = %model.id,
                        "failed to lease new MQTT client"
                    );
                }
            }
            (StatusCode::CREATED, Json(model_to_response(&model, status))).into_response()
        }
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
    path = "/api/v1/global-settings/mqtt-limit",
    extensions(("x-required-permission" = json!("view_settings"))),
    responses(
        (status = 200, description = "MQTT client limit", body = MqttLimitResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Global Settings",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_mqtt_limit(
    State(state): State<Arc<AppState>>,
    CanViewSettings(_user): CanViewSettings,
) -> Response {
    let max = state.settings.mqtt_max_clients_per_tenant();
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
    path = "/api/v1/global-settings/mqtt-limit",
    request_body = UpdateMqttLimitRequest,
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    responses(
        (status = 200, description = "MQTT client limit updated", body = MqttLimitResponse),
        (status = 400, description = "Invalid value"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Global Settings",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_mqtt_limit(
    State(state): State<Arc<AppState>>,
    CanManageGlobalSettings(_user): CanManageGlobalSettings,
    Json(req): Json<UpdateMqttLimitRequest>,
) -> Response {
    if req.max_clients_per_tenant < 1 {
        return error_response(
            StatusCode::BAD_REQUEST,
            "max_clients_per_tenant must be at least 1",
        );
    }

    // Persist to DB
    if let Err(e) = crate::settings_store::upsert_global_setting(
        state.db(),
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
        ("id" = Uuid, Path, description = "MQTT client ID")
    ),
    extensions(("x-required-permission" = json!("view_settings"))),
    responses(
        (status = 200, description = "MQTT client configuration", body = MqttClientResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "MQTT client not found")
    ),
    tag = "Settings",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_mqtt_settings(
    State(state): State<Arc<AppState>>,
    CanViewSettings(_user): CanViewSettings,
    tenant_db: TenantDb,
    Path(mqtt_client_id): Path<Uuid>,
) -> Response {
    match mqtt_client_store::load_mqtt_client_by_id(state.db(), mqtt_client_id, tenant_db.tenant_id)
        .await
    {
        Ok(Some(model)) => {
            let heartbeat_at = match mqtt_lease::Entity::find()
                .filter(mqtt_lease::Column::MqttClientId.eq(model.id))
                .one(state.db())
                .await
            {
                Ok(lease) => lease.map(|l| l.heartbeat_at),
                Err(e) => {
                    tracing::error!("Failed to load MQTT lease: {e:?}");
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            };
            let now = OffsetDateTime::now_utc();
            let status = resolve_connection_status(&model, heartbeat_at, now);
            (StatusCode::OK, Json(model_to_response(&model, status))).into_response()
        }
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
        ("id" = Uuid, Path, description = "MQTT client ID")
    ),
    request_body = UpdateMqttClientRequest,
    extensions(("x-required-permission" = json!("manage_settings"))),
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
#[tracing::instrument(skip_all)]
pub async fn update_mqtt_settings(
    State(state): State<Arc<AppState>>,
    CanManageSettings(_user): CanManageSettings,
    tenant_db: TenantDb,
    Path(mqtt_client_id): Path<Uuid>,
    Json(req): Json<UpdateMqttClientRequest>,
) -> Response {
    if let Err(e) = req.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    let existing = match mqtt_client_store::load_mqtt_client_by_id(
        state.db(),
        mqtt_client_id,
        tenant_db.tenant_id,
    )
    .await
    {
        Ok(Some(model)) => model,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Not found"),
        Err(e) => {
            tracing::error!("Failed to load MQTT client: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Resolve URL-based overrides
    let (url_transport, url_host, url_port) = if let Some(ref url_str) = req.url {
        match url_str.parse::<MqttUrl>() {
            Ok(parsed) => (Some(parsed.transport), Some(parsed.host), Some(parsed.port)),
            Err(e) => {
                return error_response(StatusCode::BAD_REQUEST, format!("invalid url: {e}"));
            }
        }
    } else {
        (None, None, None)
    };

    let transport = url_transport.or(req.transport);
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

    // Password: JSON value can be string or null
    let password: Option<Option<&str>> = if let Some(ref password_val) = req.password {
        if password_val.is_null() {
            Some(None)
        } else if let Some(s) = password_val.as_str() {
            if s.is_empty() {
                Some(None)
            } else {
                Some(Some(s))
            }
        } else {
            return error_response(StatusCode::BAD_REQUEST, "password must be a string or null");
        }
    } else {
        None
    };

    // CA PEM: JSON value can be string or null
    let ca_cert_pem: Option<Option<&str>> = if let Some(ref ca_val) = req.ca_pem {
        if ca_val.is_null() {
            Some(None)
        } else if let Some(s) = ca_val.as_str() {
            if s.is_empty() {
                Some(None)
            } else {
                Some(Some(s))
            }
        } else {
            return error_response(StatusCode::BAD_REQUEST, "ca_pem must be a string or null");
        }
    } else {
        None
    };

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
        db: state.db(),
        existing,
        enabled: req.enabled,
        transport,
        host: host.as_deref(),
        port,
        client_id: req.client_id.as_deref(),
        username,
        password,
        ca_cert_pem,
        topic_prefix: req.topic_prefix.as_deref(),
        ha_discovery: req.ha_discovery,
        ha_discovery_prefix: req.ha_discovery_prefix.as_deref(),
    })
    .await
    {
        Ok(model) => {
            // Push config update to MQTT service if assigned
            let lease_coordinator =
                MqttLeaseCoordinator::new(state.db().clone(), state.service_connections.clone());
            if let Err(e) = lease_coordinator
                .push_mqtt_client_config_update(mqtt_client_id)
                .await
            {
                tracing::warn!("Failed to push MQTT config update: {e:?}");
            }

            let heartbeat_at = match mqtt_lease::Entity::find()
                .filter(mqtt_lease::Column::MqttClientId.eq(model.id))
                .one(state.db())
                .await
            {
                Ok(lease) => lease.map(|l| l.heartbeat_at),
                Err(e) => {
                    tracing::error!("Failed to load MQTT lease: {e:?}");
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            };
            let now = OffsetDateTime::now_utc();
            let status = resolve_connection_status(&model, heartbeat_at, now);
            (StatusCode::OK, Json(model_to_response(&model, status))).into_response()
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
        ("id" = Uuid, Path, description = "MQTT client ID")
    ),
    extensions(("x-required-permission" = json!("manage_settings"))),
    responses(
        (status = 204, description = "MQTT client deleted"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "MQTT client not found")
    ),
    tag = "Settings",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn delete_mqtt_settings(
    State(state): State<Arc<AppState>>,
    CanManageSettings(_user): CanManageSettings,
    tenant_db: TenantDb,
    Path(mqtt_client_id): Path<Uuid>,
) -> Response {
    // Verify tenant ownership
    match mqtt_client_store::load_mqtt_client_by_id(state.db(), mqtt_client_id, tenant_db.tenant_id)
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Not found"),
        Err(e) => {
            tracing::error!("Failed to load MQTT client: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    }

    // Revoke lease first
    let lease_coordinator =
        MqttLeaseCoordinator::new(state.db().clone(), state.service_connections.clone());
    if let Err(e) = lease_coordinator
        .revoke_mqtt_client(mqtt_client_id, "mqtt client deleted")
        .await
    {
        tracing::warn!("Failed to revoke MQTT client lease: {e:?}");
    }

    match mqtt_client_store::delete_mqtt_client(state.db(), mqtt_client_id).await {
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

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_shared_types::MqttTransport;

    fn make_model(enabled: bool, status: MqttClientConnectionStatus) -> mqtt_client::Model {
        let now = time::OffsetDateTime::UNIX_EPOCH;
        mqtt_client::Model {
            id: uuid::Uuid::nil(),
            tenant_id: uuid::Uuid::nil(),
            enabled,
            transport: MqttTransport::Tcp,
            host: "localhost".to_string(),
            port: 1883,
            client_id: "test".to_string(),
            username: None,
            password: None,
            ca_cert_pem: None,
            topic_prefix: "test".to_string(),
            connection_status: status,
            status_updated_at: now,
            ha_discovery: false,
            ha_discovery_prefix: "homeassistant".to_string(),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn disabled_model_returns_offline() {
        let model = make_model(false, MqttClientConnectionStatus::Online);
        let now = time::OffsetDateTime::now_utc();
        assert_eq!(
            resolve_connection_status(&model, Some(now), now),
            MqttClientConnectionStatus::Offline
        );
    }

    #[test]
    fn no_heartbeat_returns_offline() {
        let model = make_model(true, MqttClientConnectionStatus::Online);
        let now = time::OffsetDateTime::now_utc();
        assert_eq!(
            resolve_connection_status(&model, None, now),
            MqttClientConnectionStatus::Offline
        );
    }

    #[test]
    fn fresh_heartbeat_returns_persisted_status() {
        let model = make_model(true, MqttClientConnectionStatus::Online);
        let now = time::OffsetDateTime::now_utc();
        // 10 seconds ago — well within the stale threshold
        let heartbeat_at = now - time::Duration::seconds(10);
        assert_eq!(
            resolve_connection_status(&model, Some(heartbeat_at), now),
            MqttClientConnectionStatus::Online
        );
    }

    #[test]
    fn stale_heartbeat_returns_offline() {
        let model = make_model(true, MqttClientConnectionStatus::Online);
        let now = time::OffsetDateTime::now_utc();
        // 5 minutes ago — much older than the stale threshold
        let heartbeat_at = now - time::Duration::seconds(300);
        assert_eq!(
            resolve_connection_status(&model, Some(heartbeat_at), now),
            MqttClientConnectionStatus::Offline
        );
    }
}
