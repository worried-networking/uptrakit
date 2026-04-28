// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
pub struct AuthenticationSettingsResponse {
    pub password_auth_enabled: bool,
}
#[derive(Serialize, Deserialize)]
pub struct UpdateAuthenticationSettingsRequest {
    pub password_auth_enabled: Option<bool>,
}
