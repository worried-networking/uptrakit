use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uptrakit_shared_types::PluginTypeId;
use uuid::Uuid;

/// A tenant-wide discovery allowlist entry.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TenantDiscoveryAllowlistEntry {
    /// Entry UUID.
    pub id: Uuid,
    /// Plugin type string (e.g. `"package_manager_homebrew"`).
    pub plugin_type: String,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
}

/// A host-specific discovery allowlist entry.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct HostDiscoveryAllowlistEntry {
    /// Entry UUID.
    pub id: Uuid,
    /// Host UUID this entry applies to.
    pub host_id: Uuid,
    /// Plugin type string (e.g. `"package_manager_apt"`).
    pub plugin_type: String,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
}

/// Request body for creating a tenant-wide or host-specific discovery allowlist entry.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateDiscoveryAllowlistEntryRequest {
    /// Plugin type to allow for discovery.
    ///
    /// Must be a known plugin type that has the `DiscoverLocalSoftware` capability.
    /// `Other`/unknown plugin types are rejected.
    pub plugin_type: PluginTypeId,
}
