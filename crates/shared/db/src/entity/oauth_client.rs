use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

/// `oauth_clients` row.
///
/// `id` is `String` (TEXT in the DB) because Client Identifier Metadata
/// Document (CIMD) client_ids are HTTPS URLs, while Dynamic Client
/// Registration (DCR) client_ids are UUID-as-text. See
/// `m20260513_000001_oauth_clients` for the full rationale and column
/// semantics.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "oauth_clients")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub id: String,
    #[sea_orm(column_type = "Text")]
    pub client_name: String,
    #[sea_orm(column_type = "Text")]
    pub client_uri: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub logo_uri: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub redirect_uris: String,
    #[sea_orm(column_type = "Text")]
    pub default_scope: String,
    #[sea_orm(column_type = "Text")]
    pub grant_types: String,
    #[sea_orm(column_type = "Text")]
    pub response_types: String,
    #[sea_orm(column_type = "Text")]
    pub token_endpoint_auth_method: String,
    #[sea_orm(column_type = "Text")]
    pub client_secret_hash: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub registration_access_token_hash: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub created_via: String,
    pub created_at: OffsetDateTime,
    pub last_used_at: Option<OffsetDateTime>,
    pub revoked_at: Option<OffsetDateTime>,
    pub metadata_cached_at: Option<OffsetDateTime>,
    #[sea_orm(column_type = "Text")]
    pub metadata_etag: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub metadata_content_hash: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub metadata_raw: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub metadata_parse_error: Option<String>,
    pub metadata_parse_error_at: Option<OffsetDateTime>,
    pub trusted_at: Option<OffsetDateTime>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
