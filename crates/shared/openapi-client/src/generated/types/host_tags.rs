// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use crate::generated::types::validation::{Validate, ValidationError};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;
/// Full host tag response with host count.
#[derive(Debug, Serialize, Deserialize)]
pub struct HostTagResponse {
    pub id: Uuid,
    pub name: String,
    pub color: String,
    pub description: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    pub host_count: u64,
}
/// Slim tag summary for embedding in host responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostTagSummary {
    pub id: Uuid,
    pub name: String,
    pub color: String,
}
/// Request to create a new host tag.
#[derive(Debug, Serialize, Deserialize)]
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
pub struct ListHostTagsQuery {
    pub page: Option<u64>,
    pub per_page: Option<u64>,
    pub search: Option<String>,
}
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
