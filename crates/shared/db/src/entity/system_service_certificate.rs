use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

/// Reason for revoking a system service certificate.
///
/// System services cannot be merged (unlike tenant services), so only two
/// revocation reasons apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum SystemRevocationReason {
    #[sea_orm(string_value = "certificate_renewed")]
    CertificateRenewed,
    #[sea_orm(string_value = "service_deactivated")]
    ServiceDeactivated,
}

/// A certificate issued to a system service.
///
/// FK points to `system_services`, not `services`.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "system_service_certificates")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub ca_fingerprint: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub serial_number: String,
    pub system_service_id: Uuid,
    pub not_before: OffsetDateTime,
    pub not_after: OffsetDateTime,
    pub revoked_at: Option<OffsetDateTime>,
    pub revocation_reason: Option<SystemRevocationReason>,
    pub created_at: OffsetDateTime,
    pub last_seen_at: Option<OffsetDateTime>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::system_service::Entity",
        from = "Column::SystemServiceId",
        to = "super::system_service::Column::Id"
    )]
    SystemService,
}

impl Related<super::system_service::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::SystemService.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
