#![allow(
    unreachable_pub,
    reason = "entity lives in pub(crate) mod entity; pub items are crate-internal by design"
)]

use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

use uptrakit_shared_db::entity::{host, plugin_config, tenant};

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "proxmox_host_mappings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub plugin_config_id: Uuid,
    pub host_id: Option<Uuid>,
    pub proxmox_node: String,
    pub proxmox_vmid: i32,
    pub proxmox_type: String,
    pub proxmox_name: Option<String>,
    pub proxmox_status: String,
    pub hostname: Option<String>,
    pub ip_addresses: Option<String>,
    pub machine_id: Option<String>,
    pub match_method: Option<String>,
    pub discovered_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "tenant::Entity",
        from = "Column::TenantId",
        to = "tenant::Column::Id"
    )]
    Tenant,
    #[sea_orm(
        belongs_to = "plugin_config::Entity",
        from = "Column::PluginConfigId",
        to = "plugin_config::Column::Id"
    )]
    PluginConfig,
    #[sea_orm(
        belongs_to = "host::Entity",
        from = "Column::HostId",
        to = "host::Column::Id"
    )]
    Host,
}

impl Related<tenant::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Tenant.def()
    }
}

impl Related<plugin_config::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::PluginConfig.def()
    }
}

impl Related<host::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Host.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

// ── TenantScoped impl (moved from shared-db tenant_scoped.rs) ──────────

impl uptrakit_tenant_db::TenantScoped for Entity {
    fn tenant_id_column() -> Self::Column {
        Column::TenantId
    }
}
