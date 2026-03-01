use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "notification_rules")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub channel_id: Uuid,
    pub event_type: String,
    pub host_id: Option<Uuid>,
    pub software_item_id: Option<Uuid>,
    pub plugin_type: Option<String>,
    pub enabled: bool,
    pub created_at: OffsetDateTime,
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
        belongs_to = "super::notification_channel::Entity",
        from = "Column::ChannelId",
        to = "super::notification_channel::Column::Id"
    )]
    NotificationChannel,
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
    #[sea_orm(has_many = "super::notification_log::Entity")]
    NotificationLogs,
}

impl Related<super::tenant::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Tenant.def()
    }
}

impl Related<super::notification_channel::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::NotificationChannel.def()
    }
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

impl Related<super::notification_log::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::NotificationLogs.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
