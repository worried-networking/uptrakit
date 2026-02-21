use crate::Result;
use crate::UptrakitClient;

impl UptrakitClient {
    /// Check server health.
    ///
    /// This endpoint does not require authentication. Returns `"ok"` on success.
    pub async fn healthz(&self) -> Result<String> {
        self.get_text_unauth(crate::paths::health::HEALTHZ).await
    }
}
