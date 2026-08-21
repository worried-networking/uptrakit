//! Single visibility predicate for plugin descriptors. Centralizes the
//! "is this plugin visible to this user" check so route handlers, the
//! surface registry, and any future filter call into one helper.

use std::sync::Arc;

use arc_swap::ArcSwap;
use uptrakit_controller_core::access::{AccessContext, AccessEngine};
use uptrakit_plugin_infrastructure_registry::{
    PluginDescriptor, PluginOps, PluginScope, PluginTypeId,
};
use uptrakit_shared_types::access::{Decision, actions};
use uptrakit_web_api_queries::instance_plugin_settings::InstancePluginSnapshot;

/// Effective enablement of a plugin's runtime functionality (ADR-0033):
/// `Tenant` scope is always effective; `Instance` scope is effective only
/// when the boot catalog constructed the plugin (`instance_enabled`) AND the
/// live snapshot says enabled; an unknown `type_id` is never effective
/// (fail-closed). The snapshot is consulted only in the `Instance` arm —
/// Tenant plugins have no row.
pub fn effective_instance_enabled(
    plugin_ops: &dyn PluginOps,
    snapshot: &InstancePluginSnapshot,
    type_id: &PluginTypeId,
) -> bool {
    match plugin_ops.get(type_id) {
        Some(descriptor) if descriptor.scope == PluginScope::Instance => {
            plugin_ops.instance_enabled(type_id) && snapshot.enabled(type_id.as_str())
        }
        Some(_) => true,
        None => false,
    }
}

/// Returns `true` if the user is allowed to see the plugin in any tenant-
/// facing listing, surface, or detail response.
///
/// - `Tenant`-scoped plugins: always visible.
/// - `Instance`-scoped + **effectively** enabled (boot ∧ live): visible to
///   everyone.
/// - `Instance`-scoped + not effectively enabled: visible only to users with
///   `system.settings:manage` authority (instance owners).
///
/// `PluginScope` is `#[non_exhaustive]`; the wildcard arm logs a warning
/// and defaults to visible — the safer side for instance owners
/// (admin debugging) at the cost of a temporary leak should a future
/// scope variant ship before this predicate is updated.
pub fn is_plugin_visible_to_user(
    descriptor: &PluginDescriptor,
    plugin_ops: &dyn PluginOps,
    snapshot: &InstancePluginSnapshot,
    engine: &AccessEngine,
    ctx: &AccessContext,
) -> bool {
    match descriptor.scope {
        PluginScope::Tenant => true,
        PluginScope::Instance => {
            let enabled = effective_instance_enabled(
                plugin_ops,
                snapshot,
                &PluginTypeId::from_static(descriptor.type_id),
            );
            enabled
                || matches!(
                    engine.authorize(ctx, &actions::SYSTEM_SETTINGS_MANAGE),
                    Decision::Allow
                )
        }
        _ => {
            tracing::warn!(
                plugin = descriptor.type_id,
                scope = %descriptor.scope,
                "unknown PluginScope variant; defaulting to visible",
            );
            true
        }
    }
}

/// [`crate::surface_proxy::SurfaceProviderVisibility`] backed by effective
/// enablement (ADR-0033). Holds live handles — the snapshot ArcSwap is
/// loaded on every call, so a runtime disable takes effect immediately on
/// every leg, including provider-origin invocations resolved inside the
/// proxy. Capturing a loaded snapshot instead would freeze boot state and
/// silently break disable-is-immediate on exactly that leg.
pub struct PluginEffectiveEnablement {
    plugin_ops: Arc<dyn PluginOps>,
    snapshot: Arc<ArcSwap<InstancePluginSnapshot>>,
}

impl PluginEffectiveEnablement {
    /// Creates the filter from the live plugin-ops and snapshot handles.
    #[must_use]
    pub fn new(
        plugin_ops: Arc<dyn PluginOps>,
        snapshot: Arc<ArcSwap<InstancePluginSnapshot>>,
    ) -> Self {
        Self {
            plugin_ops,
            snapshot,
        }
    }
}

impl crate::surface_proxy::SurfaceProviderVisibility for PluginEffectiveEnablement {
    fn plugin_provider_visible(&self, provider_id: &str) -> bool {
        // A Plugin-kind provider id IS the plugin type id (ADR-0034).
        // Ids reaching this filter are pre-gated to Plugin-kind by the
        // registry call sites (they check `provider_kind == Plugin` before
        // consulting the filter — the registry itself holds all kinds);
        // `effective_instance_enabled` is fail-closed on unknown type ids.
        let snapshot = self.snapshot.load_full();
        effective_instance_enabled(
            self.plugin_ops.as_ref(),
            snapshot.as_ref(),
            &PluginTypeId::new(provider_id),
        )
    }
}

/// Shared fixtures for both the ungated `effective_instance_enabled` tests
/// and the `db-sqlite`-gated `is_plugin_visible_to_user` engine tests below —
/// neither touches a database, so this module carries no feature gate.
#[cfg(test)]
mod test_support {
    use std::sync::OnceLock;

    use uptrakit_plugin_infrastructure_registry::{
        ConfigModel, ConfigOps, PluginFamily, RoleCreators,
    };

    use super::*;

    /// Noop functions for test descriptor.
    fn noop_validate(
        _: &serde_json::Value,
    ) -> Result<(), uptrakit_plugin_infrastructure_registry::PluginConfigValidationError> {
        Ok(())
    }
    fn noop_normalize(
        v: &serde_json::Value,
    ) -> Result<
        serde_json::Value,
        uptrakit_plugin_infrastructure_registry::PluginConfigValidationError,
    > {
        Ok(v.clone())
    }
    fn noop_sample() -> serde_json::Value {
        serde_json::json!({})
    }
    fn noop_form_schema() -> Vec<uptrakit_plugin_infrastructure_registry::FormFieldDescriptor> {
        vec![]
    }
    fn noop_validate_identifier(
        _: &str,
    ) -> Result<(), uptrakit_plugin_infrastructure_registry::PluginConfigValidationError> {
        Ok(())
    }

    /// Test fixture for a Tenant-scoped plugin.
    static TENANT_PLUGIN_DESCRIPTOR: OnceLock<PluginDescriptor> = OnceLock::new();

    pub(super) fn tenant_plugin_descriptor() -> &'static PluginDescriptor {
        TENANT_PLUGIN_DESCRIPTOR.get_or_init(|| PluginDescriptor {
            type_id: "test.tenant.scoped",
            display_name: "Test Tenant Plugin",
            family: PluginFamily::Software,
            config_model: ConfigModel::None,
            capabilities: &[],
            scope: PluginScope::Tenant,
            instance_config: None,
            sensitive_paths: &[],
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

    /// Test fixture for an Instance-scoped plugin.
    static INSTANCE_PLUGIN_DESCRIPTOR: OnceLock<PluginDescriptor> = OnceLock::new();

    pub(super) fn instance_plugin_descriptor() -> &'static PluginDescriptor {
        INSTANCE_PLUGIN_DESCRIPTOR.get_or_init(|| PluginDescriptor {
            type_id: "test.instance.scoped",
            display_name: "Test Instance Plugin",
            family: PluginFamily::Software,
            config_model: ConfigModel::None,
            capabilities: &[],
            scope: PluginScope::Instance,
            instance_config: None,
            sensitive_paths: &[],
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

    /// Catalog over the two local fixture descriptors with the Instance
    /// fixture's boot state set to `boot_enabled`. Returns the fallible
    /// build so `expect` stays inside `#[test]` bodies.
    pub(super) fn test_plugin_ops(
        boot_enabled: bool,
    ) -> uptrakit_plugin_infrastructure_core::Result<
        uptrakit_plugin_infrastructure_registry::PluginCatalog,
    > {
        uptrakit_plugin_infrastructure_registry::PluginCatalog::new(
            vec![tenant_plugin_descriptor(), instance_plugin_descriptor()],
            &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
            uptrakit_plugin_infrastructure_registry::InstancePluginStates::from_pairs([(
                "test.instance.scoped",
                boot_enabled,
            )]),
        )
    }

    /// A `db-sqlite`-only helper (an unused-function warning in a
    /// postgres-only test build is the tell that this boundary slipped) —
    /// kept here so both gated and ungated sibling modules share one fixture
    /// surface without duplicating the descriptor plumbing above.
    #[cfg(feature = "db-sqlite")]
    pub(super) fn enabled_instance_snapshot() -> InstancePluginSnapshot {
        InstancePluginSnapshot::empty().with_upserted(
            "test.instance.scoped".to_string(),
            uptrakit_web_api_queries::instance_plugin_settings::InstancePluginRow {
                enabled: true,
                config: serde_json::json!({}),
                updated_at: time::OffsetDateTime::now_utc(),
            },
        )
    }

    #[cfg(feature = "db-sqlite")]
    pub(super) fn disabled_instance_snapshot() -> InstancePluginSnapshot {
        InstancePluginSnapshot::empty().with_upserted(
            "test.instance.scoped".to_string(),
            uptrakit_web_api_queries::instance_plugin_settings::InstancePluginRow {
                enabled: false,
                config: serde_json::json!({}),
                updated_at: time::OffsetDateTime::now_utc(),
            },
        )
    }
}

/// Pure `effective_instance_enabled` coverage — no DB, must keep running in
/// a postgres-only build.
#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    #[test]
    fn effective_instance_enabled_tenant_scope_always_true() {
        let ops = test_plugin_ops(false).expect("test catalog builds");
        let snapshot = InstancePluginSnapshot::empty();
        assert!(effective_instance_enabled(
            &ops,
            &snapshot,
            &PluginTypeId::from_static(tenant_plugin_descriptor().type_id),
        ));
    }

    #[test]
    fn effective_instance_enabled_boot_and_live_enabled_true() {
        let ops = test_plugin_ops(true).expect("test catalog builds");
        let snapshot = InstancePluginSnapshot::empty().with_upserted(
            "test.instance.scoped".to_string(),
            uptrakit_web_api_queries::instance_plugin_settings::InstancePluginRow {
                enabled: true,
                config: serde_json::json!({}),
                updated_at: time::OffsetDateTime::now_utc(),
            },
        );
        assert!(effective_instance_enabled(
            &ops,
            &snapshot,
            &PluginTypeId::from_static(instance_plugin_descriptor().type_id),
        ));
    }

    #[test]
    fn effective_instance_enabled_live_disabled_false() {
        let ops = test_plugin_ops(true).expect("test catalog builds");
        let snapshot = InstancePluginSnapshot::empty().with_upserted(
            "test.instance.scoped".to_string(),
            uptrakit_web_api_queries::instance_plugin_settings::InstancePluginRow {
                enabled: false,
                config: serde_json::json!({}),
                updated_at: time::OffsetDateTime::now_utc(),
            },
        );
        assert!(!effective_instance_enabled(
            &ops,
            &snapshot,
            &PluginTypeId::from_static(instance_plugin_descriptor().type_id),
        ));
    }

    /// ADR-0033: boot-disabled catalog (pending restart) means the plugin was
    /// never constructed at boot, so it is not effective even though the
    /// live row now says enabled.
    #[test]
    fn effective_instance_enabled_boot_disabled_pending_restart_false() {
        let ops = test_plugin_ops(false).expect("test catalog builds");
        let snapshot = InstancePluginSnapshot::empty().with_upserted(
            "test.instance.scoped".to_string(),
            uptrakit_web_api_queries::instance_plugin_settings::InstancePluginRow {
                enabled: true,
                config: serde_json::json!({}),
                updated_at: time::OffsetDateTime::now_utc(),
            },
        );
        assert!(!effective_instance_enabled(
            &ops,
            &snapshot,
            &PluginTypeId::from_static(instance_plugin_descriptor().type_id),
        ));
    }

    #[test]
    fn effective_instance_enabled_unknown_type_false() {
        let ops = test_plugin_ops(true).expect("test catalog builds");
        let snapshot = InstancePluginSnapshot::empty();
        assert!(!effective_instance_enabled(
            &ops,
            &snapshot,
            &PluginTypeId::new("test.unknown.type"),
        ));
    }
}

/// `is_plugin_visible_to_user` engine-backed coverage — needs a real
/// [`AccessEngine`] + [`AccessContext`] over an in-memory sqlite DB
/// (idiom shared with `middleware/action.rs`'s test module).
#[cfg(all(test, feature = "db-sqlite"))]
#[expect(
    clippy::expect_used,
    reason = "test code: panics on failure are acceptable"
)]
mod engine_tests {
    use sea_orm::DatabaseConnection;
    use uptrakit_shared_db::access_grants::{GrantSubject, NewGrant, insert_grant};
    use uptrakit_shared_types::access::{ActionPattern, Selector};

    use super::test_support::*;
    use super::*;
    use crate::test_harness::fixtures::default_tenant_id;
    use crate::test_harness::setup_migrated_db;

    async fn grant_system_settings_manage(db: &DatabaseConnection, user_id: uuid::Uuid) {
        let patterns = vec![
            "system.settings:manage"
                .parse::<ActionPattern>()
                .expect("valid pattern"),
        ];
        insert_grant(
            db,
            NewGrant {
                subject: GrantSubject::User(user_id),
                // System-plane grants (e.g. `system.settings:manage`) must encode
                // tenant_id = NULL — see `access_grants::validate_write`'s
                // `(Plane::System, _, Some(_))` rejection.
                tenant_id: None,
                patterns: &patterns,
                selector: Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("insert grant");
    }

    #[tokio::test]
    async fn tenant_scoped_always_visible_without_authority() {
        let db = setup_migrated_db().await;
        let engine = AccessEngine::new(db.clone());
        let tenant_id = default_tenant_id(&db).await;
        let user_id = uuid::Uuid::now_v7();
        let ctx = engine
            .context(tenant_id, user_id, None)
            .await
            .expect("context");
        let ops = test_plugin_ops(false).expect("test catalog builds");
        let snapshot = InstancePluginSnapshot::empty();
        assert!(is_plugin_visible_to_user(
            tenant_plugin_descriptor(),
            &ops,
            &snapshot,
            &engine,
            &ctx,
        ));
    }

    #[tokio::test]
    async fn instance_scoped_enabled_visible_without_authority() {
        let db = setup_migrated_db().await;
        let engine = AccessEngine::new(db.clone());
        let tenant_id = default_tenant_id(&db).await;
        let user_id = uuid::Uuid::now_v7();
        let ctx = engine
            .context(tenant_id, user_id, None)
            .await
            .expect("context");
        let ops = test_plugin_ops(true).expect("test catalog builds");
        let snapshot = enabled_instance_snapshot();
        assert!(is_plugin_visible_to_user(
            instance_plugin_descriptor(),
            &ops,
            &snapshot,
            &engine,
            &ctx,
        ));
    }

    #[tokio::test]
    async fn instance_scoped_disabled_hidden_without_system_settings_manage_grant() {
        let db = setup_migrated_db().await;
        let engine = AccessEngine::new(db.clone());
        let tenant_id = default_tenant_id(&db).await;
        let user_id = uuid::Uuid::now_v7();
        let ctx = engine
            .context(tenant_id, user_id, None)
            .await
            .expect("context");
        let ops = test_plugin_ops(true).expect("test catalog builds");
        let snapshot = disabled_instance_snapshot();
        assert!(!is_plugin_visible_to_user(
            instance_plugin_descriptor(),
            &ops,
            &snapshot,
            &engine,
            &ctx,
        ));
    }

    #[tokio::test]
    async fn instance_scoped_disabled_visible_with_system_settings_manage_grant() {
        let db = setup_migrated_db().await;
        let engine = AccessEngine::new(db.clone());
        let tenant_id = default_tenant_id(&db).await;
        let user_id = uuid::Uuid::now_v7();
        grant_system_settings_manage(&db, user_id).await;
        let ctx = engine
            .context(tenant_id, user_id, None)
            .await
            .expect("context");
        let ops = test_plugin_ops(true).expect("test catalog builds");
        let snapshot = disabled_instance_snapshot();
        assert!(is_plugin_visible_to_user(
            instance_plugin_descriptor(),
            &ops,
            &snapshot,
            &engine,
            &ctx,
        ));
    }

    /// ADR-0033: boot-disabled catalog (pending restart) means the plugin was
    /// never constructed at boot, so it stays hidden even though the live
    /// row now says enabled, absent `system.settings:manage` authority.
    #[tokio::test]
    async fn pending_restart_enabled_hidden_without_grant() {
        let db = setup_migrated_db().await;
        let engine = AccessEngine::new(db.clone());
        let tenant_id = default_tenant_id(&db).await;
        let user_id = uuid::Uuid::now_v7();
        let ctx = engine
            .context(tenant_id, user_id, None)
            .await
            .expect("context");
        let ops = test_plugin_ops(false).expect("test catalog builds");
        let snapshot = enabled_instance_snapshot();
        assert!(!is_plugin_visible_to_user(
            instance_plugin_descriptor(),
            &ops,
            &snapshot,
            &engine,
            &ctx,
        ));
    }

    /// ADR-0033: `system.settings:manage` authority still overrides a
    /// pending-restart-enabled plugin — instance owners can see it to
    /// trigger the restart.
    #[tokio::test]
    async fn pending_restart_enabled_visible_with_grant() {
        let db = setup_migrated_db().await;
        let engine = AccessEngine::new(db.clone());
        let tenant_id = default_tenant_id(&db).await;
        let user_id = uuid::Uuid::now_v7();
        grant_system_settings_manage(&db, user_id).await;
        let ctx = engine
            .context(tenant_id, user_id, None)
            .await
            .expect("context");
        let ops = test_plugin_ops(false).expect("test catalog builds");
        let snapshot = enabled_instance_snapshot();
        assert!(is_plugin_visible_to_user(
            instance_plugin_descriptor(),
            &ops,
            &snapshot,
            &engine,
            &ctx,
        ));
    }
}
