use rootcause::prelude::*;
use sea_orm::{
    ActiveModelBehavior, ActiveModelTrait, DatabaseConnection, EntityTrait, IntoActiveModel,
    PaginatorTrait, QuerySelect,
};
use uptrakit_shared_db::entity::prelude::*;

use super::error::{DbMigrateError, Result};

/// Names of all application tables in FK-safe copy order
/// (parent tables first, leaf tables last).
///
/// This is the reverse of the `drop_tables!` list in the initial migration's
/// `down()` function, excluding the `controller_events` table that was dropped
/// in a later migration.
///
/// Used by tests to verify that every app table is covered by `copy_all`.
#[cfg(test)]
pub(crate) const COPY_ORDER: &[&str] = &[
    "tenants",
    "users",
    "ca_certificates",
    "roles",
    "permissions",
    "global_settings",
    "role_permissions",
    "oidc_providers",
    "user_roles",
    "user_oidc_links",
    "sessions",
    "api_tokens",
    "settings",
    "settings_versions",
    "enrollment_tokens",
    "services",
    "service_certificates",
    "system_services",
    "system_service_certificates",
    "hosts",
    "service_hosts",
    "plugin_configs",
    "software_items",
    "host_software_items",
    "host_software_item_plugins",
    "autodiscovery_ignores",
    "mqtt_clients",
    "mqtt_leases",
    "update_history",
    "update_output_lines",
    "pending_device_flows",
    "pending_oidc_flows",
    "pending_account_links",
    "pending_oidc_token_exchanges",
    "pending_oidc_registrations",
    "api_rate_limits",
    "scheduled_tasks",
    "notification_channels",
    "notification_rules",
    "notification_log",
];

/// Batch-copy all rows from `src` to `dst` in FK-safe order.
///
/// Returns the total number of rows copied across all tables.
pub async fn copy_all(
    src: &DatabaseConnection,
    dst: &DatabaseConnection,
    batch_size: u64,
) -> Result<u64> {
    let mut total = 0u64;

    macro_rules! copy {
        ($entity:ty, $name:literal) => {
            total += migrate_table::<$entity>($name, src, dst, batch_size).await?;
        };
    }

    copy!(Tenant, "tenants");
    copy!(User, "users");
    copy!(CaCertificate, "ca_certificates");
    copy!(Role, "roles");
    copy!(Permission, "permissions");
    copy!(GlobalSetting, "global_settings");
    copy!(RolePermission, "role_permissions");
    copy!(OidcProvider, "oidc_providers");
    copy!(UserRole, "user_roles");
    copy!(UserOidcLink, "user_oidc_links");
    copy!(Session, "sessions");
    copy!(ApiToken, "api_tokens");
    copy!(Setting, "settings");
    copy!(SettingsVersion, "settings_versions");
    copy!(EnrollmentToken, "enrollment_tokens");
    copy!(Service, "services");
    copy!(ServiceCertificate, "service_certificates");
    copy!(SystemService, "system_services");
    copy!(SystemServiceCertificate, "system_service_certificates");
    copy!(Host, "hosts");
    copy!(ServiceHost, "service_hosts");
    copy!(PluginConfig, "plugin_configs");
    copy!(SoftwareItem, "software_items");
    copy!(HostSoftwareItem, "host_software_items");
    copy!(HostSoftwareItemPlugin, "host_software_item_plugins");
    copy!(AutodiscoveryIgnore, "autodiscovery_ignores");
    copy!(MqttClient, "mqtt_clients");
    copy!(MqttLease, "mqtt_leases");
    copy!(UpdateHistory, "update_history");
    copy!(UpdateOutputLine, "update_output_lines");
    copy!(PendingDeviceFlow, "pending_device_flows");
    copy!(PendingOidcFlow, "pending_oidc_flows");
    copy!(PendingAccountLink, "pending_account_links");
    copy!(PendingOidcTokenExchange, "pending_oidc_token_exchanges");
    copy!(PendingOidcRegistration, "pending_oidc_registrations");
    copy!(ApiRateLimit, "api_rate_limits");
    copy!(ScheduledTask, "scheduled_tasks");
    copy!(NotificationChannel, "notification_channels");
    copy!(NotificationRule, "notification_rules");
    copy!(NotificationLog, "notification_log");

    Ok(total)
}

/// Delete all application rows from `dst` in FK-safe order (leaves first).
///
/// This is the reverse of the copy order (same as the migration `down()` drop
/// list). Running this before `copy_all` removes any seed data written by
/// `run_migrations` and ensures a clean slate for the import.
pub async fn clean_all(dst: &DatabaseConnection) -> Result<()> {
    macro_rules! clean {
        ($entity:ty, $name:literal) => {
            clean_table::<$entity>($name, dst).await?;
        };
    }

    clean!(NotificationLog, "notification_log");
    clean!(NotificationRule, "notification_rules");
    clean!(NotificationChannel, "notification_channels");
    clean!(ScheduledTask, "scheduled_tasks");
    clean!(ApiRateLimit, "api_rate_limits");
    clean!(PendingOidcRegistration, "pending_oidc_registrations");
    clean!(PendingOidcTokenExchange, "pending_oidc_token_exchanges");
    clean!(PendingAccountLink, "pending_account_links");
    clean!(PendingOidcFlow, "pending_oidc_flows");
    clean!(PendingDeviceFlow, "pending_device_flows");
    clean!(UpdateOutputLine, "update_output_lines");
    clean!(UpdateHistory, "update_history");
    clean!(MqttLease, "mqtt_leases");
    clean!(MqttClient, "mqtt_clients");
    clean!(AutodiscoveryIgnore, "autodiscovery_ignores");
    clean!(HostSoftwareItemPlugin, "host_software_item_plugins");
    clean!(HostSoftwareItem, "host_software_items");
    clean!(SoftwareItem, "software_items");
    clean!(PluginConfig, "plugin_configs");
    clean!(ServiceHost, "service_hosts");
    clean!(Host, "hosts");
    clean!(SystemServiceCertificate, "system_service_certificates");
    clean!(SystemService, "system_services");
    clean!(ServiceCertificate, "service_certificates");
    clean!(Service, "services");
    clean!(EnrollmentToken, "enrollment_tokens");
    clean!(SettingsVersion, "settings_versions");
    clean!(Setting, "settings");
    clean!(ApiToken, "api_tokens");
    clean!(Session, "sessions");
    clean!(UserOidcLink, "user_oidc_links");
    clean!(OidcProvider, "oidc_providers");
    clean!(UserRole, "user_roles");
    clean!(RolePermission, "role_permissions");
    clean!(GlobalSetting, "global_settings");
    clean!(Permission, "permissions");
    clean!(Role, "roles");
    clean!(CaCertificate, "ca_certificates");
    clean!(User, "users");
    clean!(Tenant, "tenants");

    Ok(())
}

/// Verify row counts match between `src` and `dst` for every application table.
///
/// Returns the total number of rows verified, or the first
/// [`DbMigrateError::Mismatch`] found.
pub async fn verify_all(src: &DatabaseConnection, dst: &DatabaseConnection) -> Result<u64> {
    let mut total = 0u64;

    macro_rules! verify {
        ($entity:ty, $name:literal) => {
            total += verify_table::<$entity>($name, src, dst).await?;
        };
    }

    verify!(Tenant, "tenants");
    verify!(User, "users");
    verify!(CaCertificate, "ca_certificates");
    verify!(Role, "roles");
    verify!(Permission, "permissions");
    verify!(GlobalSetting, "global_settings");
    verify!(RolePermission, "role_permissions");
    verify!(OidcProvider, "oidc_providers");
    verify!(UserRole, "user_roles");
    verify!(UserOidcLink, "user_oidc_links");
    verify!(Session, "sessions");
    verify!(ApiToken, "api_tokens");
    verify!(Setting, "settings");
    verify!(SettingsVersion, "settings_versions");
    verify!(EnrollmentToken, "enrollment_tokens");
    verify!(Service, "services");
    verify!(ServiceCertificate, "service_certificates");
    verify!(SystemService, "system_services");
    verify!(SystemServiceCertificate, "system_service_certificates");
    verify!(Host, "hosts");
    verify!(ServiceHost, "service_hosts");
    verify!(PluginConfig, "plugin_configs");
    verify!(SoftwareItem, "software_items");
    verify!(HostSoftwareItem, "host_software_items");
    verify!(HostSoftwareItemPlugin, "host_software_item_plugins");
    verify!(AutodiscoveryIgnore, "autodiscovery_ignores");
    verify!(MqttClient, "mqtt_clients");
    verify!(MqttLease, "mqtt_leases");
    verify!(UpdateHistory, "update_history");
    verify!(UpdateOutputLine, "update_output_lines");
    verify!(PendingDeviceFlow, "pending_device_flows");
    verify!(PendingOidcFlow, "pending_oidc_flows");
    verify!(PendingAccountLink, "pending_account_links");
    verify!(PendingOidcTokenExchange, "pending_oidc_token_exchanges");
    verify!(PendingOidcRegistration, "pending_oidc_registrations");
    verify!(ApiRateLimit, "api_rate_limits");
    verify!(ScheduledTask, "scheduled_tasks");
    verify!(NotificationChannel, "notification_channels");
    verify!(NotificationRule, "notification_rules");
    verify!(NotificationLog, "notification_log");

    Ok(total)
}

// ── Generic helpers ──────────────────────────────────────────────────────────

/// Batch-copy all rows of one entity table from `src` to `dst`.
///
/// Rows are read in batches of `batch_size` using offset pagination and
/// bulk-inserted into `dst`. Progress is printed to stderr.
///
/// Returns the number of rows copied.
async fn migrate_table<E>(
    name: &'static str,
    src: &DatabaseConnection,
    dst: &DatabaseConnection,
    batch_size: u64,
) -> Result<u64>
where
    E: EntityTrait + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Send + Sync + 'static,
    E::ActiveModel: ActiveModelTrait<Entity = E> + ActiveModelBehavior + Send + 'static,
{
    let total = E::find().count(src).await.map_err(|db_err| {
        report!(DbMigrateError::TableOp {
            table: name,
            db_err
        })
    })?;

    let mut copied = 0u64;
    let mut offset = 0u64;

    loop {
        let batch = E::find()
            .offset(offset)
            .limit(batch_size)
            .all(src)
            .await
            .map_err(|db_err| {
                report!(DbMigrateError::TableOp {
                    table: name,
                    db_err
                })
            })?;

        if batch.is_empty() {
            break;
        }

        let n = batch.len() as u64;
        let active: Vec<_> = batch
            .into_iter()
            .map(IntoActiveModel::into_active_model)
            .collect();

        E::insert_many(active).exec(dst).await.map_err(|db_err| {
            report!(DbMigrateError::TableOp {
                table: name,
                db_err
            })
        })?;

        copied += n;
        offset += n;

        eprintln!("  {name}: {copied}/{total} rows");
    }

    if total == 0 {
        eprintln!("  {name}: 0 rows (empty)");
    }

    Ok(copied)
}

/// Delete all rows from one entity table.
async fn clean_table<E: EntityTrait>(name: &'static str, dst: &DatabaseConnection) -> Result<()> {
    E::delete_many()
        .exec(dst)
        .await
        .map(|_| ())
        .map_err(|db_err| {
            report!(DbMigrateError::TableOp {
                table: name,
                db_err
            })
        })
}

/// Verify row count matches between `src` and `dst` for one entity table.
async fn verify_table<E>(
    name: &'static str,
    src: &DatabaseConnection,
    dst: &DatabaseConnection,
) -> Result<u64>
where
    E: EntityTrait + 'static,
    E::Model: Send + Sync + 'static,
{
    let src_count = E::find().count(src).await.map_err(|db_err| {
        report!(DbMigrateError::TableOp {
            table: name,
            db_err
        })
    })?;
    let dst_count = E::find().count(dst).await.map_err(|db_err| {
        report!(DbMigrateError::TableOp {
            table: name,
            db_err
        })
    })?;

    if src_count != dst_count {
        bail!(DbMigrateError::Mismatch {
            table: name,
            src: src_count,
            dst: dst_count
        });
    }

    Ok(src_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_order_has_all_40_tables() {
        assert_eq!(
            COPY_ORDER.len(),
            40,
            "COPY_ORDER must list all 40 app tables"
        );
    }
}
