use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::access_presets::AccessPresetResponse;

impl UptrakitClient {
    /// List all access presets.
    pub async fn list_access_presets(&self) -> Result<Vec<AccessPresetResponse>> {
        self.get(crate::paths::access_presets::BASE).await
    }
}
