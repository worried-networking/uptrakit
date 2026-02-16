use crate::Result;
use crate::UptrakitClient;
use rootcause::prelude::*;
use uptrakit_web_api_types::auth::UserResponse;
use uptrakit_web_api_types::device_auth::{
    DeviceAuthPollRequest, DeviceAuthPollResponse, DeviceAuthStartRequest,
    DeviceAuthStartResponse,
};

impl UptrakitClient {
    /// Start a device authorization flow (RFC 8628-style).
    ///
    /// This endpoint does not require authentication.
    pub async fn device_auth_start(
        &self,
        req: &DeviceAuthStartRequest,
    ) -> Result<DeviceAuthStartResponse> {
        self.post_json_unauth("/api/v1/auth/device", req).await
    }

    /// Poll for device authorization completion.
    ///
    /// Returns `Err(ClientError::RateLimited)` on HTTP 429 and
    /// `Err(ClientError::NotFound(...))` on HTTP 404. This endpoint does
    /// not require authentication.
    pub async fn device_auth_poll(
        &self,
        req: &DeviceAuthPollRequest,
    ) -> Result<DeviceAuthPollResponse> {
        let url = format!("{}/api/v1/auth/device/poll", self.base_url);
        let resp = self.http.post(&url).json(req).send().await.context_to()?;
        self.handle_response(resp).await
    }

    /// Retrieve the current authenticated user's profile.
    pub async fn me(&self) -> Result<UserResponse> {
        self.get("/api/v1/auth/me").await
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_web_api_types::device_auth::{DeviceAuthPollRequest, DeviceAuthStartRequest};

    #[test]
    fn device_auth_start_request_serialization() {
        let req = DeviceAuthStartRequest {
            client_name: Some("cli-host-2026-02-16".to_string()),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["client_name"], "cli-host-2026-02-16");
    }

    #[test]
    fn device_auth_poll_request_serialization() {
        let req = DeviceAuthPollRequest {
            device_code: "abc-123".to_string(),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["device_code"], "abc-123");
    }
}
