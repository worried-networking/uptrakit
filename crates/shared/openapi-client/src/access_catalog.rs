use crate::Result;
use crate::UptrakitClient;
use crate::types_impl::access_catalog::AccessCatalogResponse;

impl UptrakitClient {
    /// Fetch the access catalog (actions, role bundles, scope presets).
    pub async fn get_access_catalog(&self) -> Result<AccessCatalogResponse> {
        self.get(crate::paths::access_catalog::BASE).await
    }
}
