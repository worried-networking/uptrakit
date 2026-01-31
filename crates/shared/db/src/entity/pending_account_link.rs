use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "pending_account_links")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub link_token: String,
    pub provider_id: Uuid,
    #[sea_orm(column_type = "Text")]
    pub oidc_subject: String,
    #[sea_orm(column_type = "Text")]
    pub email: String,
    pub user_id: Uuid,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    #[sea_orm(column_type = "Json")]
    pub mapped_roles: serde_json::Value,
    pub existing_link_provider_id: Option<Uuid>,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
