use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "audit_logs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub actor_id: Option<Uuid>,
    pub actor_type: String,
    pub actor_display: Option<String>,
    pub action_type: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub target_display: Option<String>,
    pub outcome: String,
    pub details_json: Option<serde_json::Value>,
    pub request_id: Option<String>,
    pub occurred_at: OffsetDateTime,
}

/// No FK constraint on `tenant_id` — audit records are immutable and must
/// survive tenant lifecycle changes (deletion, migration) for compliance.
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
