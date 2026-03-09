use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "software_items")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub featured: bool,
    pub last_checked_at: Option<OffsetDateTime>,
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

impl Related<super::host::Entity> for Entity {
    fn to() -> RelationDef {
        super::host_software_item::Relation::Host.def()
    }

    fn via() -> Option<RelationDef> {
        Some(
            super::host_software_item::Relation::SoftwareItem
                .def()
                .rev(),
        )
    }
}

impl Related<super::update_history::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::UpdateHistory.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
