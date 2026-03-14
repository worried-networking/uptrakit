use std::collections::HashMap;

use crate::client::authenticated_client;
use crate::commands::CliContext;
use crate::error::Result;
use crate::output::HumanOutput;
use clap::Subcommand;
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
use uptrakit_openapi_client::types::settings_nats::{
    NatsSettingsResponse, UpdateNatsSettingsRequest,
};
use uptrakit_openapi_client::types::settings_network::{
    NetworkSettingsResponse, UpdateNetworkSettingsRequest,
};
use uptrakit_openapi_client::types::settings_reset::{ResetDataRequest, ResetDataResponse};
use uptrakit_openapi_client::types::settings_smtp::{
    SmtpSettingsResponse, UpdateSmtpSettingsRequest,
};
use uptrakit_openapi_client::types::system_alerts::SystemAlertsResponse;

#[derive(Debug, Subcommand)]
pub enum SettingsCommands {
    /// Show combined settings overview
    Show,
    /// Registration settings
    Registration {
        #[command(subcommand)]
        command: RegistrationCommands,
    },
    /// Authentication settings
    Authentication {
        #[command(subcommand)]
        command: AuthenticationCommands,
    },
    /// Agent certificate settings
    Certificates {
        #[command(subcommand)]
        command: CertificateCommands,
    },
    /// Network settings
    Network {
        #[command(subcommand)]
        command: NetworkCommands,
    },
    /// Rotate the CA certificate
    RotateCa,
    /// Renew the server TLS certificate
    RenewServerCert,
    /// MQTT client configuration
    Mqtt {
        #[command(subcommand)]
        command: MqttCommands,
    },
    /// OIDC provider management
    Oidc {
        #[command(subcommand)]
        command: OidcCommands,
    },
    /// Show system alerts
    Alerts,
    /// SMTP settings for email notifications
    Smtp {
        #[command(subcommand)]
        command: SmtpCommands,
    },
    /// NATS server URL configuration
    Nats {
        #[command(subcommand)]
        command: NatsCommands,
    },
    /// Reset all tenant-scoped data (hosts, software items, plugin configs, etc.)
    ResetData {
        /// Confirm the destructive operation (required)
        #[arg(long)]
        confirm: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum RegistrationCommands {
    /// Show registration settings
    Show,
    /// Update registration settings
    Update {
        /// Registration mode (open, invite, closed)
        #[arg(long, value_parser = super::parse_registration_mode)]
        mode: RegistrationMode,
        /// Registration token (required for invite mode)
        #[arg(long)]
        token: Option<String>,
        /// Whether OIDC users also need a registration token
        #[arg(long)]
        require_token_for_oidc: Option<bool>,
    },
}

#[derive(Debug, Subcommand)]
pub enum AuthenticationCommands {
    /// Show authentication settings
    Show,
    /// Update authentication settings
    Update {
        /// Enable or disable password authentication
        #[arg(long)]
        password_auth_enabled: Option<bool>,
    },
}

#[derive(Debug, Subcommand)]
pub enum CertificateCommands {
    /// Show agent certificate settings
    Show,
    /// Update agent certificate settings
    Update {
        /// Certificate lifetime in hours (max 17520)
        #[arg(long)]
        lifetime_hours: Option<u32>,
        /// Certificate renewal window in hours (use 0 to reset to automatic: min(14 days, lifetime/5))
        #[arg(long)]
        renewal_window_hours: Option<u16>,
    },
}

#[derive(Debug, Subcommand)]
pub enum NetworkCommands {
    /// Show network settings
    Show,
    /// Update network settings
    Update {
        /// Comma-separated trusted proxy CIDRs
        #[arg(long)]
        trusted_proxies: Option<String>,
        /// Header name for extracting real client IP
        #[arg(long)]
        real_ip_header: Option<String>,
        /// Comma-separated Subject Alternative Names for the server certificate
        #[arg(long)]
        sans: Option<String>,
        /// HTTPS listen address
        #[arg(long)]
        https_addr: Option<String>,
        /// Header for forwarded client cert info
        #[arg(long)]
        fwd_cert_info_header: Option<String>,
        /// Header for forwarded client cert PEM
        #[arg(long)]
        fwd_cert_pem_header: Option<String>,
        /// PKI address for OCSP/CRL/CA cert
        #[arg(long)]
        pki_addr: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum MqttCommands {
    /// List MQTT client configurations
    List,
    /// Show MQTT client configuration details
    Show {
        /// MQTT configuration UUID
        id: Uuid,
    },
    /// Create a new MQTT client configuration
    Create {
        /// MQTT URL (e.g. mqtt://broker:1883)
        #[arg(long)]
        url: Option<String>,
        /// Transport type (tcp, tls)
        #[arg(long)]
        transport: Option<String>,
        /// Broker hostname
        #[arg(long)]
        host: Option<String>,
        /// Broker port
        #[arg(long)]
        port: Option<u16>,
        /// Enable or disable
        #[arg(long)]
        enabled: Option<bool>,
        /// MQTT client ID
        #[arg(long)]
        client_id: Option<String>,
        /// MQTT username
        #[arg(long)]
        username: Option<String>,
        /// MQTT password
        #[arg(long)]
        password: Option<String>,
        /// Custom CA certificate in PEM format (for private brokers)
        #[arg(long, conflicts_with = "ca_pem_file")]
        ca_pem: Option<String>,
        /// Path to a PEM file containing a custom CA certificate (for private brokers)
        #[arg(long, conflicts_with = "ca_pem")]
        ca_pem_file: Option<std::path::PathBuf>,
        /// Topic prefix (e.g. uptrakit)
        #[arg(long)]
        topic_prefix: Option<String>,
        /// Enable Home Assistant MQTT discovery
        #[arg(long = "ha-discovery", action = clap::ArgAction::SetTrue, default_value_t = false)]
        ha_discovery: bool,
        /// Disable Home Assistant MQTT discovery
        #[arg(long = "no-ha-discovery", action = clap::ArgAction::SetTrue, default_value_t = false, conflicts_with = "ha_discovery")]
        no_ha_discovery: bool,
        /// Home Assistant discovery topic prefix (default: homeassistant)
        #[arg(long)]
        ha_discovery_prefix: Option<String>,
    },
    /// Update an MQTT client configuration
    Update {
        /// MQTT configuration UUID
        id: Uuid,
        /// MQTT URL
        #[arg(long)]
        url: Option<String>,
        /// Transport type (tcp, tls)
        #[arg(long)]
        transport: Option<String>,
        /// Broker hostname
        #[arg(long)]
        host: Option<String>,
        /// Broker port
        #[arg(long)]
        port: Option<u16>,
        /// Enable or disable
        #[arg(long)]
        enabled: Option<bool>,
        /// MQTT client ID
        #[arg(long)]
        client_id: Option<String>,
        /// MQTT username
        #[arg(long)]
        username: Option<String>,
        /// MQTT password
        #[arg(long)]
        password: Option<String>,
        /// Custom CA certificate in PEM format (for private brokers)
        #[arg(long, conflicts_with = "ca_pem_file")]
        ca_pem: Option<String>,
        /// Path to a PEM file containing a custom CA certificate (for private brokers)
        #[arg(long, conflicts_with = "ca_pem")]
        ca_pem_file: Option<std::path::PathBuf>,
        /// Topic prefix
        #[arg(long)]
        topic_prefix: Option<String>,
        /// Enable Home Assistant MQTT discovery
        #[arg(long = "ha-discovery", action = clap::ArgAction::SetTrue, default_value_t = false)]
        ha_discovery: bool,
        /// Disable Home Assistant MQTT discovery
        #[arg(long = "no-ha-discovery", action = clap::ArgAction::SetTrue, default_value_t = false, conflicts_with = "ha_discovery")]
        no_ha_discovery: bool,
        /// Home Assistant discovery topic prefix (default: homeassistant)
        #[arg(long)]
        ha_discovery_prefix: Option<String>,
    },
    /// Delete an MQTT client configuration
    Delete {
        /// MQTT configuration UUID
        id: Uuid,
    },
    /// MQTT client limit management
    Limit {
        #[command(subcommand)]
        command: MqttLimitCommands,
    },
}

#[derive(Debug, Subcommand)]
pub enum MqttLimitCommands {
    /// Show MQTT client limit
    Show,
    /// Update MQTT client limit
    Update {
        /// Maximum MQTT clients per tenant
        #[arg(long)]
        max: u16,
    },
}

#[derive(Debug, Subcommand)]
pub enum OidcCommands {
    /// List OIDC providers
    List,
    /// Show OIDC provider details
    Show {
        /// OIDC provider UUID
        id: Uuid,
    },
    /// Create a new OIDC provider
    Create {
        /// Provider display name
        #[arg(long)]
        name: String,
        /// URL-safe slug
        #[arg(long)]
        slug: String,
        /// Logo URL
        #[arg(long)]
        logo_url: Option<String>,
        /// OIDC issuer URL
        #[arg(long)]
        issuer_url: String,
        /// OAuth client ID
        #[arg(long)]
        client_id: String,
        /// OAuth client secret
        #[arg(long)]
        client_secret: String,
        /// OAuth scopes (default: "openid email profile groups")
        #[arg(long)]
        scopes: Option<String>,
        /// Auto-create users on first login
        #[arg(long)]
        auto_create_users: Option<bool>,
        /// JSONPath for role claim
        #[arg(long)]
        role_claim_path: Option<String>,
    },
    /// Update an OIDC provider
    Update {
        /// OIDC provider UUID
        id: Uuid,
        /// Provider display name
        #[arg(long)]
        name: Option<String>,
        /// URL-safe slug
        #[arg(long)]
        slug: Option<String>,
        /// Logo URL
        #[arg(long)]
        logo_url: Option<String>,
        /// OIDC issuer URL
        #[arg(long)]
        issuer_url: Option<String>,
        /// OAuth client ID
        #[arg(long)]
        client_id: Option<String>,
        /// OAuth client secret
        #[arg(long)]
        client_secret: Option<String>,
        /// OAuth scopes
        #[arg(long)]
        scopes: Option<String>,
        /// Auto-create users on first login
        #[arg(long)]
        auto_create_users: Option<bool>,
        /// JSONPath for role claim
        #[arg(long)]
        role_claim_path: Option<String>,
    },
    /// Delete an OIDC provider
    Delete {
        /// OIDC provider UUID
        id: Uuid,
    },
    /// Activate an OIDC provider
    Activate {
        /// OIDC provider UUID
        id: Uuid,
    },
    /// Deactivate an OIDC provider
    Deactivate {
        /// OIDC provider UUID
        id: Uuid,
    },
}

#[derive(Debug, Subcommand)]
pub enum SmtpCommands {
    /// Show current SMTP settings
    Show,
    /// Update SMTP settings
    Set {
        /// SMTP server hostname
        #[arg(long)]
        host: Option<String>,
        /// SMTP server port (default: 587)
        #[arg(long)]
        port: Option<u16>,
        /// SMTP username
        #[arg(long)]
        username: Option<String>,
        /// Clear the saved username
        #[arg(long, conflicts_with = "username")]
        clear_username: bool,
        /// SMTP password
        #[arg(long)]
        password: Option<String>,
        /// Clear the saved password
        #[arg(long, conflicts_with = "password")]
        clear_password: bool,
        /// Sender email address
        #[arg(long)]
        from_address: Option<String>,
        /// Sender display name
        #[arg(long)]
        from_name: Option<String>,
        /// Clear the saved sender display name
        #[arg(long, conflicts_with = "from_name")]
        clear_from_name: bool,
        /// TLS mode: starttls, tls, or none (default: starttls)
        #[arg(long)]
        tls_mode: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum NatsCommands {
    /// Show current NATS server URL configuration
    Show,
    /// Set the NATS server URL
    Set {
        /// NATS server URL (e.g. nats://host:4222 or nats://user:password@host:4222)
        #[arg(long)]
        url: String,
    },
    /// Clear the stored NATS server URL
    Clear,
}

pub async fn dispatch(command: SettingsCommands, ctx: &CliContext) -> Result<()> {
    match command {
        SettingsCommands::Show => {
            let resp = show_combined(
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        SettingsCommands::Registration { command } => {
            dispatch_registration(command, ctx).await?;
        }
        SettingsCommands::Authentication { command } => {
            dispatch_authentication(command, ctx).await?;
        }
        SettingsCommands::Certificates { command } => {
            dispatch_certificates(command, ctx).await?;
        }
        SettingsCommands::Network { command } => dispatch_network(command, ctx).await?,
        SettingsCommands::RotateCa => {
            let resp = rotate_ca(
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        SettingsCommands::RenewServerCert => {
            let resp = renew_server_cert(
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        SettingsCommands::Mqtt { command } => dispatch_mqtt(command, ctx).await?,
        SettingsCommands::Oidc { command } => dispatch_oidc(command, ctx).await?,
        SettingsCommands::Alerts => {
            let resp = alerts(
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        SettingsCommands::Smtp { command } => dispatch_smtp(command, ctx).await?,
        SettingsCommands::Nats { command } => dispatch_nats(command, ctx).await?,
        SettingsCommands::ResetData { confirm } => {
            if !confirm {
                eprintln!(
                    "Error: You must pass --confirm to reset all data. This action is irreversible."
                );
                std::process::exit(1);
            }
            let resp = reset_data(
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
    }
    Ok(())
}

async fn dispatch_registration(command: RegistrationCommands, ctx: &CliContext) -> Result<()> {
    match command {
        RegistrationCommands::Show => {
            let resp = registration_show(
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        RegistrationCommands::Update {
            mode,
            token,
            require_token_for_oidc,
        } => {
            let resp = registration_update(RegistrationUpdateParams {
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                mode,
                reg_token: token,
                require_token_for_oidc,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
    }
    Ok(())
}

async fn dispatch_authentication(command: AuthenticationCommands, ctx: &CliContext) -> Result<()> {
    match command {
        AuthenticationCommands::Show => {
            let resp = authentication_show(
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        AuthenticationCommands::Update {
            password_auth_enabled,
        } => {
            let resp = authentication_update(
                password_auth_enabled,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
    }
    Ok(())
}

async fn dispatch_certificates(command: CertificateCommands, ctx: &CliContext) -> Result<()> {
    match command {
        CertificateCommands::Show => {
            let resp = certificates_show(
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        CertificateCommands::Update {
            lifetime_hours,
            renewal_window_hours,
        } => {
            let resp = certificates_update(
                lifetime_hours,
                renewal_window_hours,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
    }
    Ok(())
}

async fn dispatch_network(command: NetworkCommands, ctx: &CliContext) -> Result<()> {
    match command {
        NetworkCommands::Show => {
            let resp = network_show(
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        NetworkCommands::Update {
            trusted_proxies,
            real_ip_header,
            sans,
            https_addr,
            fwd_cert_info_header,
            fwd_cert_pem_header,
            pki_addr,
        } => {
            let resp = network_update(NetworkUpdateParams {
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                trusted_proxies: trusted_proxies
                    .map(|s| s.split(',').map(|v| v.trim().to_string()).collect()),
                real_ip_header,
                sans: sans.map(|s| s.split(',').map(|v| v.trim().to_string()).collect()),
                https_addr,
                fwd_cert_info_header,
                fwd_cert_pem_header,
                pki_addr,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
    }
    Ok(())
}

async fn dispatch_smtp(command: SmtpCommands, ctx: &CliContext) -> Result<()> {
    match command {
        SmtpCommands::Show => {
            let resp = smtp_show(
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        SmtpCommands::Set {
            host,
            port,
            username,
            clear_username,
            password,
            clear_password,
            from_address,
            from_name,
            clear_from_name,
            tls_mode,
        } => {
            let resp = smtp_set(SmtpSetParams {
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
                host,
                port,
                username,
                clear_username,
                password,
                clear_password,
                from_address,
                from_name,
                clear_from_name,
                tls_mode,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
    }
    Ok(())
}

async fn dispatch_nats(command: NatsCommands, ctx: &CliContext) -> Result<()> {
    match command {
        NatsCommands::Show => {
            let resp = nats_show(
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        NatsCommands::Set { url } => {
            let resp = nats_set(
                url,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
            eprintln!(
                "NATS URL updated. The change will take effect after the controller is restarted."
            );
        }
        NatsCommands::Clear => {
            let resp = nats_clear(
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
            eprintln!(
                "NATS URL cleared. The change will take effect after the controller is restarted."
            );
        }
    }
    Ok(())
}

fn resolve_ha_discovery_flag(ha_discovery: bool, no_ha_discovery: bool) -> Option<bool> {
    if ha_discovery {
        Some(true)
    } else if no_ha_discovery {
        Some(false)
    } else {
        None
    }
}

async fn dispatch_mqtt(command: MqttCommands, ctx: &CliContext) -> Result<()> {
    match command {
        MqttCommands::List => {
            let resp = mqtt_list(
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        MqttCommands::Show { id } => {
            let resp = mqtt_show(
                &id,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        MqttCommands::Create {
            url,
            transport,
            host,
            port,
            enabled,
            client_id,
            username,
            password,
            ca_pem,
            ca_pem_file,
            topic_prefix,
            ha_discovery,
            no_ha_discovery,
            ha_discovery_prefix,
        } => {
            let ca_pem = super::resolve_ca_pem(ca_pem, ca_pem_file)?;
            let ha_discovery_flag = resolve_ha_discovery_flag(ha_discovery, no_ha_discovery);
            let resp = mqtt_create(MqttCreateParams {
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                url,
                transport,
                host,
                port,
                enabled,
                client_id,
                username,
                password,
                ca_pem,
                topic_prefix,
                ha_discovery: ha_discovery_flag,
                ha_discovery_prefix,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        MqttCommands::Update {
            id,
            url,
            transport,
            host,
            port,
            enabled,
            client_id,
            username,
            password,
            ca_pem,
            ca_pem_file,
            topic_prefix,
            ha_discovery,
            no_ha_discovery,
            ha_discovery_prefix,
        } => {
            let ca_pem = super::resolve_ca_pem(ca_pem, ca_pem_file)?;
            let ha_discovery_flag = resolve_ha_discovery_flag(ha_discovery, no_ha_discovery);
            let resp = mqtt_update(MqttUpdateParams {
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                id,
                url,
                transport,
                host,
                port,
                enabled,
                client_id,
                username,
                password,
                ca_pem,
                topic_prefix,
                ha_discovery: ha_discovery_flag,
                ha_discovery_prefix,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        MqttCommands::Delete { id } => {
            let resp = mqtt_delete(
                &id,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        MqttCommands::Limit { command } => match command {
            MqttLimitCommands::Show => {
                let resp = mqtt_limit_show(
                    ctx.server.as_deref(),
                    ctx.token.as_deref(),
                    ctx.insecure,
                    ctx.request_timeout,
                )
                .await?;
                crate::output::print_output(ctx.format, &resp)?;
            }
            MqttLimitCommands::Update { max } => {
                let resp = mqtt_limit_update(
                    max,
                    ctx.server.as_deref(),
                    ctx.token.as_deref(),
                    ctx.insecure,
                    ctx.request_timeout,
                )
                .await?;
                crate::output::print_output(ctx.format, &resp)?;
            }
        },
    }
    Ok(())
}

async fn dispatch_oidc(command: OidcCommands, ctx: &CliContext) -> Result<()> {
    match command {
        OidcCommands::List => {
            let resp = oidc_list(
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        OidcCommands::Show { id } => {
            let resp = oidc_show(
                &id,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        OidcCommands::Create {
            name,
            slug,
            logo_url,
            issuer_url,
            client_id,
            client_secret,
            scopes,
            auto_create_users,
            role_claim_path,
        } => {
            let resp = oidc_create(OidcCreateParams {
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                name,
                slug,
                logo_url,
                issuer_url,
                client_id,
                client_secret,
                scopes,
                auto_create_users,
                role_claim_path,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        OidcCommands::Update {
            id,
            name,
            slug,
            logo_url,
            issuer_url,
            client_id,
            client_secret,
            scopes,
            auto_create_users,
            role_claim_path,
        } => {
            let resp = oidc_update(OidcUpdateParams {
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                id,
                name,
                slug,
                logo_url,
                issuer_url,
                client_id,
                client_secret,
                scopes,
                auto_create_users,
                role_claim_path,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        OidcCommands::Delete { id } => {
            let resp = oidc_delete(
                &id,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        OidcCommands::Activate { id } => {
            let resp = oidc_activate(
                &id,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        OidcCommands::Deactivate { id } => {
            let resp = oidc_deactivate(
                &id,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
    }
    Ok(())
}

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
            "  Lifetime (hours):        {}\n",
            self.agent_certificates.lifetime_hours
        ));
        let window_desc = match self.agent_certificates.renewal_window_hours_override {
            None => format!(
                "automatic ({} hours)",
                self.agent_certificates.effective_renewal_window_hours
            ),
            Some(h) => format!("{h} hours (custom override)"),
        };
        out.push_str(&format!("  Renewal Window:          {window_desc}\n"));
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
            "Lifetime (hours):        {}\n",
            self.lifetime_hours
        ));
        let window_desc = match self.renewal_window_hours_override {
            None => format!(
                "automatic ({} hours, 1/5 of lifetime capped at 14 days)",
                self.effective_renewal_window_hours
            ),
            Some(h) => format!("{h} hours (custom override)"),
        };
        out.push_str(&format!("Renewal Window:          {window_desc}\n"));
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
        out.push_str(&format!("SANs:                {}\n", self.sans.join(", ")));
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
        if self.cert_regenerated == Some(true) {
            out.push_str("Cert Regenerated:    yes\n");
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

impl HumanOutput for SmtpSettingsResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "Host:          {}\n",
            self.host.as_deref().unwrap_or("-")
        ));
        out.push_str(&format!(
            "Port:          {}\n",
            self.port.map_or("-".to_string(), |p| p.to_string())
        ));
        out.push_str(&format!(
            "Username:      {}\n",
            self.username.as_deref().unwrap_or("-")
        ));
        out.push_str(&format!("Has Password:  {}\n", self.has_password));
        out.push_str(&format!(
            "From Address:  {}\n",
            self.from_address.as_deref().unwrap_or("-")
        ));
        out.push_str(&format!(
            "From Name:     {}\n",
            self.from_name.as_deref().unwrap_or("-")
        ));
        out.push_str(&format!("TLS Mode:      {}\n", self.tls_mode));
        out
    }
}

impl HumanOutput for NatsSettingsResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "URL:      {}\n",
            self.url
                .as_ref()
                .map(|u| u.to_string())
                .unwrap_or_else(|| "-".to_string())
        ));
        out.push_str(&format!("Has URL:  {}\n", self.has_url));
        out
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

impl HumanOutput for ResetDataResponse {
    fn to_human_string(&self) -> String {
        let d = &self.deleted;
        let mut out = String::from("Data reset completed:\n");
        out.push_str(&format!("  Hosts deleted:          {}\n", d.hosts));
        out.push_str(&format!("  Software items deleted: {}\n", d.software_items));
        out.push_str(&format!("  Plugin configs deleted: {}\n", d.plugin_configs));
        out.push_str(&format!("  Host tags deleted:      {}\n", d.host_tags));
        out.push_str(&format!("  Update history deleted: {}\n", d.update_history));
        out.push_str(&format!("  Update batches deleted: {}\n", d.update_batches));
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
    pub sans: Option<Vec<String>>,
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
    lifetime_hours: Option<u32>,
    renewal_window_hours: Option<u16>,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<AgentCertificateSettingsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = UpdateAgentCertificateSettingsRequest {
        lifetime_hours,
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
        sans: params.sans,
        https_addr: params.https_addr,
        forwarded_client_cert_info_header: params.fwd_cert_info_header,
        forwarded_client_cert_pem_header: params.fwd_cert_pem_header,
        pki_addr: params.pki_addr,
        regenerate_cert: None,
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

/// Parameters for setting SMTP configuration.
pub struct SmtpSetParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub clear_username: bool,
    pub password: Option<String>,
    pub clear_password: bool,
    pub from_address: Option<String>,
    pub from_name: Option<String>,
    pub clear_from_name: bool,
    pub tls_mode: Option<String>,
}

/// Show current SMTP settings.
pub async fn smtp_show(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<SmtpSettingsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.get_smtp_settings().await.context_to()
}

/// Update SMTP settings.
pub async fn smtp_set(params: SmtpSetParams<'_>) -> Result<SmtpSettingsResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let username = if params.clear_username {
        Some(serde_json::Value::Null)
    } else {
        params.username.map(serde_json::Value::String)
    };
    let password = if params.clear_password {
        Some(serde_json::Value::Null)
    } else {
        params.password.map(serde_json::Value::String)
    };
    let from_name = if params.clear_from_name {
        Some(serde_json::Value::Null)
    } else {
        params.from_name.map(serde_json::Value::String)
    };
    let req = UpdateSmtpSettingsRequest {
        host: params.host,
        port: params.port,
        username,
        password,
        from_address: params.from_address,
        from_name,
        tls_mode: params.tls_mode,
    };
    client.update_smtp_settings(&req).await.context_to()
}

/// Show current NATS settings.
pub async fn nats_show(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<NatsSettingsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.get_nats_settings().await.context_to()
}

/// Set the NATS URL.
pub async fn nats_set(
    url: String,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<NatsSettingsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = UpdateNatsSettingsRequest {
        url: Some(serde_json::Value::String(url)),
    };
    client.update_nats_settings(&req).await.context_to()
}

/// Clear the NATS URL.
pub async fn nats_clear(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<NatsSettingsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = UpdateNatsSettingsRequest {
        url: Some(serde_json::Value::Null),
    };
    client.update_nats_settings(&req).await.context_to()
}

/// Reset all tenant-scoped data.
pub async fn reset_data(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<ResetDataResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = ResetDataRequest {
        confirm: "RESET".to_string(),
    };
    client.reset_data(&req).await.context_to()
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
    use uptrakit_openapi_client::types::settings_reset::ResetDeletedCounts;
    use uptrakit_openapi_client::types::system_alerts::{AlertSeverity, SystemAlert};

    #[test]
    fn reset_data_human_output() {
        let resp = ResetDataResponse {
            deleted: ResetDeletedCounts {
                hosts: 5,
                software_items: 10,
                plugin_configs: 3,
                host_tags: 2,
                update_history: 100,
                update_batches: 4,
            },
        };
        let s = resp.to_human_string();
        assert!(s.contains("Data reset completed"), "header missing");
        assert!(s.contains("5"), "hosts count missing");
        assert!(s.contains("10"), "software_items count missing");
        assert!(s.contains("100"), "update_history count missing");
    }

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
    fn certificate_settings_human_output_auto_mode() {
        // 8760 h = 365 days
        let resp = AgentCertificateSettingsResponse {
            lifetime_hours: 8760,
            renewal_window_hours_override: None,
            effective_renewal_window_hours: 336,
        };
        let s = resp.to_human_string();
        assert!(s.contains("8760"), "lifetime_hours missing");
        assert!(s.contains("336"), "effective hours missing");
        assert!(s.contains("automatic"), "auto mode indicator missing");
    }

    #[test]
    fn certificate_settings_human_output_custom_override() {
        let resp = AgentCertificateSettingsResponse {
            lifetime_hours: 8760,
            renewal_window_hours_override: Some(72),
            effective_renewal_window_hours: 72,
        };
        let s = resp.to_human_string();
        assert!(s.contains("8760"), "lifetime_hours missing");
        assert!(s.contains("72"), "override hours missing");
        assert!(s.contains("custom"), "custom indicator missing");
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
                lifetime_hours: 8760,
                renewal_window_hours_override: None,
                effective_renewal_window_hours: 336,
            },
            enrollment_tokens: EnrollmentTokensSummary { active_count: 3 },
        };
        let s = resp.to_human_string();
        assert!(s.contains("Registration"), "registration section missing");
        assert!(
            s.contains("Authentication"),
            "authentication section missing"
        );
        assert!(s.contains("8760"), "lifetime_hours missing");
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
            sans: vec![],
            https_addr: "0.0.0.0:443".to_string(),
            forwarded_client_cert_info_header: None,
            forwarded_client_cert_pem_header: None,
            pki_addr: Some("https://pki.example.com".to_string()),
            pki_addr_warning: Some("CA rotation required".to_string()),
            cert_regenerated: None,
        };
        let s = resp.to_human_string();
        assert!(s.contains("10.0.0.0/8"), "trusted proxies missing");
        assert!(s.contains("pki.example.com"), "pki_addr missing");
        assert!(s.contains("CA rotation required"), "warning missing");
    }

    #[test]
    fn nats_settings_human_output_with_url() {
        let resp = NatsSettingsResponse {
            url: Some(uptrakit_openapi_client::types::MaskedUrl::new(
                "nats://user:secret@host:4222",
            )),
            has_url: true,
        };
        let s = resp.to_human_string();
        assert!(
            s.contains("has_url") || s.contains("Has URL"),
            "has_url missing"
        );
        // Password must not appear
        assert!(!s.contains("secret"), "password must not appear in output");
        assert!(s.contains("***"), "masked password must appear");
    }

    #[test]
    fn nats_settings_human_output_no_url() {
        let resp = NatsSettingsResponse {
            url: None,
            has_url: false,
        };
        let s = resp.to_human_string();
        assert!(
            s.contains('-') || s.contains("false"),
            "empty state should show"
        );
    }
}
