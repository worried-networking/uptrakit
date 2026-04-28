// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use crate::generated::shared_types::SecretString;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[derive(Serialize, Deserialize)]
pub struct OidcProviderInfo {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub logo_url: Option<String>,
}
#[derive(Serialize, Deserialize)]
pub struct AuthMethodsResponse {
    pub password: bool,
    pub oidc_providers: Vec<OidcProviderInfo>,
    pub setup_required: bool,
    /// Whether OIDC registration requires a registration token.
    pub registration_token_required: bool,
}
#[derive(Serialize, Deserialize)]
pub struct OidcAuthorizeResponse {
    pub authorize_url: String,
}
#[derive(Serialize, Deserialize)]
pub struct OidcLinkRequest {
    pub link_token: SecretString,
    pub password: Option<SecretString>,
}
#[derive(Serialize, Deserialize)]
pub struct OidcExchangeRequest {
    pub code: String,
}
#[derive(Serialize, Deserialize)]
pub struct OidcCompleteRegistrationRequest {
    pub registration_code: SecretString,
    pub registration_token: SecretString,
}
