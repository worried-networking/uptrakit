//! Single visibility predicate for plugin descriptors. Centralizes the
//! "is this plugin visible to this user" check so route handlers, the
//! surface registry, and any future filter call into one helper.

use uptrakit_plugin_infrastructure_registry::{PluginDescriptor, PluginScope};
use uptrakit_web_api_types::permissions::Permission;
use uptrakit_web_api_queries::instance_plugin_settings::InstancePluginSnapshot;

use crate::middleware::require_auth::AuthenticatedUser;

/// Returns `true` if the user is allowed to see the plugin in any tenant-
/// facing listing, surface, or detail response.
///
/// - `Tenant`-scoped plugins: always visible.
/// - `Instance`-scoped + enabled: visible to everyone.
/// - `Instance`-scoped + disabled: visible only to users with
///   `ManageGlobalSettings` (instance owners).
///
/// `PluginScope` is `#[non_exhaustive]`; the wildcard arm logs a warning
/// and defaults to visible — the safer side for instance owners
/// (admin debugging) at the cost of a temporary leak should a future
/// scope variant ship before this predicate is updated.
pub fn is_plugin_visible_to_user(
    descriptor: &PluginDescriptor,
    snapshot: &InstancePluginSnapshot,
    user: &AuthenticatedUser,
) -> bool {
    match descriptor.scope {
        PluginScope::Tenant => true,
        PluginScope::Instance => {
            let enabled = snapshot.enabled(descriptor.type_id);
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
    use uptrakit_plugin_infrastructure_registry::{
        ConfigModel, ConfigOps, PluginFamily, RoleCreators,
    };
    use uptrakit_controller_core::auth::AuthMethod;

    /// Noop functions for test descriptor.
    fn noop_validate(_: &serde_json::Value) -> Result<(), uptrakit_plugin_infrastructure_registry::PluginConfigValidationError> {
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
    fn noop_validate_identifier(_: &str) -> Result<(), uptrakit_plugin_infrastructure_registry::PluginConfigValidationError> {
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
            },
            surface_actions: None,
            surfaces: None,
            type_settings: None,
            config_test: None,
            sudo: None,
            raw_settings_keys: &[],
            global_provider_consumers: &[],
            migrations: None,
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
            },
            surface_actions: None,
            surfaces: None,
            type_settings: None,
            config_test: None,
            sudo: None,
            raw_settings_keys: &[],
            global_provider_consumers: &[],
            migrations: None,
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

    #[test]
    fn tenant_scoped_always_visible() {
        let snapshot = InstancePluginSnapshot::empty();
        let user = tenant_user();
        assert!(is_plugin_visible_to_user(
            tenant_plugin_descriptor(),
            &snapshot,
            &user
        ));
    }

    #[test]
    fn instance_scoped_enabled_visible_to_tenant_user() {
        let mut snapshot = InstancePluginSnapshot::empty();
        snapshot.upsert(
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
            &snapshot,
            &user
        ));
    }

    #[test]
    fn instance_scoped_disabled_hidden_from_tenant_user() {
        let mut snapshot = InstancePluginSnapshot::empty();
        snapshot.upsert(
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
            &snapshot,
            &user
        ));
    }

    #[test]
    fn instance_scoped_disabled_visible_to_admin_user() {
        let mut snapshot = InstancePluginSnapshot::empty();
        snapshot.upsert(
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
            &snapshot,
            &user
        ));
    }

    #[test]
    fn instance_scoped_enabled_visible_to_admin_user() {
        let mut snapshot = InstancePluginSnapshot::empty();
        snapshot.upsert(
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
            &snapshot,
            &user
        ));
    }
}
