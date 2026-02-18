use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

use crate::crypto::EncryptedString;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "pending_oidc_flows")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub csrf_state: String,
    pub provider_id: Uuid,
    #[sea_orm(column_type = "Text")]
    pub pkce_verifier: EncryptedString,
    #[sea_orm(column_type = "Text")]
    pub nonce: String,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
