//! Reusable data-insertion helpers and HTTP convenience wrappers for tests.

#![expect(
    clippy::expect_used,
    reason = "test fixture: panics on setup failure are acceptable"
)]
#![expect(
    clippy::string_slice,
    reason = "test code: slice indexes are at validated boundaries"
)]
#![expect(
    clippy::panic,
    reason = "test fixture: panics on setup failure are acceptable"
)]

use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use uptrakit_shared_db::access_grants::{GrantSubject, delete_grant, load_grants_for_principal};
use uptrakit_shared_db::entity::{
    host, oauth_client, permission, role, role_permission, service, service_host, user_role,
};
use uptrakit_web_api_types::SecretString;
use uptrakit_web_api_types::auth::{AuthResponse, LoginRequest, RefreshResponse, RegisterRequest};

use super::http_client::TestClient;

/// The id of the tenant seeded by the migrations.
///
/// Pairs with [`super::setup_migrated_db`] for engine-level tests that need
/// a tenant id but not the full [`super::TestApp`] HTTP stack.
pub(crate) async fn default_tenant_id(db: &DatabaseConnection) -> uuid::Uuid {
    uptrakit_shared_db::entity::tenant::Entity::find()
        .one(db)
        .await
        .expect("query tenant")
        .expect("seeded default tenant")
        .id
}

// ── HTTP helpers ────────────────────────────────────────────────────────

/// Register a user via HTTP and return the full [`AuthResponse`].
pub(crate) async fn register_user(
    client: &TestClient,
    email: &str,
    password: &str,
) -> (http::StatusCode, AuthResponse) {
    let req = RegisterRequest {
        email: email.to_string(),
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

/// Register the admin (first user), re-open registration, then register a
/// second user who gets the built-in "user" role (ViewSettings but NOT
/// ManageGlobalSettings).  Returns `(admin_token, tenant_token)`.
pub(crate) async fn register_admin_and_tenant_user(app: &super::TestApp) -> (String, String) {
    let client = app.client();

    // First user → owner role (all permissions including ManageGlobalSettings).
    let (status, admin_auth) = register_user(&client, "owner@test.local", "TestPassword123!").await;
    assert_eq!(
        status,
        http::StatusCode::CREATED,
        "admin registration failed"
    );
    let admin_token = admin_auth.access_token.expose_secret().to_string();

    // Re-open registration so the second user can sign up.
    let reopen = client
        .put_json(
            "/api/v1/settings/access",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(&admin_token)
        .header("if-match", "W/\"settings-v0\"")
        .send_status()
        .await;
    assert_eq!(
        reopen,
        http::StatusCode::OK,
        "failed to re-open registration"
    );

    // Second user → built-in "user" role: ViewSettings but NOT ManageGlobalSettings.
    let (status, tenant_auth) =
        register_user(&client, "tenant@test.local", "TestPassword123!").await;
    assert_eq!(
        status,
        http::StatusCode::CREATED,
        "tenant user registration failed"
    );
    let tenant_token = tenant_auth.access_token.expose_secret().to_string();

    (admin_token, tenant_token)
}

/// Login a user via HTTP and return the full [`AuthResponse`].
pub(crate) async fn login_user(
    client: &TestClient,
    email: &str,
    password: &str,
) -> (http::StatusCode, AuthResponse) {
    let req = LoginRequest {
        email: email.to_string(),
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

/// Insert an embedded service entity directly in the database.
pub(crate) async fn insert_embedded_service(
    db: &DatabaseConnection,
    tenant_id: uuid::Uuid,
) -> service::Model {
    let id = uuid::Uuid::now_v7();
    let owner_key = uuid::Uuid::now_v7();
    let now = time::OffsetDateTime::now_utc();
    service::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant_id),
        capabilities: Set("[]".to_string()),
        hostname: Set(format!("embedded-{}", &id.to_string()[..8])),
        friendly_name: Set(format!("Embedded Service {}", &id.to_string()[..8])),
        ip_address: Set(None),
        status: Set(service::ServiceStatus::Approved),
        enrollment_secret_hash: Set(format!("embedded:{id}")),
        client_version: Set(None),
        last_seen_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
        ping_interval_seconds: Set(None),
        enrollment_token_id: Set(None),
        cert_lifetime_hours: Set(None),
        service_app_name: Set(Some("uptrakit-agent".to_string())),
        is_embedded: Set(true),
        embedded_owner_key: Set(Some(owner_key)),
    }
    .insert(db)
    .await
    .expect("insert embedded service")
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

/// Ensure the named permissions exist in the database and are linked to
/// at least one role that the first registered user holds.
///
/// After the granular permissions migration all 32 permissions are already
/// seeded with correct role assignments, so this is effectively a no-op in
/// normal circumstances. It is kept for backwards-compatibility with tests
/// that call it and as a safety net if a permission somehow wasn't seeded.
pub(crate) async fn seed_permissions_for_owner(db: &DatabaseConnection, names: &[&str]) {
    // Use the first built-in role we find — after migration the first
    // registered user holds all 8 roles, so any role will do.
    let any_role = role::Entity::find()
        .filter(role::Column::IsBuiltIn.eq(true))
        .one(db)
        .await
        .expect("find built-in role")
        .expect("at least one built-in role must exist");

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

        // Link to a role (ignore if already linked).
        let link_exists = role_permission::Entity::find_by_id((any_role.id, perm_id))
            .one(db)
            .await
            .expect("query role_permission");
        if link_exists.is_none() {
            role_permission::ActiveModel {
                role_id: Set(any_role.id),
                permission_id: Set(perm_id),
            }
            .insert(db)
            .await
            .expect("insert role_permission");
        }
    }
}

/// Upsert a row into `instance_plugin_setting` AND publish the new snapshot
/// to the in-memory `state.instance_plugin_snapshot` ArcSwap so that the
/// next request reads the seeded value. The catalog snapshot (frozen at
/// `TestApp::new()` boot) is intentionally NOT touched; `running_enabled`
/// reflects boot state and is independent of the ArcSwap snapshot.
#[cfg(feature = "dashboard-icons")]
pub(crate) async fn upsert_instance_plugin_setting(
    app: &super::TestApp,
    plugin_type_id: &str,
    enabled: bool,
) -> uptrakit_shared_db::entity::instance_plugin_setting::Model {
    let (_previous, model) = uptrakit_web_api_queries::instance_plugin_settings::set_enabled(
        app.state.db(),
        plugin_type_id,
        enabled,
    )
    .await
    .expect("set_enabled fixture");
    // Publish updated snapshot.
    let current = app.state.instance_plugin_snapshot.load();
    let next = current.with_upserted(
        model.plugin_type_id.clone(),
        uptrakit_web_api_queries::instance_plugin_settings::InstancePluginRow {
            enabled: model.enabled,
            config: model.config.clone(),
            updated_at: model.updated_at,
        },
    );
    app.state
        .instance_plugin_snapshot
        .store(std::sync::Arc::new(next));
    model
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

/// Insert an `oauth_client` row and return its `client_id`.
///
/// When `trusted` is `true`, `trusted_at` is set to the current timestamp.
pub(crate) async fn insert_oauth_client(
    db: &DatabaseConnection,
    redirect_uri: &str,
    trusted: bool,
) -> String {
    let now = time::OffsetDateTime::now_utc();
    let client_id = format!("test-consent-client-{}", uuid::Uuid::now_v7());
    let redirect_uris_json =
        serde_json::to_string(&vec![redirect_uri]).expect("serialize redirect_uris");

    oauth_client::ActiveModel {
        id: Set(client_id.clone()),
        client_name: Set("Consent Test Client".to_string()),
        client_uri: Set(Some("https://example.com".to_string())),
        logo_uri: Set(None),
        redirect_uris: Set(redirect_uris_json),
        default_scope: Set("mcp:read".to_string()),
        grant_types: Set("authorization_code".to_string()),
        response_types: Set("code".to_string()),
        token_endpoint_auth_method: Set("none".to_string()),
        client_secret_hash: Set(None),
        registration_access_token_hash: Set(None),
        created_via: Set("test".to_string()),
        created_at: Set(now),
        last_used_at: Set(None),
        revoked_at: Set(None),
        metadata_cached_at: Set(None),
        metadata_etag: Set(None),
        metadata_content_hash: Set(None),
        metadata_raw: Set(None),
        metadata_parse_error: Set(None),
        metadata_parse_error_at: Set(None),
        trusted_at: Set(if trusted { Some(now) } else { None }),
    }
    .insert(db)
    .await
    .expect("insert oauth_client");

    client_id
}

// ── M1.5 access-engine fixtures ─────────────────────────────────────────

/// Register the first user (owner) and re-open registration so a second
/// user can sign up. Returns the owner's access token.
pub(crate) async fn open_registration(app: &super::TestApp) -> String {
    let client = app.client();
    let owner_token = register_and_get_token(&client).await;
    let reopen = client
        .put_json(
            "/api/v1/settings/access",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(&owner_token)
        .header("if-match", "W/\"settings-v0\"")
        .send_status()
        .await;
    assert_eq!(
        reopen,
        http::StatusCode::OK,
        "failed to re-open registration"
    );
    owner_token
}

/// Id of a seeded built-in role, by name.
pub(crate) async fn role_id_by_name(app: &super::TestApp, role_name: &str) -> uuid::Uuid {
    role::Entity::find()
        .filter(role::Column::Name.eq(role_name))
        .one(&app.db)
        .await
        .expect("query roles")
        .unwrap_or_else(|| panic!("seeded `{role_name}` role must exist"))
        .id
}

/// Link `role_id` to `user_id` and invalidate the user's cached authority.
pub(crate) async fn link_role(app: &super::TestApp, user_id: uuid::Uuid, role_id: uuid::Uuid) {
    user_role::ActiveModel {
        tenant_id: Set(app.tenant_id),
        user_id: Set(user_id),
        role_id: Set(role_id),
        assigned_at: Set(time::OffsetDateTime::now_utc()),
    }
    .insert(&app.db)
    .await
    .expect("assign role");
    app.state.access_engine.invalidate_subjects(&[user_id], &[]);
}

/// Register a fresh user (registration must already be open), strip its
/// auto-assigned `viewer` role, link ONLY `role_name`, invalidate the engine
/// cache, then re-login so the legacy JWT claim snapshot reflects the newly
/// linked role's legacy permission set. Returns `(user_id, access_token)`.
pub(crate) async fn register_user_with_only_role(
    app: &super::TestApp,
    email: &str,
    role_name: &str,
) -> (uuid::Uuid, String) {
    let client = app.client();
    let (status, auth) = register_user(&client, email, "TestPassword123!").await;
    assert_eq!(
        status,
        http::StatusCode::CREATED,
        "user registration failed"
    );
    let user_id = auth.user.id;

    user_role::Entity::delete_many()
        .filter(user_role::Column::UserId.eq(user_id))
        .exec(&app.db)
        .await
        .expect("strip auto-assigned viewer role");

    let role_id = role_id_by_name(app, role_name).await;
    link_role(app, user_id, role_id).await;

    let (login_status, login_auth) = login_user(&client, email, "TestPassword123!").await;
    assert_eq!(login_status, http::StatusCode::OK, "re-login failed");
    (user_id, login_auth.access_token.expose_secret().to_string())
}

/// Owner + a fresh second user holding ONLY `role_name`. Returns
/// `(user_id, access_token)` for the second user.
pub(crate) async fn stage_user_with_only_role(
    app: &super::TestApp,
    role_name: &str,
) -> (uuid::Uuid, String) {
    open_registration(app).await;
    let email = format!("{role_name}-only@test.local");
    register_user_with_only_role(app, &email, role_name).await
}

/// Owner + a fresh second user with its auto-assigned `viewer` role
/// stripped and no replacement linked. Returns `(user_id, access_token)`.
pub(crate) async fn stage_zero_role_user(app: &super::TestApp) -> (uuid::Uuid, String) {
    let client = app.client();
    open_registration(app).await;
    let (status, auth) = register_user(&client, "zero-role@test.local", "TestPassword123!").await;
    assert_eq!(
        status,
        http::StatusCode::CREATED,
        "user registration failed"
    );
    let user_id = auth.user.id;
    user_role::Entity::delete_many()
        .filter(user_role::Column::UserId.eq(user_id))
        .exec(&app.db)
        .await
        .expect("strip auto-assigned roles");
    app.state.access_engine.invalidate_subjects(&[user_id], &[]);
    (user_id, auth.access_token.expose_secret().to_string())
}

/// Delete every `access_grants` row held by `role_id` whose patterns cover
/// any of `covered`, then invalidate the role's cached authority.
///
/// Deleting all matching rows (not `.one()`) is what makes the
/// engine-vs-legacy-claim tests discriminating: a seeded role carries an
/// M1.2 grant plus later backfills, and one surviving row would keep the
/// action allowed while the JWT claim stayed identical.
pub(crate) async fn revoke_role_grants_covering(
    app: &super::TestApp,
    role_id: uuid::Uuid,
    covered: &[uptrakit_shared_types::access::Action],
) {
    let load = load_grants_for_principal(&app.db, app.tenant_id, uuid::Uuid::nil(), &[role_id])
        .await
        .expect("load role grants");
    let mut deleted_any = false;
    for grant in load.grants {
        if grant.subject == GrantSubject::Role(role_id)
            && grant
                .patterns
                .iter()
                .any(|pattern| covered.iter().any(|action| pattern.matches(action)))
        {
            delete_grant(&app.db, grant.id)
                .await
                .expect("delete covering grant");
            deleted_any = true;
        }
    }
    assert!(
        deleted_any,
        "expected at least one grant row covering {covered:?}"
    );
    app.state.access_engine.invalidate_subjects(&[], &[role_id]);
}
