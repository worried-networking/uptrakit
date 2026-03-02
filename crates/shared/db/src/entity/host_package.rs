use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "host_packages")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub plugin_config_id: Uuid,
    pub package_identifier: String,
    pub name: String,
    pub installed_version: Option<String>,
    pub installed_version_detected_at: Option<OffsetDateTime>,
    pub latest_version: Option<String>,
    pub latest_version_fetched_at: Option<OffsetDateTime>,
    #[sea_orm(column_type = "JsonBinary", nullable)]
    pub latest_release_metadata: Option<serde_json::Value>,
    #[sea_orm(default_value = "unknown")]
    pub update_category: String,
    pub enabled: bool,
    pub last_checked_at: Option<OffsetDateTime>,
    pub last_updated_at: Option<OffsetDateTime>,
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
    #[sea_orm(
        belongs_to = "super::host::Entity",
        from = "Column::HostId",
        to = "super::host::Column::Id"
    )]
    Host,
    #[sea_orm(
        belongs_to = "super::plugin_config::Entity",
        from = "Column::PluginConfigId",
        to = "super::plugin_config::Column::Id"
    )]
    PluginConfig,
    #[sea_orm(has_many = "super::host_package_update_history::Entity")]
    HostPackageUpdateHistory,
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

impl Related<super::plugin_config::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::PluginConfig.def()
    }
}

impl Related<super::host_package_update_history::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::HostPackageUpdateHistory.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
