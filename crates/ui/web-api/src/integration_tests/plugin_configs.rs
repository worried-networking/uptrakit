#![expect(
    clippy::expect_used,
    reason = "test code: panics on failure are acceptable"
)]
#![expect(clippy::panic, reason = "test code: panics on failure are acceptable")]
#![expect(
    clippy::string_slice,
    reason = "test code: slice indexes are at validated boundaries"
)]

use crate::test_harness::TestApp;
use crate::test_harness::fixtures::{
    insert_host, link_service_host, open_registration, register_and_get_token, register_user,
    revoke_role_grants_covering, role_id_by_name, stage_user_with_only_role, stage_zero_role_user,
};
#[cfg(feature = "dashboard-icons")]
use sea_orm::{ActiveModelTrait, Set};
#[cfg(not(feature = "dashboard-icons"))]
use sea_orm::{ActiveModelTrait, Set};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use std::collections::BTreeSet;
use uptrakit_shared_db::entity::audit_log;
use uptrakit_shared_db::entity::plugin_config;
use uptrakit_shared_db::entity::{service, user_role};
use uptrakit_shared_types::access::actions;
use uptrakit_wire::{ControllerMessage, TestPluginConfigResultPayload};
use uuid::Uuid;

async fn tenant_audit_row_for_action(
    db: &sea_orm::DatabaseConnection,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
) -> audit_log::Model {
    for _ in 0..50 {
        if let Some(row) = audit_log::Entity::find()
            .filter(audit_log::Column::ActionType.eq(action_type))
            .order_by_desc(audit_log::Column::OccurredAt)
            .one(db)
            .await
            .expect("query audit rows")
        {
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    panic!("expected tenant audit row");
}

#[tokio::test]
async fn list_plugin_types_returns_200() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/plugin-types")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    // The response is an array of plugin type metadata.
    assert!(body.as_array().is_some());
    assert!(
        !body.as_array().expect("array").is_empty(),
        "at least one plugin type should be registered"
    );
}

/// M1.5 OR-gate weaker arm: a `settings_manager`-only principal (seed grant
/// covers only `settings:read`, never `software:read` or
/// `system.settings:manage`) must still be able to list plugin types.
#[tokio::test]
async fn list_plugin_types_allows_view_settings_without_view_software() {
    let app = TestApp::new().await;
    let (_user_id, token) = stage_user_with_only_role(&app, "settings_manager").await;

    let (status, body): (_, serde_json::Value) = app
        .client()
        .get("/api/v1/plugin-types")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert!(
        body.as_array().is_some(),
        "plugin types response should be array"
    );
}

/// A `system_administrator`-only principal (seed grant `system.*:*`, which
/// covers `system.settings:manage` but neither `settings:read` nor
/// `software:read`) must still be able to list plugin types.
#[tokio::test]
async fn list_plugin_types_allows_manage_global_settings() {
    let app = TestApp::new().await;
    let (_user_id, token) = stage_user_with_only_role(&app, "system_administrator").await;

    let (status, body): (_, serde_json::Value) = app
        .client()
        .get("/api/v1/plugin-types")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert!(
        body.as_array().is_some(),
        "plugin types response should be array"
    );
}

/// Same OR-gate arm proof as above, for the `/api/v1/plugin-type-settings`
/// list route.
#[tokio::test]
async fn list_plugin_type_settings_allows_manage_global_settings() {
    let app = TestApp::new().await;
    let (_user_id, token) = stage_user_with_only_role(&app, "system_administrator").await;

    let (status, body): (_, serde_json::Value) = app
        .client()
        .get("/api/v1/plugin-type-settings")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert!(
        body.as_array().is_some(),
        "plugin type settings response should be array"
    );
}

/// A principal holding zero role links (and therefore zero `access_grants`
/// coverage) must be denied.
#[tokio::test]
async fn zero_role_user_cannot_list_plugin_types() {
    let app = TestApp::new().await;
    let (_user_id, token) = stage_zero_role_user(&app).await;

    let status = app
        .client()
        .get("/api/v1/plugin-types")
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(
        status,
        http::StatusCode::FORBIDDEN,
        "zero-role user must be denied the plugin types list"
    );
}

/// Regression test: a `viewer` principal's role link is left untouched, but
/// every `access_grants` row covering `software:read` / `settings:read` /
/// `system.settings:manage` for that role is deleted and the cache
/// invalidated (a role may hold more than one seed grant row, so this loops
/// rather than calling `.one()`). The deny must be grant-scoped: still being
/// linked to the `viewer` role must not itself grant access once its
/// covering grants are gone.
#[tokio::test]
async fn plugin_types_list_forbidden_after_viewer_grants_revoked() {
    let app = TestApp::new().await;
    let client = app.client();
    open_registration(&app).await;

    let (status, auth) =
        register_user(&client, "viewer-stripped@test.local", "TestPassword123!").await;
    assert_eq!(
        status,
        http::StatusCode::CREATED,
        "user registration failed"
    );
    let token = auth.access_token.expose_secret().to_string();

    let viewer_role_id = role_id_by_name(&app, "viewer").await;

    let user_has_viewer_role = user_role::Entity::find()
        .filter(user_role::Column::UserId.eq(auth.user.id))
        .filter(user_role::Column::RoleId.eq(viewer_role_id))
        .one(&app.db)
        .await
        .expect("query user_role")
        .is_some();
    assert!(
        user_has_viewer_role,
        "staged user must actually be linked to the viewer role for this test's grant-deletion \
         to be a meaningful denial, not a 403 for the wrong reason"
    );

    revoke_role_grants_covering(
        &app,
        viewer_role_id,
        &[
            actions::SOFTWARE_READ,
            actions::SETTINGS_READ,
            actions::SYSTEM_SETTINGS_MANAGE,
        ],
    )
    .await;

    let status = app
        .client()
        .get("/api/v1/plugin-types")
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(
        status,
        http::StatusCode::FORBIDDEN,
        "engine must deny the plugin types list once the covering grant is revoked"
    );
}

#[tokio::test]
async fn create_config_returns_201() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs",
            &serde_json::json!({
                "name": "My GitHub Config",
                "plugin_type": "releases.github",
                "config": {}
            }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::CREATED);
    assert!(body["id"].as_str().is_some());
    assert_eq!(body["name"], "My GitHub Config");

    let row = tenant_audit_row_for_action(
        &app.db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
    )
    .await;
    assert_eq!(
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
        row.action_type
    );
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("plugin_config"));
    assert_eq!(row.target_display.as_deref(), Some("My GitHub Config"));
    let details = row.details_json.expect("details");
    assert_eq!(details["plugin_type"], serde_json::json!("releases.github"));
    assert_eq!(
        details["config_name"],
        serde_json::json!("My GitHub Config")
    );
    assert_eq!(details["enabled"], serde_json::json!(true));
    assert_eq!(details["contains_command_fields"], serde_json::json!(false));
}

#[tokio::test]
async fn update_config_returns_200_and_writes_audit_event() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (_, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs",
            &serde_json::json!({
                "name": "Mutable Config",
                "plugin_type": "releases.github",
                "config": {}
            }),
        )
        .bearer(&token)
        .send_json()
        .await;

    let id = created["id"].as_str().expect("id");
    let (status, body): (_, serde_json::Value) = client
        .put_json(
            &format!("/api/v1/plugin-configs/{id}"),
            &serde_json::json!({
                "name": "Mutable Config Updated",
                "enabled": false
            }),
        )
        .bearer(&token)
        .header("if-match", &current_tenant_etag(&client, &token).await)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["name"], "Mutable Config Updated");
    assert_eq!(body["enabled"], false);

    let row = tenant_audit_row_for_action(
        &app.db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_UPDATE,
    )
    .await;
    assert_eq!(
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_UPDATE,
        row.action_type
    );
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("plugin_config"));
    assert_eq!(row.target_id.as_deref(), Some(id));
    assert_eq!(
        row.target_display.as_deref(),
        Some("Mutable Config Updated")
    );
    let details = row.details_json.expect("details");
    assert_eq!(details["plugin_type"], serde_json::json!("releases.github"));
    assert_eq!(
        details["config_name"],
        serde_json::json!("Mutable Config Updated")
    );
    assert_eq!(details["enabled"], serde_json::json!(false));
    assert_eq!(details["contains_command_fields"], serde_json::json!(false));
}

/// Regression test (task 6a spec §5): the frontend submits the FULL auth
/// object shape on every save, including keys from the previously-selected
/// variant carrying the masked sentinel. A Basic→Bearer switch must not let
/// the stale `auth.password` survive `restore_config_secrets`' sentinel
/// refill — the write-path prune must strip it before persistence.
#[tokio::test]
async fn docker_basic_to_bearer_switch_drops_stale_password() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (create_status, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs",
            &serde_json::json!({
                "name": "Docker Registry",
                "plugin_type": "releases.docker",
                "config": {
                    "auth": {"type": "basic", "username": "u", "password": "pw-1"}
                }
            }),
        )
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(create_status, http::StatusCode::CREATED);
    let id = created["id"].as_str().expect("id");

    let (update_status, _body): (_, serde_json::Value) = client
        .put_json(
            &format!("/api/v1/plugin-configs/{id}"),
            &serde_json::json!({
                "config": {
                    "auth": {
                        "type": "bearer",
                        "token": "tok-2",
                        "password": "***",
                        "username": "u"
                    }
                }
            }),
        )
        .bearer(&token)
        .header("if-match", &current_tenant_etag(&client, &token).await)
        .send_json()
        .await;
    assert_eq!(update_status, http::StatusCode::OK);

    let stored = plugin_config::Entity::find_by_id(Uuid::parse_str(id).expect("uuid"))
        .one(&app.db)
        .await
        .expect("query plugin_config")
        .expect("plugin_config row exists");

    let auth = stored
        .config
        .as_json()
        .get("auth")
        .expect("auth key present");
    assert!(
        auth.get("password").is_none(),
        "stale Basic password must be pruned after the Basic->Bearer switch, got: {auth:?}"
    );
    assert_eq!(auth["token"], serde_json::json!("tok-2"));
}

/// Regression test (task 6a spec §5): stored config JSON must stay sparse
/// after an update — the legacy typed round-trip materialized every
/// `#[serde(default)]` field (`include_prereleases`, `tag_strip_prefix`,
/// `verify_attestation`, `make_executable`) even when the request never sent
/// them. Only fields the caller actually sent may end up in the stored row.
#[tokio::test]
async fn github_update_stays_sparse_no_materialized_defaults() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (create_status, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs",
            &serde_json::json!({
                "name": "Sparse GitHub Config",
                "plugin_type": "releases.github",
                "config": {}
            }),
        )
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(create_status, http::StatusCode::CREATED);
    let id = created["id"].as_str().expect("id");

    let (update_status, _body): (_, serde_json::Value) = client
        .put_json(
            &format!("/api/v1/plugin-configs/{id}"),
            &serde_json::json!({
                "config": {"tag_strip_prefix": "release-"}
            }),
        )
        .bearer(&token)
        .header("if-match", &current_tenant_etag(&client, &token).await)
        .send_json()
        .await;
    assert_eq!(update_status, http::StatusCode::OK);

    let stored = plugin_config::Entity::find_by_id(Uuid::parse_str(id).expect("uuid"))
        .one(&app.db)
        .await
        .expect("query plugin_config")
        .expect("plugin_config row exists");

    let obj = stored
        .config
        .as_json()
        .as_object()
        .expect("config is object");
    assert_eq!(
        obj.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["tag_strip_prefix"],
        "stored config must contain exactly the keys the request sent, got: {obj:?}"
    );
    assert_eq!(
        stored.config.as_json()["tag_strip_prefix"],
        serde_json::json!("release-")
    );
}

/// Task 7: creating a config with the mask sentinel at a sensitive path must
/// be rejected — there is no stored row to restore from, so persisting it
/// would silently store `"***"` as the live secret.
#[tokio::test]
async fn create_with_sentinel_at_sensitive_path_is_400() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs",
            &serde_json::json!({
                "name": "Sentinel GitHub Config",
                "plugin_type": "releases.github",
                "config": {"auth_token": "***"}
            }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .expect("error message")
            .contains("still contains the masked sentinel"),
        "unexpected error body: {body:?}"
    );
}

/// Task 7: a GET-masked config echoed back verbatim on PUT must restore the
/// stored secret rather than persist the sentinel literal. Non-vacuity: the
/// raw stored value is asserted, not just the response status.
#[tokio::test]
async fn update_echoing_sentinel_preserves_stored_secret() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (create_status, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs",
            &serde_json::json!({
                "name": "Echo GitHub Config",
                "plugin_type": "releases.github",
                "config": {"auth_token": "original-secret-token"}
            }),
        )
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(create_status, http::StatusCode::CREATED);
    let id = created["id"].as_str().expect("id");

    let (get_status, masked): (_, serde_json::Value) = client
        .get(&format!("/api/v1/plugin-configs/{id}"))
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(get_status, http::StatusCode::OK);
    assert_eq!(masked["config"]["auth_token"], serde_json::json!("***"));

    let (update_status, _body): (_, serde_json::Value) = client
        .put_json(
            &format!("/api/v1/plugin-configs/{id}"),
            &serde_json::json!({
                "config": {"auth_token": "***"}
            }),
        )
        .bearer(&token)
        .header("if-match", &current_tenant_etag(&client, &token).await)
        .send_json()
        .await;
    assert_eq!(update_status, http::StatusCode::OK);

    let stored = plugin_config::Entity::find_by_id(Uuid::parse_str(id).expect("uuid"))
        .one(&app.db)
        .await
        .expect("query plugin_config")
        .expect("plugin_config row exists");

    assert_eq!(
        stored.config.as_json()["auth_token"],
        serde_json::json!("original-secret-token"),
        "the echoed sentinel must restore the previously stored secret, not overwrite it"
    );
}

/// Task 7: echoing the sentinel on a config that never had a stored value
/// for that sensitive path must be rejected — there is nothing to restore.
#[tokio::test]
async fn update_sentinel_without_stored_value_is_400() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (create_status, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs",
            &serde_json::json!({
                "name": "No-Token GitHub Config",
                "plugin_type": "releases.github",
                "config": {}
            }),
        )
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(create_status, http::StatusCode::CREATED);
    let id = created["id"].as_str().expect("id");

    let (update_status, body): (_, serde_json::Value) = client
        .put_json(
            &format!("/api/v1/plugin-configs/{id}"),
            &serde_json::json!({
                "config": {"auth_token": "***"}
            }),
        )
        .bearer(&token)
        .header("if-match", &current_tenant_etag(&client, &token).await)
        .send_json()
        .await;

    assert_eq!(update_status, http::StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .expect("error message")
            .contains("still contains the masked sentinel"),
        "unexpected error body: {body:?}"
    );
}

/// Task 4: creating a config with a live secret at a sensitive path must
/// stamp `credential_updated_at`. `credential_updated_at` is internal-only
/// (never echoed on the masked REST response), so this asserts against the
/// raw stored row, mirroring `docker_basic_to_bearer_switch_drops_stale_password`.
#[tokio::test]
async fn credential_updated_at_stamped_on_secret_create() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (create_status, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs",
            &serde_json::json!({
                "name": "Stamped GitHub Config",
                "plugin_type": "releases.github",
                "config": {"auth_token": "live-secret-token"}
            }),
        )
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(create_status, http::StatusCode::CREATED);
    let id = created["id"].as_str().expect("id");

    let stored = plugin_config::Entity::find_by_id(Uuid::parse_str(id).expect("uuid"))
        .one(&app.db)
        .await
        .expect("query plugin_config")
        .expect("plugin_config row exists");

    assert!(
        stored.credential_updated_at.is_some(),
        "a config created with a live secret must stamp credential_updated_at"
    );
}

/// Task 4: creating a config with no value at any sensitive path must leave
/// `credential_updated_at` unset — a stamp here would lie about a credential
/// having been set.
#[tokio::test]
async fn credential_updated_at_not_stamped_on_secretless_create() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (create_status, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs",
            &serde_json::json!({
                "name": "Secretless GitHub Config",
                "plugin_type": "releases.github",
                "config": {}
            }),
        )
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(create_status, http::StatusCode::CREATED);
    let id = created["id"].as_str().expect("id");

    let stored = plugin_config::Entity::find_by_id(Uuid::parse_str(id).expect("uuid"))
        .one(&app.db)
        .await
        .expect("query plugin_config")
        .expect("plugin_config row exists");

    assert!(
        stored.credential_updated_at.is_none(),
        "a config created without a live secret must not stamp credential_updated_at"
    );
}

/// Task 4: a follow-up update that touches only non-secret fields must leave
/// the previously-stamped `credential_updated_at` value byte-for-byte
/// unchanged — not merely "still Some", but pinned to the exact prior value.
#[tokio::test]
async fn credential_updated_at_untouched_on_nonsecret_update() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (create_status, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs",
            &serde_json::json!({
                "name": "Pinned Stamp GitHub Config",
                "plugin_type": "releases.github",
                "config": {"auth_token": "live-secret-token"}
            }),
        )
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(create_status, http::StatusCode::CREATED);
    let id = created["id"].as_str().expect("id");

    let before = plugin_config::Entity::find_by_id(Uuid::parse_str(id).expect("uuid"))
        .one(&app.db)
        .await
        .expect("query plugin_config")
        .expect("plugin_config row exists");
    let stamped_at = before
        .credential_updated_at
        .expect("create with a live secret must stamp credential_updated_at");

    let (update_status, _body): (_, serde_json::Value) = client
        .put_json(
            &format!("/api/v1/plugin-configs/{id}"),
            &serde_json::json!({
                "name": "Pinned Stamp GitHub Config Renamed",
                "enabled": false
            }),
        )
        .bearer(&token)
        .header("if-match", &current_tenant_etag(&client, &token).await)
        .send_json()
        .await;
    assert_eq!(update_status, http::StatusCode::OK);

    let after = plugin_config::Entity::find_by_id(Uuid::parse_str(id).expect("uuid"))
        .one(&app.db)
        .await
        .expect("query plugin_config")
        .expect("plugin_config row exists");

    assert_eq!(
        after.credential_updated_at,
        Some(stamped_at),
        "an update that never touches config must leave credential_updated_at at its exact prior value"
    );
}

/// Task 4: changing a live secret's value must re-stamp
/// `credential_updated_at`, and removing it entirely (not merely changing
/// it) must also re-stamp — `sensitive_value_changed_for` treats removal as
/// a credential change, since a stale "credential is live" record would be
/// as much of a lie as a stale timestamp.
#[tokio::test]
async fn credential_updated_at_stamped_on_secret_change_and_removal() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (create_status, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs",
            &serde_json::json!({
                "name": "Rotating GitHub Config",
                "plugin_type": "releases.github",
                "config": {"auth_token": "token-v1"}
            }),
        )
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(create_status, http::StatusCode::CREATED);
    let id = created["id"].as_str().expect("id");

    let after_create = plugin_config::Entity::find_by_id(Uuid::parse_str(id).expect("uuid"))
        .one(&app.db)
        .await
        .expect("query plugin_config")
        .expect("plugin_config row exists");
    let stamp_after_create = after_create
        .credential_updated_at
        .expect("create with a live secret must stamp credential_updated_at");

    // Change the secret's value.
    let (change_status, _body): (_, serde_json::Value) = client
        .put_json(
            &format!("/api/v1/plugin-configs/{id}"),
            &serde_json::json!({
                "config": {"auth_token": "token-v2"}
            }),
        )
        .bearer(&token)
        .header("if-match", &current_tenant_etag(&client, &token).await)
        .send_json()
        .await;
    assert_eq!(change_status, http::StatusCode::OK);

    let after_change = plugin_config::Entity::find_by_id(Uuid::parse_str(id).expect("uuid"))
        .one(&app.db)
        .await
        .expect("query plugin_config")
        .expect("plugin_config row exists");
    let stamp_after_change = after_change
        .credential_updated_at
        .expect("changing the secret's value must re-stamp credential_updated_at");
    assert_ne!(
        stamp_after_change, stamp_after_create,
        "a changed secret value must produce a new credential_updated_at stamp"
    );
    assert_eq!(
        after_change.config.as_json()["auth_token"],
        serde_json::json!("token-v2")
    );

    // Remove the secret entirely.
    let (removal_status, _body): (_, serde_json::Value) = client
        .put_json(
            &format!("/api/v1/plugin-configs/{id}"),
            &serde_json::json!({
                "config": {}
            }),
        )
        .bearer(&token)
        .header("if-match", &current_tenant_etag(&client, &token).await)
        .send_json()
        .await;
    assert_eq!(removal_status, http::StatusCode::OK);

    let after_removal = plugin_config::Entity::find_by_id(Uuid::parse_str(id).expect("uuid"))
        .one(&app.db)
        .await
        .expect("query plugin_config")
        .expect("plugin_config row exists");
    let stamp_after_removal = after_removal
        .credential_updated_at
        .expect("removing a live secret is itself a credential change and must re-stamp");
    assert_ne!(
        stamp_after_removal, stamp_after_change,
        "removing the secret must produce a new credential_updated_at stamp, not reuse the prior one"
    );
    assert!(
        after_removal
            .config
            .as_json()
            .get("auth_token")
            .is_none_or(serde_json::Value::is_null),
        "the secret must actually be gone from the stored config after removal"
    );
}

/// Task 4: a stale sensitive key silently dropped by
/// `prune_stale_sensitive_keys` (the Basic->Bearer switch covered by
/// `docker_basic_to_bearer_switch_drops_stale_password`) is itself a
/// credential change and must re-stamp `credential_updated_at` — the prune
/// runs before `sensitive_value_changed_for` sees the config, so a pruned
/// key must not be invisible to the stamp.
#[tokio::test]
async fn credential_updated_at_stamped_on_prune_removal() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (create_status, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs",
            &serde_json::json!({
                "name": "Pruned Docker Registry",
                "plugin_type": "releases.docker",
                "config": {
                    "auth": {"type": "basic", "username": "u", "password": "pw-1"}
                }
            }),
        )
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(create_status, http::StatusCode::CREATED);
    let id = created["id"].as_str().expect("id");

    let after_create = plugin_config::Entity::find_by_id(Uuid::parse_str(id).expect("uuid"))
        .one(&app.db)
        .await
        .expect("query plugin_config")
        .expect("plugin_config row exists");
    let stamp_after_create = after_create
        .credential_updated_at
        .expect("create with a live secret must stamp credential_updated_at");

    // Basic->Bearer switch: the frontend resubmits the full auth object,
    // including the stale masked `password` key, which the write path
    // prunes rather than restores as a live value.
    let (update_status, _body): (_, serde_json::Value) = client
        .put_json(
            &format!("/api/v1/plugin-configs/{id}"),
            &serde_json::json!({
                "config": {
                    "auth": {
                        "type": "bearer",
                        "token": "tok-2",
                        "password": "***",
                        "username": "u"
                    }
                }
            }),
        )
        .bearer(&token)
        .header("if-match", &current_tenant_etag(&client, &token).await)
        .send_json()
        .await;
    assert_eq!(update_status, http::StatusCode::OK);

    let after_update = plugin_config::Entity::find_by_id(Uuid::parse_str(id).expect("uuid"))
        .one(&app.db)
        .await
        .expect("query plugin_config")
        .expect("plugin_config row exists");
    let stamp_after_update = after_update
        .credential_updated_at
        .expect("the auth-variant switch must re-stamp credential_updated_at");
    assert_ne!(
        stamp_after_update, stamp_after_create,
        "pruning the stale password during the auth-variant switch is a credential change \
         and must produce a new credential_updated_at stamp"
    );
}

/// Intent pin (task 6a spec §5.2): `resolve_effective_config` merge
/// semantics don't change across the flip, but WHICH stored shape a profile
/// takes does — this pins the later-wins result for both. An
/// expanded-with-defaults profile (the legacy stored shape) shadows the
/// type-settings layer even for a field the operator never touched; a
/// sparse profile (the post-flip stored shape) lets the type-settings value
/// show through. This test passes before and after the flip — it documents
/// the merge semantics, it does not gate them.
#[test]
fn apt_effective_config_intent_pin_expanded_vs_sparse_profile() {
    let type_settings = serde_json::json!({"discovery_filter": "manual"});

    // Legacy shape: the field is present with its default value even though
    // the operator never set it, because the old restore path performed a
    // full typed round-trip that materializes every `#[serde(default)]` field.
    let expanded_profile = serde_json::json!({"discovery_filter": "all"});
    let expanded_effective = uptrakit_config_merge::resolve_effective_config(
        Some(&type_settings),
        Some(&expanded_profile),
        None,
    );
    assert_eq!(
        expanded_effective["discovery_filter"],
        serde_json::json!("all"),
        "an expanded profile's materialized default shadows the type-settings layer"
    );

    // Post-flip shape: the field is absent because the operator never set
    // it, so the type-settings layer shows through unshadowed.
    let sparse_profile = serde_json::json!({});
    let sparse_effective = uptrakit_config_merge::resolve_effective_config(
        Some(&type_settings),
        Some(&sparse_profile),
        None,
    );
    assert_eq!(
        sparse_effective["discovery_filter"],
        serde_json::json!("manual"),
        "a sparse profile lets the type-settings layer show through"
    );
}

#[tokio::test]
async fn delete_config_returns_204() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (_, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs",
            &serde_json::json!({
                "name": "To Delete Config",
                "plugin_type": "releases.github",
                "config": {}
            }),
        )
        .bearer(&token)
        .send_json()
        .await;

    let id = created["id"].as_str().expect("id");

    let status = client
        .delete(&format!("/api/v1/plugin-configs/{id}"))
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::NO_CONTENT);

    let row = tenant_audit_row_for_action(
        &app.db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
    )
    .await;
    assert_eq!(
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
        row.action_type
    );
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("plugin_config"));
    assert_eq!(row.target_id.as_deref(), Some(id));
    assert_eq!(row.target_display.as_deref(), Some("To Delete Config"));
    let details = row.details_json.expect("details");
    assert_eq!(details["plugin_type"], serde_json::json!("releases.github"));
    assert_eq!(
        details["config_name"],
        serde_json::json!("To Delete Config")
    );
    assert_eq!(details["enabled"], serde_json::json!(true));
}

async fn insert_service_with_id(
    app: &TestApp,
    id: Uuid,
    status: service::ServiceStatus,
) -> service::Model {
    let now = time::OffsetDateTime::now_utc();
    service::ActiveModel {
        id: Set(id),
        tenant_id: Set(app.tenant_id),
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
    .insert(&app.db)
    .await
    .expect("insert service")
}

#[tokio::test]
async fn test_plugin_config_prefers_active_agent_when_stale_link_exists() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let host = insert_host(&app.db, app.tenant_id).await;
    let stale_service =
        insert_service_with_id(&app, Uuid::from_u128(1), service::ServiceStatus::Approved).await;
    let active_service =
        insert_service_with_id(&app, Uuid::from_u128(2), service::ServiceStatus::Approved).await;

    link_service_host(&app.db, stale_service.id, host.id).await;
    link_service_host(&app.db, active_service.id, host.id).await;

    service::ActiveModel {
        id: Set(stale_service.id),
        deactivated_at: Set(Some(time::OffsetDateTime::now_utc())),
        ..stale_service.into()
    }
    .update(&app.db)
    .await
    .expect("deactivate stale service");

    let (mut rx, _handle) = app
        .state
        .service_connections
        .register(active_service.id, BTreeSet::new(), None, None, None)
        .await;
    let proxy = app.state.config_test_proxy.clone();
    tokio::spawn(async move {
        match rx.recv().await {
            Some(ControllerMessage::TestPluginConfig(payload)) => {
                let request_id = payload.request_id.clone();
                proxy.complete(
                    &request_id,
                    TestPluginConfigResultPayload::new(request_id.clone(), true, 1),
                );
            }
            other => panic!("unexpected message: {other:?}"),
        }
    });

    let (status, body): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs/test",
            &serde_json::json!({
                "plugin_type": "generic.shell",
                "config": { "version_command": "echo 1.0.0" },
                "host_id": host.id,
                "test_kind": "version_detection"
            }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["success"], true);
}

/// Task 10: the config-test merge step must restore secrets masked by the UI
/// echo before dispatching to the agent — a `"***"` sentinel in `body.config`
/// must never overwrite the real stored credential. Non-vacuity: the raw
/// `payload.config` the fake agent receives is asserted, not just the
/// response status. `infrastructure.proxmox` is used because it takes the
/// agent-side path (unlike controller-side release plugins, whose canned 200
/// exposes nothing about the merged config).
#[tokio::test]
async fn config_test_merge_restores_stored_secret_before_dispatch() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let config_id = Uuid::now_v7();
    let now = time::OffsetDateTime::now_utc();
    plugin_config::ActiveModel {
        id: Set(config_id),
        tenant_id: Set(app.tenant_id),
        name: Set("Proxmox Config".to_string()),
        plugin_type: Set("infrastructure.proxmox".to_string()),
        config: Set(
            uptrakit_shared_db::encrypted_columns::EncryptedPluginConfig::from_json(
                &serde_json::json!({
                    "api_url": "https://pve.example.com:8006",
                    "api_token": "svc@pve!apitoken=tok-1-secret"
                }),
            )
            .expect("encrypt test config"),
        ),
        enabled: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
        credential_updated_at: Set(None),
    }
    .insert(&app.db)
    .await
    .expect("insert plugin config");

    let host = insert_host(&app.db, app.tenant_id).await;
    let service =
        insert_service_with_id(&app, Uuid::from_u128(3), service::ServiceStatus::Approved).await;
    link_service_host(&app.db, service.id, host.id).await;

    let (mut rx, _handle) = app
        .state
        .service_connections
        .register(service.id, BTreeSet::new(), None, None, None)
        .await;
    let proxy = app.state.config_test_proxy.clone();
    let (payload_tx, payload_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        match rx.recv().await {
            Some(ControllerMessage::TestPluginConfig(payload)) => {
                let request_id = payload.request_id.clone();
                if payload_tx.send(payload.config.clone()).is_err() {
                    panic!("failed to hand off dispatched payload to test");
                }
                proxy.complete(
                    &request_id,
                    TestPluginConfigResultPayload::new(request_id.clone(), true, 1),
                );
            }
            other => panic!("unexpected message: {other:?}"),
        }
    });

    let (status, body): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs/test",
            &serde_json::json!({
                "plugin_type": "infrastructure.proxmox",
                "plugin_config_id": config_id,
                "config": { "api_token": "***" },
                "host_id": host.id,
                "test_kind": "connectivity"
            }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK, "response body: {body:?}");

    let dispatched_config = payload_rx.await.expect("fake agent received payload");
    assert_eq!(
        dispatched_config["api_token"],
        serde_json::json!("svc@pve!apitoken=tok-1-secret"),
        "the dispatched wire payload must carry the real stored secret, not the UI sentinel: {dispatched_config:?}"
    );
}

/// Task 10: with no `plugin_config_id` there is nothing to restore from, so a
/// sentinel at a sensitive path must be rejected outright.
#[tokio::test]
async fn config_test_sentinel_without_saved_config_is_400() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs/test",
            &serde_json::json!({
                "plugin_type": "infrastructure.proxmox",
                "config": {
                    "api_url": "https://pve.example.com:8006",
                    "api_token": "***"
                }
            }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .expect("error message")
            .contains("still contains the masked sentinel"),
        "unexpected error body: {body:?}"
    );
}

#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn list_plugin_types_includes_dashboard_icons_type_settings() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/plugin-types")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    let entries = body.as_array().expect("plugin types should be an array");
    let dashboard_icons = entries
        .iter()
        .find(|entry| entry["plugin_type"] == "enhancement.dashboard-icons")
        .expect("dashboard icons plugin type should be present");

    assert_eq!(dashboard_icons["display_name"], "Dashboard Icons");
    assert_eq!(
        dashboard_icons["supports_plugin_configs"],
        serde_json::json!(false)
    );
    let type_fields = dashboard_icons["type_settings_form_fields"]
        .as_array()
        .expect("type settings fields should be an array");
    assert!(
        type_fields
            .iter()
            .any(|field| field["key"] == "enabled" && field["field_type"] == "toggle"),
        "dashboard icons type settings should expose an enabled toggle"
    );
    assert_eq!(
        dashboard_icons["type_settings_sample"],
        serde_json::json!({ "enabled": true })
    );
}

#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn create_config_rejects_config_model_none_plugin_type() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs",
            &serde_json::json!({
                "name": "Should Be Rejected",
                "plugin_type": "enhancement.dashboard-icons",
                "config": {}
            }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
    let msg = body["error"].as_str().unwrap_or_default();
    assert!(
        msg.contains("does not support per-instance plugin configs"),
        "unexpected error message: {msg}"
    );
}

#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn update_config_rejects_existing_config_model_none_plugin_type() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let config_id = insert_plugin_config_for_dashboard_icons(&app).await;

    let (status, body): (_, serde_json::Value) = client
        .put_json(
            &format!("/api/v1/plugin-configs/{config_id}"),
            &serde_json::json!({
                "name": "Rejected Update"
            }),
        )
        .bearer(&token)
        .header("if-match", &current_tenant_etag(&client, &token).await)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
    let msg = body["error"].as_str().unwrap_or_default();
    assert!(
        msg.contains("does not support per-instance plugin configs"),
        "unexpected error message: {msg}"
    );
}

#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn test_config_rejects_config_model_none_plugin_type() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs/test",
            &serde_json::json!({
                "plugin_type": "enhancement.dashboard-icons",
                "config": {}
            }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
    let msg = body["error"].as_str().unwrap_or_default();
    assert!(
        msg.contains("does not support per-instance plugin configs"),
        "unexpected error message: {msg}"
    );
}

#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn upsert_type_settings_rejects_invalid_dashboard_icons_enabled_type() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/plugin-type-settings/enhancement.dashboard-icons",
            &serde_json::json!({
                "config": {
                    "enabled": "false"
                }
            }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
    let msg = body["error"].as_str().unwrap_or_default();
    assert!(
        msg.contains("Invalid plugin type settings"),
        "unexpected error message: {msg}"
    );

    let missing_status = client
        .get("/api/v1/plugin-type-settings/enhancement.dashboard-icons")
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(missing_status, http::StatusCode::NOT_FOUND);
}

/// Register the admin (first user), re-open registration, then register a
/// second user who gets the built-in "user" role (ViewSettings but NOT
/// ManageGlobalSettings).  Returns `(admin_token, tenant_token)`.
#[cfg(feature = "dashboard-icons")]
async fn register_admin_and_tenant_user(app: &TestApp) -> (String, String) {
    let client = app.client();

    // First registered user becomes owner (all permissions including ManageGlobalSettings).
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
        .header(
            "if-match",
            &current_tenant_etag(&client, &admin_token).await,
        )
        .send_status()
        .await;
    assert_eq!(
        reopen,
        http::StatusCode::OK,
        "failed to re-open registration"
    );

    // Second user gets the built-in "user" role: ViewSettings but NOT ManageGlobalSettings.
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

/// A tenant user (ViewSettings, no ManageGlobalSettings) must not see a
/// disabled Instance-scoped plugin in the `GET /api/v1/plugin-types` list.
///
/// `enhancement.dashboard-icons` is Instance-scoped and the snapshot defaults
/// to `all_disabled()` at TestApp boot, so it must be absent from the response.
#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn tenant_user_does_not_see_disabled_instance_plugin_in_plugin_types_list() {
    let app = TestApp::new().await;
    let (_admin_token, tenant_token) = register_admin_and_tenant_user(&app).await;

    let (status, body): (_, serde_json::Value) = app
        .client()
        .get("/api/v1/plugin-types")
        .bearer(&tenant_token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    let entries = body.as_array().expect("plugin-types must be an array");
    assert!(
        !entries
            .iter()
            .any(|e| e["plugin_type"] == "enhancement.dashboard-icons"),
        "tenant user must not see disabled instance-scoped plugin; entries: {entries:?}"
    );
}

/// An admin (ManageGlobalSettings) must see a disabled Instance-scoped plugin
/// in the `GET /api/v1/plugin-types` list — the predicate passes for owners.
#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn admin_sees_disabled_instance_plugin_in_plugin_types_list() {
    let app = TestApp::new().await;
    let (admin_token, _tenant_token) = register_admin_and_tenant_user(&app).await;

    let (status, body): (_, serde_json::Value) = app
        .client()
        .get("/api/v1/plugin-types")
        .bearer(&admin_token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    let entries = body.as_array().expect("plugin-types must be an array");
    assert!(
        entries
            .iter()
            .any(|e| e["plugin_type"] == "enhancement.dashboard-icons"),
        "admin must see disabled instance-scoped plugin; entries: {entries:?}"
    );
}

#[cfg(feature = "dashboard-icons")]
async fn insert_plugin_config_for_dashboard_icons(app: &TestApp) -> Uuid {
    let id = Uuid::now_v7();
    let now = time::OffsetDateTime::now_utc();

    plugin_config::ActiveModel {
        id: Set(id),
        tenant_id: Set(app.tenant_id),
        name: Set("Existing Dashboard Icons Config".to_string()),
        plugin_type: Set("enhancement.dashboard-icons".to_string()),
        config: Set(
            uptrakit_shared_db::encrypted_columns::EncryptedPluginConfig::from_json(
                &serde_json::json!({}),
            )
            .expect("encrypt test config"),
        ),
        enabled: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
        credential_updated_at: Set(None),
    }
    .insert(&app.db)
    .await
    .expect("insert plugin config");

    id
}

// ── ETag / If-Match route-layer regression tests (ADR-0017) ──────────────────
//
// Plugin-config CRUD sits behind `etag_middleware::<SettingsVersion>`. The layer
// guards PUT/PATCH only and injects an `ETag` on every 2xx, so POST/DELETE/batch
// must succeed without an `If-Match` header while PUT still requires a fresh one.

/// Read the current tenant ETag the way a real client does — GET before PUT.
///
/// The tenant `settings_version` is not necessarily 0: the layer refreshes the
/// version cache from the DB after every 2xx, so a hardcoded `W/"settings-v0"`
/// goes stale as soon as anything bumps the counter. Always fetch it.
async fn current_tenant_etag(
    client: &crate::test_harness::http_client::TestClient,
    token: &str,
) -> String {
    let resp = client
        .get("/api/v1/plugin-configs")
        .bearer(token)
        .send()
        .await;
    assert_eq!(
        resp.status(),
        http::StatusCode::OK,
        "ETag probe GET must succeed"
    );
    resp.headers()
        .get("etag")
        .expect("ETag on plugin-config list")
        .to_str()
        .expect("ASCII ETag")
        .to_string()
}

/// Create a plugin config and return its id. Deliberately sends no `If-Match`.
async fn create_config_without_if_match(
    client: &crate::test_harness::http_client::TestClient,
    token: &str,
    name: &str,
) -> String {
    let (status, body): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs",
            &serde_json::json!({
                "name": name,
                "plugin_type": "releases.github",
                "config": {}
            }),
        )
        .bearer(token)
        .send_json()
        .await;

    assert_eq!(
        status,
        http::StatusCode::CREATED,
        "POST must not require If-Match, got body: {body:?}"
    );
    // `.get()` rather than `body["id"]`: clippy's indexing_slicing test allowance
    // only covers `#[test]`-attributed fns, and this is a plain helper.
    body.get("id")
        .and_then(serde_json::Value::as_str)
        .expect("id")
        .to_string()
}

#[tokio::test]
async fn get_plugin_config_returns_etag_header() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;
    let id = create_config_without_if_match(&client, &token, "ETag Read Config").await;

    let resp = client
        .get(&format!("/api/v1/plugin-configs/{id}"))
        .bearer(&token)
        .send()
        .await;

    assert_eq!(resp.status(), http::StatusCode::OK);
    let etag = resp
        .headers()
        .get("etag")
        .expect("ETag header present on GET")
        .to_str()
        .expect("ETag is ASCII")
        .to_string();
    assert!(
        etag.starts_with("W/\"settings-v") && etag.ends_with('"'),
        "expected a weak settings-scoped ETag, got {etag:?}"
    );
}

#[tokio::test]
async fn list_plugin_configs_returns_etag_header() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let resp = client
        .get("/api/v1/plugin-configs")
        .bearer(&token)
        .send()
        .await;

    assert_eq!(resp.status(), http::StatusCode::OK);
    let etag = resp
        .headers()
        .get("etag")
        .expect("ETag header present on list")
        .to_str()
        .expect("ETag is ASCII")
        .to_string();
    assert!(
        etag.starts_with("W/\"settings-v"),
        "expected a weak settings-scoped ETag, got {etag:?}"
    );
}

#[tokio::test]
async fn put_plugin_config_with_etag_returns_200_and_fresh_etag() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;
    let id = create_config_without_if_match(&client, &token, "ETag Round Trip Config").await;

    // GET → capture the ETag the client is expected to echo back.
    let get_resp = client
        .get(&format!("/api/v1/plugin-configs/{id}"))
        .bearer(&token)
        .send()
        .await;
    assert_eq!(get_resp.status(), http::StatusCode::OK);
    let etag = get_resp
        .headers()
        .get("etag")
        .expect("ETag on GET")
        .to_str()
        .expect("ASCII")
        .to_string();

    // PUT with that ETag → 200 and the response carries an ETag of its own
    // (produced by the layer's post-write refresh_etag DB re-read).
    let put_resp = client
        .put_json(
            &format!("/api/v1/plugin-configs/{id}"),
            &serde_json::json!({ "name": "ETag Round Trip Renamed" }),
        )
        .bearer(&token)
        .header("if-match", &etag)
        .send()
        .await;

    assert_eq!(put_resp.status(), http::StatusCode::OK);
    let put_etag = put_resp
        .headers()
        .get("etag")
        .expect("PUT response must carry an ETag")
        .to_str()
        .expect("ASCII")
        .to_string();
    assert!(
        put_etag.starts_with("W/\"settings-v"),
        "expected a weak settings-scoped ETag on PUT, got {put_etag:?}"
    );
}

#[tokio::test]
async fn put_plugin_config_without_if_match_returns_428() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;
    let id = create_config_without_if_match(&client, &token, "Missing If-Match Config").await;

    let (status, body): (_, serde_json::Value) = client
        .put_json(
            &format!("/api/v1/plugin-configs/{id}"),
            &serde_json::json!({ "name": "Never Applied" }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::PRECONDITION_REQUIRED);
    assert_eq!(body["code"], "if_match.required");
}

#[tokio::test]
async fn put_plugin_config_with_stale_etag_returns_409() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;
    let id = create_config_without_if_match(&client, &token, "Stale If-Match Config").await;

    let (status, body): (_, serde_json::Value) = client
        .put_json(
            &format!("/api/v1/plugin-configs/{id}"),
            &serde_json::json!({ "name": "Never Applied" }),
        )
        .bearer(&token)
        .header("if-match", "W/\"settings-v999\"")
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::CONFLICT);
    assert_eq!(body["code"], "if_match.stale");
}

#[tokio::test]
async fn plugin_config_create_delete_and_batch_need_no_if_match() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // POST create → 201 without If-Match.
    let delete_target = create_config_without_if_match(&client, &token, "No If-Match Delete").await;

    // DELETE → 204 without If-Match.
    let delete_status = client
        .delete(&format!("/api/v1/plugin-configs/{delete_target}"))
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(delete_status, http::StatusCode::NO_CONTENT);

    // POST /batch → 200 without If-Match.
    let batch_target = create_config_without_if_match(&client, &token, "No If-Match Batch").await;
    let (batch_status, batch_body): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs/batch",
            &serde_json::json!({ "action": "delete", "ids": [batch_target] }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(batch_status, http::StatusCode::OK);
    assert_eq!(
        batch_body["succeeded"]
            .as_array()
            .expect("succeeded array")
            .len(),
        1,
        "batch delete should report one success, got {batch_body:?}"
    );
}
