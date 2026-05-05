//! Plugin-owned tables registered for the `db-migrate` subcommand.
//!
//! Order is FK-safe: parents before children. The only inter-plugin-table
//! FK in the codebase today is
//! `proxmox_protection_audit.mapping_id → proxmox_host_mappings.id`
//! (`SetNull`); `host_mapping` therefore precedes `protection_audit`.
//! `proxmox_resource_scaling_records.mapping_id` stores a mapping UUID
//! but has no enforced FK constraint, so its position is flexible.

#[cfg(feature = "migrations")]
pub(crate) fn proxmox_db_migrate_tables()
-> Vec<uptrakit_plugin_infrastructure_core::PluginTableDescriptor> {
    use uptrakit_plugin_infrastructure_core::PluginTableDescriptor;

    use crate::entity::{
        proxmox_backup_target_cache, proxmox_host_mapping, proxmox_protection_audit,
        proxmox_protection_default, proxmox_protection_item_override,
        proxmox_resource_scaling_record,
    };

    vec![
        PluginTableDescriptor::for_entity::<proxmox_host_mapping::Entity>("proxmox_host_mappings"),
        PluginTableDescriptor::for_entity::<proxmox_protection_default::Entity>(
            "proxmox_protection_defaults",
        ),
        PluginTableDescriptor::for_entity::<proxmox_protection_item_override::Entity>(
            "proxmox_protection_item_overrides",
        ),
        PluginTableDescriptor::for_entity::<proxmox_backup_target_cache::Entity>(
            "proxmox_backup_target_cache",
        ),
        PluginTableDescriptor::for_entity::<proxmox_protection_audit::Entity>(
            "proxmox_protection_audit",
        ),
        PluginTableDescriptor::for_entity::<proxmox_resource_scaling_record::Entity>(
            "proxmox_resource_scaling_records",
        ),
    ]
}

/// No-op stub used when `migrations` feature is inactive.
/// Not dead: `declare_plugin!` in `plugin.rs` references this unconditionally via `__option_expr!`.
#[cfg(not(feature = "migrations"))]
pub(crate) fn proxmox_db_migrate_tables() {}
