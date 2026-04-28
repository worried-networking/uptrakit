use crate::Result;
use crate::UptrakitClient;
use crate::generated::types::batch_actions::{BatchActionRequest, BatchActionResponse};
use crate::generated::types::pagination::{PaginatedResponse, PaginationParams};
use crate::generated::types::software_items::{
    AssignHostsRequest, CreateSoftwareItemRequest, ListSoftwareItemsParams,
    MergeSoftwareItemsExecuteRequest, MergeSoftwareItemsExecuteResponse,
    MergeSoftwareItemsPreviewRequest, MergeSoftwareItemsPreviewResponse,
    SoftwareItemDetailResponse, SoftwareItemResponse, TriggerUpdateRequest, TriggerUpdateResponse,
    TriggerVersionCheckResponse, UpdateHostAssignmentRequest, UpdateSoftwareItemRequest,
};
use uuid::Uuid;

impl UptrakitClient {
    /// List software items with pagination and optional discovery state filter.
    pub async fn list_software_items(
        &self,
        params: &ListSoftwareItemsParams,
    ) -> Result<PaginatedResponse<SoftwareItemResponse>> {
        self.get_with_query(crate::paths::software_items::BASE, params)
            .await
    }

    /// Preview a manual merge of software items.
    pub async fn preview_software_item_merge(
        &self,
        req: &MergeSoftwareItemsPreviewRequest,
    ) -> Result<MergeSoftwareItemsPreviewResponse> {
        self.post_json(crate::paths::software_items::MERGE_PREVIEW, req)
            .await
    }

    /// Execute a manual merge of software items.
    pub async fn execute_software_item_merge(
        &self,
        req: &MergeSoftwareItemsExecuteRequest,
    ) -> Result<MergeSoftwareItemsExecuteResponse> {
        self.post_json(crate::paths::software_items::MERGE_EXECUTE, req)
            .await
    }

    /// Fetch all software items across all pages.
    ///
    /// Automatically iterates through every page at [`MAX_PER_PAGE`] items per
    /// request. Use [`list_software_items`] for manual pagination control.
    ///
    /// [`MAX_PER_PAGE`]: uptrakit_web_api_types::pagination::MAX_PER_PAGE
    /// [`list_software_items`]: Self::list_software_items
    pub async fn list_all_software_items(&self) -> Result<Vec<SoftwareItemResponse>> {
        let base = PaginationParams {
            page: None,
            per_page: None,
        };
        self.fetch_all_pages(crate::paths::software_items::BASE, &base)
            .await
    }

    /// Get a single software item by ID (detailed view with host info).
    pub async fn get_software_item(&self, id: &Uuid) -> Result<SoftwareItemDetailResponse> {
        self.get(&crate::paths::software_items::by_id(id)).await
    }

    /// Create a new software item (catalog entry — name and enabled flag only).
    pub async fn create_software_item(
        &self,
        req: &CreateSoftwareItemRequest,
    ) -> Result<SoftwareItemResponse> {
        self.post_json(crate::paths::software_items::BASE, req)
            .await
    }

    /// Update an existing software item (name and/or enabled flag).
    pub async fn update_software_item(
        &self,
        id: &Uuid,
        req: &UpdateSoftwareItemRequest,
    ) -> Result<SoftwareItemResponse> {
        self.put_json(&crate::paths::software_items::by_id(id), req)
            .await
    }

    /// Delete a software item.
    pub async fn delete_software_item(&self, id: &Uuid) -> Result<()> {
        self.delete(&crate::paths::software_items::by_id(id)).await
    }

    /// Assign hosts to a software item.
    ///
    /// Each host assignment carries its own `plugin_config_id`, `package_identifier`,
    /// and optional `config_override`.
    pub async fn assign_hosts(
        &self,
        id: &Uuid,
        req: &AssignHostsRequest,
    ) -> Result<SoftwareItemDetailResponse> {
        self.post_json(&crate::paths::software_items::hosts(id), req)
            .await
    }

    /// Unassign a host from a software item.
    pub async fn unassign_host(&self, item_id: &Uuid, host_id: &Uuid) -> Result<()> {
        self.delete(&crate::paths::software_items::host(item_id, host_id))
            .await
    }

    /// Unassign a host from a software item and create an autodiscovery ignore rule.
    ///
    /// Equivalent to `DELETE .../hosts/{host_id}?ignore=true`. The ignore rule is created for
    /// the `(plugin_config_id, package_identifier)` pair stored on the host assignment,
    /// preventing re-discovery of that package on any host in the future.
    pub async fn unassign_host_with_ignore(&self, item_id: &Uuid, host_id: &Uuid) -> Result<()> {
        #[derive(serde::Serialize)]
        struct IgnoreQuery {
            ignore: bool,
        }
        self.delete_with_query(
            &crate::paths::software_items::host(item_id, host_id),
            &IgnoreQuery { ignore: true },
        )
        .await
    }

    /// Update the plugin assignment for a specific host–software-item link.
    pub async fn update_host_assignment(
        &self,
        item_id: &Uuid,
        host_id: &Uuid,
        req: &UpdateHostAssignmentRequest,
    ) -> Result<SoftwareItemDetailResponse> {
        self.put_json(&crate::paths::software_items::host(item_id, host_id), req)
            .await
    }

    /// Remove a specific plugin assignment by role and ordinal.
    pub async fn delete_plugin_assignment(
        &self,
        item_id: &Uuid,
        host_id: &Uuid,
        role: &str,
        ordinal: i32,
    ) -> Result<SoftwareItemDetailResponse> {
        self.delete_json(&crate::paths::software_items::host_plugin_assignment(
            item_id, host_id, role, ordinal,
        ))
        .await
    }

    /// Trigger a version check for a software item across all assigned hosts.
    pub async fn check_versions(&self, item_id: &Uuid) -> Result<TriggerVersionCheckResponse> {
        self.post_empty(&crate::paths::software_items::check_versions(item_id))
            .await
    }

    /// Trigger a version check for a software item on a specific host.
    pub async fn check_versions_host(
        &self,
        item_id: &Uuid,
        host_id: &Uuid,
    ) -> Result<TriggerVersionCheckResponse> {
        self.post_empty(&crate::paths::software_items::host_check_versions(
            item_id, host_id,
        ))
        .await
    }

    /// Trigger an update for a software item on a specific host.
    pub async fn trigger_update(
        &self,
        item_id: &Uuid,
        host_id: &Uuid,
        req: &TriggerUpdateRequest,
    ) -> Result<TriggerUpdateResponse> {
        self.post_json(
            &crate::paths::software_items::host_update(item_id, host_id),
            req,
        )
        .await
    }

    /// Perform a batch action on multiple software items.
    ///
    /// Supported actions: `approve`, `delete`.
    pub async fn batch_software_items(
        &self,
        req: &BatchActionRequest,
    ) -> Result<BatchActionResponse> {
        self.post_json(crate::paths::software_items::BATCH, req)
            .await
    }
}

#[cfg(test)]
mod tests {
    use crate::generated::shared_types::PluginRole;
    use crate::generated::types::software_items::{
        AssignHostsRequest, CreateSoftwareItemRequest, HostPluginRoleAssignment,
        HostSoftwareAssignment, ReleaseInfoRequest, TriggerUpdateRequest,
        UpdateHostAssignmentRequest, UpdateSoftwareItemRequest,
    };
    use uuid::Uuid;

    #[test]
    fn create_software_item_request_serialization() {
        let req = CreateSoftwareItemRequest {
            name: "Node.js".to_string(),
            featured: true,
            icon_url: None,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["name"], "Node.js");
        assert_eq!(json["featured"], true);
        // plugin fields must NOT appear in the serialized form
        assert!(json.get("provider_config_id").is_none());
        assert!(json.get("package_identifier").is_none());
    }

    #[test]
    fn update_software_item_request_serialization() {
        use crate::generated::types::software_items::IconUrlPatch;
        let req = UpdateSoftwareItemRequest {
            name: Some("Node.js LTS".to_string()),
            featured: Some(false),
            icon_url: IconUrlPatch::Keep,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["name"], "Node.js LTS");
        assert_eq!(json["featured"], false);
        // plugin fields must NOT appear
        assert!(json.get("package_identifier").is_none());
        assert!(json.get("config_override").is_none());
    }

    #[test]
    fn assign_hosts_request_serialization() {
        let pc_id = Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6").expect("valid uuid");
        let host1 = Uuid::parse_str("11111111-1111-1111-1111-111111111111").expect("valid uuid");
        let host2 = Uuid::parse_str("22222222-2222-2222-2222-222222222222").expect("valid uuid");

        let req = AssignHostsRequest {
            host_assignments: vec![
                HostSoftwareAssignment {
                    host_id: host1,
                    plugins: vec![HostPluginRoleAssignment {
                        role: PluginRole::DetectVersion,
                        ordinal: 0,
                        plugin_config_id: Some(pc_id),
                        plugin_config: None,
                        package_identifier: "nodejs/node".to_string(),
                        config_override: None,
                        execution_site: "auto".to_string(),
                    }],
                },
                HostSoftwareAssignment {
                    host_id: host2,
                    plugins: vec![HostPluginRoleAssignment {
                        role: PluginRole::DetectVersion,
                        ordinal: 0,
                        plugin_config_id: Some(pc_id),
                        plugin_config: None,
                        package_identifier: "nodejs/node".to_string(),
                        config_override: None,
                        execution_site: "auto".to_string(),
                    }],
                },
            ],
        };
        let json = serde_json::to_value(&req).expect("serialize");
        let assignments = json["host_assignments"].as_array().expect("array");
        assert_eq!(assignments.len(), 2);
        assert_eq!(
            assignments[0]["host_id"],
            "11111111-1111-1111-1111-111111111111"
        );
        let plugins = assignments[0]["plugins"].as_array().expect("plugins array");
        assert_eq!(plugins.len(), 1);
        assert_eq!(
            plugins[0]["plugin_config_id"],
            "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"
        );
        assert_eq!(plugins[0]["package_identifier"], "nodejs/node");
        assert_eq!(plugins[0]["role"], "detect_version");
    }

    #[test]
    fn update_host_assignment_request_serialization() {
        let pc_id = Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6").expect("valid uuid");
        use crate::generated::types::software_items::JsonObjectMapPatch;
        let req = UpdateHostAssignmentRequest {
            role: PluginRole::ExecuteUpdate,
            ordinal: 0,
            plugin_config_id: Some(pc_id),
            plugin_config: None,
            plugin_type: None,
            package_identifier: Some("homebrew/cask/firefox".to_string()),
            config_override: JsonObjectMapPatch::Keep,
            execution_site: None,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(
            json["plugin_config_id"],
            "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"
        );
        assert_eq!(json["package_identifier"], "homebrew/cask/firefox");
        assert_eq!(json["role"], "execute_update");
    }

    #[test]
    fn trigger_update_request_without_release_info() {
        let req = TriggerUpdateRequest {
            to_version: "2.0.0".to_string(),
            release_info: None,
            interactive: false,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["to_version"], "2.0.0");
        assert!(json["release_info"].is_null());
    }

    #[test]
    fn trigger_update_request_with_release_info() {
        let req = TriggerUpdateRequest {
            to_version: "2.0.0".to_string(),
            release_info: Some(ReleaseInfoRequest {
                tag: "v2.0.0".to_string(),
                release_url: "https://example.com/releases/v2.0.0".to_string(),
                assets: vec![],
            }),
            interactive: false,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["to_version"], "2.0.0");
        assert_eq!(json["release_info"]["tag"], "v2.0.0");
    }
}
