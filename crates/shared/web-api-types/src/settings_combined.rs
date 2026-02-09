use serde::{Deserialize, Serialize};

use crate::agents::EnrollmentTokenStatusResponse;
use crate::settings::RegistrationSettingsResponse;
use crate::settings_agent_certs::AgentCertificateSettingsResponse;
use crate::settings_auth::AuthenticationSettingsResponse;

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EnrollmentTokenStatusesResponse {
    pub agent: EnrollmentTokenStatusResponse,
    pub mqtt: EnrollmentTokenStatusResponse,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CombinedSettingsResponse {
    pub registration: RegistrationSettingsResponse,
    pub authentication: AuthenticationSettingsResponse,
    pub agent_certificates: AgentCertificateSettingsResponse,
    pub enrollment_tokens: EnrollmentTokenStatusesResponse,
}
