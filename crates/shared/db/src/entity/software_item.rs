use sea_orm::entity::prelude::*;
use time::OffsetDateTime;
use uptrakit_shared_types::SoftwareDiscoveryState;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "software_items")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub provider_config_id: Uuid,
    pub package_identifier: String,
    #[sea_orm(column_type = "JsonBinary", nullable)]
    pub config_override: Option<serde_json::Value>,
    pub enabled: bool,
    pub discovery_state: Option<SoftwareDiscoveryState>,
    pub last_checked_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub deactivated_at: Option<OffsetDateTime>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::available_version::Entity")]
    AvailableVersion,
    #[sea_orm(
        belongs_to = "super::provider_config::Entity",
        from = "Column::ProviderConfigId",
        to = "super::provider_config::Column::Id"
    )]
    ProviderConfig,
    #[sea_orm(
        belongs_to = "super::tenant::Entity",
        from = "Column::TenantId",
        to = "super::tenant::Column::Id"
    )]
    Tenant,
    #[sea_orm(has_many = "super::update_history::Entity")]
    UpdateHistory,
}

impl Related<super::available_version::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::AvailableVersion.def()
    }
}

impl Related<super::provider_config::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::ProviderConfig.def()
    }
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
