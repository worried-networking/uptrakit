//! Integration tests for the visibility predicate on
//! `GET /api/v1/plugin-type-settings/{plugin_type}`.
//!
//! # Why upsert/delete predicate-reject tests are absent
//!
//! The `PUT` and `DELETE` handlers are gated by the `CanManageSystemSettings`
//! action extractor *before* the visibility predicate runs.  A tenant user
//! lacking `system.settings:manage` authority is denied there and never
//! reaches the predicate.  An admin passes the predicate whenever the engine
//! allows `system.settings:manage` — `is_plugin_visible_to_user` asks the
//! `AccessEngine` for that action directly, so a role link alone is not
//! enough: the covering grant must also be present.  There is therefore no
//! reachable code path in the harness
//! test matrix where the predicate would return `false` for upsert/delete —
//! those tests would be vacuous duplicates of the existing extractor-gate tests.

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uptrakit_shared_db::entity::user_role;
use uptrakit_shared_types::access::actions;

use crate::test_harness::TestApp;
use crate::test_harness::fixtures::{
    open_registration, register_and_get_token, register_user, revoke_role_grants_covering,
    role_id_by_name, stage_user_with_only_role, stage_zero_role_user,
};
#[cfg(feature = "dashboard-icons")]
use crate::test_harness::fixtures::{register_admin_and_tenant_user, register_user_with_only_role};

#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn tenant_user_get_plugin_type_settings_for_disabled_instance_plugin_returns_404() {
    let app = TestApp::new().await;
    let (_admin_token, tenant_token) = register_admin_and_tenant_user(&app).await;

    // The snapshot defaults to all_disabled() at TestApp boot, so
    // enhancement.dashboard-icons is disabled → predicate rejects for tenant users.
    let status = app
        .client()
        .get("/api/v1/plugin-type-settings/enhancement.dashboard-icons")
        .bearer(&tenant_token)
        .send_status()
        .await;

    assert_eq!(
        status,
        http::StatusCode::NOT_FOUND,
        "tenant user must receive 404 for disabled instance-scoped plugin settings"
    );
}

/// An admin (ManageGlobalSettings) GETting settings for a disabled
/// Instance-scoped plugin after seeding a settings row must receive 200 —
/// the predicate passes for admins regardless of the enabled/disabled state.
///
/// Without a seeded row the admin would also get 404 ("No settings found"),
/// which is indistinguishable from the tenant-user predicate-reject 404.
/// We seed a row here so the test proves the admin *can* reach the row.
#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn admin_get_plugin_type_settings_for_disabled_instance_plugin_returns_200_after_seed() {
    let app = TestApp::new().await;
    let (admin_token, _tenant_token) = register_admin_and_tenant_user(&app).await;

    // Seed a settings row directly through the query layer so the handler can return it.
    uptrakit_web_api_queries::queries::plugin_type_settings::upsert_type_settings(
        app.state.db(),
        app.tenant_id,
        "enhancement.dashboard-icons",
        serde_json::json!({ "enabled": false }),
    )
    .await
    .expect("seed plugin type settings row");

    let (status, body): (_, serde_json::Value) = app
        .client()
        .get("/api/v1/plugin-type-settings/enhancement.dashboard-icons")
        .bearer(&admin_token)
        .send_json()
        .await;

    assert_eq!(
        status,
        http::StatusCode::OK,
        "admin must reach the settings row for a disabled instance-scoped plugin"
    );
    assert_eq!(
        body["plugin_type"], "enhancement.dashboard-icons",
        "response must identify the correct plugin type"
    );
}

/// `GET /api/v1/plugin-type-settings` (list) must include settings for disabled
/// Instance-scoped plugins when accessed by an admin. Tenant users get those
/// rows filtered out by the predicate (proved via the existing tests in
/// `plugin_configs.rs` and the disabled-plugin GET test above); this test
/// proves admins still see them in the list.
#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn admin_list_plugin_type_settings_includes_disabled_instance_plugin_after_seed() {
    let app = TestApp::new().await;
    let (admin_token, _tenant_token) = register_admin_and_tenant_user(&app).await;

    uptrakit_web_api_queries::queries::plugin_type_settings::upsert_type_settings(
        app.state.db(),
        app.tenant_id,
        "enhancement.dashboard-icons",
        serde_json::json!({ "enabled": false }),
    )
    .await
    .expect("seed plugin type settings row");

    let (status, body): (_, Vec<serde_json::Value>) = app
        .client()
        .get("/api/v1/plugin-type-settings")
        .bearer(&admin_token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK, "admin list must return 200");
    assert!(
        body.iter()
            .any(|row| row["plugin_type"] == "enhancement.dashboard-icons"),
        "admin list must include the disabled instance-scoped plugin's settings row"
    );
}

/// ADR-0033: a pending-restart-enabled plugin (live row enabled, boot-disabled
/// catalog) is NOT effectively enabled, so it stays hidden from tenant users.
#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn tenant_user_get_settings_for_pending_restart_enabled_plugin_returns_404() {
    let app = TestApp::new().await;
    let (_admin_token, tenant_token) = register_admin_and_tenant_user(&app).await;

    // TestApp's catalog boots with InstancePluginStates::all_disabled();
    // seeding the live row enabled produces exactly the pending-restart state.
    crate::test_harness::fixtures::upsert_instance_plugin_setting(
        &app,
        "enhancement.dashboard-icons",
        true,
    )
    .await;

    let status = app
        .client()
        .get("/api/v1/plugin-type-settings/enhancement.dashboard-icons")
        .bearer(&tenant_token)
        .send_status()
        .await;

    assert_eq!(
        status,
        http::StatusCode::NOT_FOUND,
        "pending-restart-enabled plugin must stay hidden from tenant users"
    );
}

/// M1.5 OR-gate weaker arm: a `settings_manager`-only principal (seed grant
/// covers only `settings:read`, never `system.settings:manage`) must still
/// be able to list plugin type settings — `authorize_any` accepts the first
/// matching arm.
#[tokio::test]
async fn settings_manager_only_user_can_list_plugin_type_settings() {
    let app = TestApp::new().await;
    let (_user_id, token) = stage_user_with_only_role(&app, "settings_manager").await;

    let status = app
        .client()
        .get("/api/v1/plugin-type-settings")
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(
        status,
        http::StatusCode::OK,
        "settings_manager (settings:read only) must be able to list plugin type settings"
    );
}

/// A principal holding zero role links (and therefore zero `access_grants`
/// coverage) must be denied.
#[tokio::test]
async fn zero_role_user_cannot_list_plugin_type_settings() {
    let app = TestApp::new().await;
    let (_user_id, token) = stage_zero_role_user(&app).await;

    let status = app
        .client()
        .get("/api/v1/plugin-type-settings")
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(
        status,
        http::StatusCode::FORBIDDEN,
        "zero-role user must be denied the plugin type settings list"
    );
}

/// Regression test: a `viewer` principal's role link is left untouched, but
/// every `access_grants` row covering `settings:read` / `system.settings:manage`
/// for that role is deleted and the cache invalidated (a role may hold more
/// than one seed grant row, so this loops rather than calling `.one()`). The
/// deny must be grant-scoped: still being linked to the `viewer` role must
/// not itself grant access once its covering grants are gone.
#[tokio::test]
async fn plugin_type_settings_list_forbidden_after_viewer_grants_revoked() {
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
        &[actions::SETTINGS_READ, actions::SYSTEM_SETTINGS_MANAGE],
    )
    .await;

    let status = app
        .client()
        .get("/api/v1/plugin-type-settings")
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(
        status,
        http::StatusCode::FORBIDDEN,
        "engine must deny the plugin type settings list once the covering grant is revoked"
    );
}

/// Instance-scoped visibility admin override: a principal holding only
/// `system.settings:manage` (linked `system_administrator` role, no
/// `settings:read`) sees a not-effectively-enabled Instance-scoped plugin's
/// settings row in the list; a `viewer`-only principal (`settings:read`, no
/// `system.settings:manage`) does not.
#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn system_administrator_sees_disabled_instance_plugin_viewer_does_not() {
    let app = TestApp::new().await;
    open_registration(&app).await;
    let (_admin_id, admin_token) =
        register_user_with_only_role(&app, "sysadmin-only@test.local", "system_administrator")
            .await;
    let (_viewer_id, viewer_token) =
        register_user_with_only_role(&app, "viewer-only@test.local", "viewer").await;

    uptrakit_web_api_queries::queries::plugin_type_settings::upsert_type_settings(
        app.state.db(),
        app.tenant_id,
        "enhancement.dashboard-icons",
        serde_json::json!({ "enabled": false }),
    )
    .await
    .expect("seed plugin type settings row");

    let (admin_status, admin_body): (_, Vec<serde_json::Value>) = app
        .client()
        .get("/api/v1/plugin-type-settings")
        .bearer(&admin_token)
        .send_json()
        .await;
    assert_eq!(admin_status, http::StatusCode::OK);
    assert!(
        admin_body
            .iter()
            .any(|row| row["plugin_type"] == "enhancement.dashboard-icons"),
        "system_administrator (system.settings:manage) must see the disabled instance-scoped plugin"
    );

    let (viewer_status, viewer_body): (_, Vec<serde_json::Value>) = app
        .client()
        .get("/api/v1/plugin-type-settings")
        .bearer(&viewer_token)
        .send_json()
        .await;
    assert_eq!(viewer_status, http::StatusCode::OK);
    assert!(
        !viewer_body
            .iter()
            .any(|row| row["plugin_type"] == "enhancement.dashboard-icons"),
        "viewer (settings:read only, no system.settings:manage) must not see the disabled \
         instance-scoped plugin"
    );
}

/// Stub plugin descriptor + catalog for the secret-masking tests below.
///
/// Task 5's registry assertion (`type_settings_and_instance_plugins_are_secret_free`)
/// proves no *real* plugin has a sensitive type-settings path today, so these
/// tests inject a synthetic Tenant-scoped descriptor with an explicit
/// `sensitive_paths: &["auth_token"]` rather than searching for a real leak —
/// this is defense-in-depth for future secret-bearing type-settings plugins.
mod secret_masking_fixture {
    use std::sync::{Arc, OnceLock};

    use uptrakit_plugin_infrastructure_core::TypeSettingsOps;
    use uptrakit_plugin_infrastructure_registry::{
        CatalogConfig, ConfigModel, ConfigOps, FormFieldDescriptor, InstancePluginStates,
        PluginCatalog, PluginConfigValidationError, PluginDescriptor, PluginFamily, PluginOps,
        PluginScope, RoleCreators,
    };

    pub(crate) const TYPE_ID: &str = "test.secret.type-settings";

    fn noop_validate(_: &serde_json::Value) -> Result<(), PluginConfigValidationError> {
        Ok(())
    }
    fn noop_normalize(
        v: &serde_json::Value,
    ) -> Result<serde_json::Value, PluginConfigValidationError> {
        Ok(v.clone())
    }
    fn noop_sample() -> serde_json::Value {
        serde_json::json!({})
    }
    fn noop_form_schema() -> Vec<FormFieldDescriptor> {
        vec![]
    }
    fn noop_validate_identifier(_: &str) -> Result<(), PluginConfigValidationError> {
        Ok(())
    }

    static TYPE_SETTINGS_OPS: TypeSettingsOps = TypeSettingsOps {
        form_schema: noop_form_schema,
        sample: noop_sample,
    };

    static DESCRIPTOR: OnceLock<PluginDescriptor> = OnceLock::new();

    fn descriptor() -> &'static PluginDescriptor {
        DESCRIPTOR.get_or_init(|| PluginDescriptor {
            type_id: TYPE_ID,
            display_name: "Test Type Settings Secret Plugin",
            family: PluginFamily::Software,
            config_model: ConfigModel::None,
            capabilities: &[],
            scope: PluginScope::Tenant,
            instance_config: None,
            sensitive_paths: &["auth_token"],
            config: ConfigOps {
                validate: noop_validate,
                normalize: noop_normalize,
                sample: noop_sample,
                form_schema: noop_form_schema,
                validate_identifier: noop_validate_identifier,
            },
            roles: RoleCreators {
                discoverer: None,
                version_detector: None,
                release_fetcher: None,
                package_indexer: None,
                update_executor: None,
                lifecycle_hook: None,
                notification_transport: None,
                software_item_lifecycle: None,
                controller_update_protection: None,
                controller_update_hook: None,
                infra: None,
                installed_version_enricher: None,
            },
            surfaces: None,
            type_settings: Some(&TYPE_SETTINGS_OPS),
            config_test: None,
            sudo: None,
            raw_settings_keys: &[],
            global_provider_consumers: &[],
            migrations: None,
            agent_migrations: None,
            agent_surfaces: None,
            reset_tenant_data: None,
            db_migrate_tables: None,
        })
    }

    /// Build a `PluginOps` catalog containing only this stub descriptor —
    /// injected via `TestApp::with_plugin_surfaces` so the real catalog
    /// (which is provably secret-free per Task 5) is never on the hook for
    /// exercising this code path.
    ///
    /// Returns the fallible catalog build so callers (all `#[tokio::test]`
    /// fn bodies) can `.expect()` it themselves — `.expect()` outside a
    /// `#[test]` fn body is clippy-denied workspace-wide.
    pub(crate) fn plugin_ops() -> uptrakit_plugin_infrastructure_core::Result<Arc<dyn PluginOps>> {
        PluginCatalog::new(
            vec![descriptor()],
            &CatalogConfig::default(),
            InstancePluginStates::all_disabled(),
        )
        .map(|catalog| Arc::new(catalog) as Arc<dyn PluginOps>)
    }
}

#[tokio::test]
async fn type_settings_get_masks_sensitive_paths() {
    let app = TestApp::with_plugin_surfaces(Some(
        secret_masking_fixture::plugin_ops()
            .expect("build stub plugin catalog for type-settings masking tests"),
    ))
    .await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    uptrakit_web_api_queries::queries::plugin_type_settings::upsert_type_settings(
        app.state.db(),
        app.tenant_id,
        secret_masking_fixture::TYPE_ID,
        serde_json::json!({ "auth_token": "t1", "filter": "x" }),
    )
    .await
    .expect("seed plugin type settings row");

    let (status, body): (_, serde_json::Value) = app
        .client()
        .get(&format!(
            "/api/v1/plugin-type-settings/{}",
            secret_masking_fixture::TYPE_ID
        ))
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK, "expected masked settings row");
    assert_eq!(
        body["config"]["auth_token"],
        serde_json::json!("***"),
        "sensitive path must be masked on GET"
    );
    assert_eq!(
        body["config"]["filter"],
        serde_json::json!("x"),
        "non-sensitive path must pass through unmasked"
    );
}

#[tokio::test]
async fn type_settings_put_sentinel_preserves_secret_and_stays_sparse() {
    let app = TestApp::with_plugin_surfaces(Some(
        secret_masking_fixture::plugin_ops()
            .expect("build stub plugin catalog for type-settings masking tests"),
    ))
    .await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    uptrakit_web_api_queries::queries::plugin_type_settings::upsert_type_settings(
        app.state.db(),
        app.tenant_id,
        secret_masking_fixture::TYPE_ID,
        serde_json::json!({ "auth_token": "t1", "filter": "x" }),
    )
    .await
    .expect("seed plugin type settings row");

    let status = app
        .client()
        .put_json(
            &format!(
                "/api/v1/plugin-type-settings/{}",
                secret_masking_fixture::TYPE_ID
            ),
            &serde_json::json!({ "config": { "auth_token": "***", "filter": "y" } }),
        )
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(
        status,
        http::StatusCode::OK,
        "sentinel PUT must be accepted"
    );

    let stored = uptrakit_web_api_queries::queries::plugin_type_settings::get_type_settings(
        app.state.db(),
        app.tenant_id,
        secret_masking_fixture::TYPE_ID,
    )
    .await
    .expect("load stored plugin type settings")
    .expect("row must exist after PUT");

    assert_eq!(
        stored.config.as_json()["auth_token"],
        serde_json::json!("t1"),
        "sentinel must be restored to the real stored secret"
    );
    assert_eq!(
        stored.config.as_json()["filter"],
        serde_json::json!("y"),
        "non-sensitive field must be updated"
    );

    let stored_keys: std::collections::BTreeSet<&str> = stored
        .config
        .as_json()
        .as_object()
        .expect("stored config must be an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        stored_keys,
        std::collections::BTreeSet::from(["auth_token", "filter"]),
        "stored config must contain exactly the submitted keys — a serde-default expansion \
         adding keys is the regression this test exists to catch"
    );
}

#[tokio::test]
async fn type_settings_put_sentinel_with_no_stored_row_is_rejected() {
    let app = TestApp::with_plugin_surfaces(Some(
        secret_masking_fixture::plugin_ops()
            .expect("build stub plugin catalog for type-settings masking tests"),
    ))
    .await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = app
        .client()
        .put_json(
            &format!(
                "/api/v1/plugin-type-settings/{}",
                secret_masking_fixture::TYPE_ID
            ),
            &serde_json::json!({ "config": { "auth_token": "***" } }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(
        status,
        http::StatusCode::BAD_REQUEST,
        "sentinel PUT with no stored row must be rejected: {body}"
    );
    let message = body["error"].as_str().unwrap_or_default();
    assert!(
        message.contains("still contains the masked sentinel"),
        "error message must name the sentinel: {body}"
    );
}
