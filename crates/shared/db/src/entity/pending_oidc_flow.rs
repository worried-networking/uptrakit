use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

use uptrakit_crypto::EncryptedString;

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
    /// Canonical-host `redirect_uri` pinned at authorize time; not a
    /// secret, appears in the OIDC redirect URL and audit trail anyway.
    #[sea_orm(column_type = "Text")]
    pub redirect_uri: String,
    /// Originating request's scheme+host, snapshotted for callback replay.
    #[sea_orm(column_type = "Text")]
    pub return_origin: String,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
