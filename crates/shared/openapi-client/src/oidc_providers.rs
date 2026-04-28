use crate::Result;
use crate::UptrakitClient;
use crate::types_impl::oidc_providers::{
    CreateOidcProviderRequest, OidcProviderResponse, UpdateOidcProviderRequest,
};
use uuid::Uuid;

impl UptrakitClient {
    /// Create a new OIDC provider.
    pub async fn create_oidc_provider(
        &self,
        req: &CreateOidcProviderRequest,
    ) -> Result<OidcProviderResponse> {
        self.post_json(crate::paths::oidc_providers::BASE, req)
            .await
    }

    /// List all OIDC providers.
    pub async fn list_oidc_providers(&self) -> Result<Vec<OidcProviderResponse>> {
        self.get(crate::paths::oidc_providers::BASE).await
    }

    /// Get a single OIDC provider by ID.
    pub async fn get_oidc_provider(&self, id: &Uuid) -> Result<OidcProviderResponse> {
        self.get(&crate::paths::oidc_providers::by_id(id)).await
    }

    /// Update an existing OIDC provider.
    pub async fn update_oidc_provider(
        &self,
        id: &Uuid,
        req: &UpdateOidcProviderRequest,
    ) -> Result<OidcProviderResponse> {
        self.put_json(&crate::paths::oidc_providers::by_id(id), req)
            .await
    }

    /// Delete an OIDC provider.
    pub async fn delete_oidc_provider(&self, id: &Uuid) -> Result<()> {
        self.delete(&crate::paths::oidc_providers::by_id(id)).await
    }

    /// Activate an OIDC provider.
    pub async fn activate_oidc_provider(&self, id: &Uuid) -> Result<OidcProviderResponse> {
        self.post_empty(&crate::paths::oidc_providers::activate(id))
            .await
    }

    /// Deactivate an OIDC provider.
    pub async fn deactivate_oidc_provider(&self, id: &Uuid) -> Result<OidcProviderResponse> {
        self.post_empty(&crate::paths::oidc_providers::deactivate(id))
            .await
    }
}

#[cfg(test)]
mod tests {
    use crate::types_impl::SecretString;
    use crate::types_impl::oidc_providers::{CreateOidcProviderRequest, UpdateOidcProviderRequest};
    use std::collections::HashMap;

    #[test]
    fn create_oidc_provider_request_serialization() {
        let req = CreateOidcProviderRequest {
            name: "Google".to_string(),
            slug: "google".to_string(),
            logo_url: None,
            issuer_url: "https://accounts.google.com".to_string(),
            client_id: "client-id-123".to_string(),
            client_secret: SecretString::new("client-secret-456"),
            scopes: "openid email profile groups".to_string(),
            auto_create_users: true,
            allow_private_network_issuers: Some(true),
            role_claim_path: None,
            role_mapping: HashMap::new(),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["name"], "Google");
        assert_eq!(json["slug"], "google");
        assert_eq!(json["issuer_url"], "https://accounts.google.com");
        assert_eq!(json["client_id"], "client-id-123");
        assert_eq!(json["client_secret"], "client-secret-456");
        assert_eq!(json["auto_create_users"], true);
    }

    #[test]
    fn update_oidc_provider_request_serialization() {
        let req = UpdateOidcProviderRequest {
            name: Some("Google Workspace".to_string()),
            slug: None,
            logo_url: None,
            issuer_url: None,
            client_id: None,
            client_secret: None,
            scopes: None,
            auto_create_users: Some(false),
            allow_private_network_issuers: Some(false),
            role_claim_path: None,
            role_mapping: None,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["name"], "Google Workspace");
        assert_eq!(json["auto_create_users"], false);
    }
}
