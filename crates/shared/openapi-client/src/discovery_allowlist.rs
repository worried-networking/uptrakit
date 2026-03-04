//! Client methods for the discovery plugin allowlist endpoints.

use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::discovery_allowlist::{
    CreateDiscoveryAllowlistEntryRequest, HostDiscoveryAllowlistEntry,
    TenantDiscoveryAllowlistEntry,
};
use uuid::Uuid;

impl UptrakitClient {
    /// List all tenant-wide discovery allowlist entries.
    ///
    /// An empty list means no restrictions — all discovery plugin types will run.
    pub async fn list_discovery_allowlist(&self) -> Result<Vec<TenantDiscoveryAllowlistEntry>> {
        self.get(crate::paths::discovery_allowlist::BASE).await
    }

    /// Add a plugin type to the tenant-wide discovery allowlist.
    ///
    /// Idempotent: returns the existing entry if it already exists (HTTP 201).
    pub async fn add_discovery_allowlist_entry(
        &self,
        req: &CreateDiscoveryAllowlistEntryRequest,
    ) -> Result<TenantDiscoveryAllowlistEntry> {
        self.post_json(crate::paths::discovery_allowlist::BASE, req)
            .await
    }

    /// Remove a tenant-wide discovery allowlist entry.
    pub async fn remove_discovery_allowlist_entry(&self, id: &Uuid) -> Result<()> {
        self.delete(&crate::paths::discovery_allowlist::by_id(id))
            .await
    }

    /// List host-specific discovery allowlist entries.
    ///
    /// An empty list means the host inherits the tenant-wide allowlist.
    pub async fn list_host_discovery_allowlist(
        &self,
        host_id: &Uuid,
    ) -> Result<Vec<HostDiscoveryAllowlistEntry>> {
        self.get(&crate::paths::discovery_allowlist::host_base(host_id))
            .await
    }

    /// Add a plugin type to a host's discovery allowlist.
    ///
    /// Idempotent: returns the existing entry if it already exists (HTTP 201).
    pub async fn add_host_discovery_allowlist_entry(
        &self,
        host_id: &Uuid,
        req: &CreateDiscoveryAllowlistEntryRequest,
    ) -> Result<HostDiscoveryAllowlistEntry> {
        self.post_json(&crate::paths::discovery_allowlist::host_base(host_id), req)
            .await
    }

    /// Remove a host-specific discovery allowlist entry.
    pub async fn remove_host_discovery_allowlist_entry(
        &self,
        host_id: &Uuid,
        entry_id: &Uuid,
    ) -> Result<()> {
        self.delete(&crate::paths::discovery_allowlist::host_entry(
            host_id, entry_id,
        ))
        .await
    }
}
