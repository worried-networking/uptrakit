// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use crate::generated::shared_types::PluginTypeId;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;
/// A tenant-wide discovery allowlist entry.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TenantDiscoveryAllowlistEntry {
    /// Entry UUID.
    pub id: Uuid,
    /// Plugin type string (e.g. `"package_manager_homebrew"`).
    pub plugin_type: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}
/// A host-specific discovery allowlist entry.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HostDiscoveryAllowlistEntry {
    /// Entry UUID.
    pub id: Uuid,
    /// Host UUID this entry applies to.
    pub host_id: Uuid,
    /// Plugin type string (e.g. `"package_manager_apt"`).
    pub plugin_type: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}
/// Request body for creating a tenant-wide or host-specific discovery allowlist entry.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CreateDiscoveryAllowlistEntryRequest {
    /// Plugin type to allow for discovery.
    ///
    /// Must be a known plugin type that has the `DiscoverLocalSoftware` capability.
    /// `Other`/unknown plugin types are rejected.
    pub plugin_type: PluginTypeId,
}
