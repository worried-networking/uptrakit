use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uptrakit_shared_types::PluginTypeId;
use uuid::Uuid;

use crate::validation::{Validate, ValidationError};

/// A tenant-wide discovery allowlist entry.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TenantDiscoveryAllowlistEntry {
    /// Entry UUID.
    pub id: Uuid,
    /// Plugin type string (e.g. `"package-manager.homebrew"`).
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
    /// Plugin type string (e.g. `"package-manager.apt"`).
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

impl Validate for CreateDiscoveryAllowlistEntryRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        // No format/length invariants beyond field types; capability/existence checks are handler-side.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_discovery_allowlist_entry_request_validate_is_ok() {
        let req = CreateDiscoveryAllowlistEntryRequest {
            plugin_type: PluginTypeId::new("package-manager.apt"),
        };
        req.validate()
            .expect("CreateDiscoveryAllowlistEntryRequest should validate");
    }
}
