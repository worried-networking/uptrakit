use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "pending_device_flows")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub device_code: String,
    #[sea_orm(unique, column_type = "Text")]
    pub user_code: String,
    #[sea_orm(column_type = "Text")]
    pub status: String,
    pub user_id: Option<Uuid>,
    pub client_name: Option<String>,
    pub created_at: OffsetDateTime,
    pub last_polled_at: Option<OffsetDateTime>,
    pub expires_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
