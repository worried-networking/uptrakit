use sea_orm::entity::prelude::*;

/// A pending Proxmox host-mapping match deferred until after `ReportHosts`.
///
/// Written immediately after a successful "Bootstrap via Discovered Guest"
/// action and drained by the background loop after each `ReportHosts` send,
/// once the controller has registered the host and the FK constraint on
/// `proxmox_host_mappings.host_id` can be satisfied.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "pending_proxmox_matches")]
#[allow(unreachable_pub)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true)]
    pub id: i32,
    /// Agent-local UUID of the bootstrapped SSH host (`ssh_hosts.id`).
    pub host_id: String,
    /// Controller-side mapping UUID (`proxmox_host_mappings.id`).
    pub mapping_id: String,
    /// ISO 8601 timestamp of when the pending match was recorded.
    pub created_at: String,
}

#[allow(unreachable_pub)]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
