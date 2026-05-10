//! `instance_plugin_setting` — per-plugin enable state and instance-wide
//! configuration for Instance-Scoped Plugins. One row per plugin_type_id.
//! Row absence ⇒ plugin defaults to disabled with empty config.
use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "instance_plugin_setting")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub plugin_type_id: String,
    pub enabled: bool,
    #[sea_orm(column_type = "Json")]
    pub config: serde_json::Value,
    pub updated_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
