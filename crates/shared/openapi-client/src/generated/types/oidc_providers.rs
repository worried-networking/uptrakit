use crate::generated::shared_types::SecretString;
use crate::generated::types::validation::{Validate, ValidationError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use time::OffsetDateTime;
use uuid::Uuid;
pub fn default_scopes() -> String {
    "openid email profile groups".to_string()
}
pub fn default_auto_create() -> bool {
    true
}
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateOidcProviderRequest {
    pub name: String,
    pub slug: String,
    pub logo_url: Option<String>,
    pub issuer_url: String,
    pub client_id: String,
    pub client_secret: SecretString,
    #[serde(default = "default_scopes")]
    pub scopes: String,
    #[serde(default = "default_auto_create")]
    pub auto_create_users: bool,
    pub allow_private_network_issuers: Option<bool>,
    pub role_claim_path: Option<String>,
    #[serde(default)]
    pub role_mapping: HashMap<String, String>,
}
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateOidcProviderRequest {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub logo_url: Option<String>,
    pub issuer_url: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub scopes: Option<String>,
    pub auto_create_users: Option<bool>,
    pub allow_private_network_issuers: Option<bool>,
    pub role_claim_path: Option<String>,
    pub role_mapping: Option<HashMap<String, String>>,
}
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OidcProviderResponse {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub logo_url: Option<String>,
    pub issuer_url: String,
    pub client_id: String,
    pub has_client_secret: bool,
    pub scopes: String,
    pub auto_create_users: bool,
    pub allow_private_network_issuers: bool,
    pub role_claim_path: Option<String>,
    pub role_mapping: HashMap<String, String>,
    pub is_active: bool,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub updated_at: OffsetDateTime,
}
impl Validate for CreateOidcProviderRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.name.is_empty() {
            return Err(ValidationError {
                field: "name",
                message: "must not be empty".to_string(),
            });
        }
        if self.slug.len() > 64 {
            return Err(ValidationError {
                field: "slug",
                message: "must be at most 64 characters".to_string(),
            });
        }
        let first = self.slug.as_bytes().first().copied().unwrap_or(0);
        let valid_slug = !self.slug.is_empty()
            && (first.is_ascii_lowercase() || first.is_ascii_digit())
            && self
                .slug
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !valid_slug {
            return Err(ValidationError {
                field: "slug",
                message: "must match ^[a-z0-9][a-z0-9-]*$".to_string(),
            });
        }
        if !self.issuer_url.starts_with("http://") && !self.issuer_url.starts_with("https://") {
            return Err(ValidationError {
                field: "issuer_url",
                message: "must start with http:// or https://".to_string(),
            });
        }
        if self.client_id.is_empty() {
            return Err(ValidationError {
                field: "client_id",
                message: "must not be empty".to_string(),
            });
        }
        Ok(())
    }
}
