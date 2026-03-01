use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "notification_log")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub channel_id: Uuid,
    pub rule_id: Uuid,
    pub event_type: String,
    #[sea_orm(column_type = "Json")]
    pub event_payload: serde_json::Value,
    pub status: String,
    pub error_message: Option<String>,
    pub action_token: Option<Uuid>,
    pub action_taken: Option<String>,
    pub created_at: OffsetDateTime,
    pub delivered_at: Option<OffsetDateTime>,
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
        belongs_to = "super::notification_rule::Entity",
        from = "Column::RuleId",
        to = "super::notification_rule::Column::Id"
    )]
    NotificationRule,
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

impl Related<super::notification_rule::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::NotificationRule.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
