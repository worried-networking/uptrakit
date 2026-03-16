use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

/// A tenant-scoped service config entry.
///
/// Used by services (e.g. the MQTT bridge) to persist named configuration
/// in the controller DB. Sensitive values are stored encrypted and decrypted
/// by the controller before delivery to the service.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "tenant_service_config")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// The `service_app_name` of the service that owns this entry.
    pub service_name: String,
    pub tenant_id: Uuid,
    /// Entry key (e.g. `"clients.{uuid}"`).
    pub key: String,
    /// Stored value — JSON text, optionally encrypted.
    pub value: String,
    /// When `true`, `value` is an `EncryptedString`; the controller decrypts it before delivery.
    pub is_sensitive: bool,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::tenant::Entity",
        from = "Column::TenantId",
        to = "super::tenant::Column::Id"
    )]
    Tenant,
}

impl Related<super::tenant::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Tenant.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
