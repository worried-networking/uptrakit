use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "update_output_lines")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub update_history_id: Uuid,
    pub stream: String,
    #[sea_orm(column_type = "Text")]
    pub output: String,
    pub created_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::update_history::Entity",
        from = "Column::UpdateHistoryId",
        to = "super::update_history::Column::Id"
    )]
    UpdateHistory,
}

impl Related<super::update_history::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::UpdateHistory.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
