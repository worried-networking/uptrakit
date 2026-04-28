//! Client methods for autodiscovery endpoints.

use crate::Result;
use crate::UptrakitClient;
use crate::generated::types::autodiscovery::{
    CreateSoftwareIgnoreRequest, SoftwareIgnoreResponse, TriggerDiscoveryResponse,
};
use crate::generated::types::batch_actions::{BatchActionRequest, BatchActionResponse};
use crate::generated::types::pagination::PaginatedResponse;
use crate::generated::types::software_items::SoftwareItemResponse;
use uuid::Uuid;

/// Query parameters for listing software ignore rules.
#[derive(serde::Serialize)]
pub struct ListIgnoresParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_page: Option<u64>,
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

    /// Trigger autodiscovery for a specific plugin config across all agents.
    pub async fn discover_plugin_config(&self, id: &Uuid) -> Result<TriggerDiscoveryResponse> {
        self.post_empty(&crate::paths::plugin_configs::discover(id))
            .await
    }

    /// List software ignore rules for this tenant.
    pub async fn list_software_ignores(
        &self,
        params: &ListIgnoresParams,
    ) -> Result<PaginatedResponse<SoftwareIgnoreResponse>> {
        self.get_with_query(crate::paths::autodiscovery::IGNORES, params)
            .await
    }

    /// Create a software ignore rule (idempotent).
    pub async fn create_software_ignore(
        &self,
        req: &CreateSoftwareIgnoreRequest,
    ) -> Result<SoftwareIgnoreResponse> {
        self.post_json(crate::paths::autodiscovery::IGNORES, req)
            .await
    }

    /// Delete a software ignore rule by ID.
    pub async fn delete_software_ignore(&self, id: &Uuid) -> Result<()> {
        self.delete(&crate::paths::autodiscovery::ignore_by_id(id))
            .await
    }

    /// Perform a batch action on multiple software ignore rules.
    ///
    /// Supported actions: `delete`.
    pub async fn batch_software_ignores(
        &self,
        req: &BatchActionRequest,
    ) -> Result<BatchActionResponse> {
        self.post_json(crate::paths::autodiscovery::BATCH, req)
            .await
    }
}
