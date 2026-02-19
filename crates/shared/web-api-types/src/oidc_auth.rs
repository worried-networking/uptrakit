use serde::{Deserialize, Serialize};
use uptrakit_shared_types::SecretString;
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OidcProviderInfo {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub logo_url: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AuthMethodsResponse {
    pub password: bool,
    pub oidc_providers: Vec<OidcProviderInfo>,
    pub setup_required: bool,
    /// Whether OIDC registration requires a registration token.
    pub registration_token_required: bool,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OidcAuthorizeResponse {
    pub authorize_url: String,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OidcLinkRequest {
    pub link_token: SecretString,
    pub password: Option<SecretString>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OidcExchangeRequest {
    pub code: String,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OidcCompleteRegistrationRequest {
    pub registration_code: SecretString,
    pub registration_token: SecretString,
}
