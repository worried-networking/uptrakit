use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

/// Type of service (agent or MQTT).
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum ServiceType {
    #[sea_orm(string_value = "agent")]
    Agent,
    #[sea_orm(string_value = "mqtt")]
    Mqtt,
}

/// Status of a service in the enrollment/approval workflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum ServiceStatus {
    #[sea_orm(string_value = "pending")]
    Pending,
    #[sea_orm(string_value = "approved")]
    Approved,
    #[sea_orm(string_value = "rejected")]
    Rejected,
    #[sea_orm(string_value = "deactivated")]
    Deactivated,
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "services")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub service_type: ServiceType,
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

impl Related<super::host::Entity> for Entity {
    fn to() -> RelationDef {
        super::service_host::Relation::Host.def()
    }

    fn via() -> Option<RelationDef> {
        Some(super::service_host::Relation::Service.def().rev())
    }
}

impl ActiveModelBehavior for ActiveModel {}
