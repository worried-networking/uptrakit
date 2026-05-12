use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

/// `oauth_refresh_tokens` row.
///
/// Opaque, hashed refresh token with family-replay detection. See
/// `m20260513_000005_oauth_refresh_tokens` for the full schema rationale
/// (sliding + absolute TTLs, lineage tracking via `parent_id` /
/// `family_id`, FK semantics).
///
/// `parent_id` is an opaque lineage marker — although it references rows
/// in this same table, no FK is declared on it (intentional; replay
/// detection scans by `family_id`).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "oauth_refresh_tokens")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub family_id: Uuid,
    pub parent_id: Option<Uuid>,
    #[sea_orm(unique)]
    pub token_hash: String,
    pub client_id: String,
    pub user_id: Uuid,
    pub consent_id: Uuid,
    pub scope: String,
    pub resource: String,
    pub issued_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    pub family_expires_at: OffsetDateTime,
    pub rotated_at: Option<OffsetDateTime>,
    pub revoked_at: Option<OffsetDateTime>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
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
    #[sea_orm(
        belongs_to = "super::oauth_consent::Entity",
        from = "Column::ConsentId",
        to = "super::oauth_consent::Column::Id"
    )]
    OauthConsent,
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

impl Related<super::oauth_consent::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::OauthConsent.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
