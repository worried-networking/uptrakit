use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// Response for trigger-discovery endpoints.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TriggerDiscoveryResponse {
    /// Number of provider assignments queued for discovery.
    pub providers_queued: u32,
    /// Human-readable summary message.
    pub message: String,
}

/// Response for bulk-discard (delete all pending discovered items) endpoints.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DiscardDiscoveredResponse {
    /// Number of pending items soft-deleted.
    pub discarded_count: u32,
}

/// A single entry in the autodiscovery ignore list.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AutodiscoveryIgnoreResponse {
    /// Ignore rule UUID.
    pub id: Uuid,
    /// Provider config UUID this rule applies to.
    pub provider_config_id: Uuid,
    /// Display name of the referenced provider config.
    pub provider_config_name: String,
    /// Provider type string (e.g. `"homebrew"`).
    pub provider_type: String,
    /// Package identifier to suppress.
    pub package_identifier: String,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
}

/// Request body for creating an autodiscovery ignore rule.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateAutodiscoveryIgnoreRequest {
    /// Provider config UUID the rule applies to.
    pub provider_config_id: Uuid,
    /// Package identifier to permanently suppress from future discoveries.
    pub package_identifier: String,
}
