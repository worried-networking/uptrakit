use crate::generated::types::permissions::Permission;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
/// A role with its assigned permissions.
#[derive(Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RoleResponse {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub is_built_in: bool,
    pub permissions: Vec<Permission>,
}
