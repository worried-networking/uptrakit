use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::server_cert::RenewServerCertResponse;
use uptrakit_web_api_types::settings::{
    RegistrationSettingsResponse, UpdateRegistrationSettingsRequest,
};
use uptrakit_web_api_types::settings_agent_certs::{
    AgentCertificateSettingsResponse, UpdateAgentCertificateSettingsRequest,
};
use uptrakit_web_api_types::settings_auth::{
    AuthenticationSettingsResponse, UpdateAuthenticationSettingsRequest,
};
use uptrakit_web_api_types::settings_ca::RotateCaResponse;
use uptrakit_web_api_types::settings_combined::CombinedSettingsResponse;
use uptrakit_web_api_types::settings_network::{
    NetworkSettingsResponse, UpdateNetworkSettingsRequest,
};

impl UptrakitClient {
    /// Get combined settings (registration, authentication, certificates, enrollment tokens).
    pub async fn get_combined_settings(&self) -> Result<CombinedSettingsResponse> {
        self.get("/api/v1/settings").await
    }

    /// Get registration settings.
    pub async fn get_registration_settings(&self) -> Result<RegistrationSettingsResponse> {
        self.get("/api/v1/settings/registration").await
    }

    /// Update registration settings.
    pub async fn update_registration_settings(
        &self,
        req: &UpdateRegistrationSettingsRequest,
    ) -> Result<RegistrationSettingsResponse> {
        self.put_json("/api/v1/settings/registration", req).await
    }

    /// Get authentication settings.
    pub async fn get_authentication_settings(&self) -> Result<AuthenticationSettingsResponse> {
        self.get("/api/v1/settings/authentication").await
    }

    /// Update authentication settings.
    pub async fn update_authentication_settings(
        &self,
        req: &UpdateAuthenticationSettingsRequest,
    ) -> Result<AuthenticationSettingsResponse> {
        self.put_json("/api/v1/settings/authentication", req).await
    }

    /// Get agent certificate settings.
    pub async fn get_agent_certificate_settings(&self) -> Result<AgentCertificateSettingsResponse> {
        self.get("/api/v1/settings/agent-certificates").await
    }

    /// Update agent certificate settings.
    pub async fn update_agent_certificate_settings(
        &self,
        req: &UpdateAgentCertificateSettingsRequest,
    ) -> Result<AgentCertificateSettingsResponse> {
        self.put_json("/api/v1/settings/agent-certificates", req)
            .await
    }

    /// Get network settings.
    pub async fn get_network_settings(&self) -> Result<NetworkSettingsResponse> {
        self.get("/api/v1/settings/network").await
    }

    /// Update network settings.
    pub async fn update_network_settings(
        &self,
        req: &UpdateNetworkSettingsRequest,
    ) -> Result<NetworkSettingsResponse> {
        self.put_json("/api/v1/settings/network", req).await
    }

    /// Rotate the CA certificate.
    pub async fn rotate_ca(&self) -> Result<RotateCaResponse> {
        self.post_empty("/api/v1/settings/rotate-ca").await
    }

    /// Renew the server TLS certificate.
    pub async fn renew_server_certificate(&self) -> Result<RenewServerCertResponse> {
        self.post_empty("/api/v1/settings/renew-server-certificate")
            .await
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_web_api_types::registration::RegistrationMode;
    use uptrakit_web_api_types::settings::UpdateRegistrationSettingsRequest;
    use uptrakit_web_api_types::settings_agent_certs::UpdateAgentCertificateSettingsRequest;
    use uptrakit_web_api_types::settings_auth::UpdateAuthenticationSettingsRequest;
    use uptrakit_web_api_types::settings_network::UpdateNetworkSettingsRequest;

    #[test]
    fn update_registration_settings_serialization() {
        let req = UpdateRegistrationSettingsRequest {
            mode: RegistrationMode::Invite,
            token: Some("my-invite-token".to_string()),
            require_token_for_oidc: Some(true),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["mode"], "invite");
        assert_eq!(json["token"], "my-invite-token");
        assert_eq!(json["require_token_for_oidc"], true);
    }

    #[test]
    fn update_authentication_settings_serialization() {
        let req = UpdateAuthenticationSettingsRequest {
            password_auth_enabled: Some(false),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["password_auth_enabled"], false);
    }

    #[test]
    fn update_agent_certificate_settings_serialization() {
        let req = UpdateAgentCertificateSettingsRequest {
            lifetime_days: Some(365),
            renewal_window_hours: Some(72),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["lifetime_days"], 365);
        assert_eq!(json["renewal_window_hours"], 72);
    }

    #[test]
    fn update_network_settings_serialization() {
        let req = UpdateNetworkSettingsRequest {
            trusted_proxies: Some(vec!["10.0.0.0/8".to_string()]),
            real_ip_header: Some("X-Real-IP".to_string()),
            extra_sans: None,
            https_addr: None,
            forwarded_client_cert_info_header: None,
            forwarded_client_cert_pem_header: None,
            pki_addr: None,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["trusted_proxies"][0], "10.0.0.0/8");
        assert_eq!(json["real_ip_header"], "X-Real-IP");
    }
}
