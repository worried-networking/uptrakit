#![expect(
    clippy::expect_used,
    clippy::string_slice,
    reason = "integration test infrastructure: panics are acceptable in fixture helpers"
)]

use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};
use uptrakit_shared_db::entity::{host, service, service_host};
use uptrakit_web_api_types::SecretString;
use uptrakit_web_api_types::auth::{AuthResponse, LoginRequest, RefreshResponse, RegisterRequest};

use super::http_client::TestClient;

// ── HTTP helpers ────────────────────────────────────────────────────────

/// Register a user via HTTP and return the full [`AuthResponse`].
pub(crate) async fn register_user(
    client: &TestClient,
    email: &str,
    password: &str,
) -> (http::StatusCode, AuthResponse) {
    let req = RegisterRequest {
        email: email.parse().expect("valid test email"),
        first_name: "Test".to_string(),
        last_name: "User".to_string(),
        password: SecretString::new(password),
        registration_token: None,
    };
    client
        .post_json("/api/v1/auth/register", &req)
        .send_json()
        .await
}

/// Register a user and return just the access token (panics on failure).
pub(crate) async fn register_and_get_token(client: &TestClient) -> String {
    let (status, auth) = register_user(client, "owner@test.local", "TestPassword123!").await;
    assert_eq!(status, http::StatusCode::CREATED, "registration failed");
    auth.access_token.expose_secret().to_string()
}

/// GET `/api/v1/settings/access` and return the `ETag` response header.
///
/// Used by tests that need to call mutation endpoints protected by
/// `IfMatch<SettingsVersion>`.
pub(crate) async fn get_settings_etag(client: &TestClient, token: &str) -> String {
    let resp = client
        .get("/api/v1/settings/access")
        .bearer(token)
        .send()
        .await;
    resp.headers()
        .get("etag")
        .expect("ETag header must be present on GET /api/v1/settings/access")
        .to_str()
        .expect("ETag is ASCII")
        .to_string()
}

/// Login a user via HTTP and return the full [`AuthResponse`].
pub(crate) async fn login_user(
    client: &TestClient,
    email: &str,
    password: &str,
) -> (http::StatusCode, AuthResponse) {
    let req = LoginRequest {
        email: email.parse().expect("valid test email"),
        password: SecretString::new(password),
    };
    client
        .post_json("/api/v1/auth/login", &req)
        .send_json()
        .await
}

/// Refresh a token via HTTP.
pub(crate) async fn refresh_token(
    client: &TestClient,
    refresh: &str,
) -> (http::StatusCode, RefreshResponse) {
    let body = serde_json::json!({ "refresh_token": refresh });
    client
        .post_json("/api/v1/auth/refresh", &body)
        .send_json()
        .await
}

// ── Direct DB helpers ───────────────────────────────────────────────────

/// Insert a service entity directly in the database.
pub(crate) async fn insert_service(
    db: &DatabaseConnection,
    tenant_id: uuid::Uuid,
    status: service::ServiceStatus,
) -> service::Model {
    let id = uuid::Uuid::now_v7();
    let now = time::OffsetDateTime::now_utc();
    service::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant_id),
        capabilities: Set("[]".to_string()),
        hostname: Set(format!("host-{}", &id.to_string()[..8])),
        friendly_name: Set(format!("Service {}", &id.to_string()[..8])),
        ip_address: Set(Some("10.0.0.1".to_string())),
        status: Set(status),
        enrollment_secret_hash: Set(format!("secret-{id}")),
        client_version: Set(None),
        last_seen_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
        ping_interval_seconds: Set(None),
        enrollment_token_id: Set(None),
        cert_lifetime_hours: Set(None),
        service_app_name: Set(None),
        is_embedded: Set(false),
        embedded_owner_key: Set(None),
    }
    .insert(db)
    .await
    .expect("insert service")
}

/// Insert a host entity directly in the database.
pub(crate) async fn insert_host(db: &DatabaseConnection, tenant_id: uuid::Uuid) -> host::Model {
    let id = uuid::Uuid::now_v7();
    let now = time::OffsetDateTime::now_utc();
    host::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant_id),
        machine_id: Set(format!("machine-{}", &id.to_string()[..8])),
        hostname: Set(format!("host-{}", &id.to_string()[..8])),
        friendly_name: Set(format!("Host {}", &id.to_string()[..8])),
        os_type: Set(Some("linux".to_string())),
        os_version: Set(Some("Ubuntu 22.04".to_string())),
        architecture: Set(Some("x86_64".to_string())),
        ip_address: Set(Some("10.0.0.2".to_string())),
        host_features: Set(None),
        last_seen_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    }
    .insert(db)
    .await
    .expect("insert host")
}

/// Link a service to a host via the join table.
pub(crate) async fn link_service_host(
    db: &DatabaseConnection,
    service_id: uuid::Uuid,
    host_id: uuid::Uuid,
) {
    let now = time::OffsetDateTime::now_utc();
    service_host::ActiveModel {
        service_id: Set(service_id),
        host_id: Set(host_id),
        linked_at: Set(now),
    }
    .insert(db)
    .await
    .expect("link service_host");
}
