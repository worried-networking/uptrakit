use serde::{Deserialize, Serialize};

use crate::enrollment_tokens::EnrollmentTokensSummary;
use crate::settings_agent_certs::AgentCertificateSettingsResponse;
use crate::settings_nats::NatsSettingsResponse;
use crate::settings_network::NetworkSettingsResponse;

#[non_exhaustive]
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CombinedSettingsResponse {
    pub agent_certificates: AgentCertificateSettingsResponse,
    pub enrollment_tokens: EnrollmentTokensSummary,
    pub multi_tenancy_enabled: bool,
}

impl CombinedSettingsResponse {
    /// Create a new `CombinedSettingsResponse`.
    #[must_use]
    pub fn new(
        agent_certificates: AgentCertificateSettingsResponse,
        enrollment_tokens: EnrollmentTokensSummary,
        multi_tenancy_enabled: bool,
    ) -> Self {
        Self {
            agent_certificates,
            enrollment_tokens,
            multi_tenancy_enabled,
        }
    }
}

/// Combined response for all global (infrastructure-scoped) settings.
#[non_exhaustive]
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct GlobalSettingsCombinedResponse {
    pub network: NetworkSettingsResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nats: Option<NatsSettingsResponse>,
}

impl GlobalSettingsCombinedResponse {
    /// Create a new `GlobalSettingsCombinedResponse`.
    #[must_use]
    pub fn new(network: NetworkSettingsResponse, nats: Option<NatsSettingsResponse>) -> Self {
        Self { network, nats }
    }
}
