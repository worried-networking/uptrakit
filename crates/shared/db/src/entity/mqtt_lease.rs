use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "mqtt_leases")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub mqtt_client_id: Uuid,
    pub instance_id: String,
    pub heartbeat_at: OffsetDateTime,
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
        belongs_to = "super::mqtt_client::Entity",
        from = "Column::MqttClientId",
        to = "super::mqtt_client::Column::Id"
    )]
    MqttClient,
}

impl Related<super::tenant::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Tenant.def()
    }
}

impl Related<super::mqtt_client::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::MqttClient.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
