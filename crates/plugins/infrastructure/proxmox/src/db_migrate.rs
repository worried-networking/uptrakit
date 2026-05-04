//! Plugin-owned tables registered for the `db-migrate` subcommand.
//!
//! Order is FK-safe: parents before children. The only inter-plugin-table
//! FK in the codebase today is
//! `proxmox_protection_audit.mapping_id → proxmox_host_mappings.id`
//! (`SetNull`); `host_mapping` therefore precedes `protection_audit`.
//! The other three Proxmox tables FK only into core tables, so their
//! relative position within this list does not matter for FK safety.

#[cfg(feature = "migrations")]
pub(crate) fn proxmox_db_migrate_tables()
-> Vec<uptrakit_plugin_infrastructure_core::PluginTableDescriptor> {
    use uptrakit_plugin_infrastructure_core::PluginTableDescriptor;

    use crate::entity::{
        proxmox_backup_target_cache, proxmox_host_mapping, proxmox_protection_audit,
        proxmox_protection_default, proxmox_protection_item_override,
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
    ]
}

/// No-op stub used when `migrations` feature is inactive.
#[cfg(not(feature = "migrations"))]
#[allow(dead_code)]
pub(crate) fn proxmox_db_migrate_tables() {}
