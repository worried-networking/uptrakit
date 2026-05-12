use crate::Result;
use crate::UptrakitClient;
use crate::types_impl::auth::{
    AuthResponse, LoginRequest, LogoutRequest, RefreshRequest, RefreshResponse, RegisterRequest,
    UserResponse,
};
use crate::types_impl::device_auth::{DeviceAuthApproveRequest, DeviceAuthApproveResponse};
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
}

#[cfg(test)]
mod tests {
    use crate::types_impl::SecretString;
    use crate::types_impl::auth::{LoginRequest, LogoutRequest, RefreshRequest, RegisterRequest};
    use crate::types_impl::device_auth::DeviceAuthApproveRequest;

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
}
