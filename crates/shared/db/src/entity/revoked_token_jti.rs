use sea_orm::entity::prelude::*;

/// A revoked access token identified by its JTI claim.
///
/// Persisted so that token revocations survive controller restarts. The
/// in-memory [`crate::auth::token_denylist::TokenDenylist`] is seeded from
/// this table at startup. New entries are written here by the originating
/// controller; other instances receive the revocation via NATS and update
/// their own in-memory caches without writing to DB again.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "revoked_token_jtis")]
pub struct Model {
    /// JWT ID (`jti`) claim — unique per token.
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub jti: String,
    /// Unix timestamp (seconds) after which this row may be purged.
    ///
    /// Set to the token's `exp` claim so that the entry is kept alive exactly
    /// as long as the token itself could be presented.
    pub expires_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
