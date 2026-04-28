// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use crate::generated::shared_types::SecretString;
use crate::generated::types::validation::{Validate, ValidationError};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;
#[derive(Serialize, Deserialize)]
pub struct CreateApiTokenRequest {
    pub name: String,
}
impl Validate for CreateApiTokenRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.name.trim().is_empty() {
            return Err(ValidationError {
                field: "name",
                message: "name must not be empty".to_string(),
            });
        }
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
pub struct CreateApiTokenResponse {
    pub id: Uuid,
    pub token: SecretString,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}
#[derive(Serialize, Deserialize)]
pub struct ApiTokenResponse {
    pub id: Uuid,
    pub name: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_used_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub revoked_at: Option<OffsetDateTime>,
}
#[derive(Serialize, Deserialize)]
pub struct ApiTokenListResponse {
    pub tokens: Vec<ApiTokenResponse>,
}
