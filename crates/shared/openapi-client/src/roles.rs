use crate::Result;
use crate::UptrakitClient;
use crate::types_impl::roles::{CreateRoleRequest, RoleResponse, UpdateRoleRequest};
use uuid::Uuid;

impl UptrakitClient {
    /// List roles for the active tenant plus the global built-ins.
    pub async fn list_roles(&self) -> Result<Vec<RoleResponse>> {
        self.get(crate::paths::roles::BASE).await
    }

    /// Get a single role by ID.
    pub async fn get_role(&self, id: &Uuid) -> Result<RoleResponse> {
        self.get(&crate::paths::roles::by_id(id)).await
    }

    /// Create a tenant-scoped custom role.
    pub async fn create_role(&self, req: &CreateRoleRequest) -> Result<RoleResponse> {
        self.post_json(crate::paths::roles::BASE, req).await
    }

    /// Rename/re-describe an own-tenant custom role.
    pub async fn update_role(&self, id: &Uuid, req: &UpdateRoleRequest) -> Result<RoleResponse> {
        self.put_json(&crate::paths::roles::by_id(id), req).await
    }

    /// Delete an own-tenant custom role.
    pub async fn delete_role(&self, id: &Uuid) -> Result<()> {
        self.delete(&crate::paths::roles::by_id(id)).await
    }
}
