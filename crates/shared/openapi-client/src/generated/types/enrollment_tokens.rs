use crate::generated::shared_types::SecretString;
use crate::generated::types::validation::{Validate, ValidationError};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;
/// Request to create a new enrollment token.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateEnrollmentTokenRequest {
    /// Human-readable name for this token.
    pub name: String,
    /// Restrict the token to services with at least one of these capabilities.
    /// `None` means wildcard (any service type).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_capabilities: Option<Vec<String>>,
    /// Maximum number of enrollments allowed with this token.
    /// `None` means unlimited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_uses: Option<u32>,
    /// Token lifetime in seconds from now. `None` means never expires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in_seconds: Option<u64>,
}
impl Validate for CreateEnrollmentTokenRequest {
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
/// Response returned when a new enrollment token is created.
/// The plaintext `token` is only available in this response.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EnrollmentTokenCreatedResponse {
    pub id: Uuid,
    /// The plaintext token value. Only returned once at creation time.
    pub token: SecretString,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_capabilities: Option<Vec<String>>,
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
/// Response for an enrollment token (without the plaintext token).
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EnrollmentTokenResponse {
    pub id: Uuid,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_capabilities: Option<Vec<String>>,
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
/// Query parameters for listing enrollment tokens.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
pub struct ListEnrollmentTokensQuery {
    /// Page number (1-indexed). Defaults to 1.
    pub page: Option<u64>,
    /// Items per page. Defaults to 20, max 1000.
    pub per_page: Option<u64>,
}
impl ListEnrollmentTokensQuery {
    pub fn pagination(&self) -> crate::generated::types::pagination::PaginationParams {
        crate::generated::types::pagination::PaginationParams {
            page: self.page,
            per_page: self.per_page,
        }
    }
}
/// Summary of enrollment tokens for the combined settings response.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EnrollmentTokensSummary {
    /// Number of active (non-revoked, non-expired, uses remaining) tokens.
    pub active_count: u32,
}
