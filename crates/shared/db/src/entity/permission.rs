use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "permissions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(unique)]
    pub name: String,
    pub description: Option<String>,
    pub created_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::role_permission::Entity")]
    RolePermissions,
}

impl Related<super::role_permission::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::RolePermissions.def()
    }
}

impl Related<super::role::Entity> for Entity {
    fn to() -> RelationDef {
        super::role_permission::Relation::Role.def()
    }

    fn via() -> Option<RelationDef> {
        Some(super::role_permission::Relation::Permission.def().rev())
    }
}

impl ActiveModelBehavior for ActiveModel {}
