use crate::Result;
use crate::UptrakitClient;
use crate::types_impl::permissions::Permission;

impl UptrakitClient {
    /// List all available permissions.
    pub async fn list_permissions(&self) -> Result<Vec<Permission>> {
        self.get(crate::paths::permissions::BASE).await
    }
}
