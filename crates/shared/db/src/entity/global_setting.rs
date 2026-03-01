use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "global_settings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub key: String,
    #[sea_orm(column_type = "Json")]
    pub value: serde_json::Value,
    pub updated_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
