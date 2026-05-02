use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uptrakit_shared_types::SecretString;
use uuid::Uuid;

use crate::validation::{Validate, ValidationError};

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
    pub fn pagination(&self) -> crate::pagination::PaginationParams {
        crate::pagination::PaginationParams {
            page: self.page,
            per_page: self.per_page,
        }
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — is_ok/is_err provides readable failure messages"
    )]
    use super::*;
    use time::macros::datetime;

    fn valid_request() -> CreateSystemEnrollmentTokenRequest {
        CreateSystemEnrollmentTokenRequest {
            name: "MQTT Bridge Token".to_string(),
            max_uses: None,
            expires_in_seconds: None,
        }
    }

    #[test]
    fn validate_accepts_valid_request() {
        assert!(valid_request().validate().is_ok());
    }

    #[test]
    fn validate_rejects_empty_name() {
        let mut req = valid_request();
        req.name = "".to_string();
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "name");
    }

    #[test]
    fn validate_rejects_whitespace_name() {
        let mut req = valid_request();
        req.name = "  ".to_string();
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_rejects_zero_max_uses() {
        let mut req = valid_request();
        req.max_uses = Some(0);
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "max_uses");
    }

    #[test]
    fn validate_accepts_positive_max_uses() {
        let mut req = valid_request();
        req.max_uses = Some(1);
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_rejects_zero_expires_in_seconds() {
        let mut req = valid_request();
        req.expires_in_seconds = Some(0);
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "expires_in_seconds");
    }

    #[test]
    fn validate_accepts_positive_expires_in_seconds() {
        let mut req = valid_request();
        req.expires_in_seconds = Some(3600);
        assert!(req.validate().is_ok());
    }

    #[test]
    fn response_round_trip() {
        let resp = SystemEnrollmentTokenResponse {
            id: Uuid::nil(),
            name: "test".into(),
            max_uses: Some(5),
            current_uses: 2,
            expires_at: Some(datetime!(2026-12-31 23:59:59 UTC)),
            created_at: datetime!(2026-01-01 0:00:00 UTC),
            revoked_at: None,
            created_by_user_id: None,
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let de: SystemEnrollmentTokenResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.name, "test");
        assert_eq!(de.max_uses, Some(5));
        assert_eq!(de.current_uses, 2);
        assert!(de.expires_at.is_some());
        assert!(de.revoked_at.is_none());
    }

    #[test]
    fn list_query_pagination() {
        let query = ListSystemEnrollmentTokensQuery {
            page: Some(2),
            per_page: Some(10),
        };
        let params = query.pagination();
        assert_eq!(params.page, Some(2));
        assert_eq!(params.per_page, Some(10));
    }

    #[test]
    fn create_request_round_trip() {
        let req = CreateSystemEnrollmentTokenRequest {
            name: "MQTT Bridge".to_string(),
            max_uses: Some(10),
            expires_in_seconds: Some(3600),
        };
        let json = serde_json::to_string(&req).expect("serialize");
        let de: CreateSystemEnrollmentTokenRequest =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de.name, "MQTT Bridge");
        assert_eq!(de.max_uses, Some(10));
        assert_eq!(de.expires_in_seconds, Some(3600));
    }
}
