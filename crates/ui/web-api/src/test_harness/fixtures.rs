//! Reusable data-insertion helpers and HTTP convenience wrappers for tests.

use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set,
};
use uptrakit_shared_db::entity::{host, permission, role, role_permission, service, service_host};
use uptrakit_web_api_types::SecretString;
use uptrakit_web_api_types::auth::{AuthResponse, LoginRequest, RefreshResponse, RegisterRequest};

use super::http_client::TestClient;

// ── HTTP helpers ────────────────────────────────────────────────────────

/// Register a user via HTTP and return the full [`AuthResponse`].
pub async fn register_user(
    client: &TestClient,
    email: &str,
    password: &str,
) -> (http::StatusCode, AuthResponse) {
    let req = RegisterRequest {
        email: email.to_string(),
        first_name: "Test".to_string(),
        last_name: "User".to_string(),
        password: SecretString::new(password.to_string()),
        registration_token: None,
    };
    client.post_json("/api/v1/auth/register", &req).send_json().await
}

/// Register a user and return just the access token (panics on failure).
pub async fn register_and_get_token(client: &TestClient) -> String {
    let (status, auth) = register_user(client, "owner@test.local", "TestPassword123!").await;
    assert_eq!(status, http::StatusCode::CREATED, "registration failed");
    auth.access_token.expose_secret().to_string()
}

/// Login a user via HTTP and return the full [`AuthResponse`].
pub async fn login_user(
    client: &TestClient,
    email: &str,
    password: &str,
) -> (http::StatusCode, AuthResponse) {
    let req = LoginRequest {
        email: email.to_string(),
        password: SecretString::new(password.to_string()),
    };
    client.post_json("/api/v1/auth/login", &req).send_json().await
}

/// Refresh a token via HTTP.
pub async fn refresh_token(
    client: &TestClient,
    refresh: &str,
) -> (http::StatusCode, RefreshResponse) {
    let body = serde_json::json!({ "refresh_token": refresh });
    client.post_json("/api/v1/auth/refresh", &body).send_json().await
}

// ── Direct DB helpers ───────────────────────────────────────────────────

/// Insert a service entity directly in the database.
pub async fn insert_service(
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
    }
    .insert(db)
    .await
    .expect("insert service")
}

/// Insert a host entity directly in the database.
pub async fn insert_host(db: &DatabaseConnection, tenant_id: uuid::Uuid) -> host::Model {
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
        last_seen_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    }
    .insert(db)
    .await
    .expect("insert host")
}

/// Seed permissions that are not yet in the initial migration and assign them
/// to the `owner` role.  This lets integration tests exercise endpoints that
/// require these permissions without altering the production migration chain.
pub async fn seed_permissions_for_owner(db: &DatabaseConnection, names: &[&str]) {
    let owner_role = role::Entity::find()
        .filter(role::Column::Name.eq("owner"))
        .one(db)
        .await
        .expect("find owner role")
        .expect("owner role must exist");

    let now = time::OffsetDateTime::now_utc();
    for name in names {
        // Skip if already seeded by a migration.
        let existing = permission::Entity::find()
            .filter(permission::Column::Name.eq(*name))
            .one(db)
            .await
            .expect("query permission");
        let perm_id = if let Some(p) = existing {
            p.id
        } else {
            let id = uuid::Uuid::now_v7();
            permission::ActiveModel {
                id: Set(id),
                name: Set(name.to_string()),
                description: Set(Some(name.to_string())),
                created_at: Set(now),
            }
            .insert(db)
            .await
            .expect("insert permission");
            id
        };

        // Link to owner role (ignore if already linked).
        let link_exists = role_permission::Entity::find_by_id((owner_role.id, perm_id))
            .one(db)
            .await
            .expect("query role_permission");
        if link_exists.is_none() {
            role_permission::ActiveModel {
                role_id: Set(owner_role.id),
                permission_id: Set(perm_id),
            }
            .insert(db)
            .await
            .expect("insert role_permission");
        }
    }
}

/// Link a service to a host via the join table.
pub async fn link_service_host(
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
