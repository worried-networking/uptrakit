use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uptrakit_shared_types::access::bounds::MAX_GRANT_DESCRIPTION_LEN;
use uuid::Uuid;

use crate::validation::{Validate, ValidationError};

/// A role — global built-in or tenant-scoped custom.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RoleResponse {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub is_built_in: bool,
    /// `null` for the global built-ins; the owning tenant for custom roles.
    pub tenant_id: Option<Uuid>,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, format = DateTime)
    )]
    pub created_at: OffsetDateTime,
}

/// Body for `POST /api/v1/roles`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateRoleRequest {
    /// 1-64 chars: lowercase alphanumeric plus `-`/`_`, starting with a letter
    pub name: String,
    pub description: Option<String>,
}

/// Body for `PUT /api/v1/roles/{id}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateRoleRequest {
    pub name: String,
    pub description: Option<String>,
}

fn validate_role_name(name: &str) -> Result<(), ValidationError> {
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase());
    let rest_ok =
        chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if !first_ok || !rest_ok || name.len() > 64 {
        return Err(ValidationError {
            field: "name",
            message: "must be 1-64 chars: lowercase alphanumeric plus '-'/'_', letter-first"
                .to_string(),
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

impl Validate for CreateRoleRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_role_name(&self.name)?;
        validate_description_field(self.description.as_deref())
    }
}

impl Validate for UpdateRoleRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_role_name(&self.name)?;
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
    fn create_request_validation_name_bounds() {
        let ok = CreateRoleRequest {
            name: "custom-role_1".to_string(),
            description: None,
        };
        assert!(ok.validate().is_ok());

        let empty = CreateRoleRequest {
            name: String::new(),
            ..ok.clone()
        };
        assert!(empty.validate().is_err(), "empty name rejected");

        let starts_with_digit = CreateRoleRequest {
            name: "1role".to_string(),
            ..ok.clone()
        };
        assert!(
            starts_with_digit.validate().is_err(),
            "must start with a letter"
        );

        let uppercase = CreateRoleRequest {
            name: "Role".to_string(),
            ..ok.clone()
        };
        assert!(uppercase.validate().is_err(), "must be lowercase");

        let bad_char = CreateRoleRequest {
            name: "role name".to_string(),
            ..ok.clone()
        };
        assert!(bad_char.validate().is_err(), "space not allowed");

        let too_long = CreateRoleRequest {
            name: "a".repeat(65),
            ..ok
        };
        assert!(too_long.validate().is_err(), "max 64 chars");
    }

    #[test]
    fn update_request_validation_description_bound() {
        let ok = UpdateRoleRequest {
            name: "custom-role".to_string(),
            description: Some("d".repeat(MAX_GRANT_DESCRIPTION_LEN)),
        };
        assert!(ok.validate().is_ok());

        let too_long = UpdateRoleRequest {
            description: Some("d".repeat(MAX_GRANT_DESCRIPTION_LEN + 1)),
            ..ok
        };
        assert!(too_long.validate().is_err());
    }
}
