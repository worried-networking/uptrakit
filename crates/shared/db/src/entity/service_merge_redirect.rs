//! Mapping from a deactivated source Service UUID to its merge target.
//!
//! Written by `merge_service` inside the same `BEGIN IMMEDIATE` transaction
//! that deactivates the source row. Read on the bearer-secret WS auth path
//! when an Agent's persisted `service_id` no longer matches an active row,
//! so the controller can resolve the Agent to its current canonical identity.

use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "service_merge_redirect")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub source_id: Uuid,
    pub target_id: Uuid,
    pub redirected_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::service::Entity",
        from = "Column::TargetId",
        to = "super::service::Column::Id"
    )]
    Service,
}

impl Related<super::service::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Service.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
