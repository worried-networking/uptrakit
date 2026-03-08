use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "host_software_item_plugins")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub host_id: Uuid,
    pub software_item_id: Uuid,
    pub host_software_item_id: Uuid,
    pub plugin_config_id: Uuid,
    /// Plugin role: "detect_version", "fetch_releases", "execute_update".
    /// Stored as TEXT; parsed to PluginRole at application boundaries.
    pub role: String,
    /// Ordering within the same role. Currently always 0 for single-instance
    /// roles; reserved for future multi-instance roles (e.g. hooks).
    pub ordinal: i32,
    pub package_identifier: String,
    #[sea_orm(column_type = "JsonBinary", nullable)]
    pub config_override: Option<serde_json::Value>,
    /// Controls where this plugin's operation executes:
    /// "auto" (default), "agent", or "controller".
    pub execution_site: String,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
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
        from = "Column::PluginConfigId",
        to = "super::plugin_config::Column::Id"
    )]
    PluginConfig,
    #[sea_orm(
        belongs_to = "super::host_software_item::Entity",
        from = "Column::HostSoftwareItemId",
        to = "super::host_software_item::Column::Id"
    )]
    HostSoftwareItem,
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
        Relation::PluginConfig.def()
    }
}

impl Related<super::host_software_item::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::HostSoftwareItem.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
