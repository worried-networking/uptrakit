//! Integration tests for `/api/v1/instance-plugins`.

#![cfg_attr(
    feature = "dashboard-icons",
    expect(
        clippy::expect_used,
        reason = "test code: panics on failure are acceptable"
    )
)]
#![cfg_attr(
    feature = "dashboard-icons",
    expect(clippy::panic, reason = "test code: panics on failure are acceptable")
)]

#[cfg(feature = "dashboard-icons")]
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
#[cfg(feature = "dashboard-icons")]
use uptrakit_shared_db::entity::system_audit_log;

#[cfg(feature = "dashboard-icons")]
use uptrakit_shared_db::access_grants::{GrantSubject, NewGrant, insert_grant};
#[cfg(feature = "dashboard-icons")]
use uptrakit_shared_types::access::{ActionPattern, Selector};

use crate::test_harness::TestApp;
use crate::test_harness::fixtures::register_and_get_token;
#[cfg(feature = "dashboard-icons")]
use crate::test_harness::fixtures::upsert_instance_plugin_setting;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Poll the system audit log up to 50 × 10 ms for the first row with the
/// given `action_type`, ordered by most-recent first.
///
/// Instance-plugin audit entries use `system_scope()` (no tenant_id) and are
/// therefore routed to `system_audit_logs`, not `audit_logs`.
#[cfg(feature = "dashboard-icons")]
async fn poll_system_audit_row(
    db: &sea_orm::DatabaseConnection,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
) -> system_audit_log::Model {
    for _ in 0..50 {
        if let Some(row) = system_audit_log::Entity::find()
            .filter(system_audit_log::Column::ActionType.eq(action_type))
            .order_by_desc(system_audit_log::Column::OccurredAt)
            .one(db)
            .await
            .expect("query system audit rows")
        {
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("expected system audit row for action {action_type}");
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// GET /api/v1/instance-plugins requires `system.settings:manage`
/// (`CanManageSystemSettings`). A principal holding only the unrelated
/// `settings:read` grant is authenticated but under-privileged for this
/// endpoint, so it must still be rejected with 403 — not merely because it
/// carries no grant at all.
#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn list_requires_manage_global_settings() {
    let app = TestApp::new().await;
    let client = app.client();

    let user_id = uuid::Uuid::now_v7();
    let token = app
        .jwt
        .create_access_token(user_id, "password", None, None)
        .expect("mint token");
    let patterns = vec!["settings:read".parse::<ActionPattern>().expect("pattern")];
    insert_grant(
        &app.db,
        NewGrant {
            subject: GrantSubject::User(user_id),
            tenant_id: Some(app.tenant_id),
            patterns: &patterns,
            selector: Selector::All,
            description: None,
            created_by: None,
        },
    )
    .await
    .expect("insert grant");
    app.state.access_engine.invalidate_subjects(&[user_id], &[]);

    let status = client
        .get("/api/v1/instance-plugins")
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::FORBIDDEN);
}

/// GET /api/v1/instance-plugins returns all instance-scoped plugins.
/// Pre-seeding dashboard-icons as enabled must be reflected in `enabled`; the
/// catalog snapshot was built before the seed so `running_enabled` stays false.
#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn list_returns_all_instance_scoped_plugins_with_state() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // Seed the setting so the snapshot sees it enabled.
    upsert_instance_plugin_setting(&app, "enhancement.dashboard-icons", true).await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/instance-plugins")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    let entries = body.as_array().expect("response should be an array");

    let dashboard_icons = entries
        .iter()
        .find(|e| e["plugin_type"] == "enhancement.dashboard-icons")
        .expect("enhancement.dashboard-icons should appear in list");

    assert_eq!(
        dashboard_icons["enabled"], true,
        "enabled must reflect the seeded value"
    );
    assert_eq!(
        dashboard_icons["running_enabled"], false,
        "running_enabled reflects catalog state at boot (InstancePluginStates::all_disabled)"
    );
}

/// PUT /api/v1/instance-plugins/{plugin_type}/enabled persists the flag and
/// emits an INSTANCE_PLUGIN_TOGGLED audit row.
#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn set_enabled_persists_and_audits() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/instance-plugins/enhancement.dashboard-icons/enabled",
            &serde_json::json!({ "enabled": true }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(
        body["enabled"], true,
        "response must reflect the new enabled state"
    );

    let row = poll_system_audit_row(
        &app.db,
        uptrakit_audit_log::AuditActionType::INSTANCE_PLUGIN_TOGGLED,
    )
    .await;
    assert_eq!(
        uptrakit_audit_log::AuditActionType::INSTANCE_PLUGIN_TOGGLED,
        row.action_type
    );
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Success.as_str()
    );
    assert_eq!(row.target_type.as_deref(), Some("instance_plugin"));
    assert_eq!(
        row.target_id.as_deref(),
        Some("enhancement.dashboard-icons")
    );

    // V2 stateful: before snapshot is {} (AbsentView — no prior row existed).
    let before = row
        .before_snapshot
        .expect("stateful entry must have before_snapshot");
    assert_eq!(
        before,
        serde_json::json!({}),
        "before must be empty object for first-toggle (AbsentView)"
    );

    // V2 stateful: after snapshot reflects the new enabled state.
    let after = row
        .after_snapshot
        .expect("stateful entry must have after_snapshot");
    assert_eq!(
        after["plugin_type_id"],
        serde_json::json!("enhancement.dashboard-icons")
    );
    assert_eq!(after["enabled"], serde_json::json!(true));
}

/// PUT .../enabled with an unrecognised plugin type returns 404.
#[tokio::test]
async fn set_enabled_for_unknown_plugin_returns_404() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let status = client
        .put_json(
            "/api/v1/instance-plugins/totally_made_up/enabled",
            &serde_json::json!({ "enabled": true }),
        )
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::NOT_FOUND);
}

/// PUT .../enabled for a plugin that exists but is Tenant-scoped must return
/// 404 — the endpoint must not leak existence through a different status code.
#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn set_enabled_for_tenant_scoped_plugin_returns_404() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // `package-manager.apt` is a Tenant-scoped plugin in the default catalog.
    let status = client
        .put_json(
            "/api/v1/instance-plugins/package-manager.apt/enabled",
            &serde_json::json!({ "enabled": true }),
        )
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::NOT_FOUND);
}

/// PUT .../config on a kill-switch-only instance plugin (instance_config = None)
/// must return 400, not 500.
#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn upsert_config_for_kill_switch_only_plugin_returns_400() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/instance-plugins/enhancement.dashboard-icons/config",
            &serde_json::json!({ "config": {} }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
    let msg = body["error"].as_str().unwrap_or_default();
    assert!(
        msg.contains("no instance configuration schema"),
        "unexpected error message: {msg}"
    );
}

/// PUT .../config with a non-object `config` field is rejected by the Validate
/// impl before route logic runs — the handler must return 400.
#[cfg(feature = "dashboard-icons")]
#[tokio::test]
async fn upsert_config_validates_against_validate_trait() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, _body): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/instance-plugins/enhancement.dashboard-icons/config",
            &serde_json::json!({ "config": "not-an-object" }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
}

// TODO: upsert_config_validates_against_instance_config_schema_and_persists
//
// This test is deferred because `enhancement.dashboard-icons` has no
// `instance_config` descriptor (it is a kill-switch-only plugin), so covering
// schema validation and persistence would require introducing a synthetic test
// descriptor into the catalog — a significant harness change that is out of
// scope for the initial integration-test pass. Add a real instance_config
// descriptor to a future plugin and cover this path then.

/// Stub Instance-scoped plugin descriptor + catalog for the secret-masking
/// test below.
///
/// Task 5's registry assertion (`type_settings_and_instance_plugins_are_secret_free`)
/// proves no *real* plugin has a sensitive instance-config path today (the
/// deferred TODO above is exactly this gap — no live instance_config
/// descriptor exists yet), so this test injects a synthetic descriptor with
/// an explicit `sensitive_paths: &["auth_token"]` rather than searching for a
/// real leak — this is defense-in-depth for future secret-bearing
/// instance-config plugins.
mod secret_masking_fixture {
    use std::sync::{Arc, OnceLock};

    use uptrakit_plugin_infrastructure_core::InstanceConfigOps;
    use uptrakit_plugin_infrastructure_registry::{
        CatalogConfig, ConfigModel, ConfigOps, FormFieldDescriptor, InstancePluginStates,
        PluginCatalog, PluginConfigValidationError, PluginDescriptor, PluginFamily, PluginOps,
        PluginScope, RoleCreators,
    };

    pub(crate) const TYPE_ID: &str = "test.secret.instance-config";

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
    fn noop_instance_config_validate(
        _: &serde_json::Value,
    ) -> Result<(), PluginConfigValidationError> {
        Ok(())
    }

    static INSTANCE_CONFIG_OPS: InstanceConfigOps = InstanceConfigOps {
        form_schema: noop_form_schema,
        sample: noop_sample,
        validate: noop_instance_config_validate,
    };

    static DESCRIPTOR: OnceLock<PluginDescriptor> = OnceLock::new();

    fn descriptor() -> &'static PluginDescriptor {
        DESCRIPTOR.get_or_init(|| PluginDescriptor {
            type_id: TYPE_ID,
            display_name: "Test Instance Config Secret Plugin",
            family: PluginFamily::Software,
            config_model: ConfigModel::None,
            capabilities: &[],
            scope: PluginScope::Instance,
            instance_config: Some(&INSTANCE_CONFIG_OPS),
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
            type_settings: None,
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
            InstancePluginStates::from_pairs([(TYPE_ID, true)]),
        )
        .map(|catalog| Arc::new(catalog) as Arc<dyn PluginOps>)
    }
}

#[tokio::test]
async fn instance_plugin_config_masked_and_restored() {
    let app = TestApp::with_plugin_surfaces(Some(
        secret_masking_fixture::plugin_ops()
            .expect("build stub plugin catalog for instance-config masking test"),
    ))
    .await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let seed_status = client
        .put_json(
            &format!(
                "/api/v1/instance-plugins/{}/config",
                secret_masking_fixture::TYPE_ID
            ),
            &serde_json::json!({ "config": { "auth_token": "t1", "filter": "x" } }),
        )
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(seed_status, http::StatusCode::OK, "seed PUT must succeed");

    let (get_status, get_body): (_, serde_json::Value) = client
        .get(&format!(
            "/api/v1/instance-plugins/{}",
            secret_masking_fixture::TYPE_ID
        ))
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(get_status, http::StatusCode::OK);
    assert_eq!(
        get_body["current_config"]["auth_token"],
        serde_json::json!("***"),
        "sensitive path must be masked on GET"
    );
    assert_eq!(
        get_body["current_config"]["filter"],
        serde_json::json!("x"),
        "non-sensitive path must pass through unmasked"
    );

    let sentinel_status = client
        .put_json(
            &format!(
                "/api/v1/instance-plugins/{}/config",
                secret_masking_fixture::TYPE_ID
            ),
            &serde_json::json!({ "config": { "auth_token": "***", "filter": "y" } }),
        )
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(
        sentinel_status,
        http::StatusCode::OK,
        "sentinel PUT must be accepted"
    );

    let snapshot = app.state.instance_plugin_snapshot.load();
    let stored = snapshot
        .get(secret_masking_fixture::TYPE_ID)
        .expect("row must exist in snapshot after PUT");
    assert_eq!(
        stored.config["auth_token"],
        serde_json::json!("t1"),
        "sentinel must be restored to the real stored secret"
    );
    assert_eq!(
        stored.config["filter"],
        serde_json::json!("y"),
        "non-sensitive field must be updated"
    );
}
