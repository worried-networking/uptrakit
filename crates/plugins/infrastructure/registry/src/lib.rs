//! Plugin Registry for Uptrakit
//!
//! This crate provides the plugin catalog and descriptor assembly:
//!
//! - **Catalog construction**: Build a `PluginCatalog` from all compiled-in descriptors
//! - **Descriptor-based creation**: Create plugin role instances via descriptor function pointers
//! - **Sudo command collection**: Gather sudo requirements from all plugins

#[cfg(feature = "agent-infra")]
pub mod agent_infra;
pub mod error;
pub mod registry;
#[cfg(feature = "test-support")]
pub mod test_support;

pub use error::{PluginRegistryError, Result};
pub use registry::{
    all_descriptors, all_required_sudo_commands, compatible_sudo_commands_for_host, get_descriptor,
    is_interactive_dispatch_plugin, is_package_manager_plugin, plugin_family,
};

// Re-export commonly used types for convenience
pub use uptrakit_plugin_infrastructure_core::{
    CatalogConfig, ConfigModel, ControllerPostUpdateContext, ControllerProtectionContext,
    ControllerProtectionDecision, ControllerRuntime, ControllerUpdateProtection,
    GlobalProviderLookup, HostRuntime, MetadataAwareHostRuntime, NotificationTransport,
    PluginCapability, PluginCatalog, PluginConfigValidationError, PluginDescriptor, PluginMeta,
    PostUpdateOutcome, SoftwareItemCreatedEvent, SoftwareItemLifecycle,
    SoftwareItemLifecycleContext, SoftwareItemPatch, SudoCommandEntry, SudoHelperScript,
    SurfaceActionController, SurfaceActionError, UpdateProtectionController,
};
pub use uptrakit_shared_types::{PluginTypeId, plugin_ids};

// Re-export PluginOps traits
pub use uptrakit_plugin_infrastructure_core::{
    ControllerUpdateHookOps, ControllerUpdateProtectionOps, NotificationOps, PluginConfigOps,
    PluginMetadataOps, PluginOps, PluginOpsError, PluginSurfaceActionOps, PluginSurfaceOps,
    SoftwareItemLifecycleOps,
};

// Re-export update-hook types (require plugin-ops feature)
#[cfg(feature = "plugin-ops")]
pub use uptrakit_plugin_infrastructure_core::{
    ControllerUpdateHook, UpdateHookController, UpdateHookPostContext, UpdateHookPreContext,
};

// Re-export transactional email error for callers that use `send_transactional_email`.
#[cfg(feature = "notifications")]
pub use uptrakit_plugin_infrastructure_core::TransactionalEmailError;

// Re-export descriptor surface-action context (typed controller boundary).
pub use uptrakit_plugin_infrastructure_core::SurfaceActionContext;

// Re-export executor types for downstream convenience
pub use uptrakit_command::{CommandExecutor, LocalCommandExecutor};

// --- Additive re-exports (DESIGN-0001 / ST-0015) ---

pub use uptrakit_plugin_infrastructure_core::host_requirements::RoleKey;
pub use uptrakit_plugin_infrastructure_core::roles::ReleaseFetcher;
pub use uptrakit_plugin_infrastructure_core::{
    BatchDetectItem, BatchFetchItem, BatchFetchResult, BatchUpdateItem, ExecuteUpdateResult,
    HostCapabilities, HostCompatibility, InfraBundle, PluginError, PluginFamily,
    ServiceMetadataProvider, UpdateLifecycleContext, construct_host_runtime,
};
pub use uptrakit_plugin_infrastructure_core::{
    FormFieldDescriptor, FormFieldType, FormSelectOptionDescriptor, FormSelectSourceDescriptor,
    SurfaceActionDescriptor, SurfaceActionLibrary, SurfaceActionUi, SurfaceFormDescriptor,
    SurfaceRowCondition, SurfaceRowVisibleWhen, SurfaceWorkflowStep,
};

/// Canonical plugin-result alias re-exported by the registry.
///
/// Source: expands to
/// `std::result::Result<T, rootcause::Report<uptrakit_plugin_infrastructure_core::PluginError>>`.
/// The underlying `PluginError` originates in `uptrakit-plugin-infrastructure-core`; the alias
/// itself is defined and owned by `uptrakit-plugin-infrastructure-registry` (this crate).
///
/// Intended usage: downstream consumers should import this alias through the registry-qualified
/// path `uptrakit_plugin_infrastructure_registry::PluginResult` rather than spelling
/// `Result<_, rootcause::Report<PluginError>>` by hand or re-importing `rootcause` and
/// `PluginError` independently. Consumers must not conflate this alias with the crate-local
/// `Result<T>` alias from `error.rs`, which wraps `PluginRegistryError` and is unrelated.
pub type PluginResult<T> = std::result::Result<T, rootcause::Report<PluginError>>;

pub use uptrakit_plugin_infrastructure_core::{
    PluginHttpClientBuildError, PluginHttpClientConfig, SsrfMode, build_plugin_http_client,
};

pub use uptrakit_notification_plugin_core::{
    DeliveryMessage, MessageAction, NotificationPluginError, escape_html,
};

/// Build a `PluginCatalog` from all compiled-in descriptors.
///
/// This is the primary entry point for controller startup. The `config`
/// carries deployment-level settings (SSRF policy, shared HTTP client, etc.).
pub fn build_catalog(
    config: &CatalogConfig,
) -> uptrakit_plugin_infrastructure_core::Result<PluginCatalog> {
    PluginCatalog::new(all_descriptors(), config)
}

/// Call all registered plugin `reset_tenant_data` callbacks within the given transaction.
///
/// Only available when `migrations` feature is active (matching `ResetTenantDataFn` real type).
///
/// # Ordering note
/// Currently only Proxmox registers a callback. If a future plugin's tables have Restrict FK
/// dependencies on another plugin's tables, the iteration order (registration order in
/// `all_descriptors()`) could cause a FK violation.
/// TODO: add a `reset_order` field to `PluginDescriptor` if multiple plugins need ordered teardown.
#[cfg(feature = "migrations")]
pub async fn reset_plugin_tenant_data(
    tenant_id: uuid::Uuid,
    txn: &sea_orm::DatabaseTransaction,
) -> std::result::Result<(), sea_orm::DbErr> {
    for descriptor in all_descriptors() {
        if let Some(reset_fn) = descriptor.reset_tenant_data {
            reset_fn(tenant_id, txn).await?;
        }
    }
    Ok(())
}

// ── db-migrate dispatch ────────────────────────────────────────────────────

/// Copy every plugin's tables from `src` to `dst`. Returns total rows.
///
/// Iterates plugin descriptors in registration order. Within each plugin,
/// iterates tables in the order returned by `db_migrate_tables` (FK-safe:
/// parent tables first).
///
/// # Ordering note
///
/// Currently only Proxmox registers `db_migrate_tables`. If multiple
/// plugins register tables with FKs across plugin boundaries, registration
/// order would matter. We do not have such cross-plugin FKs today; if a
/// future plugin introduces one, add a `migration_order` hint to
/// `PluginDescriptor` (mirroring the same TODO already documented for
/// `reset_plugin_tenant_data`).
#[cfg(feature = "migrations")]
pub async fn copy_plugin_tables(
    src: &sea_orm::DatabaseConnection,
    dst: &sea_orm::DatabaseConnection,
    batch_size: u64,
) -> std::result::Result<
    u64,
    rootcause::Report<uptrakit_shared_db::migrate_core_tables::TableMigrateError>,
> {
    use rootcause::prelude::*;
    use uptrakit_shared_db::migrate_core_tables::TableMigrateError;

    let mut total = 0u64;
    for descriptor in all_descriptors() {
        if let Some(tables_fn) = descriptor.db_migrate_tables {
            for table in tables_fn() {
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
        }
    }
    Ok(total)
}

#[cfg(feature = "migrations")]
pub async fn clean_plugin_tables(
    dst: &sea_orm::DatabaseConnection,
) -> std::result::Result<
    (),
    rootcause::Report<uptrakit_shared_db::migrate_core_tables::TableMigrateError>,
> {
    use rootcause::prelude::*;
    use uptrakit_shared_db::migrate_core_tables::TableMigrateError;

    for descriptor in all_descriptors() {
        if let Some(tables_fn) = descriptor.db_migrate_tables {
            // Reverse for FK-safe deletion (children before parents).
            for table in tables_fn().into_iter().rev() {
                (table.clean)(dst).await.map_err(|err| {
                    report!(TableMigrateError::Db {
                        table: table.name,
                        err
                    })
                })?;
            }
        }
    }
    Ok(())
}

#[cfg(feature = "migrations")]
pub async fn verify_plugin_tables(
    src: &sea_orm::DatabaseConnection,
    dst: &sea_orm::DatabaseConnection,
) -> std::result::Result<
    u64,
    rootcause::Report<uptrakit_shared_db::migrate_core_tables::TableMigrateError>,
> {
    use rootcause::prelude::*;
    use uptrakit_shared_db::migrate_core_tables::TableMigrateError;

    let mut total = 0u64;
    for descriptor in all_descriptors() {
        if let Some(tables_fn) = descriptor.db_migrate_tables {
            for table in tables_fn() {
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
        }
    }
    Ok(total)
}
