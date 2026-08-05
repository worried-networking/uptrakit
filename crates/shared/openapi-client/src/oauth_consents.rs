//! End-user OAuth authorized-apps management (`/api/oauth/consents`).

use crate::Result;
use crate::UptrakitClient;
use crate::types_impl::oauth::OAuthConsentResponse;
use uuid::Uuid;

impl UptrakitClient {
    /// List the current user's active OAuth consents (newest-granted first).
    pub async fn list_consents(&self) -> Result<Vec<OAuthConsentResponse>> {
        self.get(crate::paths::oauth::CONSENTS).await
    }

    /// Revoke one of the current user's OAuth consents.
    pub async fn revoke_consent(&self, id: &Uuid) -> Result<()> {
        self.delete(&crate::paths::oauth::consent_by_id(id)).await
    }
}
