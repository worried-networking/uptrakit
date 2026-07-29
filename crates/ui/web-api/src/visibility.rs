//! Single visibility predicate for plugin descriptors. Centralizes the
//! "is this plugin visible to this user" check so route handlers, the
//! surface registry, and any future filter call into one helper.

use uptrakit_plugin_infrastructure_registry::{
    PluginDescriptor, PluginOps, PluginScope, PluginTypeId,
};
use uptrakit_web_api_queries::instance_plugin_settings::InstancePluginSnapshot;
use uptrakit_web_api_types::permissions::Permission;

use crate::middleware::require_auth::AuthenticatedUser;

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
///   `ManageGlobalSettings` (instance owners).
///
/// `PluginScope` is `#[non_exhaustive]`; the wildcard arm logs a warning
/// and defaults to visible — the safer side for instance owners
/// (admin debugging) at the cost of a temporary leak should a future
/// scope variant ship before this predicate is updated.
pub fn is_plugin_visible_to_user(
    descriptor: &PluginDescriptor,
    plugin_ops: &dyn PluginOps,
    snapshot: &InstancePluginSnapshot,
    user: &AuthenticatedUser,
) -> bool {
    match descriptor.scope {
        PluginScope::Tenant => true,
        PluginScope::Instance => {
            let enabled = effective_instance_enabled(
                plugin_ops,
                snapshot,
                &PluginTypeId::from_static(descriptor.type_id),
            );
            enabled || user.has_permission(Permission::ManageGlobalSettings)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;
    use uptrakit_controller_core::auth::AuthMethod;
    use uptrakit_plugin_infrastructure_registry::{
        ConfigModel, ConfigOps, PluginFamily, RoleCreators,
    };

    /// Noop functions for test descriptor.
    fn noop_validate(
        _: &serde_json::Value,
    ) -> Result<(), uptrakit_plugin_infrastructure_registry::PluginConfigValidationError> {
        Ok(())
    }
    fn noop_mask(v: &serde_json::Value) -> serde_json::Value {
        v.clone()
    }
    fn noop_restore(_: &mut serde_json::Value, _: &serde_json::Value) {}
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

    fn tenant_plugin_descriptor() -> &'static PluginDescriptor {
        TENANT_PLUGIN_DESCRIPTOR.get_or_init(|| PluginDescriptor {
            type_id: "test.tenant.scoped",
            display_name: "Test Tenant Plugin",
            family: PluginFamily::Software,
            config_model: ConfigModel::None,
            capabilities: &[],
            scope: PluginScope::Tenant,
            instance_config: None,
            config: ConfigOps {
                validate: noop_validate,
                mask_secrets: noop_mask,
                restore_secrets: noop_restore,
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

    fn instance_plugin_descriptor() -> &'static PluginDescriptor {
        INSTANCE_PLUGIN_DESCRIPTOR.get_or_init(|| PluginDescriptor {
            type_id: "test.instance.scoped",
            display_name: "Test Instance Plugin",
            family: PluginFamily::Software,
            config_model: ConfigModel::None,
            capabilities: &[],
            scope: PluginScope::Instance,
            instance_config: None,
            config: ConfigOps {
                validate: noop_validate,
                mask_secrets: noop_mask,
                restore_secrets: noop_restore,
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

    /// Create a tenant-level user with no admin permissions.
    fn tenant_user() -> AuthenticatedUser {
        AuthenticatedUser::new(
            uuid::Uuid::nil(),
            AuthMethod::Password,
            vec![Permission::ViewSoftware],
            None,
        )
    }

    /// Create an instance owner with ManageGlobalSettings permission.
    fn admin_user() -> AuthenticatedUser {
        AuthenticatedUser::new(
            uuid::Uuid::nil(),
            AuthMethod::Password,
            vec![Permission::ManageGlobalSettings],
            None,
        )
    }

    /// Catalog over the two local fixture descriptors with the Instance
    /// fixture's boot state set to `boot_enabled`. Returns the fallible
    /// build so `expect` stays inside `#[test]` bodies.
    fn test_plugin_ops(
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

    #[test]
    fn tenant_scoped_always_visible() {
        let ops = test_plugin_ops(false).expect("test catalog builds");
        let snapshot = InstancePluginSnapshot::empty();
        let user = tenant_user();
        assert!(is_plugin_visible_to_user(
            tenant_plugin_descriptor(),
            &ops,
            &snapshot,
            &user
        ));
    }

    #[test]
    fn instance_scoped_enabled_visible_to_tenant_user() {
        let ops = test_plugin_ops(true).expect("test catalog builds");
        let snapshot = InstancePluginSnapshot::empty().with_upserted(
            "test.instance.scoped".to_string(),
            uptrakit_web_api_queries::instance_plugin_settings::InstancePluginRow {
                enabled: true,
                config: serde_json::json!({}),
                updated_at: time::OffsetDateTime::now_utc(),
            },
        );
        let user = tenant_user();
        assert!(is_plugin_visible_to_user(
            instance_plugin_descriptor(),
            &ops,
            &snapshot,
            &user
        ));
    }

    #[test]
    fn instance_scoped_disabled_hidden_from_tenant_user() {
        let ops = test_plugin_ops(true).expect("test catalog builds");
        let snapshot = InstancePluginSnapshot::empty().with_upserted(
            "test.instance.scoped".to_string(),
            uptrakit_web_api_queries::instance_plugin_settings::InstancePluginRow {
                enabled: false,
                config: serde_json::json!({}),
                updated_at: time::OffsetDateTime::now_utc(),
            },
        );
        let user = tenant_user();
        assert!(!is_plugin_visible_to_user(
            instance_plugin_descriptor(),
            &ops,
            &snapshot,
            &user
        ));
    }

    #[test]
    fn instance_scoped_disabled_visible_to_admin_user() {
        let ops = test_plugin_ops(true).expect("test catalog builds");
        let snapshot = InstancePluginSnapshot::empty().with_upserted(
            "test.instance.scoped".to_string(),
            uptrakit_web_api_queries::instance_plugin_settings::InstancePluginRow {
                enabled: false,
                config: serde_json::json!({}),
                updated_at: time::OffsetDateTime::now_utc(),
            },
        );
        let user = admin_user();
        assert!(is_plugin_visible_to_user(
            instance_plugin_descriptor(),
            &ops,
            &snapshot,
            &user
        ));
    }

    #[test]
    fn instance_scoped_enabled_visible_to_admin_user() {
        let ops = test_plugin_ops(true).expect("test catalog builds");
        let snapshot = InstancePluginSnapshot::empty().with_upserted(
            "test.instance.scoped".to_string(),
            uptrakit_web_api_queries::instance_plugin_settings::InstancePluginRow {
                enabled: true,
                config: serde_json::json!({}),
                updated_at: time::OffsetDateTime::now_utc(),
            },
        );
        let user = admin_user();
        assert!(is_plugin_visible_to_user(
            instance_plugin_descriptor(),
            &ops,
            &snapshot,
            &user
        ));
    }

    /// ADR-0033: boot-disabled catalog (pending restart) means the plugin was
    /// never constructed at boot, so it stays hidden from tenant users even
    /// though the live row now says enabled.
    #[test]
    fn pending_restart_enabled_hidden_from_tenant_user() {
        let ops = test_plugin_ops(false).expect("test catalog builds");
        let snapshot = InstancePluginSnapshot::empty().with_upserted(
            "test.instance.scoped".to_string(),
            uptrakit_web_api_queries::instance_plugin_settings::InstancePluginRow {
                enabled: true,
                config: serde_json::json!({}),
                updated_at: time::OffsetDateTime::now_utc(),
            },
        );
        let user = tenant_user();
        assert!(!is_plugin_visible_to_user(
            instance_plugin_descriptor(),
            &ops,
            &snapshot,
            &user
        ));
    }

    /// ADR-0033: admin override still applies to a pending-restart-enabled
    /// plugin — instance owners can see it to trigger the restart.
    #[test]
    fn pending_restart_enabled_visible_to_admin_user() {
        let ops = test_plugin_ops(false).expect("test catalog builds");
        let snapshot = InstancePluginSnapshot::empty().with_upserted(
            "test.instance.scoped".to_string(),
            uptrakit_web_api_queries::instance_plugin_settings::InstancePluginRow {
                enabled: true,
                config: serde_json::json!({}),
                updated_at: time::OffsetDateTime::now_utc(),
            },
        );
        let user = admin_user();
        assert!(is_plugin_visible_to_user(
            instance_plugin_descriptor(),
            &ops,
            &snapshot,
            &user
        ));
    }
}
