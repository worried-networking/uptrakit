//! SeaORM entities for Proxmox plugin's agent-local tables.

// ── proxmox_host_state ───────────────────────────────────────────────────────

/// Tracks Proxmox VE infrastructure state for each SSH host.
///
/// This table is owned by the Proxmox infrastructure plugin and stored in the
/// agent-ssh's local SQLite database. It replaces the former PVE-specific
/// columns (`is_pve_node`, `pve_plugin_config_id`, `pve_node_name`) on the
/// `ssh_hosts` table.
pub mod proxmox_host_state {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "proxmox_host_state")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub host_id: String,
        pub is_pve_node: bool,
        pub pve_node_name: Option<String>,
        pub pve_plugin_config_id: Option<String>,
        pub created_at: String,
        pub updated_at: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

// ── proxmox_pending_matches ──────────────────────────────────────────────────

/// Deferred host-to-guest mapping matches.
///
/// Created immediately after a successful "Bootstrap via Discovered Guest"
/// action. Drained after the next `ReportHosts` send, because the controller
/// must create the `hosts` row before the FK constraint on
/// `proxmox_host_mappings.host_id` can be satisfied.
pub mod proxmox_pending_match {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "proxmox_pending_matches")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = true)]
        pub id: i32,
        pub host_id: String,
        pub mapping_id: String,
        pub created_at: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
