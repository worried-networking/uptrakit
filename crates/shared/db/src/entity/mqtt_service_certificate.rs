use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

/// Reason for revoking an MQTT service certificate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum MqttServiceCertificateRevocationReason {
    #[sea_orm(string_value = "certificate_renewed")]
    CertificateRenewed,
    #[sea_orm(string_value = "service_deactivated")]
    ServiceDeactivated,
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "mqtt_service_certificates")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub ca_fingerprint: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub serial_number: String,
    pub mqtt_service_id: Uuid,
    pub not_before: OffsetDateTime,
    pub not_after: OffsetDateTime,
    pub revoked_at: Option<OffsetDateTime>,
    pub revocation_reason: Option<MqttServiceCertificateRevocationReason>,
    pub created_at: OffsetDateTime,
    pub last_seen_at: Option<OffsetDateTime>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::mqtt_service::Entity",
        from = "Column::MqttServiceId",
        to = "super::mqtt_service::Column::Id"
    )]
    MqttService,
}

impl Related<super::mqtt_service::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::MqttService.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
