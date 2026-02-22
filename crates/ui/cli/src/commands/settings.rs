use std::collections::HashMap;

use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::{OutputFormat, print_output};
use rootcause::prelude::*;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::SecretString;
use uptrakit_openapi_client::types::oidc_providers::{
    CreateOidcProviderRequest, UpdateOidcProviderRequest,
};
use uptrakit_openapi_client::types::registration::RegistrationMode;
use uptrakit_openapi_client::types::settings::UpdateRegistrationSettingsRequest;
use uptrakit_openapi_client::types::settings_agent_certs::UpdateAgentCertificateSettingsRequest;
use uptrakit_openapi_client::types::settings_auth::UpdateAuthenticationSettingsRequest;
use uptrakit_openapi_client::types::settings_mqtt::{
    CreateMqttClientRequest, UpdateMqttClientRequest, UpdateMqttLimitRequest,
};
use uptrakit_openapi_client::types::settings_network::UpdateNetworkSettingsRequest;

/// Show combined settings overview.
pub async fn show_combined(
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let resp = client.get_combined_settings().await.context_to()?;

    let mut human = String::new();
    human.push_str("Registration:\n");
    human.push_str(&format!(
        "  Mode:                    {}\n",
        resp.registration.mode.as_str()
    ));
    human.push_str(&format!(
        "  Require Token for OIDC:  {}\n",
        resp.registration.require_token_for_oidc
    ));
    human.push_str("\nAuthentication:\n");
    human.push_str(&format!(
        "  Password Auth Enabled:   {}\n",
        resp.authentication.password_auth_enabled
    ));
    human.push_str("\nAgent Certificates:\n");
    human.push_str(&format!(
        "  Lifetime (days):         {}\n",
        resp.agent_certificates.lifetime_days
    ));
    human.push_str(&format!(
        "  Renewal Window (hours):  {}\n",
        resp.agent_certificates.renewal_window_hours
    ));
    human.push_str("\nEnrollment Tokens:\n");
    human.push_str(&format!(
        "  Agent Token Configured:  {}\n",
        resp.enrollment_tokens.agent.configured
    ));
    human.push_str(&format!(
        "  MQTT Token Configured:   {}\n",
        resp.enrollment_tokens.mqtt.configured
    ));

    print_output(format, &human, &resp)
}

/// Show registration settings.
pub async fn registration_show(
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let resp = client.get_registration_settings().await.context_to()?;

    let mut human = String::new();
    human.push_str(&format!(
        "Mode:                    {}\n",
        resp.mode.as_str()
    ));
    human.push_str(&format!(
        "Require Token for OIDC:  {}\n",
        resp.require_token_for_oidc
    ));

    print_output(format, &human, &resp)
}

/// Parameters for updating registration settings.
pub struct RegistrationUpdateParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub format: OutputFormat,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub mode: RegistrationMode,
    pub reg_token: Option<String>,
    pub require_token_for_oidc: Option<bool>,
}

/// Update registration settings.
pub async fn registration_update(params: RegistrationUpdateParams<'_>) -> Result<()> {
    let client = authenticated_client(params.server, params.token, params.insecure, params.request_timeout)?;
    let req = UpdateRegistrationSettingsRequest {
        mode: params.mode,
        token: params.reg_token.map(SecretString::new),
        require_token_for_oidc: params.require_token_for_oidc,
    };
    let resp = client
        .update_registration_settings(&req)
        .await
        .context_to()?;

    let mut human = String::new();
    human.push_str("Registration settings updated.\n");
    human.push_str(&format!(
        "Mode:                    {}\n",
        resp.mode.as_str()
    ));
    human.push_str(&format!(
        "Require Token for OIDC:  {}\n",
        resp.require_token_for_oidc
    ));

    print_output(params.format, &human, &resp)
}

/// Show authentication settings.
pub async fn authentication_show(
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let resp = client.get_authentication_settings().await.context_to()?;

    let human = format!("Password Auth Enabled:  {}\n", resp.password_auth_enabled);

    print_output(format, &human, &resp)
}

/// Update authentication settings.
pub async fn authentication_update(
    password_auth_enabled: Option<bool>,
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = UpdateAuthenticationSettingsRequest {
        password_auth_enabled,
    };
    let resp = client
        .update_authentication_settings(&req)
        .await
        .context_to()?;

    let mut human = String::new();
    human.push_str("Authentication settings updated.\n");
    human.push_str(&format!(
        "Password Auth Enabled:  {}\n",
        resp.password_auth_enabled
    ));

    print_output(format, &human, &resp)
}

/// Show agent certificate settings.
pub async fn certificates_show(
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let resp = client.get_agent_certificate_settings().await.context_to()?;

    let mut human = String::new();
    human.push_str(&format!(
        "Lifetime (days):         {}\n",
        resp.lifetime_days
    ));
    human.push_str(&format!(
        "Renewal Window (hours):  {}\n",
        resp.renewal_window_hours
    ));

    print_output(format, &human, &resp)
}

/// Update agent certificate settings.
pub async fn certificates_update(
    lifetime_days: Option<u16>,
    renewal_window_hours: Option<u16>,
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = UpdateAgentCertificateSettingsRequest {
        lifetime_days,
        renewal_window_hours,
    };
    let resp = client
        .update_agent_certificate_settings(&req)
        .await
        .context_to()?;

    let mut human = String::new();
    human.push_str("Certificate settings updated.\n");
    human.push_str(&format!(
        "Lifetime (days):         {}\n",
        resp.lifetime_days
    ));
    human.push_str(&format!(
        "Renewal Window (hours):  {}\n",
        resp.renewal_window_hours
    ));

    print_output(format, &human, &resp)
}

/// Show network settings.
pub async fn network_show(
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let resp = client.get_network_settings().await.context_to()?;

    let mut human = String::new();
    human.push_str(&format!(
        "Trusted Proxies:     {}\n",
        resp.trusted_proxies.join(", ")
    ));
    human.push_str(&format!("Real IP Header:      {}\n", resp.real_ip_header));
    human.push_str(&format!(
        "Extra SANs:          {}\n",
        resp.extra_sans.join(", ")
    ));
    human.push_str(&format!("HTTPS Address:       {}\n", resp.https_addr));
    human.push_str(&format!(
        "Fwd Cert Info Header: {}\n",
        resp.forwarded_client_cert_info_header
            .as_deref()
            .unwrap_or("-")
    ));
    human.push_str(&format!(
        "Fwd Cert PEM Header: {}\n",
        resp.forwarded_client_cert_pem_header
            .as_deref()
            .unwrap_or("-")
    ));
    human.push_str(&format!(
        "PKI Address:         {}\n",
        resp.pki_addr.as_deref().unwrap_or("-")
    ));

    print_output(format, &human, &resp)
}

/// Parameters for updating network settings.
pub struct NetworkUpdateParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub format: OutputFormat,
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

/// Update network settings.
pub async fn network_update(params: NetworkUpdateParams<'_>) -> Result<()> {
    let client = authenticated_client(params.server, params.token, params.insecure, params.request_timeout)?;
    let req = UpdateNetworkSettingsRequest {
        trusted_proxies: params.trusted_proxies,
        real_ip_header: params.real_ip_header,
        extra_sans: params.extra_sans,
        https_addr: params.https_addr,
        forwarded_client_cert_info_header: params.fwd_cert_info_header,
        forwarded_client_cert_pem_header: params.fwd_cert_pem_header,
        pki_addr: params.pki_addr,
    };
    let resp = client.update_network_settings(&req).await.context_to()?;

    let mut human = String::new();
    human.push_str("Network settings updated.\n");
    human.push_str(&format!(
        "Trusted Proxies:     {}\n",
        resp.trusted_proxies.join(", ")
    ));
    human.push_str(&format!("Real IP Header:      {}\n", resp.real_ip_header));
    human.push_str(&format!(
        "Extra SANs:          {}\n",
        resp.extra_sans.join(", ")
    ));
    human.push_str(&format!("HTTPS Address:       {}\n", resp.https_addr));
    human.push_str(&format!(
        "Fwd Cert Info Header: {}\n",
        resp.forwarded_client_cert_info_header
            .as_deref()
            .unwrap_or("-")
    ));
    human.push_str(&format!(
        "Fwd Cert PEM Header: {}\n",
        resp.forwarded_client_cert_pem_header
            .as_deref()
            .unwrap_or("-")
    ));
    human.push_str(&format!(
        "PKI Address:         {}\n",
        resp.pki_addr.as_deref().unwrap_or("-")
    ));
    if let Some(ref warning) = resp.pki_addr_warning {
        human.push_str(&format!("Warning:             {warning}\n"));
    }

    print_output(params.format, &human, &resp)
}

/// Rotate the CA certificate.
pub async fn rotate_ca(
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let resp = client.rotate_ca().await.context_to()?;

    let human = format!("{}\n", resp.message);

    print_output(format, &human, &resp)
}

/// Renew the server TLS certificate.
pub async fn renew_server_cert(
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let resp = client.renew_server_certificate().await.context_to()?;

    let human = format!("{}\n", resp.message);

    print_output(format, &human, &resp)
}

/// List all MQTT client configurations.
pub async fn mqtt_list(
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let resp = client.list_mqtt_settings().await.context_to()?;

    let mut human = String::new();
    if resp.is_empty() {
        human.push_str("No MQTT configurations found.\n");
    } else {
        human.push_str(&format!(
            "{:<38} {:<8} {:<10} {:<25} {:<6} STATUS\n",
            "ID", "ENABLED", "TRANSPORT", "HOST", "PORT"
        ));
        for m in &resp {
            human.push_str(&format!(
                "{:<38} {:<8} {:<10} {:<25} {:<6} {}\n",
                m.id, m.enabled, m.transport, m.host, m.port, m.connection_status
            ));
        }
    }

    print_output(format, &human, &resp)
}

/// Show a single MQTT client configuration.
pub async fn mqtt_show(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let resp = client.get_mqtt_settings(id).await.context_to()?;

    let mut human = String::new();
    human.push_str(&format!("ID:            {}\n", resp.id));
    human.push_str(&format!("Enabled:       {}\n", resp.enabled));
    human.push_str(&format!("Transport:     {}\n", resp.transport));
    human.push_str(&format!("Host:          {}\n", resp.host));
    human.push_str(&format!("Port:          {}\n", resp.port));
    human.push_str(&format!("URL:           {}\n", resp.url));
    human.push_str(&format!("Client ID:     {}\n", resp.client_id));
    human.push_str(&format!(
        "Username:      {}\n",
        resp.username.as_deref().unwrap_or("-")
    ));
    human.push_str(&format!("Has Password:  {}\n", resp.has_password));
    human.push_str(&format!("Topic Prefix:  {}\n", resp.topic_prefix));
    human.push_str(&format!("Status:        {}\n", resp.connection_status));

    print_output(format, &human, &resp)
}

/// Parameters for creating an MQTT client configuration.
pub struct MqttCreateParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub format: OutputFormat,
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
}

/// Create a new MQTT client configuration.
pub async fn mqtt_create(params: MqttCreateParams<'_>) -> Result<()> {
    let client = authenticated_client(params.server, params.token, params.insecure, params.request_timeout)?;
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
    };
    let resp = client.create_mqtt_settings(&req).await.context_to()?;

    let mut human = String::new();
    human.push_str("MQTT configuration created.\n");
    human.push_str(&format!("ID:            {}\n", resp.id));
    human.push_str(&format!("Enabled:       {}\n", resp.enabled));
    human.push_str(&format!("URL:           {}\n", resp.url));
    human.push_str(&format!("Status:        {}\n", resp.connection_status));

    print_output(params.format, &human, &resp)
}

/// Parameters for updating an MQTT client configuration.
pub struct MqttUpdateParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub format: OutputFormat,
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
}

/// Update an existing MQTT client configuration.
pub async fn mqtt_update(params: MqttUpdateParams<'_>) -> Result<()> {
    let client = authenticated_client(params.server, params.token, params.insecure, params.request_timeout)?;
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
    };
    let resp = client
        .update_mqtt_settings(&params.id, &req)
        .await
        .context_to()?;

    let mut human = String::new();
    human.push_str("MQTT configuration updated.\n");
    human.push_str(&format!("ID:            {}\n", resp.id));
    human.push_str(&format!("Enabled:       {}\n", resp.enabled));
    human.push_str(&format!("URL:           {}\n", resp.url));
    human.push_str(&format!("Status:        {}\n", resp.connection_status));

    print_output(params.format, &human, &resp)
}

/// Delete an MQTT client configuration.
pub async fn mqtt_delete(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.delete_mqtt_settings(id).await.context_to()?;

    let value = serde_json::json!({"message": format!("MQTT configuration {id} deleted.")});
    let human = format!("MQTT configuration {id} deleted.\n");

    print_output(format, &human, &value)
}

/// Show MQTT client limit.
pub async fn mqtt_limit_show(
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let resp = client.get_mqtt_limit().await.context_to()?;

    let human = format!("Max Clients Per Tenant:  {}\n", resp.max_clients_per_tenant);

    print_output(format, &human, &resp)
}

/// Update MQTT client limit.
pub async fn mqtt_limit_update(
    max: u16,
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = UpdateMqttLimitRequest {
        max_clients_per_tenant: max,
    };
    let resp = client.update_mqtt_limit(&req).await.context_to()?;

    let mut human = String::new();
    human.push_str("MQTT limit updated.\n");
    human.push_str(&format!(
        "Max Clients Per Tenant:  {}\n",
        resp.max_clients_per_tenant
    ));

    print_output(format, &human, &resp)
}

/// List all OIDC providers.
pub async fn oidc_list(
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let resp = client.list_oidc_providers().await.context_to()?;

    let mut human = String::new();
    if resp.is_empty() {
        human.push_str("No OIDC providers found.\n");
    } else {
        human.push_str(&format!(
            "{:<38} {:<20} {:<15} ACTIVE\n",
            "ID", "NAME", "SLUG"
        ));
        for p in &resp {
            human.push_str(&format!(
                "{:<38} {:<20} {:<15} {}\n",
                p.id, p.name, p.slug, p.is_active
            ));
        }
    }

    print_output(format, &human, &resp)
}

/// Show a single OIDC provider.
pub async fn oidc_show(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let resp = client.get_oidc_provider(id).await.context_to()?;

    let mut human = String::new();
    human.push_str(&format!("ID:               {}\n", resp.id));
    human.push_str(&format!("Name:             {}\n", resp.name));
    human.push_str(&format!("Slug:             {}\n", resp.slug));
    if let Some(ref logo) = resp.logo_url {
        human.push_str(&format!("Logo URL:         {}\n", logo));
    }
    human.push_str(&format!("Issuer URL:       {}\n", resp.issuer_url));
    human.push_str(&format!("Client ID:        {}\n", resp.client_id));
    human.push_str(&format!("Has Secret:       {}\n", resp.has_client_secret));
    human.push_str(&format!("Scopes:           {}\n", resp.scopes));
    human.push_str(&format!("Auto Create Users: {}\n", resp.auto_create_users));
    if let Some(ref path) = resp.role_claim_path {
        human.push_str(&format!("Role Claim Path:  {}\n", path));
    }
    if !resp.role_mapping.is_empty() {
        let mappings: Vec<String> = resp
            .role_mapping
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        human.push_str(&format!("Role Mapping:     {}\n", mappings.join(", ")));
    }
    human.push_str(&format!("Active:           {}\n", resp.is_active));
    human.push_str(&format!("Created:          {}\n", resp.created_at));
    human.push_str(&format!("Updated:          {}\n", resp.updated_at));

    print_output(format, &human, &resp)
}

/// Parameters for creating an OIDC provider.
pub struct OidcCreateParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub format: OutputFormat,
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

/// Create a new OIDC provider.
pub async fn oidc_create(params: OidcCreateParams<'_>) -> Result<()> {
    let client = authenticated_client(params.server, params.token, params.insecure, params.request_timeout)?;
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
    let resp = client.create_oidc_provider(&req).await.context_to()?;

    let mut human = String::new();
    human.push_str("OIDC provider created.\n");
    human.push_str(&format!("ID:     {}\n", resp.id));
    human.push_str(&format!("Name:   {}\n", resp.name));
    human.push_str(&format!("Slug:   {}\n", resp.slug));
    human.push_str(&format!("Active: {}\n", resp.is_active));

    print_output(params.format, &human, &resp)
}

/// Parameters for updating an OIDC provider.
pub struct OidcUpdateParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub format: OutputFormat,
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

/// Update an existing OIDC provider.
pub async fn oidc_update(params: OidcUpdateParams<'_>) -> Result<()> {
    let client = authenticated_client(params.server, params.token, params.insecure, params.request_timeout)?;
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
    let resp = client
        .update_oidc_provider(&params.id, &req)
        .await
        .context_to()?;

    let mut human = String::new();
    human.push_str("OIDC provider updated.\n");
    human.push_str(&format!("ID:     {}\n", resp.id));
    human.push_str(&format!("Name:   {}\n", resp.name));
    human.push_str(&format!("Slug:   {}\n", resp.slug));
    human.push_str(&format!("Active: {}\n", resp.is_active));

    print_output(params.format, &human, &resp)
}

/// Delete an OIDC provider.
pub async fn oidc_delete(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.delete_oidc_provider(id).await.context_to()?;

    let value = serde_json::json!({"message": format!("OIDC provider {id} deleted.")});
    let human = format!("OIDC provider {id} deleted.\n");

    print_output(format, &human, &value)
}

/// Activate an OIDC provider.
pub async fn oidc_activate(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let resp = client.activate_oidc_provider(id).await.context_to()?;

    let mut human = String::new();
    human.push_str(&format!("OIDC provider {} activated.\n", resp.id));
    human.push_str(&format!("Name:   {}\n", resp.name));
    human.push_str(&format!("Active: {}\n", resp.is_active));

    print_output(format, &human, &resp)
}

/// Deactivate an OIDC provider.
pub async fn oidc_deactivate(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let resp = client.deactivate_oidc_provider(id).await.context_to()?;

    let mut human = String::new();
    human.push_str(&format!("OIDC provider {} deactivated.\n", resp.id));
    human.push_str(&format!("Name:   {}\n", resp.name));
    human.push_str(&format!("Active: {}\n", resp.is_active));

    print_output(format, &human, &resp)
}

/// Show system alerts.
pub async fn alerts(
    server: Option<&str>,
    token: Option<&str>,
    format: OutputFormat,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<()> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let resp = client.get_system_alerts().await.context_to()?;

    let mut human = String::new();
    if resp.alerts.is_empty() {
        human.push_str("No active alerts.\n");
    } else {
        human.push_str(&format!("{:<10} {:<30} MESSAGE\n", "SEVERITY", "TITLE"));
        for alert in &resp.alerts {
            human.push_str(&format!(
                "{:<10} {:<30} {}\n",
                alert.severity, alert.title, alert.message
            ));
        }
    }

    print_output(format, &human, &resp)
}
