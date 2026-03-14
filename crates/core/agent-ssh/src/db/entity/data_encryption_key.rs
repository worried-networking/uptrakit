use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

/// A wrapped data encryption key (DEK) in the local SSH agent database.
///
/// Same schema as the controller's `data_encryption_key` entity.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "data_encryption_keys")]
#[allow(unreachable_pub)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(column_type = "Text", unique)]
    pub key_id: String,
    #[sea_orm(column_type = "Text")]
    pub wrapped_key: String,
    #[sea_orm(column_type = "Text")]
    pub kek_fingerprint: String,
    #[sea_orm(column_type = "Text")]
    pub status: String,
    pub created_at: OffsetDateTime,
    pub retired_at: Option<OffsetDateTime>,
}

#[allow(unreachable_pub)]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
