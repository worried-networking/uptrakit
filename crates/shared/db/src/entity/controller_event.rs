use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "controller_events")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub source_controller_id: Uuid,
    pub target_service_id: Option<Uuid>,
    #[sea_orm(column_type = "Text", nullable)]
    pub target_service_type: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub message_json: String,
    pub created_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
