//! Shared types and (in Phase C) per-table operations for the
//! `db-migrate` subcommand.
//!
//! This module hosts `TableMigrateError`, returned by both the registry
//! plugin-table helpers (in `plugin-infrastructure-registry`) and the
//! core helpers (added in Phase C). Hosting it in `shared-db` avoids a
//! dependency cycle: `plugin-infrastructure-core` already takes
//! `uptrakit-shared-db` as an optional dep, so the reverse direction is
//! impossible.

#![cfg(feature = "db-migrate")]

use rootcause::prelude::*;

/// Errors produced by per-table copy / clean / verify operations.
///
/// Surfaces the table name in both variants so the orchestrator in
/// `controller-runtime/db_migrate/tables.rs` can convert into the
/// existing `DbMigrateError::TableOp` and `DbMigrateError::Mismatch`
/// variants via a single `.context_to()?` boundary, without losing
/// context.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum TableMigrateError {
    /// A SeaORM driver error occurred for `table`.
    #[error("table `{table}` operation failed: {err}")]
    Db {
        table: &'static str,
        #[source]
        err: sea_orm::DbErr,
    },
    /// `verify` found different row counts for `table`.
    #[error("row count mismatch for table `{table}`: source={src}, target={dst}")]
    Mismatch {
        table: &'static str,
        src: u64,
        dst: u64,
    },
}

/// Module-local `Result` alias following the project's `Report<E>`
/// convention (see `docs/development/error-handling.md`).
pub type Result<T> = std::result::Result<T, Report<TableMigrateError>>;

// ── Per-table generic helpers (moved from controller-runtime/db_migrate/tables.rs) ─

use sea_orm::{
    ActiveModelBehavior, ActiveModelTrait, DatabaseConnection, EntityTrait, IntoActiveModel,
    PaginatorTrait, QuerySelect,
};

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
    let total = E::find()
        .count(src)
        .await
        .map_err(|err| report!(TableMigrateError::Db { table: name, err }))?;

    let mut copied = 0u64;
    let mut offset = 0u64;
    loop {
        let batch = E::find()
            .offset(offset)
            .limit(batch_size)
            .all(src)
            .await
            .map_err(|err| report!(TableMigrateError::Db { table: name, err }))?;
        if batch.is_empty() {
            break;
        }
        let n = batch.len() as u64;
        let active: Vec<_> = batch
            .into_iter()
            .map(IntoActiveModel::into_active_model)
            .collect();
        E::insert_many(active)
            .exec(dst)
            .await
            .map_err(|err| report!(TableMigrateError::Db { table: name, err }))?;
        copied += n;
        offset += n;
        eprintln!("  {name}: {copied}/{total} rows");
    }
    if total == 0 {
        eprintln!("  {name}: 0 rows (empty)");
    }
    Ok(copied)
}

async fn clean_table<E: EntityTrait>(name: &'static str, dst: &DatabaseConnection) -> Result<()> {
    E::delete_many()
        .exec(dst)
        .await
        .map(|_| ())
        .map_err(|err| report!(TableMigrateError::Db { table: name, err }))
}

async fn verify_table<E>(
    name: &'static str,
    src: &DatabaseConnection,
    dst: &DatabaseConnection,
) -> Result<u64>
where
    E: EntityTrait + 'static,
    E::Model: Send + Sync + 'static,
{
    let src_count = E::find()
        .count(src)
        .await
        .map_err(|err| report!(TableMigrateError::Db { table: name, err }))?;
    let dst_count = E::find()
        .count(dst)
        .await
        .map_err(|err| report!(TableMigrateError::Db { table: name, err }))?;
    if src_count != dst_count {
        bail!(TableMigrateError::Mismatch {
            table: name,
            src: src_count,
            dst: dst_count,
        });
    }
    Ok(src_count)
}

/// FK-safe order of all **core** application tables (no plugin tables).
///
/// Used by [`copy`] / [`clean`] / [`verify`] and by the
/// `migration_coverage_complete` integration test in
/// `controller-runtime`.
pub const CORE_COPY_ORDER: &[&str] = &[
    "tenants",
    "users",
    "ca_certificates",
    "crl_cache",
    "roles",
    "permissions",
    "global_settings",
    "data_encryption_keys",
    "role_permissions",
    "oidc_providers",
    "user_roles",
    "user_oidc_links",
    "sessions",
    "api_tokens",
    "revoked_token_jtis",
    "revoked_token_users",
    "email_change_requests",
    "settings",
    "settings_version",
    "enrollment_tokens",
    "services",
    "service_certificates",
    "system_services",
    "system_service_certificates",
    "global_service_config",
    "embedded_service_runtime_states",
    "hosts",
    "service_hosts",
    "host_tags",
    "host_tag_assignments",
    "plugin_configs",
    "plugin_type_settings",
    "tenant_discovery_allowlist",
    "host_discovery_allowlist",
    "software_items",
    "host_software_items",
    "host_software_item_plugins",
    "software_ignores",
    "tenant_service_config",
    "update_batches",
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
    "audit_logs",
    "system_audit_logs",
    "system_enrollment_tokens",
];

/// Copy every core table from `src` to `dst`. Returns total rows.
pub async fn copy(
    src: &DatabaseConnection,
    dst: &DatabaseConnection,
    batch_size: u64,
) -> Result<u64> {
    use crate::entity::prelude::*;

    let mut total = 0u64;

    macro_rules! copy {
        ($entity:ty, $name:literal) => {
            total += migrate_table::<$entity>($name, src, dst, batch_size).await?;
        };
    }

    copy!(Tenant, "tenants");
    copy!(User, "users");
    copy!(CaCertificate, "ca_certificates");
    copy!(CrlCache, "crl_cache");
    copy!(Role, "roles");
    copy!(Permission, "permissions");
    copy!(GlobalSetting, "global_settings");
    copy!(DataEncryptionKey, "data_encryption_keys");
    copy!(RolePermission, "role_permissions");
    copy!(OidcProvider, "oidc_providers");
    copy!(UserRole, "user_roles");
    copy!(UserOidcLink, "user_oidc_links");
    copy!(Session, "sessions");
    copy!(ApiToken, "api_tokens");
    copy!(RevokedTokenJti, "revoked_token_jtis");
    copy!(RevokedTokenUser, "revoked_token_users");
    copy!(EmailChangeRequest, "email_change_requests");
    copy!(Setting, "settings");
    copy!(SettingsVersion, "settings_version");
    copy!(EnrollmentToken, "enrollment_tokens");
    copy!(Service, "services");
    copy!(ServiceCertificate, "service_certificates");
    copy!(SystemService, "system_services");
    copy!(SystemServiceCertificate, "system_service_certificates");
    copy!(GlobalServiceConfig, "global_service_config");
    copy!(
        EmbeddedServiceRuntimeState,
        "embedded_service_runtime_states"
    );
    copy!(Host, "hosts");
    copy!(ServiceHost, "service_hosts");
    copy!(HostTag, "host_tags");
    copy!(HostTagAssignment, "host_tag_assignments");
    copy!(PluginConfig, "plugin_configs");
    copy!(PluginTypeSetting, "plugin_type_settings");
    copy!(TenantDiscoveryAllowlist, "tenant_discovery_allowlist");
    copy!(HostDiscoveryAllowlist, "host_discovery_allowlist");
    copy!(SoftwareItem, "software_items");
    copy!(HostSoftwareItem, "host_software_items");
    copy!(HostSoftwareItemPlugin, "host_software_item_plugins");
    copy!(SoftwareIgnore, "software_ignores");
    copy!(TenantServiceConfig, "tenant_service_config");
    copy!(UpdateBatch, "update_batches");
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
    copy!(AuditLog, "audit_logs");
    copy!(SystemAuditLog, "system_audit_logs");
    copy!(SystemEnrollmentToken, "system_enrollment_tokens");

    Ok(total)
}

/// Delete every core table on `dst` in reverse FK-safe order.
pub async fn clean(dst: &DatabaseConnection) -> Result<()> {
    use crate::entity::prelude::*;

    macro_rules! clean {
        ($entity:ty, $name:literal) => {
            clean_table::<$entity>($name, dst).await?;
        };
    }

    clean!(SystemEnrollmentToken, "system_enrollment_tokens");
    clean!(SystemAuditLog, "system_audit_logs");
    clean!(AuditLog, "audit_logs");
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
    clean!(UpdateBatch, "update_batches");
    clean!(TenantServiceConfig, "tenant_service_config");
    clean!(SoftwareIgnore, "software_ignores");
    clean!(HostSoftwareItemPlugin, "host_software_item_plugins");
    clean!(HostSoftwareItem, "host_software_items");
    clean!(SoftwareItem, "software_items");
    clean!(HostDiscoveryAllowlist, "host_discovery_allowlist");
    clean!(TenantDiscoveryAllowlist, "tenant_discovery_allowlist");
    clean!(PluginTypeSetting, "plugin_type_settings");
    clean!(PluginConfig, "plugin_configs");
    clean!(HostTagAssignment, "host_tag_assignments");
    clean!(HostTag, "host_tags");
    clean!(ServiceHost, "service_hosts");
    clean!(Host, "hosts");
    clean!(SystemServiceCertificate, "system_service_certificates");
    clean!(GlobalServiceConfig, "global_service_config");
    clean!(
        EmbeddedServiceRuntimeState,
        "embedded_service_runtime_states"
    );
    clean!(SystemService, "system_services");
    clean!(ServiceCertificate, "service_certificates");
    clean!(Service, "services");
    clean!(EnrollmentToken, "enrollment_tokens");
    clean!(SettingsVersion, "settings_version");
    clean!(Setting, "settings");
    clean!(EmailChangeRequest, "email_change_requests");
    clean!(RevokedTokenUser, "revoked_token_users");
    clean!(RevokedTokenJti, "revoked_token_jtis");
    clean!(ApiToken, "api_tokens");
    clean!(Session, "sessions");
    clean!(UserOidcLink, "user_oidc_links");
    clean!(OidcProvider, "oidc_providers");
    clean!(UserRole, "user_roles");
    clean!(RolePermission, "role_permissions");
    clean!(DataEncryptionKey, "data_encryption_keys");
    clean!(GlobalSetting, "global_settings");
    clean!(Permission, "permissions");
    clean!(Role, "roles");
    clean!(CrlCache, "crl_cache");
    clean!(CaCertificate, "ca_certificates");
    clean!(User, "users");
    clean!(Tenant, "tenants");

    Ok(())
}

/// Verify row counts match between `src` and `dst` for every core table.
/// Returns total rows verified.
pub async fn verify(src: &DatabaseConnection, dst: &DatabaseConnection) -> Result<u64> {
    use crate::entity::prelude::*;

    let mut total = 0u64;

    macro_rules! verify {
        ($entity:ty, $name:literal) => {
            total += verify_table::<$entity>($name, src, dst).await?;
        };
    }

    verify!(Tenant, "tenants");
    verify!(User, "users");
    verify!(CaCertificate, "ca_certificates");
    verify!(CrlCache, "crl_cache");
    verify!(Role, "roles");
    verify!(Permission, "permissions");
    verify!(GlobalSetting, "global_settings");
    verify!(DataEncryptionKey, "data_encryption_keys");
    verify!(RolePermission, "role_permissions");
    verify!(OidcProvider, "oidc_providers");
    verify!(UserRole, "user_roles");
    verify!(UserOidcLink, "user_oidc_links");
    verify!(Session, "sessions");
    verify!(ApiToken, "api_tokens");
    verify!(RevokedTokenJti, "revoked_token_jtis");
    verify!(RevokedTokenUser, "revoked_token_users");
    verify!(EmailChangeRequest, "email_change_requests");
    verify!(Setting, "settings");
    verify!(SettingsVersion, "settings_version");
    verify!(EnrollmentToken, "enrollment_tokens");
    verify!(Service, "services");
    verify!(ServiceCertificate, "service_certificates");
    verify!(SystemService, "system_services");
    verify!(SystemServiceCertificate, "system_service_certificates");
    verify!(GlobalServiceConfig, "global_service_config");
    verify!(
        EmbeddedServiceRuntimeState,
        "embedded_service_runtime_states"
    );
    verify!(Host, "hosts");
    verify!(ServiceHost, "service_hosts");
    verify!(HostTag, "host_tags");
    verify!(HostTagAssignment, "host_tag_assignments");
    verify!(PluginConfig, "plugin_configs");
    verify!(PluginTypeSetting, "plugin_type_settings");
    verify!(TenantDiscoveryAllowlist, "tenant_discovery_allowlist");
    verify!(HostDiscoveryAllowlist, "host_discovery_allowlist");
    verify!(SoftwareItem, "software_items");
    verify!(HostSoftwareItem, "host_software_items");
    verify!(HostSoftwareItemPlugin, "host_software_item_plugins");
    verify!(SoftwareIgnore, "software_ignores");
    verify!(TenantServiceConfig, "tenant_service_config");
    verify!(UpdateBatch, "update_batches");
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
    verify!(AuditLog, "audit_logs");
    verify!(SystemAuditLog, "system_audit_logs");
    verify!(SystemEnrollmentToken, "system_enrollment_tokens");

    Ok(total)
}
