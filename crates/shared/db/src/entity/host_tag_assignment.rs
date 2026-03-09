use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "host_tag_assignments")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub host_tag_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub host_id: Uuid,
    pub assigned_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::host_tag::Entity",
        from = "Column::HostTagId",
        to = "super::host_tag::Column::Id"
    )]
    HostTag,
    #[sea_orm(
        belongs_to = "super::host::Entity",
        from = "Column::HostId",
        to = "super::host::Column::Id"
    )]
    Host,
}

impl Related<super::host_tag::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::HostTag.def()
    }
}

impl Related<super::host::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Host.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
