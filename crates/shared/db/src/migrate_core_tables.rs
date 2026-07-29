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

use std::pin::Pin;

use sea_orm::{
    ActiveModelBehavior, ActiveModelTrait, DatabaseConnection, DbErr, EntityTrait, IntoActiveModel,
    PaginatorTrait, QuerySelect,
};

// ── Name-less per-entity helpers (DbErr-returning; the boundary fns below
//    attach the table name). Local mirror of the shape in
//    `plugin-infrastructure-core::db_migrate` — shared-db cannot depend on
//    plugin crates (dependency direction + plugin-semantic-boundary gate).

async fn copy_one<E>(
    src: &DatabaseConnection,
    dst: &DatabaseConnection,
    batch_size: u64,
) -> std::result::Result<u64, DbErr>
where
    E: EntityTrait + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Send + Sync + 'static,
    E::ActiveModel: ActiveModelTrait<Entity = E> + ActiveModelBehavior + Send + 'static,
{
    let mut copied = 0u64;
    let mut offset = 0u64;
    loop {
        let batch = E::find().offset(offset).limit(batch_size).all(src).await?;
        if batch.is_empty() {
            break;
        }
        let n = batch.len() as u64;
        let active: Vec<_> = batch
            .into_iter()
            .map(IntoActiveModel::into_active_model)
            .collect();
        E::insert_many(active).exec(dst).await?;
        copied += n;
        offset += n;
    }
    Ok(copied)
}

async fn clean_one<E: EntityTrait>(dst: &DatabaseConnection) -> std::result::Result<(), DbErr> {
    E::delete_many().exec(dst).await.map(|_| ())
}

async fn verify_one<E>(
    src: &DatabaseConnection,
    dst: &DatabaseConnection,
) -> std::result::Result<(u64, u64), DbErr>
where
    E: EntityTrait + 'static,
    E::Model: Send + Sync + 'static,
{
    let src_count = E::find().count(src).await?;
    let dst_count = E::find().count(dst).await?;
    Ok((src_count, dst_count))
}

// ── Type-erased descriptor (local mirror of `PluginTableDescriptor`) ──────

type CopyBatchFn = for<'a> fn(
    src: &'a DatabaseConnection,
    dst: &'a DatabaseConnection,
    batch_size: u64,
) -> Pin<
    Box<dyn std::future::Future<Output = std::result::Result<u64, DbErr>> + Send + 'a>,
>;

type CleanFn = for<'a> fn(
    dst: &'a DatabaseConnection,
) -> Pin<
    Box<dyn std::future::Future<Output = std::result::Result<(), DbErr>> + Send + 'a>,
>;

type VerifyFn = for<'a> fn(
    src: &'a DatabaseConnection,
    dst: &'a DatabaseConnection,
) -> Pin<
    Box<dyn std::future::Future<Output = std::result::Result<(u64, u64), DbErr>> + Send + 'a>,
>;

/// Per-table copy/clean/verify operations for one **core** table.
///
/// Local mirror of `PluginTableDescriptor` in `plugin-infrastructure-core`
/// (an import is impossible: dependency direction + the
/// plugin-semantic-boundary CI gate).
pub struct CoreTableDescriptor {
    /// Table name as it appears in the database (matches
    /// `#[sea_orm(table_name = "...")]` on the entity).
    pub name: &'static str,
    copy_batch: CopyBatchFn,
    clean: CleanFn,
    verify: VerifyFn,
}

impl CoreTableDescriptor {
    /// `pub(crate)` (not private): [`crate::access_grants::core_table_descriptor`]
    /// builds the `access_grants` descriptor from the engine-owned module —
    /// the only place permitted to name the `access_grant` entity
    /// (`ci/verify_engine_owned_entities.sh`) — so this constructor must be
    /// callable from a sibling module in the same crate.
    pub(crate) fn for_entity<E>(name: &'static str) -> Self
    where
        E: EntityTrait + 'static,
        E::Model: IntoActiveModel<E::ActiveModel> + Send + Sync + 'static,
        E::ActiveModel: ActiveModelTrait<Entity = E> + ActiveModelBehavior + Send + 'static,
    {
        Self {
            name,
            copy_batch: |src, dst, batch| Box::pin(copy_one::<E>(src, dst, batch)),
            clean: |dst| Box::pin(clean_one::<E>(dst)),
            verify: |src, dst| Box::pin(verify_one::<E>(src, dst)),
        }
    }
}

/// Single authority for the **core** application tables db-migrate covers,
/// in FK-safe order (parents before children).
///
/// To add a table: one `CoreTableDescriptor::for_entity::<PreludeAlias>("table_name")`
/// line at an FK-safe position (after every table it references). `copy()`
/// and `verify()` iterate forward, `clean()` iterates in reverse — there is
/// no second list to sync. The `migration_coverage_complete` test in
/// `controller-runtime/src/db_migrate/tables.rs` fails until every live
/// table is either listed here, registered by a plugin descriptor, or in
/// its `AGENT_ONLY_TABLES` exclusion.
pub fn core_tables() -> Vec<CoreTableDescriptor> {
    use crate::entity::prelude::*;

    vec![
        CoreTableDescriptor::for_entity::<Tenant>("tenants"),
        // Built via the engine-owned module, not `for_entity::<access_grant::Entity>`
        // directly — `ci/verify_engine_owned_entities.sh` bans naming the
        // entity outside `access_grants.rs`/the migration dir.
        crate::access_grants::core_table_descriptor(),
        CoreTableDescriptor::for_entity::<User>("users"),
        CoreTableDescriptor::for_entity::<CaCertificate>("ca_certificates"),
        CoreTableDescriptor::for_entity::<CrlCache>("crl_cache"),
        CoreTableDescriptor::for_entity::<Role>("roles"),
        CoreTableDescriptor::for_entity::<Permission>("permissions"),
        CoreTableDescriptor::for_entity::<GlobalSetting>("global_settings"),
        CoreTableDescriptor::for_entity::<DataEncryptionKey>("data_encryption_keys"),
        CoreTableDescriptor::for_entity::<RolePermission>("role_permissions"),
        CoreTableDescriptor::for_entity::<OidcProvider>("oidc_providers"),
        CoreTableDescriptor::for_entity::<UserRole>("user_roles"),
        CoreTableDescriptor::for_entity::<UserOidcLink>("user_oidc_links"),
        CoreTableDescriptor::for_entity::<UserTotp>("user_totp"),
        CoreTableDescriptor::for_entity::<UserRecoveryCode>("user_recovery_codes"),
        CoreTableDescriptor::for_entity::<MfaChallenge>("mfa_challenges"),
        CoreTableDescriptor::for_entity::<Session>("sessions"),
        CoreTableDescriptor::for_entity::<ApiToken>("api_tokens"),
        CoreTableDescriptor::for_entity::<RevokedTokenJti>("revoked_token_jtis"),
        CoreTableDescriptor::for_entity::<RevokedTokenUser>("revoked_token_users"),
        CoreTableDescriptor::for_entity::<EmailChangeRequest>("email_change_requests"),
        CoreTableDescriptor::for_entity::<Setting>("settings"),
        CoreTableDescriptor::for_entity::<SettingsVersion>("settings_version"),
        CoreTableDescriptor::for_entity::<EnrollmentToken>("enrollment_tokens"),
        CoreTableDescriptor::for_entity::<Service>("services"),
        CoreTableDescriptor::for_entity::<ServiceCertificate>("service_certificates"),
        CoreTableDescriptor::for_entity::<ServiceMergeRedirect>("service_merge_redirect"),
        CoreTableDescriptor::for_entity::<SystemService>("system_services"),
        CoreTableDescriptor::for_entity::<SystemServiceCertificate>("system_service_certificates"),
        CoreTableDescriptor::for_entity::<GlobalServiceConfig>("global_service_config"),
        CoreTableDescriptor::for_entity::<EmbeddedServiceRuntimeState>(
            "embedded_service_runtime_states",
        ),
        CoreTableDescriptor::for_entity::<Host>("hosts"),
        CoreTableDescriptor::for_entity::<ServiceHost>("service_hosts"),
        CoreTableDescriptor::for_entity::<HostTag>("host_tags"),
        CoreTableDescriptor::for_entity::<HostTagAssignment>("host_tag_assignments"),
        CoreTableDescriptor::for_entity::<PluginConfig>("plugin_configs"),
        CoreTableDescriptor::for_entity::<PluginTypeSetting>("plugin_type_settings"),
        CoreTableDescriptor::for_entity::<InstancePluginSetting>("instance_plugin_setting"),
        CoreTableDescriptor::for_entity::<TenantDiscoveryAllowlist>("tenant_discovery_allowlist"),
        CoreTableDescriptor::for_entity::<HostDiscoveryAllowlist>("host_discovery_allowlist"),
        CoreTableDescriptor::for_entity::<SoftwareItem>("software_items"),
        CoreTableDescriptor::for_entity::<HostSoftwareItem>("host_software_items"),
        CoreTableDescriptor::for_entity::<HostSoftwareItemPlugin>("host_software_item_plugins"),
        CoreTableDescriptor::for_entity::<SoftwareIgnore>("software_ignores"),
        CoreTableDescriptor::for_entity::<TenantServiceConfig>("tenant_service_config"),
        CoreTableDescriptor::for_entity::<UpdateBatch>("update_batches"),
        CoreTableDescriptor::for_entity::<UpdateHistory>("update_history"),
        CoreTableDescriptor::for_entity::<UpdateOutputLine>("update_output_lines"),
        CoreTableDescriptor::for_entity::<PendingDeviceFlow>("pending_device_flows"),
        CoreTableDescriptor::for_entity::<PendingOidcFlow>("pending_oidc_flows"),
        CoreTableDescriptor::for_entity::<PendingAccountLink>("pending_account_links"),
        CoreTableDescriptor::for_entity::<PendingOidcTokenExchange>("pending_oidc_token_exchanges"),
        CoreTableDescriptor::for_entity::<PendingOidcRegistration>("pending_oidc_registrations"),
        CoreTableDescriptor::for_entity::<OauthClient>("oauth_clients"),
        CoreTableDescriptor::for_entity::<OauthConsent>("oauth_consents"),
        CoreTableDescriptor::for_entity::<OauthAuthorizationRequest>(
            "oauth_authorization_requests",
        ),
        CoreTableDescriptor::for_entity::<OauthAuthorizationCode>("oauth_authorization_codes"),
        CoreTableDescriptor::for_entity::<OauthRefreshToken>("oauth_refresh_tokens"),
        CoreTableDescriptor::for_entity::<OauthControllerInstance>("oauth_controller_instances"),
        CoreTableDescriptor::for_entity::<ApiRateLimit>("api_rate_limits"),
        CoreTableDescriptor::for_entity::<ScheduledTask>("scheduled_tasks"),
        CoreTableDescriptor::for_entity::<NotificationChannel>("notification_channels"),
        CoreTableDescriptor::for_entity::<NotificationRule>("notification_rules"),
        CoreTableDescriptor::for_entity::<NotificationLog>("notification_log"),
        CoreTableDescriptor::for_entity::<AuditLog>("audit_logs"),
        CoreTableDescriptor::for_entity::<SystemAuditLog>("system_audit_logs"),
        CoreTableDescriptor::for_entity::<SystemEnrollmentToken>("system_enrollment_tokens"),
    ]
}

/// Copy every core table from `src` to `dst`. Returns total rows.
pub async fn copy(
    src: &DatabaseConnection,
    dst: &DatabaseConnection,
    batch_size: u64,
) -> Result<u64> {
    let mut total = 0u64;
    for table in core_tables() {
        let copied = (table.copy_batch)(src, dst, batch_size)
            .await
            .map_err(|err| {
                report!(TableMigrateError::Db {
                    table: table.name,
                    err
                })
            })?;
        eprintln!("  {}: {copied} rows", table.name);
        total += copied;
    }
    Ok(total)
}

/// Delete every core table on `dst` in reverse FK-safe order.
pub async fn clean(dst: &DatabaseConnection) -> Result<()> {
    for table in core_tables().into_iter().rev() {
        (table.clean)(dst).await.map_err(|err| {
            report!(TableMigrateError::Db {
                table: table.name,
                err
            })
        })?;
    }
    Ok(())
}

/// Verify row counts match between `src` and `dst` for every core table.
/// Returns total rows verified.
pub async fn verify(src: &DatabaseConnection, dst: &DatabaseConnection) -> Result<u64> {
    let mut total = 0u64;
    for table in core_tables() {
        let (src_count, dst_count) = (table.verify)(src, dst).await.map_err(|err| {
            report!(TableMigrateError::Db {
                table: table.name,
                err
            })
        })?;
        if src_count != dst_count {
            bail!(TableMigrateError::Mismatch {
                table: table.name,
                src: src_count,
                dst: dst_count,
            });
        }
        total += src_count;
    }
    Ok(total)
}
