use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "host_package_update_history")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub host_package_id: Uuid,
    pub from_version: Option<String>,
    pub to_version: Option<String>,
    pub status: String,
    #[sea_orm(column_type = "Text", nullable)]
    pub output: Option<String>,
    pub output_bytes: i64,
    pub actor_type: String,
    pub actor_id: String,
    #[sea_orm(default_value = "unknown")]
    pub update_category: String,
    pub started_at: Option<OffsetDateTime>,
    pub completed_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub batch_id: Option<Uuid>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::tenant::Entity",
        from = "Column::TenantId",
        to = "super::tenant::Column::Id"
    )]
    Tenant,
    #[sea_orm(
        belongs_to = "super::host::Entity",
        from = "Column::HostId",
        to = "super::host::Column::Id"
    )]
    Host,
    #[sea_orm(
        belongs_to = "super::host_package::Entity",
        from = "Column::HostPackageId",
        to = "super::host_package::Column::Id"
    )]
    HostPackage,
    #[sea_orm(
        belongs_to = "super::update_batch::Entity",
        from = "Column::BatchId",
        to = "super::update_batch::Column::Id"
    )]
    UpdateBatch,
}

impl Related<super::tenant::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Tenant.def()
    }
}

impl Related<super::host::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Host.def()
    }
}

impl Related<super::host_package::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::HostPackage.def()
    }
}

impl Related<super::update_batch::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::UpdateBatch.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
