use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::batch_actions::{BatchActionRequest, BatchActionResponse};
use uptrakit_web_api_types::host_packages::{
    CreateHostPackageIgnoreRequest, HostPackageDetailResponse, HostPackageIgnoreResponse,
    HostPackageResponse, ListHostPackagesParams, PromoteHostPackageRequest,
    UpdateHostPackageRequest,
};
use uptrakit_web_api_types::pagination::PaginatedResponse;
use uptrakit_web_api_types::software_items::SoftwareItemDetailResponse;
use uuid::Uuid;

impl UptrakitClient {
    /// List host packages with pagination and filtering.
    pub async fn list_host_packages(
        &self,
        host_id: &Uuid,
        params: &ListHostPackagesParams,
    ) -> Result<PaginatedResponse<HostPackageResponse>> {
        self.get_with_query(&crate::paths::host_packages::base(host_id), params)
            .await
    }

    /// Get a single host package by ID (with update history).
    pub async fn get_host_package(
        &self,
        host_id: &Uuid,
        package_id: &Uuid,
    ) -> Result<HostPackageDetailResponse> {
        self.get(&crate::paths::host_packages::by_id(host_id, package_id))
            .await
    }

    /// Update a host package (e.g. enable/disable).
    pub async fn update_host_package(
        &self,
        host_id: &Uuid,
        package_id: &Uuid,
        req: &UpdateHostPackageRequest,
    ) -> Result<HostPackageResponse> {
        self.put_json(
            &crate::paths::host_packages::by_id(host_id, package_id),
            req,
        )
        .await
    }

    /// Delete (soft-deactivate) a host package. Optionally create an ignore rule.
    pub async fn delete_host_package(
        &self,
        host_id: &Uuid,
        package_id: &Uuid,
        ignore: bool,
    ) -> Result<()> {
        if ignore {
            #[derive(serde::Serialize)]
            struct IgnoreParam {
                ignore: bool,
            }
            self.delete_with_query(
                &crate::paths::host_packages::by_id(host_id, package_id),
                &IgnoreParam { ignore: true },
            )
            .await
        } else {
            self.delete(&crate::paths::host_packages::by_id(host_id, package_id))
                .await
        }
    }

    /// List package ignore rules for a host.
    pub async fn list_host_package_ignores(
        &self,
        host_id: &Uuid,
    ) -> Result<Vec<HostPackageIgnoreResponse>> {
        self.get(&crate::paths::host_packages::ignores(host_id))
            .await
    }

    /// Create a package ignore rule for a host.
    pub async fn create_host_package_ignore(
        &self,
        host_id: &Uuid,
        req: &CreateHostPackageIgnoreRequest,
    ) -> Result<HostPackageIgnoreResponse> {
        self.post_json(&crate::paths::host_packages::ignores(host_id), req)
            .await
    }

    /// Remove a package ignore rule.
    pub async fn delete_host_package_ignore(&self, host_id: &Uuid, ignore_id: &Uuid) -> Result<()> {
        self.delete(&crate::paths::host_packages::ignore_by_id(
            host_id, ignore_id,
        ))
        .await
    }

    /// Promote a host package to a tracked software item.
    ///
    /// Creates a software item alongside the host package (additive). If the host is
    /// already assigned to a matching software item, the existing item is returned
    /// (idempotent).
    pub async fn promote_host_package(
        &self,
        host_id: &Uuid,
        package_id: &Uuid,
        req: &PromoteHostPackageRequest,
    ) -> Result<SoftwareItemDetailResponse> {
        self.post_json(
            &crate::paths::host_packages::promote(host_id, package_id),
            req,
        )
        .await
    }

    /// Perform a batch action on multiple host packages.
    ///
    /// Supported actions: `delete`, `enable`, `disable`.
    pub async fn batch_host_packages(
        &self,
        host_id: &Uuid,
        req: &BatchActionRequest,
    ) -> Result<BatchActionResponse> {
        self.post_json(&crate::paths::host_packages::batch(host_id), req)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_host_packages_params_serialization() {
        let params = ListHostPackagesParams {
            page: Some(1),
            per_page: Some(20),
            enabled: Some(true),
            has_update: None,
            category: None,
            search: None,
        };
        let qs = serde_urlencoded::to_string(&params).expect("serialize");
        assert!(qs.contains("page=1"));
        assert!(qs.contains("per_page=20"));
        assert!(qs.contains("enabled=true"));
    }

    #[test]
    fn list_host_packages_params_skips_none() {
        let params = ListHostPackagesParams {
            page: None,
            per_page: None,
            enabled: None,
            has_update: None,
            category: None,
            search: None,
        };
        let qs = serde_urlencoded::to_string(&params).expect("serialize");
        assert!(qs.is_empty());
    }

    #[test]
    fn create_host_package_ignore_serialization() {
        let req = CreateHostPackageIgnoreRequest {
            plugin_config_id: uuid::Uuid::nil(),
            package_identifier: "nginx".to_string(),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["package_identifier"], "nginx");
    }

    #[test]
    fn update_host_package_request_serialization() {
        let req = UpdateHostPackageRequest { enabled: false };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["enabled"], false);
    }

    #[test]
    fn host_packages_path_construction() {
        let host_id = uuid::Uuid::nil();
        let pkg_id = uuid::Uuid::nil();
        assert_eq!(
            crate::paths::host_packages::base(&host_id),
            format!("/api/v1/hosts/{host_id}/packages")
        );
        assert_eq!(
            crate::paths::host_packages::by_id(&host_id, &pkg_id),
            format!("/api/v1/hosts/{host_id}/packages/{pkg_id}")
        );
        assert_eq!(
            crate::paths::host_packages::promote(&host_id, &pkg_id),
            format!("/api/v1/hosts/{host_id}/packages/{pkg_id}/promote")
        );
        assert_eq!(
            crate::paths::host_packages::ignores(&host_id),
            format!("/api/v1/hosts/{host_id}/package-ignores")
        );
        assert_eq!(
            crate::paths::host_packages::ignore_by_id(&host_id, &pkg_id),
            format!("/api/v1/hosts/{host_id}/package-ignores/{pkg_id}")
        );
    }

    #[test]
    fn promote_request_serialization() {
        let req = PromoteHostPackageRequest {
            name: Some("My App".to_string()),
            software_item_id: None,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["name"], "My App");
        assert!(json.get("software_item_id").is_none() || json["software_item_id"].is_null());
    }
}
