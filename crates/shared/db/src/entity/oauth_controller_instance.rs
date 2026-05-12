use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

/// `oauth_controller_instances` row.
///
/// Per-process record used by the multi-controller boot guard. See
/// `m20260513_000006_oauth_controller_instances` for the full rationale.
/// No foreign keys: rows are self-contained.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "oauth_controller_instances")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub instance_id: Uuid,
    pub jwt_secret_fingerprint: String,
    pub started_at: OffsetDateTime,
    pub last_seen_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
