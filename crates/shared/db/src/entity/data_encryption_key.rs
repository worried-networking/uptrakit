use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

/// A wrapped data encryption key (DEK) used for envelope encryption.
///
/// The master key (KEK) wraps/unwraps these DEKs at controller startup.
/// Data is encrypted with the DEK, not the KEK directly. This enables O(1)
/// master key rotation by re-wrapping DEKs without touching encrypted data.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "data_encryption_keys")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// First 8 hex chars of SHA-256(DEK). Used to identify the DEK in
    /// `ENC:v3:<key_id>:<hex>` ciphertext format.
    #[sea_orm(column_type = "Text", unique)]
    pub key_id: String,
    /// Hex-encoded `nonce || ciphertext || tag` — the DEK wrapped with the KEK.
    #[sea_orm(column_type = "Text")]
    pub wrapped_key: String,
    /// First 16 hex chars of SHA-256(KEK) that was used to wrap this DEK.
    /// Enables detection of KEK mismatches during startup.
    #[sea_orm(column_type = "Text")]
    pub kek_fingerprint: String,
    /// `"active"` for the current DEK, `"retired"` for old DEKs kept for
    /// decryption of existing ciphertext.
    #[sea_orm(column_type = "Text")]
    pub status: String,
    pub created_at: OffsetDateTime,
    pub retired_at: Option<OffsetDateTime>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
