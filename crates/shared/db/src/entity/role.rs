use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "roles")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(unique)]
    pub name: String,
    pub description: Option<String>,
    pub is_built_in: bool,
    pub created_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::user_role::Entity")]
    UserRoles,
    #[sea_orm(has_many = "super::role_permission::Entity")]
    RolePermissions,
}

impl Related<super::user_role::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::UserRoles.def()
    }
}

impl Related<super::role_permission::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::RolePermissions.def()
    }
}

impl Related<super::user::Entity> for Entity {
    fn to() -> RelationDef {
        super::user_role::Relation::User.def()
    }

    fn via() -> Option<RelationDef> {
        Some(super::user_role::Relation::Role.def().rev())
    }
}

impl Related<super::permission::Entity> for Entity {
    fn to() -> RelationDef {
        super::role_permission::Relation::Permission.def()
    }

    fn via() -> Option<RelationDef> {
        Some(super::role_permission::Relation::Role.def().rev())
    }
}

impl ActiveModelBehavior for ActiveModel {}
