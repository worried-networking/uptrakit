#![allow(
    unreachable_pub,
    reason = "entity lives in pub(crate) mod entity; pub items are crate-internal by design"
)]

use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

use crate::scaling_mode::ScalingMode;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "proxmox_scaling_defaults")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub plugin_config_id: Uuid,
    pub(crate) scaling_mode: ScalingMode,
    pub absolute_cores: Option<i32>,
    pub absolute_memory_mb: Option<i32>,
    pub delta_cores: Option<i32>,
    pub delta_memory_mb: Option<i32>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
