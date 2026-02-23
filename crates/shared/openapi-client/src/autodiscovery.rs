//! Client methods for autodiscovery endpoints.

use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::autodiscovery::{
    AutodiscoveryIgnoreResponse, CreateAutodiscoveryIgnoreRequest, DiscardDiscoveredResponse,
    TriggerDiscoveryResponse,
};
use uptrakit_web_api_types::pagination::PaginatedResponse;
use uptrakit_web_api_types::software_items::SoftwareItemResponse;
use uuid::Uuid;

/// Query parameters for listing autodiscovery ignore rules.
#[derive(serde::Serialize)]
pub struct ListIgnoresParams<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_page: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_config_id: Option<&'a Uuid>,
}

/// Query parameters for `DELETE /api/v1/hosts/{id}/discovered`.
#[derive(serde::Serialize)]
struct DiscardDiscoveredQuery<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_config_id: Option<&'a Uuid>,
}

impl UptrakitClient {
    /// Approve a pending discovered software item.
    ///
    /// Sets `discovery_state = approved` and `enabled = true`.
    pub async fn approve_software_item(&self, id: &Uuid) -> Result<SoftwareItemResponse> {
        self.post_empty(&crate::paths::software_items::approve(id))
            .await
    }

    /// Trigger autodiscovery on a specific host.
    pub async fn discover_host(&self, host_id: &Uuid) -> Result<TriggerDiscoveryResponse> {
        self.post_empty(&crate::paths::hosts::discover(host_id))
            .await
    }

    /// Trigger autodiscovery for a specific provider config across all agents.
    pub async fn discover_provider_config(
        &self,
        id: &Uuid,
    ) -> Result<TriggerDiscoveryResponse> {
        self.post_empty(&crate::paths::provider_configs::discover(id))
            .await
    }

    /// Bulk-discard all pending discovered software items for a host.
    ///
    /// Optionally filter by provider config. No ignore rules are created.
    pub async fn discard_host_discovered(
        &self,
        host_id: &Uuid,
        provider_config_id: Option<&Uuid>,
    ) -> Result<DiscardDiscoveredResponse> {
        let query = DiscardDiscoveredQuery { provider_config_id };
        self.delete_with_query_json(&crate::paths::hosts::discovered(host_id), &query)
            .await
    }

    /// Bulk-discard all pending discovered software items for a provider config.
    ///
    /// No ignore rules are created.
    pub async fn discard_provider_config_discovered(
        &self,
        id: &Uuid,
    ) -> Result<DiscardDiscoveredResponse> {
        self.delete_json(&crate::paths::provider_configs::discovered(id))
            .await
    }

    /// List autodiscovery ignore rules for this tenant.
    pub async fn list_autodiscovery_ignores(
        &self,
        params: &ListIgnoresParams<'_>,
    ) -> Result<PaginatedResponse<AutodiscoveryIgnoreResponse>> {
        self.get_with_query(crate::paths::autodiscovery::IGNORES, params)
            .await
    }

    /// Create an autodiscovery ignore rule (idempotent).
    pub async fn create_autodiscovery_ignore(
        &self,
        req: &CreateAutodiscoveryIgnoreRequest,
    ) -> Result<AutodiscoveryIgnoreResponse> {
        self.post_json(crate::paths::autodiscovery::IGNORES, req)
            .await
    }

    /// Delete an autodiscovery ignore rule by ID.
    pub async fn delete_autodiscovery_ignore(&self, id: &Uuid) -> Result<()> {
        self.delete(&crate::paths::autodiscovery::ignore_by_id(id))
            .await
    }
}
