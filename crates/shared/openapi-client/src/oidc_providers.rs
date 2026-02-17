use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::oidc_providers::{
    CreateOidcProviderRequest, OidcProviderResponse, UpdateOidcProviderRequest,
};

impl UptrakitClient {
    /// Create a new OIDC provider.
    pub async fn create_oidc_provider(
        &self,
        req: &CreateOidcProviderRequest,
    ) -> Result<OidcProviderResponse> {
        self.post_json("/api/v1/settings/oidc-providers", req).await
    }

    /// List all OIDC providers.
    pub async fn list_oidc_providers(&self) -> Result<Vec<OidcProviderResponse>> {
        self.get("/api/v1/settings/oidc-providers").await
    }

    /// Get a single OIDC provider by ID.
    pub async fn get_oidc_provider(&self, id: &str) -> Result<OidcProviderResponse> {
        let path = format!("/api/v1/settings/oidc-providers/{id}");
        self.get(&path).await
    }

    /// Update an existing OIDC provider.
    pub async fn update_oidc_provider(
        &self,
        id: &str,
        req: &UpdateOidcProviderRequest,
    ) -> Result<OidcProviderResponse> {
        let path = format!("/api/v1/settings/oidc-providers/{id}");
        self.put_json(&path, req).await
    }

    /// Delete an OIDC provider.
    pub async fn delete_oidc_provider(&self, id: &str) -> Result<()> {
        let path = format!("/api/v1/settings/oidc-providers/{id}");
        self.delete(&path).await
    }

    /// Activate an OIDC provider.
    pub async fn activate_oidc_provider(&self, id: &str) -> Result<OidcProviderResponse> {
        let path = format!("/api/v1/settings/oidc-providers/{id}/activate");
        self.post_empty(&path).await
    }

    /// Deactivate an OIDC provider.
    pub async fn deactivate_oidc_provider(&self, id: &str) -> Result<OidcProviderResponse> {
        let path = format!("/api/v1/settings/oidc-providers/{id}/deactivate");
        self.post_empty(&path).await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use uptrakit_web_api_types::oidc_providers::{
        CreateOidcProviderRequest, UpdateOidcProviderRequest,
    };

    #[test]
    fn create_oidc_provider_request_serialization() {
        let req = CreateOidcProviderRequest {
            name: "Google".to_string(),
            slug: "google".to_string(),
            logo_url: None,
            issuer_url: "https://accounts.google.com".to_string(),
            client_id: "client-id-123".to_string(),
            client_secret: "client-secret-456".to_string(),
            scopes: "openid email profile groups".to_string(),
            auto_create_users: true,
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
            role_claim_path: None,
            role_mapping: None,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["name"], "Google Workspace");
        assert_eq!(json["auto_create_users"], false);
    }
}
