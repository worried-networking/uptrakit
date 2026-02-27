use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uptrakit_shared_types::SecretString;
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
    pub fn pagination(&self) -> crate::pagination::PaginationParams {
        crate::pagination::PaginationParams {
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

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn create_request_round_trip_all_fields() {
        let req = CreateEnrollmentTokenRequest {
            name: "CI deploy".to_string(),
            allowed_capabilities: Some(vec!["software_discovery".into()]),
            max_uses: Some(10),
            expires_in_seconds: Some(3600),
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let de: CreateEnrollmentTokenRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.name, "CI deploy");
        assert_eq!(
            de.allowed_capabilities.as_deref(),
            Some(["software_discovery".to_string()].as_slice())
        );
        assert_eq!(de.max_uses, Some(10));
        assert_eq!(de.expires_in_seconds, Some(3600));
    }

    #[test]
    fn create_request_round_trip_minimal() {
        let json = r#"{"name":"wildcard"}"#;
        let de: CreateEnrollmentTokenRequest =
            serde_json::from_str(json).expect("deserialization should succeed");
        assert_eq!(de.name, "wildcard");
        assert!(de.allowed_capabilities.is_none());
        assert!(de.max_uses.is_none());
        assert!(de.expires_in_seconds.is_none());
    }

    #[test]
    fn enrollment_token_response_round_trip() {
        let resp = EnrollmentTokenResponse {
            id: Uuid::nil(),
            name: "test".into(),
            allowed_capabilities: None,
            max_uses: Some(5),
            current_uses: 2,
            expires_at: Some(datetime!(2026-12-31 23:59:59 UTC)),
            created_at: datetime!(2026-01-01 0:00:00 UTC),
            revoked_at: None,
            created_by_user_id: None,
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let de: EnrollmentTokenResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.name, "test");
        assert_eq!(de.max_uses, Some(5));
        assert_eq!(de.current_uses, 2);
        assert!(de.expires_at.is_some());
        assert!(de.revoked_at.is_none());
    }

    #[test]
    fn enrollment_tokens_summary_round_trip() {
        let summary = EnrollmentTokensSummary { active_count: 3 };
        let json = serde_json::to_string(&summary).expect("serialization should succeed");
        let de: EnrollmentTokensSummary =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.active_count, 3);
    }

    #[test]
    fn list_query_pagination() {
        let query = ListEnrollmentTokensQuery {
            page: Some(2),
            per_page: Some(10),
        };
        let params = query.pagination();
        assert_eq!(params.page, Some(2));
        assert_eq!(params.per_page, Some(10));
    }
}
