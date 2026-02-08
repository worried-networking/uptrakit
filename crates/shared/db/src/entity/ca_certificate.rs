use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "ca_certificates")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub fingerprint: String,
    pub cert_pem: String,
    pub key_pem: String,
    pub not_before: OffsetDateTime,
    pub not_after: OffsetDateTime,
    pub activated_at: OffsetDateTime,
    pub deactivated_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
