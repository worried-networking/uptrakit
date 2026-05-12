use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

/// `oauth_consents` row.
///
/// Represents a user's grant of a specific scope set to an OAuth Client.
/// See `m20260512_000002_oauth_consents` for the full schema rationale,
/// including the partial UNIQUE index on active (non-revoked) consents
/// per `(user_id, client_id)`.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "oauth_consents")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub user_id: Uuid,
    pub client_id: String,
    pub scopes: String,
    pub cimd_content_hash_at_grant: Option<String>,
    pub revalidation_required_at: Option<OffsetDateTime>,
    pub granted_at: OffsetDateTime,
    pub revoked_at: Option<OffsetDateTime>,
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
        belongs_to = "super::oauth_client::Entity",
        from = "Column::ClientId",
        to = "super::oauth_client::Column::Id"
    )]
    OauthClient,
}

impl Related<super::user::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::User.def()
    }
}

impl Related<super::oauth_client::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::OauthClient.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
