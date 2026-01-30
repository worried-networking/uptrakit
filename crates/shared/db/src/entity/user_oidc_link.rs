use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "user_oidc_links")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub user_id: Uuid,
    pub provider_id: Uuid,
    pub oidc_subject: String,
    pub linked_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::user::Entity",
        from = "Column::UserId",
        to = "super::user::Column::Id"
    )]
    User,
    #[sea_orm(
        belongs_to = "super::oidc_provider::Entity",
        from = "Column::ProviderId",
        to = "super::oidc_provider::Column::Id"
    )]
    OidcProvider,
}

impl Related<super::user::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::User.def()
    }
}

impl Related<super::oidc_provider::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::OidcProvider.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
