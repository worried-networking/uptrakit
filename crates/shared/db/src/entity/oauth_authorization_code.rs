use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

/// `oauth_authorization_codes` row.
///
/// Single-use authorization code minted at consent time and redeemed once
/// at `/oauth/token`. See `m20260512_000004_oauth_authorization_codes`
/// for the full schema rationale.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "oauth_authorization_codes")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(unique)]
    pub code_hash: String,
    pub request_id: Uuid,
    pub client_id: String,
    pub user_id: Uuid,
    pub redirect_uri: String,
    pub scope: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub resource: String,
    pub issued_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    pub consumed_at: Option<OffsetDateTime>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::oauth_authorization_request::Entity",
        from = "Column::RequestId",
        to = "super::oauth_authorization_request::Column::RequestId"
    )]
    OauthAuthorizationRequest,
    #[sea_orm(
        belongs_to = "super::oauth_client::Entity",
        from = "Column::ClientId",
        to = "super::oauth_client::Column::Id"
    )]
    OauthClient,
    #[sea_orm(
        belongs_to = "super::user::Entity",
        from = "Column::UserId",
        to = "super::user::Column::Id"
    )]
    User,
}

impl Related<super::oauth_authorization_request::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::OauthAuthorizationRequest.def()
    }
}

impl Related<super::oauth_client::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::OauthClient.def()
    }
}

impl Related<super::user::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::User.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
