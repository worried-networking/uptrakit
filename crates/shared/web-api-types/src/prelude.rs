//! Convenience re-exports for API client developers.
//!
//! Import this module to get the most commonly used request/response types:
//!
//! ```rust
//! use uptrakit_web_api_types::prelude::*;
//! ```

// ── Auth ─────────────────────────────────────────────────────────────
pub use crate::auth::{AuthResponse, LoginRequest, RegisterRequest, UserResponse};

// ── Device auth ─────────────────────────────────────────────────────
pub use crate::device_auth::DeviceAuthPollResponse;
pub use uptrakit_shared_types::DeviceAuthStatus;

// ── Services ─────────────────────────────────────────────────────────
pub use crate::services::{ServiceResponse, ServiceStatus};

// ── Hosts ────────────────────────────────────────────────────────────
pub use crate::hosts::{HostAgentSummary, HostResponse};

// ── Software items ───────────────────────────────────────────────────
pub use crate::software_items::{
    SoftwareItemDetailResponse, SoftwareItemHostSummary, SoftwareItemResponse,
    TriggerUpdateRequest, TriggerUpdateResponse, TriggerUpdateStatus, TriggerVersionCheckResponse,
};

// ── Plugin configs ───────────────────────────────────────────────────
pub use crate::plugin_configs::{CreatePluginConfigRequest, PluginConfigResponse};

// ── Update history ───────────────────────────────────────────────────
pub use crate::update_history::{UpdateHistoryResponse, UpdateStatus};

// ── API tokens ───────────────────────────────────────────────────────
pub use crate::api_tokens::{ApiTokenListResponse, ApiTokenResponse, CreateApiTokenResponse};

// ── OIDC ─────────────────────────────────────────────────────────────
pub use crate::oidc_auth::{AuthMethodsResponse, OidcProviderInfo};
pub use crate::oidc_providers::OidcProviderResponse;

// ── Settings ─────────────────────────────────────────────────────────
pub use crate::settings::RegistrationSettingsResponse;

// ── MQTT ─────────────────────────────────────────────────────────────
pub use crate::mqtt_transport::{MqttTransport, ParseMqttTransportError};
pub use crate::settings_mqtt::MqttClientConnectionStatus;

// ── System alerts ────────────────────────────────────────────────────
pub use crate::system_alerts::AlertSeverity;

// ── Autodiscovery ────────────────────────────────────────────────────
pub use crate::autodiscovery::{
    CreateSoftwareIgnoreRequest, SoftwareIgnoreResponse, TriggerDiscoveryResponse,
};

// ── SMTP Settings ────────────────────────────────────────────────────
pub use crate::settings_smtp::{SmtpSettingsResponse, UpdateSmtpSettingsRequest};

// ── Notifications ───────────────────────────────────────────────────
pub use crate::notifications::{
    CreateNotificationChannelRequest, CreateNotificationRuleRequest, NotificationChannelResponse,
    NotificationChannelType, NotificationDeliveryStatus, NotificationEventType,
    NotificationLogResponse, NotificationRuleResponse, TestNotificationResponse,
    UpdateNotificationChannelRequest, UpdateNotificationRuleRequest,
};

// ── Common ───────────────────────────────────────────────────────────
pub use crate::error::ErrorResponse;
pub use crate::pagination::{PaginatedResponse, PaginationParams};
pub use crate::permissions::Permission;
pub use crate::registration::RegistrationMode;
