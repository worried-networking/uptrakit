use crate::generated::shared_types::SecretString;
use crate::generated::types::validation::{Validate, ValidationError};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;
/// Request to create a new system enrollment token.
///
/// System enrollment tokens are globally scoped (no tenant) and are used
/// to auto-approve system service enrollments (MQTT bridge, external scheduler).
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateSystemEnrollmentTokenRequest {
    /// Human-readable name for this token.
    pub name: String,
    /// Maximum number of system service enrollments allowed with this token.
    /// `None` means unlimited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_uses: Option<u32>,
    /// Token lifetime in seconds from now. `None` means never expires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in_seconds: Option<u64>,
}
impl Validate for CreateSystemEnrollmentTokenRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.name.trim().is_empty() {
            return Err(ValidationError {
                field: "name",
                message: "name must not be empty".to_string(),
            });
        }
        if let Some(max_uses) = self.max_uses
            && max_uses == 0
        {
            return Err(ValidationError {
                field: "max_uses",
                message: "max_uses must be greater than 0".to_string(),
            });
        }
        if let Some(expires_in) = self.expires_in_seconds
            && expires_in == 0
        {
            return Err(ValidationError {
                field: "expires_in_seconds",
                message: "expires_in_seconds must be greater than 0".to_string(),
            });
        }
        Ok(())
    }
}
/// Response returned when a new system enrollment token is created.
///
/// The plaintext `token` is only available in this response; it cannot be
/// retrieved later. Store it securely immediately after creation.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SystemEnrollmentTokenCreatedResponse {
    pub id: Uuid,
    /// The plaintext token value. Only returned once at creation time.
    pub token: SecretString,
    pub name: String,
    pub max_uses: Option<u32>,
    pub current_uses: u32,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = Option<String>, format = DateTime)
    )]
    pub expires_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
    pub created_by_user_id: Option<Uuid>,
}
/// Response for a system enrollment token (without the plaintext token).
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SystemEnrollmentTokenResponse {
    pub id: Uuid,
    pub name: String,
    pub max_uses: Option<u32>,
    pub current_uses: u32,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = Option<String>, format = DateTime)
    )]
    pub expires_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = Option<String>, format = DateTime)
    )]
    pub revoked_at: Option<OffsetDateTime>,
    pub created_by_user_id: Option<Uuid>,
}
/// Query parameters for listing system enrollment tokens.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
pub struct ListSystemEnrollmentTokensQuery {
    /// Page number (1-indexed). Defaults to 1.
    pub page: Option<u64>,
    /// Items per page. Defaults to 20, max 1000.
    pub per_page: Option<u64>,
}
impl ListSystemEnrollmentTokensQuery {
    pub fn pagination(&self) -> crate::generated::types::pagination::PaginationParams {
        crate::generated::types::pagination::PaginationParams {
            page: self.page,
            per_page: self.per_page,
        }
    }
}
