use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

/// Cluster-visible runtime state for an embedded service.
///
/// This table stores ephemeral HA-visible state only. The owning controller
/// keeps it fresh while the embedded service is yielded; API readers ignore
/// stale rows.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "embedded_service_runtime_states")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub service_id: Uuid,
    /// JSON array of external service UUID strings causing the yield.
    pub yielded_to_json: Option<String>,
    pub updated_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
