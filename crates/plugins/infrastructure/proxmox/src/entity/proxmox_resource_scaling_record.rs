#![allow(
    unreachable_pub,
    reason = "entity lives in pub(crate) mod entity; pub items are crate-internal by design"
)]

use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "proxmox_resource_scaling_records")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub update_history_id: Uuid,
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub software_item_id: Uuid,
    pub plugin_config_id: Uuid,
    pub mapping_id: Uuid,
    pub vm_type: String,
    pub original_cores: i32,
    pub original_memory_mb: i64,
    pub scaled_cores: i32,
    pub scaled_memory_mb: i64,
    pub scale_status: String,
    pub restore_status: String,
    pub error_message: Option<String>,
    pub scaling_mode_used: String,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
