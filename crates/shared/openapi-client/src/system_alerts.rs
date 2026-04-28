use crate::Result;
use crate::UptrakitClient;
use crate::generated::types::system_alerts::SystemAlertsResponse;

impl UptrakitClient {
    /// Get active system alerts.
    pub async fn get_system_alerts(&self) -> Result<SystemAlertsResponse> {
        self.get(crate::paths::system_alerts::ALERTS).await
    }
}
