use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

// Canonical types from shared-types with feature-gated SeaORM derives.
pub use uptrakit_shared_types::ServiceStatus;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "services")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    #[sea_orm(column_type = "Text")]
    pub capabilities: String,
    pub hostname: String,
    pub friendly_name: String,
    pub ip_address: Option<String>,
    pub status: ServiceStatus,
    #[sea_orm(unique)]
    pub enrollment_secret_hash: String,
    pub client_version: Option<String>,
    pub last_seen_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub deactivated_at: Option<OffsetDateTime>,
    pub ping_interval_seconds: Option<i32>,
    pub enrollment_token_id: Option<Uuid>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::service_certificate::Entity")]
    ServiceCertificate,
    #[sea_orm(
        belongs_to = "super::tenant::Entity",
        from = "Column::TenantId",
        to = "super::tenant::Column::Id"
    )]
    Tenant,
    #[sea_orm(
        belongs_to = "super::enrollment_token::Entity",
        from = "Column::EnrollmentTokenId",
        to = "super::enrollment_token::Column::Id"
    )]
    EnrollmentToken,
}

impl Related<super::service_certificate::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::ServiceCertificate.def()
    }
}

impl Related<super::tenant::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Tenant.def()
    }
}

impl Related<super::enrollment_token::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::EnrollmentToken.def()
    }
}

impl Related<super::host::Entity> for Entity {
    fn to() -> RelationDef {
        super::service_host::Relation::Host.def()
    }

    fn via() -> Option<RelationDef> {
        Some(super::service_host::Relation::Service.def().rev())
    }
}

impl ActiveModelBehavior for ActiveModel {}
