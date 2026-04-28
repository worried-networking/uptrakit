use crate::generated::types::enrollment_tokens::EnrollmentTokensSummary;
use crate::generated::types::settings::RegistrationSettingsResponse;
use crate::generated::types::settings_agent_certs::AgentCertificateSettingsResponse;
use crate::generated::types::settings_auth::AuthenticationSettingsResponse;
use crate::generated::types::settings_nats::NatsSettingsResponse;
use crate::generated::types::settings_network::NetworkSettingsResponse;
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CombinedSettingsResponse {
    pub registration: RegistrationSettingsResponse,
    pub authentication: AuthenticationSettingsResponse,
    pub agent_certificates: AgentCertificateSettingsResponse,
    pub enrollment_tokens: EnrollmentTokensSummary,
    pub multi_tenancy_enabled: bool,
}
/// Combined response for all global (infrastructure-scoped) settings.
///
/// Returned by `GET /api/v1/global-settings`. This is the single call a
/// global-settings UI page needs to populate all its sections. The `nats`
/// field is omitted from serialized output when it is `None` (i.e. when the
/// controller is compiled without NATS support).
///
/// System service enrollment tokens are managed via the dedicated
/// `/api/v1/system-enrollment-tokens` endpoints.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct GlobalSettingsCombinedResponse {
    pub network: NetworkSettingsResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nats: Option<NatsSettingsResponse>,
}
