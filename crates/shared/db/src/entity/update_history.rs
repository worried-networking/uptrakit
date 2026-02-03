use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::None)")]
pub enum UpdateStatus {
    #[sea_orm(string_value = "pending")]
    Pending,
    #[sea_orm(string_value = "in_progress")]
    InProgress,
    #[sea_orm(string_value = "completed")]
    Completed,
    #[sea_orm(string_value = "failed")]
    Failed,
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "update_history")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub host_id: Uuid,
    pub software_item_id: Uuid,
    pub from_version: Option<String>,
    pub to_version: String,
    pub status: UpdateStatus,
    #[sea_orm(column_type = "Text")]
    pub output: String,
    pub initiated_by: String,
    pub started_at: OffsetDateTime,
    pub completed_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::host::Entity",
        from = "Column::HostId",
        to = "super::host::Column::Id"
    )]
    Host,
    #[sea_orm(
        belongs_to = "super::software_item::Entity",
        from = "Column::SoftwareItemId",
        to = "super::software_item::Column::Id"
    )]
    SoftwareItem,
}

impl Related<super::host::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Host.def()
    }
}

impl Related<super::software_item::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::SoftwareItem.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
