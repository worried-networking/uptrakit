use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

/// A system-scoped enrollment token used to auto-approve system service
/// enrollments (MQTT bridge, external scheduler).
///
/// Unlike [`super::enrollment_token::Model`], this entity:
/// - Has no `tenant_id` (global scope)
/// - Has no `allowed_capabilities` (system services have fixed capabilities)
/// - Has no FK to `users` for `created_by_user_id` (stored for audit only)
///
/// Token secrets are never stored in plaintext. The `token_hash` column holds
/// an Argon2id hash of the randomly-generated plaintext token.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "system_enrollment_tokens")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(column_type = "Text")]
    pub name: String,
    #[sea_orm(column_type = "Text", unique)]
    pub token_hash: String,
    pub max_uses: Option<i32>,
    pub current_uses: i32,
    pub expires_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub revoked_at: Option<OffsetDateTime>,
    /// Audit-only: the user ID who created this token.
    /// Stored without a FK because users are tenant-scoped.
    pub created_by_user_id: Option<Uuid>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::system_service::Entity")]
    SystemService,
}

impl Related<super::system_service::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::SystemService.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
