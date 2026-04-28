// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
//! Convenience re-exports for API client developers.
//!
//! Import this module to get the most commonly used request/response types:
//!
//! ```ignore
//! use uptrakit_web_api_types::prelude::*;
//! ```ignore
pub use crate::generated::shared_types::DeviceAuthStatus;
pub use crate::generated::types::api_tokens::{
    ApiTokenListResponse, ApiTokenResponse, CreateApiTokenResponse,
};
pub use crate::generated::types::auth::{
    AuthResponse, LoginRequest, RegisterRequest, UserResponse,
};
pub use crate::generated::types::autodiscovery::{
    CreateSoftwareIgnoreRequest, SoftwareIgnoreResponse, TriggerDiscoveryResponse,
};
pub use crate::generated::types::device_auth::DeviceAuthPollResponse;
pub use crate::generated::types::error::ErrorResponse;
pub use crate::generated::types::hosts::{HostAgentSummary, HostResponse};
pub use crate::generated::types::notifications::{
    CreateNotificationChannelRequest, CreateNotificationRuleRequest, NotificationChannelResponse,
    NotificationDeliveryStatus, NotificationEventType, NotificationLogResponse,
    NotificationRuleResponse, TestNotificationResponse, UpdateNotificationChannelRequest,
    UpdateNotificationRuleRequest,
};
pub use crate::generated::types::oidc_auth::{AuthMethodsResponse, OidcProviderInfo};
pub use crate::generated::types::oidc_providers::OidcProviderResponse;
pub use crate::generated::types::pagination::{PaginatedResponse, PaginationParams};
pub use crate::generated::types::permissions::Permission;
pub use crate::generated::types::plugin_configs::{
    CreatePluginConfigRequest, PluginConfigResponse,
};
pub use crate::generated::types::registration::RegistrationMode;
pub use crate::generated::types::services::{ServiceResponse, ServiceStatus};
pub use crate::generated::types::settings::RegistrationSettingsResponse;
pub use crate::generated::types::software_items::{
    SoftwareItemDetailResponse, SoftwareItemHostSummary, SoftwareItemResponse,
    TriggerUpdateRequest, TriggerUpdateResponse, TriggerUpdateStatus, TriggerVersionCheckResponse,
};
pub use crate::generated::types::system_alerts::AlertSeverity;
pub use crate::generated::types::update_history::{UpdateHistoryResponse, UpdateStatus};
