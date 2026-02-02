use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "software_items")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub name: String,
    pub provider_config_id: Uuid,
    pub package_identifier: String,
    #[sea_orm(column_type = "JsonBinary", nullable)]
    pub config_override: Option<serde_json::Value>,
    pub enabled: bool,
    pub last_checked_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub deactivated_at: Option<OffsetDateTime>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::provider_config::Entity",
        from = "Column::ProviderConfigId",
        to = "super::provider_config::Column::Id"
    )]
    ProviderConfig,
}

impl Related<super::provider_config::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::ProviderConfig.def()
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

impl ActiveModelBehavior for ActiveModel {}
