use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uptrakit_shared_types::SecretString;
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
    pub role_claim_path: Option<String>,
    pub role_mapping: HashMap<String, String>,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
}
