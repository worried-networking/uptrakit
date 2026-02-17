use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::auth::AuthResponse;
use uptrakit_web_api_types::oidc_auth::{
    OidcAuthorizeResponse, OidcCompleteRegistrationRequest, OidcExchangeRequest, OidcLinkRequest,
};

impl UptrakitClient {
    /// Get the OIDC authorization URL for a provider.
    ///
    /// This endpoint does not require authentication.
    pub async fn oidc_authorize(&self, provider_id: &str) -> Result<OidcAuthorizeResponse> {
        let path = format!("/api/v1/auth/oidc/{provider_id}/authorize");
        self.get_unauth(&path).await
    }

    /// Exchange an OIDC authorization code for tokens.
    ///
    /// This endpoint does not require authentication.
    pub async fn oidc_exchange(&self, req: &OidcExchangeRequest) -> Result<AuthResponse> {
        self.post_json_unauth("/api/v1/auth/oidc/exchange", req)
            .await
    }

    /// Link an OIDC account to an existing user.
    pub async fn oidc_link(&self, req: &OidcLinkRequest) -> Result<AuthResponse> {
        self.post_json("/api/v1/auth/oidc/link", req).await
    }

    /// Complete OIDC registration with a registration code and token.
    ///
    /// This endpoint does not require authentication.
    pub async fn oidc_complete_registration(
        &self,
        req: &OidcCompleteRegistrationRequest,
    ) -> Result<AuthResponse> {
        self.post_json_unauth("/api/v1/auth/oidc/complete-registration", req)
            .await
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_web_api_types::oidc_auth::{
        OidcCompleteRegistrationRequest, OidcExchangeRequest, OidcLinkRequest,
    };

    #[test]
    fn oidc_exchange_request_serialization() {
        let req = OidcExchangeRequest {
            code: "auth-code-xyz".to_string(),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["code"], "auth-code-xyz");
    }

    #[test]
    fn oidc_link_request_serialization() {
        let req = OidcLinkRequest {
            link_token: "link-tok-abc".to_string(),
            password: Some("SecurePass123".to_string()),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["link_token"], "link-tok-abc");
        assert_eq!(json["password"], "SecurePass123");
    }

    #[test]
    fn oidc_link_request_without_password() {
        let req = OidcLinkRequest {
            link_token: "link-tok-abc".to_string(),
            password: None,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["link_token"], "link-tok-abc");
        assert!(json["password"].is_null());
    }

    #[test]
    fn oidc_complete_registration_request_serialization() {
        let req = OidcCompleteRegistrationRequest {
            registration_code: "reg-code-123".to_string(),
            registration_token: "reg-tok-456".to_string(),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["registration_code"], "reg-code-123");
        assert_eq!(json["registration_token"], "reg-tok-456");
    }
}
