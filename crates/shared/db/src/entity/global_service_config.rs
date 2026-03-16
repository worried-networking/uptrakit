use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

/// A global (cross-tenant) service config entry.
///
/// Used by services for configuration that applies globally rather than
/// per-tenant. Sensitive values are stored encrypted.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "global_service_config")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// The `service_app_name` of the service that owns this entry.
    pub service_name: String,
    /// Entry key.
    pub key: String,
    /// Stored value — JSON text, optionally encrypted.
    pub value: String,
    /// When `true`, `value` is an `EncryptedString`; the controller decrypts it before delivery.
    pub is_sensitive: bool,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
