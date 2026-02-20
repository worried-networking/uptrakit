use sea_orm::entity::prelude::*;
use time::OffsetDateTime;
use uptrakit_shared_types::DeviceAuthStatus;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "pending_device_flows")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(unique, column_type = "Text")]
    pub device_code_hash: String,
    #[sea_orm(unique, column_type = "Text")]
    pub user_code: String,
    pub status: DeviceAuthStatus,
    pub user_id: Option<Uuid>,
    pub client_name: Option<String>,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
