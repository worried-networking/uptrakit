use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::system_alerts::SystemAlertsResponse;

impl UptrakitClient {
    /// Get active system alerts.
    pub async fn get_system_alerts(&self) -> Result<SystemAlertsResponse> {
        self.get("/api/v1/system/alerts").await
    }
}
