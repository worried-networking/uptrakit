//! Request/response types for `/api/v1/access/grants` (M1.6a).

use serde::{Deserialize, Serialize};
use uptrakit_shared_types::access::Selector;
use uptrakit_shared_types::access::bounds::{
    MAX_GRANT_DESCRIPTION_LEN, MAX_PATTERN_LEN, MAX_PATTERNS_PER_GRANT,
};
use uuid::Uuid;

use crate::validation::{Validate, ValidationError};

/// Grant subject discriminator.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum GrantSubjectTypeParam {
    /// A user-subject grant.
    User,
    /// A role-subject grant (always a global row).
    Role,
}

/// Body for `POST /api/v1/access/grants`.
///
/// Subject and tenant encoding are fixed at creation: user-subject
/// tenant-plane grants are stored under the caller's active tenant;
/// system-plane and role-subject grants are global rows. Re-subject or
/// re-scope is delete + create.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateAccessGrantRequest {
    pub subject_type: GrantSubjectTypeParam,
    pub subject_id: Uuid,
    /// Action patterns in string form (`"hosts:read"`, `"settings.*:manage"`).
    pub patterns: Vec<String>,
    /// Defaults to `All`; non-`All` selectors validate fully on write but
    /// are rejected until M2.3 enforcement ships.
    #[serde(default = "default_selector")]
    pub selector: Selector,
    pub description: Option<String>,
}

/// Body for `PUT /api/v1/access/grants/{id}` (patterns/selector/description
/// only — subject and tenant encoding are immutable).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateAccessGrantRequest {
    pub patterns: Vec<String>,
    #[serde(default = "default_selector")]
    pub selector: Selector,
    pub description: Option<String>,
}

/// A stored grant row.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AccessGrantResponse {
    pub id: Uuid,
    /// `null` for global rows (system-plane and role-subject grants).
    pub tenant_id: Option<Uuid>,
    pub subject_type: GrantSubjectTypeParam,
    pub subject_id: Uuid,
    pub patterns: Vec<String>,
    pub selector: Selector,
    pub description: Option<String>,
}

/// Query parameters for `GET /api/v1/access/grants`. `subject_type` and
/// `subject_id` must be supplied together or not at all.
#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
pub struct ListAccessGrantsQuery {
    /// Filter to one subject (requires `subject_id`).
    pub subject_type: Option<GrantSubjectTypeParam>,
    /// Filter to one subject (requires `subject_type`).
    pub subject_id: Option<Uuid>,
}

fn default_selector() -> Selector {
    Selector::All
}

fn validate_patterns_field(patterns: &[String]) -> Result<(), ValidationError> {
    if patterns.is_empty() || patterns.len() > MAX_PATTERNS_PER_GRANT {
        return Err(ValidationError {
            field: "patterns",
            message: format!("must contain between 1 and {MAX_PATTERNS_PER_GRANT} patterns"),
        });
    }
    if patterns
        .iter()
        .any(|p| p.is_empty() || p.len() > MAX_PATTERN_LEN)
    {
        return Err(ValidationError {
            field: "patterns",
            message: format!("each pattern must be between 1 and {MAX_PATTERN_LEN} bytes"),
        });
    }
    Ok(())
}

fn validate_description_field(description: Option<&str>) -> Result<(), ValidationError> {
    if let Some(desc) = description
        && desc.chars().count() > MAX_GRANT_DESCRIPTION_LEN
    {
        return Err(ValidationError {
            field: "description",
            message: format!("must be at most {MAX_GRANT_DESCRIPTION_LEN} characters"),
        });
    }
    Ok(())
}

impl Validate for CreateAccessGrantRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_patterns_field(&self.patterns)?;
        validate_description_field(self.description.as_deref())
    }
}

impl Validate for UpdateAccessGrantRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_patterns_field(&self.patterns)?;
        validate_description_field(self.description.as_deref())
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — is_ok/is_err provides readable failure messages"
    )]
    use super::*;

    #[test]
    fn create_request_validation_bounds() {
        let ok = CreateAccessGrantRequest {
            subject_type: GrantSubjectTypeParam::User,
            subject_id: uuid::Uuid::new_v4(),
            patterns: vec!["hosts:read".to_string()],
            selector: Selector::All,
            description: None,
        };
        assert!(ok.validate().is_ok());

        let empty = CreateAccessGrantRequest {
            patterns: vec![],
            ..ok.clone()
        };
        assert!(empty.validate().is_err(), "empty pattern list rejected");

        let too_many = CreateAccessGrantRequest {
            patterns: vec!["hosts:read".to_string(); MAX_PATTERNS_PER_GRANT + 1],
            ..ok.clone()
        };
        assert!(too_many.validate().is_err());

        let too_long = CreateAccessGrantRequest {
            patterns: vec!["x".repeat(MAX_PATTERN_LEN + 1)],
            ..ok.clone()
        };
        assert!(too_long.validate().is_err());

        let desc = CreateAccessGrantRequest {
            description: Some("d".repeat(MAX_GRANT_DESCRIPTION_LEN + 1)),
            ..ok
        };
        assert!(desc.validate().is_err());
    }
}
