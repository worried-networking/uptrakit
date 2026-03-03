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
    #[sea_orm(string_value = "fetch_releases")]
    FetchReleases,
    #[sea_orm(string_value = "detect_version")]
    DetectVersion,
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
            Self::FetchReleases => "Fetch Latest Releases",
            Self::DetectVersion => "Detect Installed Versions",
            Self::ServiceCertCheck => "Service Cert Check",
            Self::CrlRenewal => "CRL Renewal",
        }
    }

    /// Whether this task type must run on the controller's embedded scheduler.
    ///
    /// Internal tasks require direct in-process access to controller resources
    /// (revocation notify, CA rotation trigger, service connections) and must
    /// not be delegated to the external scheduler.
    pub fn is_internal(&self) -> bool {
        matches!(
            self,
            Self::CrlRenewal | Self::CaRotationCheck | Self::ServiceCertCheck
        )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_internal_returns_true_for_internal_tasks() {
        assert!(ScheduledTaskType::CrlRenewal.is_internal());
        assert!(ScheduledTaskType::CaRotationCheck.is_internal());
        assert!(ScheduledTaskType::ServiceCertCheck.is_internal());
    }

    #[test]
    fn is_internal_returns_false_for_external_tasks() {
        assert!(!ScheduledTaskType::AuthCleanup.is_internal());
        assert!(!ScheduledTaskType::StaleLeaseCleanup.is_internal());
        assert!(!ScheduledTaskType::FetchReleases.is_internal());
        assert!(!ScheduledTaskType::DetectVersion.is_internal());
    }
}
