use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uptrakit_shared_types::SecretString;
use uuid::Uuid;

use crate::validation::{Validate, ValidationError};

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateApiTokenResponse {
    pub id: Uuid,
    pub token: SecretString,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, format = DateTime)
    )]
    pub created_at: OffsetDateTime,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ApiTokenResponse {
    pub id: Uuid,
    pub name: String,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, format = DateTime)
    )]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = Option<String>, format = DateTime)
    )]
    pub last_used_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = Option<String>, format = DateTime)
    )]
    pub revoked_at: Option<OffsetDateTime>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ApiTokenListResponse {
    pub tokens: Vec<ApiTokenResponse>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn sample_uuid() -> Uuid {
        Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6")
            .expect("hard-coded UUID should be valid")
    }

    // ── CreateApiTokenRequest ────────────────────────────────────────

    #[test]
    fn create_api_token_request_round_trip() {
        let req = CreateApiTokenRequest {
            name: "my-token".to_string(),
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let deserialized: CreateApiTokenRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.name, "my-token");
    }

    #[test]
    fn create_api_token_request_empty_name() {
        let json = r#"{"name":""}"#;
        let req: CreateApiTokenRequest =
            serde_json::from_str(json).expect("deserialization should succeed");
        assert_eq!(req.name, "");
    }

    // ── Validate ─────────────────────────────────────────────────────

    #[test]
    fn validate_rejects_empty_name() {
        let req = CreateApiTokenRequest {
            name: "".to_string(),
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "name");
    }

    #[test]
    fn validate_rejects_whitespace_only_name() {
        let req = CreateApiTokenRequest {
            name: "   ".to_string(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_accepts_non_empty_name() {
        let req = CreateApiTokenRequest {
            name: "my-token".to_string(),
        };
        assert!(req.validate().is_ok());
    }

    // ── CreateApiTokenResponse ───────────────────────────────────────

    #[test]
    fn create_api_token_response_round_trip() {
        let resp = CreateApiTokenResponse {
            id: sample_uuid(),
            token: SecretString::new("secret-token-value".to_string()),
            created_at: datetime!(2025-01-01 0:00:00 UTC),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: CreateApiTokenResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.id, sample_uuid());
        assert_eq!(deserialized.token.expose_secret(), "secret-token-value");
        // Verify the timestamp serializes as an RFC3339 string
        let json_value =
            serde_json::to_value(&resp).expect("serialization to Value should succeed");
        assert_eq!(
            json_value.get("created_at").and_then(|v| v.as_str()),
            Some("2025-01-01T00:00:00Z")
        );
    }

    #[test]
    fn create_api_token_response_secret_string_serializes_plaintext() {
        let resp = CreateApiTokenResponse {
            id: sample_uuid(),
            token: SecretString::new("plaintext-token".to_string()),
            created_at: datetime!(2025-06-15 12:00:00 UTC),
        };
        let json_value =
            serde_json::to_value(&resp).expect("serialization to Value should succeed");
        let token_field = json_value
            .get("token")
            .expect("token field should be present");
        assert_eq!(
            token_field.as_str(),
            Some("plaintext-token"),
            "SecretString should serialize as the plaintext value"
        );
    }

    // ── ApiTokenResponse ─────────────────────────────────────────────

    #[test]
    fn api_token_response_round_trip_all_fields() {
        let resp = ApiTokenResponse {
            id: sample_uuid(),
            name: "deploy-key".to_string(),
            created_at: datetime!(2025-01-01 0:00:00 UTC),
            last_used_at: Some(datetime!(2025-06-01 10:00:00 UTC)),
            revoked_at: Some(datetime!(2025-07-01 10:00:00 UTC)),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: ApiTokenResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.id, sample_uuid());
        assert_eq!(deserialized.name, "deploy-key");
        assert!(deserialized.last_used_at.is_some());
        assert!(deserialized.revoked_at.is_some());
    }

    #[test]
    fn api_token_response_round_trip_none_fields() {
        let resp = ApiTokenResponse {
            id: sample_uuid(),
            name: "ci-token".to_string(),
            created_at: datetime!(2025-01-01 0:00:00 UTC),
            last_used_at: None,
            revoked_at: None,
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: ApiTokenResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(deserialized.last_used_at.is_none());
        assert!(deserialized.revoked_at.is_none());
    }

    #[test]
    fn api_token_response_none_fields_serialize_as_null() {
        let resp = ApiTokenResponse {
            id: sample_uuid(),
            name: "test".to_string(),
            created_at: datetime!(2025-01-01 0:00:00 UTC),
            last_used_at: None,
            revoked_at: None,
        };
        let json_value =
            serde_json::to_value(&resp).expect("serialization to Value should succeed");
        let obj = json_value
            .as_object()
            .expect("top-level value should be an object");
        assert!(
            obj.get("last_used_at")
                .expect("last_used_at should be present")
                .is_null()
        );
        assert!(
            obj.get("revoked_at")
                .expect("revoked_at should be present")
                .is_null()
        );
    }

    // ── ApiTokenListResponse ─────────────────────────────────────────

    #[test]
    fn api_token_list_response_round_trip() {
        let resp = ApiTokenListResponse {
            tokens: vec![
                ApiTokenResponse {
                    id: sample_uuid(),
                    name: "token-1".to_string(),
                    created_at: datetime!(2025-01-01 0:00:00 UTC),
                    last_used_at: None,
                    revoked_at: None,
                },
                ApiTokenResponse {
                    id: Uuid::parse_str("b1b2b3b4-c1c2-d1d2-e1e2-f1f2f3f4f5f6")
                        .expect("hard-coded UUID should be valid"),
                    name: "token-2".to_string(),
                    created_at: datetime!(2025-02-01 0:00:00 UTC),
                    last_used_at: Some(datetime!(2025-03-01 0:00:00 UTC)),
                    revoked_at: None,
                },
            ],
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: ApiTokenListResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.tokens.len(), 2);
        assert_eq!(deserialized.tokens[0].name, "token-1");
        assert_eq!(deserialized.tokens[1].name, "token-2");
    }

    #[test]
    fn api_token_list_response_empty_list() {
        let resp = ApiTokenListResponse { tokens: vec![] };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: ApiTokenListResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(deserialized.tokens.is_empty());
    }
}
