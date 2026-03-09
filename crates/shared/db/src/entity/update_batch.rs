use sea_orm::entity::prelude::*;
use time::OffsetDateTime;
use uptrakit_shared_types::BatchStatus;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "update_batches")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub batch_type: String,
    pub status: BatchStatus,
    pub total_count: i32,
    pub actor_type: String,
    pub actor_id: String,
    #[sea_orm(column_type = "Text")]
    pub output: String,
    pub output_bytes: i64,
    pub created_at: OffsetDateTime,
    pub completed_at: Option<OffsetDateTime>,
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

impl Related<super::update_history::Entity> for Entity {
    fn to() -> RelationDef {
        super::update_history::Relation::UpdateBatch.def().rev()
    }
}

impl ActiveModelBehavior for ActiveModel {}
