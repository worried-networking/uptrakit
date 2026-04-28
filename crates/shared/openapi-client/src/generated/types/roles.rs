// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use crate::generated::types::permissions::Permission;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
/// A role with its assigned permissions.
#[derive(Serialize, Deserialize, Clone)]
pub struct RoleResponse {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub is_built_in: bool,
    pub permissions: Vec<Permission>,
}
