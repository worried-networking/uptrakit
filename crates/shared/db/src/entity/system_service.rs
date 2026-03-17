use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

/// Status of a system service.
///
/// Mirrors [`uptrakit_shared_types::ServiceStatus`] but is defined here as a
/// DB-derived enum because `system_services` is a separate table and
/// `ServiceStatus` is used for both. Having a local type avoids cross-crate
/// coupling in SeaORM derive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum SystemServiceStatus {
    #[sea_orm(string_value = "pending")]
    Pending,
    #[sea_orm(string_value = "approved")]
    Approved,
    #[sea_orm(string_value = "rejected")]
    Rejected,
    #[sea_orm(string_value = "deactivated")]
    Deactivated,
}

/// A tenant-agnostic infrastructure service (MQTT bridge, external scheduler).
///
/// Unlike [`super::service::Model`], this entity has no `tenant_id` — system
/// services are global and serve all tenants. Enrollment is authenticated via
/// [`super::system_enrollment_token::Model`] tokens (Argon2id-hashed, named,
/// expirable) rather than a plaintext global setting.
///
/// `system_enrollment_token_id` is audit-only: the token that approved this
/// enrollment. No FK constraint — tokens may be revoked or deleted after the
/// service has already enrolled.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "system_services")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(column_type = "Text")]
    pub capabilities: String,
    pub hostname: String,
    pub friendly_name: String,
    pub ip_address: Option<String>,
    pub status: SystemServiceStatus,
    #[sea_orm(unique)]
    pub enrollment_secret_hash: String,
    pub client_version: Option<String>,
    pub last_seen_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub deactivated_at: Option<OffsetDateTime>,
    pub ping_interval_seconds: Option<i32>,
    pub cert_lifetime_hours: Option<i32>,
    /// Audit trail: ID of the system enrollment token used to approve enrollment.
    /// `None` for services enrolled manually or before this feature was introduced.
    pub system_enrollment_token_id: Option<Uuid>,
    /// The binary/crate name of the service (e.g., `"uptrakit-mqtt"`).
    /// Null for services enrolled before this field was introduced.
    pub service_app_name: Option<String>,
    /// Whether this row represents a controller-embedded service.
    pub is_embedded: bool,
    /// Stable controller installation identity that owns this embedded row.
    /// Null for non-embedded services and legacy rows before ownership is claimed.
    pub embedded_owner_key: Option<Uuid>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::system_service_certificate::Entity")]
    SystemServiceCertificate,
    #[sea_orm(
        belongs_to = "super::system_enrollment_token::Entity",
        from = "Column::SystemEnrollmentTokenId",
        to = "super::system_enrollment_token::Column::Id"
    )]
    SystemEnrollmentToken,
}

impl Related<super::system_service_certificate::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::SystemServiceCertificate.def()
    }
}

impl Related<super::system_enrollment_token::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::SystemEnrollmentToken.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
