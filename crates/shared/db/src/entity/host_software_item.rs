use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "host_software_items")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub host_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub software_item_id: Uuid,
    pub provider_config_id: Uuid,
    pub package_identifier: String,
    #[sea_orm(column_type = "JsonBinary", nullable)]
    pub config_override: Option<serde_json::Value>,
    pub installed_version: Option<String>,
    pub installed_version_detected_at: Option<OffsetDateTime>,
    pub last_updated_at: Option<OffsetDateTime>,
    pub linked_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::host::Entity",
        from = "Column::HostId",
        to = "super::host::Column::Id"
    )]
    Host,
    #[sea_orm(
        belongs_to = "super::software_item::Entity",
        from = "Column::SoftwareItemId",
        to = "super::software_item::Column::Id"
    )]
    SoftwareItem,
    #[sea_orm(
        belongs_to = "super::plugin_config::Entity",
        from = "Column::ProviderConfigId",
        to = "super::plugin_config::Column::Id"
    )]
    ProviderConfig,
}

impl Related<super::host::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Host.def()
    }
}

impl Related<super::software_item::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::SoftwareItem.def()
    }
}

impl Related<super::plugin_config::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::ProviderConfig.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
