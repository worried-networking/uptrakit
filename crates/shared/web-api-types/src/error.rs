use serde::{Deserialize, Serialize};

/// Standard error response returned by all API endpoints.
///
/// All error responses from the API are JSON objects with an `error` field
/// containing a human-readable message, and an optional `code` field for
/// machine-readable error classification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ErrorResponse {
    /// Human-readable error message.
    pub error: String,
    /// Optional machine-readable error code for programmatic handling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}
