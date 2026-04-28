// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use crate::generated::types::validation::{Validate, ValidationError};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;
/// Response for trigger-discovery endpoints.
#[derive(Serialize, Deserialize)]
pub struct TriggerDiscoveryResponse {
    /// Number of plugin assignments queued for discovery.
    pub plugins_queued: u32,
    /// Human-readable summary message.
    pub message: String,
}
/// A single entry in the software ignore list.
#[derive(Serialize, Deserialize)]
pub struct SoftwareIgnoreResponse {
    /// Ignore rule UUID.
    pub id: Uuid,
    /// Software item display name to suppress.
    pub name: String,
    /// When set, this ignore rule applies only to the given host.
    /// `None` means the rule is tenant-wide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_id: Option<Uuid>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}
/// Request body for creating a software ignore rule.
#[derive(Serialize, Deserialize)]
pub struct CreateSoftwareIgnoreRequest {
    /// Software item display name to permanently suppress from future discoveries.
    pub name: String,
    /// Optionally scope the ignore rule to a specific host.
    /// `None` means the rule applies tenant-wide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_id: Option<Uuid>,
}
impl Validate for CreateSoftwareIgnoreRequest {
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
