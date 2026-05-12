use crate::Result;
use crate::UptrakitClient;
use crate::types_impl::auth::{
    AuthResponse, LoginRequest, LogoutRequest, RefreshRequest, RefreshResponse, RegisterRequest,
    UserResponse,
};
use crate::types_impl::device_auth::{DeviceAuthApproveRequest, DeviceAuthApproveResponse};
use crate::types_impl::oauth::{
    DeviceAuthDenyRequest, DeviceAuthDenyResponse, DeviceAuthLookupQuery, DeviceAuthLookupResponse,
    DeviceAuthorizationRequest, DeviceAuthorizationResponse, OAuthAuthorizationServerMetadata,
    OAuthTokenRequest, OAuthTokenResponse,
};
use crate::types_impl::oidc_auth::AuthMethodsResponse;

impl UptrakitClient {
    /// Register a new user account.
    ///
    /// This endpoint does not require authentication.
    pub async fn register(&self, req: &RegisterRequest) -> Result<AuthResponse> {
        self.post_json_unauth(crate::paths::auth::REGISTER, req)
            .await
    }

    /// Log in with email and password.
    ///
    /// This endpoint does not require authentication.
    pub async fn login(&self, req: &LoginRequest) -> Result<AuthResponse> {
        self.post_json_unauth(crate::paths::auth::LOGIN, req).await
    }

    /// Refresh an access token using a refresh token.
    ///
    /// This endpoint does not require authentication.
    pub async fn refresh(&self, req: &RefreshRequest) -> Result<RefreshResponse> {
        self.post_json_unauth(crate::paths::auth::REFRESH, req)
            .await
    }

    /// Log out by revoking a refresh token.
    pub async fn logout(&self, req: &LogoutRequest) -> Result<()> {
        self.post_json_no_content(crate::paths::auth::LOGOUT, req)
            .await
    }

    /// List available authentication methods.
    ///
    /// This endpoint does not require authentication.
    pub async fn auth_methods(&self) -> Result<AuthMethodsResponse> {
        self.get_unauth(crate::paths::auth::METHODS).await
    }

    /// Approve a pending device authorization request.
    pub async fn device_auth_approve(
        &self,
        req: &DeviceAuthApproveRequest,
    ) -> Result<DeviceAuthApproveResponse> {
        self.post_json(crate::paths::auth::DEVICE_APPROVE, req)
            .await
    }

    /// Retrieve the current authenticated user's profile.
    pub async fn me(&self) -> Result<UserResponse> {
        self.get(crate::paths::auth::ME).await
    }

    /// Start an RFC 8628 device authorization flow.
    ///
    /// Per RFC 8628 §3.1. Returns the device_code, user_code, verification URIs,
    /// expiry, and recommended polling interval. This endpoint does not require
    /// authentication.
    pub async fn oauth_device_authorization(
        &self,
        req: &DeviceAuthorizationRequest,
    ) -> Result<DeviceAuthorizationResponse> {
        self.post_form_unauth(crate::paths::auth::OAUTH_DEVICE_AUTHORIZATION, req)
            .await
    }

    /// Exchange a device_code for an access token.
    ///
    /// Per RFC 6749 §3.2 / RFC 8628 §3.4. Form-urlencoded body. On HTTP 400 the
    /// caller receives `Err(ClientError::OAuthError(OAuthErrorResponse))` with
    /// the typed `OAuthErrorCode`. This endpoint does not require
    /// authentication.
    pub async fn oauth_token(&self, req: &OAuthTokenRequest) -> Result<OAuthTokenResponse> {
        self.post_form_unauth(crate::paths::auth::OAUTH_TOKEN, req)
            .await
    }

    /// Fetch the RFC 8414 §3 authorization server metadata document.
    ///
    /// Public; no authentication required.
    pub async fn oauth_authorization_server_metadata(
        &self,
    ) -> Result<OAuthAuthorizationServerMetadata> {
        self.get_unauth(crate::paths::auth::OAUTH_METADATA).await
    }

    /// Deny a pending device authorization request (UI-internal).
    pub async fn device_auth_deny(
        &self,
        req: &DeviceAuthDenyRequest,
    ) -> Result<DeviceAuthDenyResponse> {
        self.post_json(crate::paths::auth::DEVICE_DENY, req).await
    }

    /// Look up the `client_name` + `expires_at` for a pending flow.
    ///
    /// Authenticated; requires `CanViewServices`. Query parameters are
    /// serialised by `reqwest::RequestBuilder::query` (which uses
    /// `serde_urlencoded` internally) so no manual URL building is required.
    pub async fn device_auth_lookup(
        &self,
        query: &DeviceAuthLookupQuery,
    ) -> Result<DeviceAuthLookupResponse> {
        let url = format!("{}{}", self.base_url, crate::paths::auth::DEVICE_LOOKUP);
        let req = self
            .http
            .get(&url)
            .bearer_auth(self.token_or_err()?)
            .query(query);
        let resp = self.send_with_retry(req).await?;
        self.handle_response(resp).await
    }
}

#[cfg(test)]
mod tests {
    use crate::types_impl::SecretString;
    use crate::types_impl::auth::{LoginRequest, LogoutRequest, RefreshRequest, RegisterRequest};
    use crate::types_impl::device_auth::DeviceAuthApproveRequest;
    use crate::types_impl::oauth::{
        DeviceAuthDenyRequest, DeviceAuthorizationRequest, OAuthTokenRequest,
    };

    #[test]
    fn register_request_serialization() {
        let req = RegisterRequest {
            email: "admin@example.com".to_string(),
            first_name: "Admin".to_string(),
            last_name: "User".to_string(),
            password: SecretString::new("SecurePass123"),
            registration_token: None,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["email"], "admin@example.com");
        assert_eq!(json["first_name"], "Admin");
        assert_eq!(json["last_name"], "User");
        assert_eq!(json["password"], "SecurePass123");
    }

    #[test]
    fn register_request_with_token_serialization() {
        let req = RegisterRequest {
            email: "admin@example.com".to_string(),
            first_name: "Admin".to_string(),
            last_name: "User".to_string(),
            password: SecretString::new("SecurePass123"),
            registration_token: Some(SecretString::new("invite-tok-abc")),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["registration_token"], "invite-tok-abc");
    }

    #[test]
    fn login_request_serialization() {
        let req = LoginRequest {
            email: "admin@example.com".to_string(),
            password: SecretString::new("SecurePass123"),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["email"], "admin@example.com");
        assert_eq!(json["password"], "SecurePass123");
    }

    #[test]
    fn logout_request_serialization() {
        let req = LogoutRequest {
            refresh_token: Some(SecretString::new("refresh-tok-xyz")),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["refresh_token"], "refresh-tok-xyz");
    }

    #[test]
    fn refresh_request_serialization() {
        let req = RefreshRequest {
            refresh_token: Some(SecretString::new("refresh-tok-xyz")),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["refresh_token"], "refresh-tok-xyz");
    }

    #[test]
    fn device_auth_approve_request_serialization() {
        let req = DeviceAuthApproveRequest {
            user_code: "ABCD-1234".to_string(),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["user_code"], "ABCD-1234");
    }

    #[test]
    fn device_authorization_request_form_serialization() {
        use serde_urlencoded;
        let req = DeviceAuthorizationRequest {
            client_id: "uptrakit-cli".into(),
            scope: None,
            client_name: Some("cli-host-2026-05-12".into()),
        };
        let encoded = serde_urlencoded::to_string(&req).expect("encode");
        assert!(encoded.contains("client_id=uptrakit-cli"));
        assert!(encoded.contains("client_name=cli-host-2026-05-12"));
        assert!(!encoded.contains("scope="), "scope omitted when None");
    }

    #[test]
    fn oauth_token_request_form_serialization() {
        use serde_urlencoded;
        let req = OAuthTokenRequest {
            grant_type: "urn:ietf:params:oauth:grant-type:device_code".into(),
            device_code: Some("abc-123".into()),
            client_id: Some("uptrakit-cli".into()),
        };
        let encoded = serde_urlencoded::to_string(&req).expect("encode");
        assert!(
            encoded.contains("grant_type=urn"),
            "grant_type URI preserved verbatim"
        );
        assert!(encoded.contains("device_code=abc-123"));
        assert!(encoded.contains("client_id=uptrakit-cli"));
    }

    #[test]
    fn device_auth_deny_request_serialization() {
        let req = DeviceAuthDenyRequest {
            user_code: "ABCD-EFGH".into(),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["user_code"], "ABCD-EFGH");
    }
}
