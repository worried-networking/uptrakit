use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "roles")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub is_built_in: bool,
    pub created_at: OffsetDateTime,
    /// `None` = global role (the eight built-ins); `Some` = tenant-defined
    /// custom role (creatable from M1.6a). Per-scope name uniqueness is
    /// enforced by the partial index pair `uix_roles_global_name` /
    /// `uix_roles_tenant_name`, not a column constraint.
    ///
    /// M1.6a note: role deletion must also delete the role's `access_grants`
    /// rows — `access_grants.subject_id` carries no FK, so nothing cascades.
    pub tenant_id: Option<Uuid>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::user_role::Entity")]
    UserRoles,
}

impl Related<super::user_role::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::UserRoles.def()
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

impl ActiveModelBehavior for ActiveModel {}
