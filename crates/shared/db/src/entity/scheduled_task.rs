use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

/// Type of scheduled task.
///
/// New variants may be added in future versions. External code that matches on this enum
/// must include a wildcard arm to handle unknown variants added during rolling upgrades.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
#[non_exhaustive]
pub enum ScheduledTaskType {
    #[sea_orm(string_value = "auth_cleanup")]
    AuthCleanup,
    #[sea_orm(string_value = "stale_lease_cleanup")]
    StaleLeaseCleanup,
    #[sea_orm(string_value = "ca_rotation_check")]
    CaRotationCheck,
    #[sea_orm(string_value = "version_check")]
    VersionCheck,
    #[sea_orm(string_value = "service_cert_check")]
    ServiceCertCheck,
    #[sea_orm(string_value = "crl_renewal")]
    CrlRenewal,
}

impl ScheduledTaskType {
    /// Human-readable label for display.
    pub fn label(&self) -> &'static str {
        match self {
            Self::AuthCleanup => "Auth Cleanup",
            Self::StaleLeaseCleanup => "Stale Lease Cleanup",
            Self::CaRotationCheck => "CA Rotation Check",
            Self::VersionCheck => "Version Check",
            Self::ServiceCertCheck => "Service Cert Check",
            Self::CrlRenewal => "CRL Renewal",
        }
    }
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "scheduled_tasks")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub task_type: ScheduledTaskType,
    pub cron_expression: String,
    pub enabled: bool,
    pub task_config: Option<serde_json::Value>,
    pub last_run_at: Option<OffsetDateTime>,
    pub next_run_at: OffsetDateTime,
    pub locked_by: Option<Uuid>,
    pub locked_at: Option<OffsetDateTime>,
    pub last_error: Option<String>,
    pub run_count: i64,
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
