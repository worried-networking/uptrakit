// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use crate::generated::shared_types::SecretString;
use crate::generated::types::registration::RegistrationMode;
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
pub struct RegistrationSettingsResponse {
    pub mode: RegistrationMode,
    /// Whether OIDC users also need a registration token (only relevant in `invite` mode).
    pub require_token_for_oidc: bool,
}
#[derive(Serialize, Deserialize)]
pub struct UpdateRegistrationSettingsRequest {
    pub mode: RegistrationMode,
    /// Required when mode is `invite`. The plaintext token will be hashed before storage.
    pub token: Option<SecretString>,
    /// Whether OIDC users also need a registration token (only relevant in `invite` mode).
    pub require_token_for_oidc: Option<bool>,
}
