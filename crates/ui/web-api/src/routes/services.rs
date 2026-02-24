use crate::AppState;
use crate::SettingKey;
use crate::auth::{password, token};
use crate::error_response::error_response;
use crate::middleware::permission::{CanManageAgents, CanViewAgents};
use crate::queries::services::{self as svc_queries, ServiceQueryError};
use crate::settings_store::{delete_setting, load_setting, upsert_setting};
use crate::tenant_db::TenantDb;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use uptrakit_internal_wire::{ApprovedPayload, ControllerMessage, RejectedPayload};

pub use uptrakit_web_api_types::pagination::PaginatedResponse;
pub use uptrakit_web_api_types::services::{
    EnrollmentTokenResponse, EnrollmentTokenStatusResponse, ListServicesQuery, MergeAgentRequest,
    MessageResponse, ServiceResponse, ServiceStatus, ServiceType, UpdateServiceRequest,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Determine the correct `SettingKey` for the enrollment token hash based on
/// the `type` query parameter.
fn enrollment_setting_key(type_param: Option<&ServiceType>) -> SettingKey {
    match type_param {
        Some(ServiceType::Mqtt) => SettingKey::MqttEnrollmentTokenHash,
        Some(ServiceType::SshAgent) => SettingKey::SshAgentEnrollmentTokenHash,
        Some(ServiceType::Agent) | None => SettingKey::EnrollmentTokenHash,
        Some(_) => SettingKey::EnrollmentTokenHash,
    }
}

// ---------------------------------------------------------------------------
// Endpoints
// ---------------------------------------------------------------------------

/// List all services (agents and/or MQTT)
#[utoipa::path(
    get,
    path = "/api/v1/services",
    params(
        ("type" = Option<String>, Query, description = "Filter by service type (agent, mqtt)"),
        ("status" = Option<String>, Query, description = "Filter by status (pending, approved, rejected, deactivated)"),
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)")
    ),
    responses(
        (status = 200, description = "Paginated list of services", body = PaginatedResponse<ServiceResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Services",
    extensions(("x-required-permission" = json!("view_agents"))),
    security(("bearer_token" = []))
)]
pub async fn list_services(
    tenant_db: TenantDb,
    CanViewAgents(_user): CanViewAgents,
    Query(query): Query<ListServicesQuery>,
) -> Response {
    match svc_queries::list_services(&tenant_db, &query).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!("Failed to list services: {}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Get a single service by ID
#[utoipa::path(
    get,
    path = "/api/v1/services/{id}",
    params(
        ("id" = String, Path, description = "Service UUID")
    ),
    responses(
        (status = 200, description = "Service details", body = ServiceResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Service not found")
    ),
    tag = "Services",
    extensions(("x-required-permission" = json!("view_agents"))),
    security(("bearer_token" = []))
)]
pub async fn get_service(
    tenant_db: TenantDb,
    CanViewAgents(_user): CanViewAgents,
    Path(id): Path<String>,
) -> Response {
    let service_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid service ID"),
    };

    match svc_queries::get_active_service(&tenant_db, service_id).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Service not found"),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Update a service's configurable settings (e.g. ping interval)
#[utoipa::path(
    put,
    path = "/api/v1/services/{id}",
    params(
        ("id" = String, Path, description = "Service UUID")
    ),
    request_body = UpdateServiceRequest,
    responses(
        (status = 200, description = "Service updated", body = ServiceResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Service not found")
    ),
    tag = "Services",
    extensions(("x-required-permission" = json!("manage_agents"))),
    security(("bearer_token" = []))
)]
pub async fn update_service(
    tenant_db: TenantDb,
    CanManageAgents(_user): CanManageAgents,
    Path(id): Path<String>,
    Json(body): Json<UpdateServiceRequest>,
) -> Response {
    let service_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid service ID"),
    };

    match svc_queries::update_service_settings(&tenant_db, service_id, body.ping_interval_seconds)
        .await
    {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Service not found"),
        Err(e) => {
            tracing::error!("Failed to update service: {}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Approve a pending service
#[utoipa::path(
    post,
    path = "/api/v1/services/{id}/approve",
    params(
        ("id" = String, Path, description = "Service UUID")
    ),
    responses(
        (status = 200, description = "Service approved", body = ServiceResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Service not found")
    ),
    tag = "Services",
    extensions(("x-required-permission" = json!("manage_agents"))),
    security(("bearer_token" = []))
)]
pub async fn approve_service(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageAgents(_user): CanManageAgents,
    Path(id): Path<String>,
) -> Response {
    let service_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid service ID"),
    };

    let resp = match svc_queries::approve_service(&tenant_db, service_id).await {
        Ok(r) => r,
        Err(ServiceQueryError::NotFound) => {
            return error_response(StatusCode::NOT_FOUND, "Service not found");
        }
        Err(ServiceQueryError::NotPending) => {
            return error_response(StatusCode::BAD_REQUEST, "Service is not in pending status");
        }
        Err(ServiceQueryError::Db(e)) => {
            tracing::error!("Failed to approve service: {}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        Err(e) => {
            tracing::error!("Unexpected error approving service: {:?}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Push approval via WebSocket (local + cross-controller outbox).
    let _ = state
        .notification_service
        .send(
            &service_id,
            ControllerMessage::Approved(ApprovedPayload { service_id }),
        )
        .await;

    (StatusCode::OK, Json(resp)).into_response()
}

/// Reject a pending service
#[utoipa::path(
    post,
    path = "/api/v1/services/{id}/reject",
    params(
        ("id" = String, Path, description = "Service UUID")
    ),
    responses(
        (status = 200, description = "Service rejected", body = ServiceResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Service not found")
    ),
    tag = "Services",
    extensions(("x-required-permission" = json!("manage_agents"))),
    security(("bearer_token" = []))
)]
pub async fn reject_service(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageAgents(_user): CanManageAgents,
    Path(id): Path<String>,
) -> Response {
    let service_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid service ID"),
    };

    let resp = match svc_queries::reject_service(&tenant_db, service_id).await {
        Ok(r) => r,
        Err(ServiceQueryError::NotFound) => {
            return error_response(StatusCode::NOT_FOUND, "Service not found");
        }
        Err(ServiceQueryError::NotPending) => {
            return error_response(StatusCode::BAD_REQUEST, "Service is not in pending status");
        }
        Err(ServiceQueryError::Db(e)) => {
            tracing::error!("Failed to reject service: {}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        Err(e) => {
            tracing::error!("Unexpected error rejecting service: {:?}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Push rejection via WebSocket (local + cross-controller outbox).
    let _ = state
        .notification_service
        .send(
            &service_id,
            ControllerMessage::Rejected(RejectedPayload { service_id }),
        )
        .await;

    // Terminate active WebSocket connection.
    state.service_connections.unregister(&service_id).await;

    (StatusCode::OK, Json(resp)).into_response()
}

/// Deactivate a service (soft-delete)
#[utoipa::path(
    delete,
    path = "/api/v1/services/{id}",
    params(
        ("id" = String, Path, description = "Service UUID")
    ),
    responses(
        (status = 200, description = "Service deactivated", body = MessageResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Service not found")
    ),
    tag = "Services",
    extensions(("x-required-permission" = json!("manage_agents"))),
    security(("bearer_token" = []))
)]
pub async fn deactivate_service(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageAgents(_user): CanManageAgents,
    Path(id): Path<String>,
) -> Response {
    let service_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid service ID"),
    };

    match svc_queries::deactivate_service(&tenant_db, service_id, state.default_tenant_id).await {
        Ok(true) => {
            state.revocation_notify.notify_one();
            state.service_connections.unregister(&service_id).await;
            (
                StatusCode::OK,
                Json(MessageResponse {
                    message: "Service deactivated".to_string(),
                }),
            )
                .into_response()
        }
        Ok(false) => error_response(StatusCode::NOT_FOUND, "Service not found"),
        Err(e) => {
            tracing::error!("Failed to deactivate service: {}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Merge a pending (source) agent into an existing approved (target) agent.
///
/// This operation is only valid for agent services. MQTT services cannot be merged.
#[utoipa::path(
    post,
    path = "/api/v1/services/{target_id}/merge",
    params(
        ("target_id" = String, Path, description = "Target service UUID (approved agent)")
    ),
    request_body = MergeAgentRequest,
    responses(
        (status = 200, description = "Services merged", body = ServiceResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Service not found")
    ),
    tag = "Services",
    extensions(("x-required-permission" = json!("manage_agents"))),
    security(("bearer_token" = []))
)]
pub async fn merge_service(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageAgents(_user): CanManageAgents,
    Path(target_id): Path<String>,
    Json(body): Json<MergeAgentRequest>,
) -> Response {
    let target_uuid = match uuid::Uuid::parse_str(&target_id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid target service ID"),
    };

    let source_uuid = body.source_id;

    if target_uuid == source_uuid {
        return error_response(StatusCode::BAD_REQUEST, "Cannot merge service into itself");
    }

    let target_connected = state.service_connections.is_connected(&target_uuid).await;

    let resp = match svc_queries::merge_service(
        &tenant_db,
        target_uuid,
        source_uuid,
        target_connected,
        state.default_tenant_id,
    )
    .await
    {
        Ok(r) => r,
        Err(ServiceQueryError::NotFound) => {
            return error_response(StatusCode::NOT_FOUND, "Target service not found");
        }
        Err(ServiceQueryError::SourceNotFound) => {
            return error_response(StatusCode::NOT_FOUND, "Source service not found");
        }
        Err(ServiceQueryError::TargetConnected) => {
            return error_response(
                StatusCode::CONFLICT,
                "Target service is currently connected",
            );
        }
        Err(ServiceQueryError::NotAgentType) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "Merge is only supported for agent services",
            );
        }
        Err(ServiceQueryError::NotApproved) => {
            return error_response(StatusCode::BAD_REQUEST, "Target service must be approved");
        }
        Err(ServiceQueryError::NotPending) => {
            return error_response(StatusCode::BAD_REQUEST, "Source service must be pending");
        }
        Err(ServiceQueryError::Db(e)) => {
            tracing::error!("Failed to merge services: {}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    state.revocation_notify.notify_one();
    state.service_connections.unregister(&source_uuid).await;

    tracing::info!(
        target_id = %target_uuid,
        source_id = %source_uuid,
        "services merged: source deactivated, target updated"
    );

    (StatusCode::OK, Json(resp)).into_response()
}

/// Generate a new enrollment token
#[utoipa::path(
    post,
    path = "/api/v1/services/enrollment-token",
    params(
        ("type" = Option<String>, Query, description = "Service type (agent, mqtt). Defaults to agent.")
    ),
    responses(
        (status = 201, description = "Enrollment token generated", body = EnrollmentTokenResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Services",
    extensions(("x-required-permission" = json!("manage_agents"))),
    security(("bearer_token" = []))
)]
pub async fn create_enrollment_token(
    State(state): State<Arc<AppState>>,
    CanManageAgents(_user): CanManageAgents,
    Query(query): Query<ListServicesQuery>,
) -> Response {
    let setting_key = enrollment_setting_key(query.r#type.as_ref());

    let plaintext = match token::generate_secure_token() {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to generate enrollment token: {:?}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let hash = match password::hash_password(&plaintext) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("Failed to hash enrollment token: {:?}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = upsert_setting(
        state.db(),
        state.default_tenant_id,
        setting_key,
        serde_json::Value::String(hash.expose_secret().to_string()),
    )
    .await
    {
        tracing::error!("Failed to store enrollment token hash: {:?}", e);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    (
        StatusCode::CREATED,
        Json(EnrollmentTokenResponse {
            token: uptrakit_web_api_types::SecretString::new(plaintext),
        }),
    )
        .into_response()
}

/// Revoke the enrollment token
#[utoipa::path(
    delete,
    path = "/api/v1/services/enrollment-token",
    params(
        ("type" = Option<String>, Query, description = "Service type (agent, mqtt). Defaults to agent.")
    ),
    responses(
        (status = 200, description = "Enrollment token revoked", body = MessageResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Services",
    extensions(("x-required-permission" = json!("manage_agents"))),
    security(("bearer_token" = []))
)]
pub async fn revoke_enrollment_token(
    State(state): State<Arc<AppState>>,
    CanManageAgents(_user): CanManageAgents,
    Query(query): Query<ListServicesQuery>,
) -> Response {
    let setting_key = enrollment_setting_key(query.r#type.as_ref());

    if let Err(e) = delete_setting(state.db(), state.default_tenant_id, setting_key).await {
        tracing::error!("Failed to delete enrollment token: {:?}", e);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    (
        StatusCode::OK,
        Json(MessageResponse {
            message: "Enrollment token revoked".to_string(),
        }),
    )
        .into_response()
}

/// Check if an enrollment token is configured
#[utoipa::path(
    get,
    path = "/api/v1/services/enrollment-token/status",
    params(
        ("type" = Option<String>, Query, description = "Service type (agent, mqtt). Defaults to agent.")
    ),
    responses(
        (status = 200, description = "Enrollment token status", body = EnrollmentTokenStatusResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Services",
    extensions(("x-required-permission" = json!("manage_agents"))),
    security(("bearer_token" = []))
)]
pub async fn enrollment_token_status(
    State(state): State<Arc<AppState>>,
    CanManageAgents(_user): CanManageAgents,
    Query(query): Query<ListServicesQuery>,
) -> Response {
    let setting_key = enrollment_setting_key(query.r#type.as_ref());

    let configured = matches!(
        load_setting(state.db(), state.default_tenant_id, setting_key).await,
        Ok(Some(_))
    );

    (
        StatusCode::OK,
        Json(EnrollmentTokenStatusResponse { configured }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::permissions::Permission;
    use crate::auth::registration::{RegistrationMode, RegistrationSettings};
    use crate::cert_signer::{AgentCertSigner, CertSignerError, SignedCertBundle};
    use crate::middleware::permission::CanManageAgents;
    use crate::middleware::require_auth::AuthenticatedUser;
    use crate::settings::Settings;
    use crate::tenant_db::TenantDb;
    use axum::Json;
    use axum::extract::{Path, State};
    use axum::http::StatusCode;
    use sea_orm::{
        ActiveModelTrait, ConnectOptions, ConnectionTrait, Database, DatabaseConnection,
        EntityTrait, Set,
    };
    use time::OffsetDateTime;
    use uptrakit_shared_db::entity::{
        prelude::{AuthMethod, Service},
        service,
    };

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        Database::connect(opt).await.expect("test db")
    }

    async fn setup_test_db() -> DatabaseConnection {
        let db = test_db().await;

        db.execute_unprepared(
            "CREATE TABLE services (
                id TEXT PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                service_type TEXT NOT NULL,
                hostname TEXT NOT NULL,
                friendly_name TEXT NOT NULL,
                ip_address TEXT,
                status TEXT NOT NULL,
                enrollment_secret_hash TEXT NOT NULL UNIQUE,
                client_version TEXT,
                last_seen_at INTEGER,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                deactivated_at INTEGER,
                ping_interval_seconds INTEGER
            )",
        )
        .await
        .unwrap();

        db.execute_unprepared(
            "CREATE TABLE service_hosts (
                service_id TEXT NOT NULL,
                host_id TEXT NOT NULL,
                linked_at INTEGER NOT NULL,
                PRIMARY KEY (service_id, host_id)
            )",
        )
        .await
        .unwrap();

        db.execute_unprepared(
            "CREATE TABLE settings_version (
                tenant_id TEXT PRIMARY KEY,
                version INTEGER NOT NULL,
                global_version INTEGER NOT NULL,
                revocation_version INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            )",
        )
        .await
        .unwrap();

        db
    }

    async fn test_state(db: DatabaseConnection, tenant_id: uuid::Uuid) -> Arc<AppState> {
        struct NoopCertSigner;
        #[async_trait::async_trait]
        impl AgentCertSigner for NoopCertSigner {
            async fn sign_agent_csr(
                &self,
                _: &str,
                _: &uuid::Uuid,
                _: time::Duration,
            ) -> std::result::Result<SignedCertBundle, rootcause::Report<CertSignerError>>
            {
                Err(rootcause::Report::new(CertSignerError::Signing(
                    "noop signer".to_string(),
                )))
            }
            fn active_ca_fingerprint(&self) -> String {
                "0".repeat(64)
            }
        }

        let ca_pem = "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n";
        let snapshot_data = crate::ca_snapshot::CaPublicSnapshot {
            active_cert_pem: ca_pem.to_string(),
            active_fingerprint: "0".repeat(64),
            previous_cert_pem: None,
            previous_fingerprint: None,
            trusted_cas: vec![crate::ca_snapshot::TrustedCaPublic {
                cert_pem: ca_pem.to_string(),
                fingerprint: "0".repeat(64),
                not_after: time::OffsetDateTime::now_utc() + time::Duration::days(365),
            }],
            trusted_ca_cns: Vec::new(),
            bundle_pem: ca_pem.to_string(),
            bundle_hash: "0".repeat(64),
            managed: true,
            active_not_after: time::OffsetDateTime::now_utc() + time::Duration::days(365),
            pki_addr: None,
        };
        let (_ca_tx, ca_rx) = tokio::sync::watch::channel(snapshot_data);
        let ca_key_store: crate::CaKeyStoreRef =
            Arc::new(tokio::sync::RwLock::new(crate::ca_snapshot::CaKeyStore {
                active_key_pem: zeroize::Zeroizing::new(String::new()),
                previous_key_pem: None,
                trusted_ca_keys: vec![],
            }));

        let rustls_cfg = {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
            let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
            let cert = rcgen::CertificateParams::new(vec!["localhost".into()])
                .unwrap()
                .self_signed(&key_pair)
                .unwrap();
            let server_config = rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(
                    vec![rustls::pki_types::CertificateDer::from(cert.der().to_vec())],
                    rustls::pki_types::PrivateKeyDer::try_from(key_pair.serialize_der()).unwrap(),
                )
                .unwrap();
            axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(server_config))
        };

        let notification_service = crate::notification_service::NotificationService::new(
            db.clone(),
            crate::service_connections::ServiceConnectionRegistry::new(),
            uuid::Uuid::nil(),
        );

        Arc::new(AppState {
            ca_snapshot: ca_rx,
            ca_key_store,
            #[cfg(feature = "oidc")]
            oidc_flow_store: crate::auth::oidc_state::OidcFlowStore::new(db.clone()),
            #[cfg(feature = "oidc")]
            account_link_store: crate::auth::oidc_state::AccountLinkStore::new(db.clone()),
            #[cfg(feature = "oidc")]
            oidc_token_exchange_store: crate::auth::oidc_state::OidcTokenExchangeStore::new(
                db.clone(),
            ),
            #[cfg(feature = "oidc")]
            oidc_registration_store: crate::auth::oidc_state::OidcRegistrationStore::new(
                db.clone(),
            ),
            device_flow_store: crate::auth::device_flow::DeviceFlowStore::new(db.clone()),
            rate_limit_store: crate::auth::rate_limit::RateLimitStore::new(db.clone()),
            db,
            default_tenant_id: tenant_id,
            settings: Settings::new(
                RegistrationSettings {
                    mode: RegistrationMode::Open,
                    token_hash: None,
                    require_token_for_oidc: false,
                },
                7,
            ),
            cert_signer: Arc::new(NoopCertSigner),
            service_connections: crate::service_connections::ServiceConnectionRegistry::new(),
            revocation_notify: Arc::new(tokio::sync::Notify::const_new()),
            jwt: Arc::new(crate::auth::jwt::JwtManager::from_secret(
                b"test-secret-for-service-merge-tests",
            )),
            pki_path: std::path::PathBuf::from("/tmp/test-pki"),
            rustls_config: rustls_cfg,
            crl_pem_cache: Arc::new(tokio::sync::RwLock::new(String::new())),
            ca_rotation_trigger: Arc::new(tokio::sync::Notify::const_new()),
            controller_id: uuid::Uuid::nil(),
            notification_service,
            token_denylist: Arc::new(crate::auth::token_denylist::TokenDenylist::new()),
            provider_ops: Arc::new(uptrakit_provider_registry::ProviderRegistry),
        })
    }

    #[tokio::test]
    async fn merge_service_rolls_back_on_failure() {
        let db = setup_test_db().await;
        let tenant_id = uuid::Uuid::now_v7();
        let state = test_state(db.clone(), tenant_id).await;

        let now = OffsetDateTime::now_utc();
        let target = service::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            service_type: Set(service::ServiceType::Agent),
            hostname: Set("target-host".to_string()),
            friendly_name: Set("Target".to_string()),
            ip_address: Set(None),
            status: Set(service::ServiceStatus::Approved),
            enrollment_secret_hash: Set("target-hash".to_string()),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
        };
        let target = target.insert(&db).await.unwrap();

        let source = service::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            service_type: Set(service::ServiceType::Agent),
            hostname: Set("source-host".to_string()),
            friendly_name: Set("Source".to_string()),
            ip_address: Set(None),
            status: Set(service::ServiceStatus::Pending),
            enrollment_secret_hash: Set("source-hash".to_string()),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
        };
        let source = source.insert(&db).await.unwrap();

        let auth_user = AuthenticatedUser {
            user_id: uuid::Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::ManageAgents],
        };

        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

        let response = merge_service(
            State(Arc::clone(&state)),
            tenant_db,
            CanManageAgents::new(auth_user),
            Path(target.id.to_string()),
            Json(MergeAgentRequest {
                source_id: source.id,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let source_after = Service::find_by_id(source.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert!(source_after.deactivated_at.is_none());
        assert_eq!(source_after.enrollment_secret_hash, "source-hash");
    }
}
