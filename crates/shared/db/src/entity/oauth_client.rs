use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

/// `oauth_clients` row.
///
/// `id` is `String` (TEXT in the DB) because Client Identifier Metadata
/// Document (CIMD) client_ids are HTTPS URLs, while Dynamic Client
/// Registration (DCR) client_ids are UUID-as-text. See
/// `m20260512_000001_oauth_clients` for the full rationale and column
/// semantics.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "oauth_clients")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub id: String,
    pub client_name: String,
    pub client_uri: Option<String>,
    pub logo_uri: Option<String>,
    pub redirect_uris: String,
    pub default_scope: String,
    pub grant_types: String,
    pub response_types: String,
    pub token_endpoint_auth_method: String,
    pub client_secret_hash: Option<String>,
    pub registration_access_token_hash: Option<String>,
    pub created_via: String,
    pub created_at: OffsetDateTime,
    pub last_used_at: Option<OffsetDateTime>,
    pub revoked_at: Option<OffsetDateTime>,
    pub metadata_cached_at: Option<OffsetDateTime>,
    pub metadata_etag: Option<String>,
    pub metadata_content_hash: Option<String>,
    pub metadata_raw: Option<String>,
    pub metadata_parse_error: Option<String>,
    pub metadata_parse_error_at: Option<OffsetDateTime>,
    pub trusted_at: Option<OffsetDateTime>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
