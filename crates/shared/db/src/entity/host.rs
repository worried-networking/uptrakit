use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "hosts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub machine_id: String,
    pub hostname: String,
    pub friendly_name: String,
    pub os_type: Option<String>,
    pub os_version: Option<String>,
    pub architecture: Option<String>,
    pub ip_address: Option<String>,
    pub last_seen_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub deactivated_at: Option<OffsetDateTime>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::tenant::Entity",
        from = "Column::TenantId",
        to = "super::tenant::Column::Id"
    )]
    Tenant,
    #[sea_orm(has_many = "super::update_history::Entity")]
    UpdateHistory,
}

impl Related<super::tenant::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Tenant.def()
    }
}

impl Related<super::service::Entity> for Entity {
    fn to() -> RelationDef {
        super::service_host::Relation::Service.def()
    }

    fn via() -> Option<RelationDef> {
        Some(super::service_host::Relation::Host.def().rev())
    }
}

impl Related<super::software_item::Entity> for Entity {
    fn to() -> RelationDef {
        super::host_software_item::Relation::SoftwareItem.def()
    }

    fn via() -> Option<RelationDef> {
        Some(super::host_software_item::Relation::Host.def().rev())
    }
}

impl Related<super::update_history::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::UpdateHistory.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
