use sea_orm::entity::prelude::*;
use time::OffsetDateTime;
use uptrakit_shared_types::SessionTokenType;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "sessions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub user_id: Uuid,
    #[sea_orm(unique)]
    pub refresh_token_hash: String,
    pub auth_method: String,
    pub oidc_provider_id: Option<Uuid>,
    pub token_type: SessionTokenType,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    pub revoked_at: Option<OffsetDateTime>,
    pub user_agent: Option<String>,
    pub ip_address: Option<String>,
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
        from = "Column::OidcProviderId",
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
