// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use crate::generated::types::permissions::Permission;
use crate::generated::types::validation::{Validate, ValidationError};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
/// A user with their assigned roles and resolved permissions.
#[derive(Serialize, Deserialize, Clone)]
pub struct UserWithRolesResponse {
    pub id: Uuid,
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub is_active: bool,
    pub roles: Vec<UserRoleSummary>,
    pub permissions: Vec<Permission>,
}
/// Summary of a role assigned to a user.
#[derive(Serialize, Deserialize, Clone)]
pub struct UserRoleSummary {
    pub id: Uuid,
    pub name: String,
}
/// Request to replace a user's roles.
#[derive(Serialize, Deserialize)]
pub struct UpdateUserRolesRequest {
    /// List of role IDs to assign. Replaces all existing role assignments.
    pub role_ids: Vec<Uuid>,
}
impl Validate for UpdateUserRolesRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.role_ids.is_empty() {
            return Err(ValidationError {
                field: "role_ids",
                message: "at least one role must be assigned".to_string(),
            });
        }
        if self.role_ids.len() > 20 {
            return Err(ValidationError {
                field: "role_ids",
                message: "cannot assign more than 20 roles".to_string(),
            });
        }
        Ok(())
    }
}
/// Request to activate or deactivate a user.
#[derive(Serialize, Deserialize)]
pub struct UpdateUserActiveRequest {
    pub is_active: bool,
}
/// Request to apply an access preset to a user.
#[derive(Serialize, Deserialize)]
pub struct ApplyPresetRequest {
    /// The access preset name to apply.
    pub preset: String,
}
impl Validate for ApplyPresetRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.preset.is_empty() {
            return Err(ValidationError {
                field: "preset",
                message: "preset name must not be empty".to_string(),
            });
        }
        Ok(())
    }
}
