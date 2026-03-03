use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "audit_logs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub actor_id: Uuid,
    pub actor_type: String,
    pub auth_method: String,
    pub http_method: String,
    pub http_path: String,
    pub route_pattern: Option<String>,
    pub http_status: i32,
    pub client_ip: Option<String>,
    pub user_agent: Option<String>,
    pub duration_ms: i64,
    pub occurred_at: OffsetDateTime,
}

/// No FK constraint on `tenant_id` — audit records are immutable and must
/// survive tenant lifecycle changes (deletion, migration) for compliance.
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
