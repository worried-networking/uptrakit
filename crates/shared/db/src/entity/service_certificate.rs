use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

/// Reason for revoking a service certificate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum RevocationReason {
    #[sea_orm(string_value = "certificate_renewed")]
    CertificateRenewed,
    #[sea_orm(string_value = "service_deactivated")]
    ServiceDeactivated,
    #[sea_orm(string_value = "service_merged")]
    ServiceMerged,
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "service_certificates")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub ca_fingerprint: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub serial_number: String,
    pub service_id: Uuid,
    pub not_before: OffsetDateTime,
    pub not_after: OffsetDateTime,
    pub revoked_at: Option<OffsetDateTime>,
    pub revocation_reason: Option<RevocationReason>,
    pub created_at: OffsetDateTime,
    pub last_seen_at: Option<OffsetDateTime>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::service::Entity",
        from = "Column::ServiceId",
        to = "super::service::Column::Id"
    )]
    Service,
}

impl Related<super::service::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Service.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
