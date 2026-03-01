use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "host_software_items")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub host_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub software_item_id: Uuid,
    pub installed_version: Option<String>,
    pub installed_version_detected_at: Option<OffsetDateTime>,
    pub latest_version: Option<String>,
    pub latest_version_fetched_at: Option<OffsetDateTime>,
    #[sea_orm(column_type = "JsonBinary", nullable)]
    pub latest_release_metadata: Option<serde_json::Value>,
    pub last_updated_at: Option<OffsetDateTime>,
    pub linked_at: OffsetDateTime,
    /// Classification of the available update (security, bugfix, feature, unknown).
    #[sea_orm(default_value = "unknown")]
    pub update_category: String,
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
    #[sea_orm(has_many = "super::host_software_item_plugin::Entity")]
    HostSoftwareItemPlugins,
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

impl Related<super::host_software_item_plugin::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::HostSoftwareItemPlugins.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
