use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::validation::{Validate, ValidationError};

/// Full host tag response with host count.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct HostTagResponse {
    pub id: Uuid,
    pub name: String,
    pub color: String,
    pub description: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, format = DateTime)
    )]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, format = DateTime)
    )]
    pub updated_at: OffsetDateTime,
    pub host_count: u64,
}

/// Slim tag summary for embedding in host responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct HostTagSummary {
    pub id: Uuid,
    pub name: String,
    pub color: String,
}

/// Request to create a new host tag.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateHostTagRequest {
    pub name: String,
    pub color: Option<String>,
    pub description: Option<String>,
}

impl Validate for CreateHostTagRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.name.is_empty() || self.name.len() > 100 {
            return Err(ValidationError {
                field: "name",
                message: "must be between 1 and 100 characters".to_string(),
            });
        }
        if let Some(ref desc) = self.description
            && desc.len() > 500
        {
            return Err(ValidationError {
                field: "description",
                message: "must be at most 500 characters".to_string(),
            });
        }
        if let Some(ref color) = self.color {
            validate_hex_color(color)?;
        }
        Ok(())
    }
}

/// Request to update an existing host tag.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateHostTagRequest {
    pub name: Option<String>,
    pub color: Option<String>,
    /// Use `null` to clear, omit to keep.
    pub description: Option<serde_json::Value>,
}

impl Validate for UpdateHostTagRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if let Some(ref name) = self.name
            && (name.is_empty() || name.len() > 100)
        {
            return Err(ValidationError {
                field: "name",
                message: "must be between 1 and 100 characters".to_string(),
            });
        }
        if let Some(ref desc) = self.description
            && !desc.is_null()
        {
            if let Some(s) = desc.as_str() {
                if s.len() > 500 {
                    return Err(ValidationError {
                        field: "description",
                        message: "must be at most 500 characters".to_string(),
                    });
                }
            } else {
                return Err(ValidationError {
                    field: "description",
                    message: "must be a string or null".to_string(),
                });
            }
        }
        if let Some(ref color) = self.color {
            validate_hex_color(color)?;
        }
        Ok(())
    }
}

/// Request to set the full list of tags on a host.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SetHostTagsRequest {
    pub tag_ids: Vec<Uuid>,
}

impl Validate for SetHostTagsRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.tag_ids.len() > 50 {
            return Err(ValidationError {
                field: "tag_ids",
                message: "at most 50 tags can be assigned".to_string(),
            });
        }
        Ok(())
    }
}

/// Query parameters for listing host tags.
#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
pub struct ListHostTagsQuery {
    pub page: Option<u64>,
    pub per_page: Option<u64>,
    pub search: Option<String>,
}

#[expect(
    clippy::string_slice,
    reason = "safe: '#' is ASCII (1 byte), so color[1..] is always on a char boundary; starts_with('#') is checked above"
)]
fn validate_hex_color(color: &str) -> Result<(), ValidationError> {
    if color.len() != 7
        || !color.starts_with('#')
        || !color[1..].chars().all(|c| c.is_ascii_hexdigit())
    {
        return Err(ValidationError {
            field: "color",
            message: "must be a valid hex color (e.g. #3B82F6)".to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — is_ok/is_err provides readable failure messages"
    )]
    use super::*;

    fn sample_uuid() -> Uuid {
        Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6")
            .expect("hard-coded UUID should be valid")
    }

    // ── HostTagResponse ──────────────────────────────────────────────

    #[test]
    fn host_tag_response_round_trip() {
        let resp = HostTagResponse {
            id: sample_uuid(),
            name: "production".to_string(),
            color: "#3B82F6".to_string(),
            description: Some("Production hosts".to_string()),
            created_at: time::macros::datetime!(2025-01-01 0:00:00 UTC),
            updated_at: time::macros::datetime!(2025-01-01 0:00:00 UTC),
            host_count: 5,
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: HostTagResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.id, sample_uuid());
        assert_eq!(deserialized.name, "production");
        assert_eq!(deserialized.color, "#3B82F6");
        assert_eq!(
            deserialized.description.as_deref(),
            Some("Production hosts")
        );
        assert_eq!(deserialized.host_count, 5);
    }

    // ── HostTagSummary ───────────────────────────────────────────────

    #[test]
    fn host_tag_summary_round_trip() {
        let summary = HostTagSummary {
            id: sample_uuid(),
            name: "staging".to_string(),
            color: "#EF4444".to_string(),
        };
        let json = serde_json::to_string(&summary).expect("serialization should succeed");
        let deserialized: HostTagSummary =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.name, "staging");
        assert_eq!(deserialized.color, "#EF4444");
    }

    // ── CreateHostTagRequest ─────────────────────────────────────────

    #[test]
    fn create_host_tag_request_valid() {
        let req = CreateHostTagRequest {
            name: "production".to_string(),
            color: Some("#3B82F6".to_string()),
            description: Some("Prod hosts".to_string()),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn create_host_tag_request_no_color() {
        let req = CreateHostTagRequest {
            name: "test".to_string(),
            color: None,
            description: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn create_host_tag_request_empty_name() {
        let req = CreateHostTagRequest {
            name: String::new(),
            color: None,
            description: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn create_host_tag_request_name_too_long() {
        let req = CreateHostTagRequest {
            name: "x".repeat(101),
            color: None,
            description: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn create_host_tag_request_description_too_long() {
        let req = CreateHostTagRequest {
            name: "ok".to_string(),
            color: None,
            description: Some("x".repeat(501)),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn create_host_tag_request_invalid_color() {
        let req = CreateHostTagRequest {
            name: "ok".to_string(),
            color: Some("red".to_string()),
            description: None,
        };
        assert!(req.validate().is_err());
    }

    // ── UpdateHostTagRequest ─────────────────────────────────────────

    #[test]
    fn update_host_tag_request_round_trip() {
        let req = UpdateHostTagRequest {
            name: Some("new-name".to_string()),
            color: Some("#10B981".to_string()),
            description: Some(serde_json::Value::String("updated".to_string())),
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let deserialized: UpdateHostTagRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.name.as_deref(), Some("new-name"));
    }

    #[test]
    fn update_host_tag_request_clear_description() {
        let req = UpdateHostTagRequest {
            name: None,
            color: None,
            description: Some(serde_json::Value::Null),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn update_host_tag_request_description_non_string() {
        let req = UpdateHostTagRequest {
            name: None,
            color: None,
            description: Some(serde_json::json!(42)),
        };
        assert!(req.validate().is_err());
    }

    // ── SetHostTagsRequest ───────────────────────────────────────────

    #[test]
    fn set_host_tags_request_valid() {
        let req = SetHostTagsRequest {
            tag_ids: vec![sample_uuid()],
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn set_host_tags_request_empty() {
        let req = SetHostTagsRequest { tag_ids: vec![] };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn set_host_tags_request_too_many() {
        let req = SetHostTagsRequest {
            tag_ids: (0..51).map(|_| Uuid::new_v4()).collect(),
        };
        assert!(req.validate().is_err());
    }

    // ── ListHostTagsQuery ────────────────────────────────────────────

    #[test]
    fn list_host_tags_query_from_empty_json() {
        let json = r#"{}"#;
        let query: ListHostTagsQuery =
            serde_json::from_str(json).expect("deserialization should succeed");
        assert!(query.page.is_none());
        assert!(query.search.is_none());
    }

    // ── validate_hex_color ───────────────────────────────────────────

    #[test]
    fn valid_hex_colors() {
        assert!(validate_hex_color("#3B82F6").is_ok());
        assert!(validate_hex_color("#000000").is_ok());
        assert!(validate_hex_color("#FFFFFF").is_ok());
        assert!(validate_hex_color("#abcdef").is_ok());
    }

    #[test]
    fn invalid_hex_colors() {
        assert!(validate_hex_color("red").is_err());
        assert!(validate_hex_color("#GGG").is_err());
        assert!(validate_hex_color("3B82F6").is_err());
        assert!(validate_hex_color("#3B82F").is_err());
        assert!(validate_hex_color("#3B82F6F").is_err());
    }
}
