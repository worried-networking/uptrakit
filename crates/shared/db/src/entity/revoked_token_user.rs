use sea_orm::entity::prelude::*;

/// A user-level token revocation entry.
///
/// All access tokens for the given user with `iat < iat_cutoff` are denied.
/// Persisted so that revocations survive controller restarts; seeded into the
/// in-memory denylist at startup.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "revoked_token_users")]
pub struct Model {
    /// User UUID stored as a 16-byte blob.
    #[sea_orm(primary_key, auto_increment = false)]
    pub user_id: Uuid,
    /// Deny tokens issued strictly before this unix timestamp (seconds).
    pub iat_cutoff: i64,
    /// Remove this entry from the denylist after this unix timestamp.
    ///
    /// Set to `iat_cutoff + ACCESS_TOKEN_EXPIRY_SECS` so that pre-revocation
    /// tokens (which can live up to 15 minutes) are still blocked until they
    /// expire naturally.
    pub purge_after: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
