use serde::{Deserialize, Serialize};
use uptrakit_shared_types::{DeviceAuthStatus, SecretString};

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceAuthStartRequest {
    pub client_name: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceAuthStartResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceAuthPollRequest {
    pub device_code: String,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceAuthPollResponse {
    pub status: DeviceAuthStatus,
    pub token: Option<SecretString>,
    pub token_name: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceAuthApproveRequest {
    pub user_code: String,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceAuthApproveResponse {
    pub message: String,
}

// ---------------------------------------------------------------------------
// SSE event payloads for the device auth stream
// ---------------------------------------------------------------------------

/// SSE `authorized` event payload: the device flow was approved and a token
/// is available.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceAuthAuthorizedSse {
    /// The API token for the authorized device.
    pub token: SecretString,
    /// Human-readable name of the token (matches the client name or default).
    pub token_name: String,
}

/// SSE `expired` event payload: the device flow expired before approval.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceAuthExpiredSse {
    /// Human-readable explanation of the expiry.
    pub message: String,
}
