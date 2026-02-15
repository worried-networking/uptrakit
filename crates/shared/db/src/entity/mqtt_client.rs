use sea_orm::entity::prelude::*;
use time::OffsetDateTime;
use uptrakit_shared_types::{MqttClientConnectionStatus, MqttTransport};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "mqtt_clients")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub enabled: bool,
    pub transport: MqttTransport,
    pub host: String,
    pub port: i32,
    pub client_id: String,
    pub username: Option<String>,
    pub password: Option<crate::crypto::EncryptedString>,
    pub topic_prefix: String,
    pub connection_status: MqttClientConnectionStatus,
    pub status_updated_at: OffsetDateTime,
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
}

impl Related<super::tenant::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Tenant.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
