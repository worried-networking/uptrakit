use crate::Result;
use crate::UptrakitClient;
use crate::types_impl::access_presets::AccessPresetResponse;

impl UptrakitClient {
    /// List all access presets.
    pub async fn list_access_presets(&self) -> Result<Vec<AccessPresetResponse>> {
        self.get(crate::paths::access_presets::BASE).await
    }
}
