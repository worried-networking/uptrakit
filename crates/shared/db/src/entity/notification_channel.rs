use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "notification_channels")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub channel_type: String,
    pub config: uptrakit_crypto::EncryptedString,
    pub enabled: bool,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::tenant::Entity",
        from = "Column::TenantId",
        to = "super::tenant::Column::Id"
    )]
    Tenant,
    #[sea_orm(has_many = "super::notification_rule::Entity")]
    NotificationRules,
    #[sea_orm(has_many = "super::notification_log::Entity")]
    NotificationLogs,
}

impl Related<super::tenant::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Tenant.def()
    }
}

impl Related<super::notification_rule::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::NotificationRules.def()
    }
}

impl Related<super::notification_log::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::NotificationLogs.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
