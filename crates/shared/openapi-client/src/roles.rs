use crate::Result;
use crate::UptrakitClient;
use crate::types_impl::roles::RoleResponse;
use uuid::Uuid;

impl UptrakitClient {
    /// List all roles with their permissions.
    pub async fn list_roles(&self) -> Result<Vec<RoleResponse>> {
        self.get(crate::paths::roles::BASE).await
    }

    /// Get a single role by ID with its permissions.
    pub async fn get_role(&self, id: &Uuid) -> Result<RoleResponse> {
        self.get(&crate::paths::roles::by_id(id)).await
    }
}
