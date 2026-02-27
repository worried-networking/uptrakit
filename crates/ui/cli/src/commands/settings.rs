use std::collections::HashMap;

use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::HumanOutput;
use rootcause::prelude::*;
use serde::Serialize;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::SecretString;
use uptrakit_openapi_client::types::oidc_providers::{
    CreateOidcProviderRequest, OidcProviderResponse, UpdateOidcProviderRequest,
};
use uptrakit_openapi_client::types::registration::RegistrationMode;
use uptrakit_openapi_client::types::server_cert::RenewServerCertResponse;
use uptrakit_openapi_client::types::settings::{
    RegistrationSettingsResponse, UpdateRegistrationSettingsRequest,
};
use uptrakit_openapi_client::types::settings_agent_certs::{
    AgentCertificateSettingsResponse, UpdateAgentCertificateSettingsRequest,
};
use uptrakit_openapi_client::types::settings_auth::{
    AuthenticationSettingsResponse, UpdateAuthenticationSettingsRequest,
};
use uptrakit_openapi_client::types::settings_ca::RotateCaResponse;
use uptrakit_openapi_client::types::settings_combined::CombinedSettingsResponse;
use uptrakit_openapi_client::types::settings_mqtt::{
    CreateMqttClientRequest, MqttClientResponse, MqttLimitResponse, UpdateMqttClientRequest,
    UpdateMqttLimitRequest,
};
use uptrakit_openapi_client::types::settings_network::{
    NetworkSettingsResponse, UpdateNetworkSettingsRequest,
};
use uptrakit_openapi_client::types::system_alerts::SystemAlertsResponse;

// ── Local types ──────────────────────────────────────────────────────────────

/// Returned by delete operations that have no server response body.
#[derive(Debug, Serialize)]
pub struct DeletedOutput {
    pub message: String,
}

impl HumanOutput for DeletedOutput {
    fn to_human_string(&self) -> String {
        format!("{}\n", self.message)
    }
}

// ── Human output ────────────────────────────────────────────────────────────

impl HumanOutput for CombinedSettingsResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::from("Registration:\n");
        out.push_str(&format!(
            "  Mode:                    {}\n",
            self.registration.mode.as_str()
        ));
        out.push_str(&format!(
            "  Require Token for OIDC:  {}\n",
            self.registration.require_token_for_oidc
        ));
        out.push_str("\nAuthentication:\n");
        out.push_str(&format!(
            "  Password Auth Enabled:   {}\n",
            self.authentication.password_auth_enabled
        ));
        out.push_str("\nAgent Certificates:\n");
        out.push_str(&format!(
            "  Lifetime (days):         {}\n",
            self.agent_certificates.lifetime_days
        ));
        out.push_str(&format!(
            "  Renewal Window (hours):  {}\n",
            self.agent_certificates.renewal_window_hours
        ));
        out.push_str("\nEnrollment Tokens:\n");
        out.push_str(&format!(
            "  Active:                  {}\n",
            self.enrollment_tokens.active_count
        ));
        out
    }
}

impl HumanOutput for RegistrationSettingsResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "Mode:                    {}\n",
            self.mode.as_str()
        ));
        out.push_str(&format!(
            "Require Token for OIDC:  {}\n",
            self.require_token_for_oidc
        ));
        out
    }
}

impl HumanOutput for AuthenticationSettingsResponse {
    fn to_human_string(&self) -> String {
        format!("Password Auth Enabled:  {}\n", self.password_auth_enabled)
    }
}

impl HumanOutput for AgentCertificateSettingsResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "Lifetime (days):         {}\n",
            self.lifetime_days
        ));
        out.push_str(&format!(
            "Renewal Window (hours):  {}\n",
            self.renewal_window_hours
        ));
        out
    }
}

impl HumanOutput for NetworkSettingsResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "Trusted Proxies:     {}\n",
            self.trusted_proxies.join(", ")
        ));
        out.push_str(&format!("Real IP Header:      {}\n", self.real_ip_header));
        out.push_str(&format!(
            "Extra SANs:          {}\n",
            self.extra_sans.join(", ")
        ));
        out.push_str(&format!("HTTPS Address:       {}\n", self.https_addr));
        out.push_str(&format!(
            "Fwd Cert Info Header: {}\n",
            self.forwarded_client_cert_info_header
                .as_deref()
                .unwrap_or("-")
        ));
        out.push_str(&format!(
            "Fwd Cert PEM Header: {}\n",
            self.forwarded_client_cert_pem_header
                .as_deref()
                .unwrap_or("-")
        ));
        out.push_str(&format!(
            "PKI Address:         {}\n",
            self.pki_addr.as_deref().unwrap_or("-")
        ));
        if let Some(ref warning) = self.pki_addr_warning {
            out.push_str(&format!("Warning:             {warning}\n"));
        }
        out
    }
}

impl HumanOutput for Vec<MqttClientResponse> {
    fn to_human_string(&self) -> String {
        if self.is_empty() {
            return "No MQTT configurations found.\n".to_string();
        }
        let mut out = format!(
            "{:<38} {:<8} {:<10} {:<25} {:<6} STATUS\n",
            "ID", "ENABLED", "TRANSPORT", "HOST", "PORT"
        );
        for m in self {
            out.push_str(&format!(
                "{:<38} {:<8} {:<10} {:<25} {:<6} {}\n",
                m.id, m.enabled, m.transport, m.host, m.port, m.connection_status
            ));
        }
        out
    }
}

impl HumanOutput for MqttClientResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("ID:            {}\n", self.id));
        out.push_str(&format!("Enabled:       {}\n", self.enabled));
        out.push_str(&format!("Transport:     {}\n", self.transport));
        out.push_str(&format!("Host:          {}\n", self.host));
        out.push_str(&format!("Port:          {}\n", self.port));
        out.push_str(&format!("URL:           {}\n", self.url));
        out.push_str(&format!("Client ID:     {}\n", self.client_id));
        out.push_str(&format!(
            "Username:      {}\n",
            self.username.as_deref().unwrap_or("-")
        ));
        out.push_str(&format!("Has Password:  {}\n", self.has_password));
        out.push_str(&format!("Topic Prefix:  {}\n", self.topic_prefix));
        out.push_str(&format!("HA Discovery:  {}\n", self.ha_discovery));
        if self.ha_discovery {
            out.push_str(&format!("HA Prefix:     {}\n", self.ha_discovery_prefix));
        }
        out.push_str(&format!("Status:        {}\n", self.connection_status));
        out
    }
}

impl HumanOutput for MqttLimitResponse {
    fn to_human_string(&self) -> String {
        format!("Max Clients Per Tenant:  {}\n", self.max_clients_per_tenant)
    }
}

impl HumanOutput for Vec<OidcProviderResponse> {
    fn to_human_string(&self) -> String {
        if self.is_empty() {
            return "No OIDC providers found.\n".to_string();
        }
        let mut out = format!("{:<38} {:<20} {:<15} ACTIVE\n", "ID", "NAME", "SLUG");
        for p in self {
            out.push_str(&format!(
                "{:<38} {:<20} {:<15} {}\n",
                p.id, p.name, p.slug, p.is_active
            ));
        }
        out
    }
}

impl HumanOutput for OidcProviderResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("ID:               {}\n", self.id));
        out.push_str(&format!("Name:             {}\n", self.name));
        out.push_str(&format!("Slug:             {}\n", self.slug));
        if let Some(ref logo) = self.logo_url {
            out.push_str(&format!("Logo URL:         {}\n", logo));
        }
        out.push_str(&format!("Issuer URL:       {}\n", self.issuer_url));
        out.push_str(&format!("Client ID:        {}\n", self.client_id));
        out.push_str(&format!("Has Secret:       {}\n", self.has_client_secret));
        out.push_str(&format!("Scopes:           {}\n", self.scopes));
        out.push_str(&format!("Auto Create Users: {}\n", self.auto_create_users));
        if let Some(ref path) = self.role_claim_path {
            out.push_str(&format!("Role Claim Path:  {}\n", path));
        }
        if !self.role_mapping.is_empty() {
            let mappings: Vec<String> = self
                .role_mapping
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            out.push_str(&format!("Role Mapping:     {}\n", mappings.join(", ")));
        }
        out.push_str(&format!("Active:           {}\n", self.is_active));
        out.push_str(&format!("Created:          {}\n", self.created_at));
        out.push_str(&format!("Updated:          {}\n", self.updated_at));
        out
    }
}

impl HumanOutput for SystemAlertsResponse {
    fn to_human_string(&self) -> String {
        if self.alerts.is_empty() {
            return "No active alerts.\n".to_string();
        }
        let mut out = format!("{:<10} {:<30} MESSAGE\n", "SEVERITY", "TITLE");
        for alert in &self.alerts {
            out.push_str(&format!(
                "{:<10} {:<30} {}\n",
                alert.severity, alert.title, alert.message
            ));
        }
        out
    }
}

// ── Params ───────────────────────────────────────────────────────────────────

/// Parameters for updating registration settings.
pub struct RegistrationUpdateParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub mode: RegistrationMode,
    pub reg_token: Option<String>,
    pub require_token_for_oidc: Option<bool>,
}

/// Parameters for updating network settings.
pub struct NetworkUpdateParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub trusted_proxies: Option<Vec<String>>,
    pub real_ip_header: Option<String>,
    pub extra_sans: Option<Vec<String>>,
    pub https_addr: Option<String>,
    pub fwd_cert_info_header: Option<String>,
    pub fwd_cert_pem_header: Option<String>,
    pub pki_addr: Option<String>,
}

/// Parameters for creating an MQTT client configuration.
pub struct MqttCreateParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub url: Option<String>,
    pub transport: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub enabled: Option<bool>,
    pub client_id: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub ca_pem: Option<String>,
    pub topic_prefix: Option<String>,
    pub ha_discovery: Option<bool>,
    pub ha_discovery_prefix: Option<String>,
}

/// Parameters for updating an MQTT client configuration.
pub struct MqttUpdateParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub id: Uuid,
    pub url: Option<String>,
    pub transport: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub enabled: Option<bool>,
    pub client_id: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub ca_pem: Option<String>,
    pub topic_prefix: Option<String>,
    pub ha_discovery: Option<bool>,
    pub ha_discovery_prefix: Option<String>,
}

/// Parameters for creating an OIDC provider.
pub struct OidcCreateParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub name: String,
    pub slug: String,
    pub logo_url: Option<String>,
    pub issuer_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub scopes: Option<String>,
    pub auto_create_users: Option<bool>,
    pub role_claim_path: Option<String>,
}

/// Parameters for updating an OIDC provider.
pub struct OidcUpdateParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub id: Uuid,
    pub name: Option<String>,
    pub slug: Option<String>,
    pub logo_url: Option<String>,
    pub issuer_url: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub scopes: Option<String>,
    pub auto_create_users: Option<bool>,
    pub role_claim_path: Option<String>,
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// Show combined settings overview.
pub async fn show_combined(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<CombinedSettingsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.get_combined_settings().await.context_to()
}

/// Show registration settings.
pub async fn registration_show(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<RegistrationSettingsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.get_registration_settings().await.context_to()
}

/// Update registration settings.
pub async fn registration_update(
    params: RegistrationUpdateParams<'_>,
) -> Result<RegistrationSettingsResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = UpdateRegistrationSettingsRequest {
        mode: params.mode,
        token: params.reg_token.map(SecretString::new),
        require_token_for_oidc: params.require_token_for_oidc,
    };
    client.update_registration_settings(&req).await.context_to()
}

/// Show authentication settings.
pub async fn authentication_show(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<AuthenticationSettingsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.get_authentication_settings().await.context_to()
}

/// Update authentication settings.
pub async fn authentication_update(
    password_auth_enabled: Option<bool>,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<AuthenticationSettingsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = UpdateAuthenticationSettingsRequest {
        password_auth_enabled,
    };
    client
        .update_authentication_settings(&req)
        .await
        .context_to()
}

/// Show agent certificate settings.
pub async fn certificates_show(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<AgentCertificateSettingsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.get_agent_certificate_settings().await.context_to()
}

/// Update agent certificate settings.
pub async fn certificates_update(
    lifetime_days: Option<u16>,
    renewal_window_hours: Option<u16>,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<AgentCertificateSettingsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = UpdateAgentCertificateSettingsRequest {
        lifetime_days,
        renewal_window_hours,
    };
    client
        .update_agent_certificate_settings(&req)
        .await
        .context_to()
}

/// Show network settings.
pub async fn network_show(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<NetworkSettingsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.get_network_settings().await.context_to()
}

/// Update network settings.
pub async fn network_update(params: NetworkUpdateParams<'_>) -> Result<NetworkSettingsResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = UpdateNetworkSettingsRequest {
        trusted_proxies: params.trusted_proxies,
        real_ip_header: params.real_ip_header,
        extra_sans: params.extra_sans,
        https_addr: params.https_addr,
        forwarded_client_cert_info_header: params.fwd_cert_info_header,
        forwarded_client_cert_pem_header: params.fwd_cert_pem_header,
        pki_addr: params.pki_addr,
    };
    client.update_network_settings(&req).await.context_to()
}

/// Rotate the CA certificate.
pub async fn rotate_ca(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<RotateCaResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.rotate_ca().await.context_to()
}

/// Renew the server TLS certificate.
pub async fn renew_server_cert(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<RenewServerCertResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.renew_server_certificate().await.context_to()
}

/// List all MQTT client configurations.
pub async fn mqtt_list(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<Vec<MqttClientResponse>> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.list_mqtt_settings().await.context_to()
}

/// Show a single MQTT client configuration.
pub async fn mqtt_show(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<MqttClientResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.get_mqtt_settings(id).await.context_to()
}

/// Create a new MQTT client configuration.
pub async fn mqtt_create(params: MqttCreateParams<'_>) -> Result<MqttClientResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let transport = params
        .transport
        .map(|t| t.parse())
        .transpose()
        .context_to()?;
    let req = CreateMqttClientRequest {
        url: params.url,
        transport,
        host: params.host,
        port: params.port,
        enabled: params.enabled,
        client_id: params.client_id,
        username: params.username,
        password: params.password.map(SecretString::new),
        ca_pem: params.ca_pem.map(SecretString::new),
        topic_prefix: params.topic_prefix,
        ha_discovery: params.ha_discovery,
        ha_discovery_prefix: params.ha_discovery_prefix,
    };
    client.create_mqtt_settings(&req).await.context_to()
}

/// Update an existing MQTT client configuration.
pub async fn mqtt_update(params: MqttUpdateParams<'_>) -> Result<MqttClientResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let transport = params
        .transport
        .map(|t| t.parse())
        .transpose()
        .context_to()?;
    let username = params.username.map(serde_json::Value::String);
    let password = params.password.map(serde_json::Value::String);
    let ca_pem = params.ca_pem.map(serde_json::Value::String);
    let req = UpdateMqttClientRequest {
        url: params.url,
        transport,
        host: params.host,
        port: params.port,
        enabled: params.enabled,
        client_id: params.client_id,
        username,
        password,
        ca_pem,
        topic_prefix: params.topic_prefix,
        ha_discovery: params.ha_discovery,
        ha_discovery_prefix: params.ha_discovery_prefix,
    };
    client
        .update_mqtt_settings(&params.id, &req)
        .await
        .context_to()
}

/// Delete an MQTT client configuration.
pub async fn mqtt_delete(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<DeletedOutput> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.delete_mqtt_settings(id).await.context_to()?;
    Ok(DeletedOutput {
        message: format!("MQTT configuration {id} deleted."),
    })
}

/// Show MQTT client limit.
pub async fn mqtt_limit_show(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<MqttLimitResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.get_mqtt_limit().await.context_to()
}

/// Update MQTT client limit.
pub async fn mqtt_limit_update(
    max: u16,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<MqttLimitResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = UpdateMqttLimitRequest {
        max_clients_per_tenant: max,
    };
    client.update_mqtt_limit(&req).await.context_to()
}

/// List all OIDC providers.
pub async fn oidc_list(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<Vec<OidcProviderResponse>> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.list_oidc_providers().await.context_to()
}

/// Show a single OIDC provider.
pub async fn oidc_show(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<OidcProviderResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.get_oidc_provider(id).await.context_to()
}

/// Create a new OIDC provider.
pub async fn oidc_create(params: OidcCreateParams<'_>) -> Result<OidcProviderResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = CreateOidcProviderRequest {
        name: params.name,
        slug: params.slug,
        logo_url: params.logo_url,
        issuer_url: params.issuer_url,
        client_id: params.client_id,
        client_secret: SecretString::new(params.client_secret),
        scopes: params
            .scopes
            .unwrap_or_else(|| "openid email profile groups".to_string()),
        auto_create_users: params.auto_create_users.unwrap_or(true),
        role_claim_path: params.role_claim_path,
        role_mapping: HashMap::new(),
    };
    client.create_oidc_provider(&req).await.context_to()
}

/// Update an existing OIDC provider.
pub async fn oidc_update(params: OidcUpdateParams<'_>) -> Result<OidcProviderResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = UpdateOidcProviderRequest {
        name: params.name,
        slug: params.slug,
        logo_url: params.logo_url,
        issuer_url: params.issuer_url,
        client_id: params.client_id,
        client_secret: params.client_secret.map(SecretString::new),
        scopes: params.scopes,
        auto_create_users: params.auto_create_users,
        role_claim_path: params.role_claim_path,
        role_mapping: None,
    };
    client
        .update_oidc_provider(&params.id, &req)
        .await
        .context_to()
}

/// Delete an OIDC provider.
pub async fn oidc_delete(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<DeletedOutput> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.delete_oidc_provider(id).await.context_to()?;
    Ok(DeletedOutput {
        message: format!("OIDC provider {id} deleted."),
    })
}

/// Activate an OIDC provider.
pub async fn oidc_activate(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<OidcProviderResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.activate_oidc_provider(id).await.context_to()
}

/// Deactivate an OIDC provider.
pub async fn oidc_deactivate(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<OidcProviderResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.deactivate_oidc_provider(id).await.context_to()
}

/// Show system alerts.
pub async fn alerts(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<SystemAlertsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.get_system_alerts().await.context_to()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_openapi_client::types::enrollment_tokens::EnrollmentTokensSummary;
    use uptrakit_openapi_client::types::registration::RegistrationMode;
    use uptrakit_openapi_client::types::system_alerts::{AlertSeverity, SystemAlert};

    #[test]
    fn deleted_output_human() {
        let out = DeletedOutput {
            message: "MQTT configuration abc deleted.".to_string(),
        };
        assert!(out.to_human_string().contains("abc deleted"));
    }

    #[test]
    fn registration_settings_human_output() {
        let resp = RegistrationSettingsResponse {
            mode: RegistrationMode::Invite,
            require_token_for_oidc: true,
        };
        let s = resp.to_human_string();
        assert!(s.contains("invite"), "mode missing");
        assert!(s.contains("true"), "require_token_for_oidc missing");
    }

    #[test]
    fn authentication_settings_human_output() {
        let resp = AuthenticationSettingsResponse {
            password_auth_enabled: false,
        };
        let s = resp.to_human_string();
        assert!(s.contains("false"), "password_auth_enabled missing");
    }

    #[test]
    fn certificate_settings_human_output() {
        let resp = AgentCertificateSettingsResponse {
            lifetime_days: 365,
            renewal_window_hours: 168,
        };
        let s = resp.to_human_string();
        assert!(s.contains("365"), "lifetime_days missing");
        assert!(s.contains("168"), "renewal_window_hours missing");
    }

    #[test]
    fn combined_settings_human_output() {
        let resp = CombinedSettingsResponse {
            registration: RegistrationSettingsResponse {
                mode: RegistrationMode::Open,
                require_token_for_oidc: false,
            },
            authentication: AuthenticationSettingsResponse {
                password_auth_enabled: true,
            },
            agent_certificates: AgentCertificateSettingsResponse {
                lifetime_days: 365,
                renewal_window_hours: 168,
            },
            enrollment_tokens: EnrollmentTokensSummary { active_count: 3 },
        };
        let s = resp.to_human_string();
        assert!(s.contains("Registration"), "registration section missing");
        assert!(
            s.contains("Authentication"),
            "authentication section missing"
        );
        assert!(s.contains("365"), "lifetime_days missing");
    }

    #[test]
    fn mqtt_list_empty() {
        let resp: Vec<MqttClientResponse> = vec![];
        assert!(resp.to_human_string().contains("No MQTT"));
    }

    #[test]
    fn oidc_list_empty() {
        let resp: Vec<OidcProviderResponse> = vec![];
        assert!(resp.to_human_string().contains("No OIDC"));
    }

    #[test]
    fn mqtt_limit_human_output() {
        let resp = MqttLimitResponse {
            max_clients_per_tenant: 10,
        };
        assert!(resp.to_human_string().contains("10"));
    }

    #[test]
    fn system_alerts_empty() {
        let resp = SystemAlertsResponse { alerts: vec![] };
        assert!(resp.to_human_string().contains("No active alerts"));
    }

    #[test]
    fn system_alerts_has_rows() {
        let resp = SystemAlertsResponse {
            alerts: vec![SystemAlert {
                id: "alert-1".to_string(),
                severity: AlertSeverity::Warning,
                title: "High CPU".to_string(),
                message: "CPU usage above threshold.".to_string(),
                action: None,
            }],
        };
        let s = resp.to_human_string();
        assert!(s.contains("warning"), "severity missing");
        assert!(s.contains("High CPU"), "title missing");
        assert!(s.contains("CPU usage"), "message missing");
    }

    #[test]
    fn network_settings_human_output() {
        let resp = NetworkSettingsResponse {
            trusted_proxies: vec!["10.0.0.0/8".to_string()],
            real_ip_header: "X-Forwarded-For".to_string(),
            extra_sans: vec![],
            https_addr: "0.0.0.0:443".to_string(),
            forwarded_client_cert_info_header: None,
            forwarded_client_cert_pem_header: None,
            pki_addr: Some("https://pki.example.com".to_string()),
            pki_addr_warning: Some("CA rotation required".to_string()),
        };
        let s = resp.to_human_string();
        assert!(s.contains("10.0.0.0/8"), "trusted proxies missing");
        assert!(s.contains("pki.example.com"), "pki_addr missing");
        assert!(s.contains("CA rotation required"), "warning missing");
    }
}
