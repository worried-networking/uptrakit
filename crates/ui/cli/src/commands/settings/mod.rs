pub mod authentication;
pub mod certificates;
pub mod nats;
pub mod network;
pub mod oidc;
pub mod provider_github;
pub mod registration;

pub use authentication::AuthenticationCommands;
pub use certificates::CertificateCommands;
pub use nats::NatsCommands;
pub use network::NetworkCommands;
pub use oidc::OidcCommands;
pub use provider_github::ProviderGithubCommands;
pub use registration::RegistrationCommands;

use crate::client::authenticated_client;
use crate::commands::CliContext;
use crate::error::Result;
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;
use serde::Serialize;
use uptrakit_openapi_client::types::server_cert::RenewServerCertResponse;
use uptrakit_openapi_client::types::settings_ca::RotateCaResponse;
use uptrakit_openapi_client::types::settings_combined::CombinedSettingsResponse;
use uptrakit_openapi_client::types::settings_reset::{ResetDataRequest, ResetDataResponse};
use uptrakit_openapi_client::types::system_alerts::SystemAlertsResponse;

use self::authentication::{authentication_show, authentication_update};
use self::certificates::{certificates_show, certificates_update};
use self::nats::{nats_clear, nats_set, nats_show};
use self::network::{NetworkUpdateParams, network_show, network_update};
use self::oidc::dispatch_oidc;
use self::provider_github::{provider_github_clear, provider_github_set, provider_github_show};
use self::registration::{RegistrationUpdateParams, registration_show, registration_update};

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
    /// OIDC provider management
    Oidc {
        #[command(subcommand)]
        command: OidcCommands,
    },
    /// Show system alerts
    Alerts,
    /// NATS server URL configuration
    Nats {
        #[command(subcommand)]
        command: NatsCommands,
    },
    /// Shared GitHub provider defaults
    ProviderGithub {
        #[command(subcommand)]
        command: ProviderGithubCommands,
    },
    /// Reset all tenant-scoped data (hosts, software items, plugin configs, etc.)
    ResetData {
        /// Confirm the destructive operation (required)
        #[arg(long)]
        confirm: bool,
    },
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
        out.push_str(&format!(
            "  Multi-Tenancy Enabled:   {}\n",
            self.multi_tenancy_enabled
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

// ── Dispatch ─────────────────────────────────────────────────────────────────

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
        SettingsCommands::Registration { command } => match command {
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
        },
        SettingsCommands::Authentication { command } => match command {
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
        },
        SettingsCommands::Certificates { command } => match command {
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
        },
        SettingsCommands::Network { command } => match command {
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
        },
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
        SettingsCommands::Nats { command } => match command {
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
        },
        SettingsCommands::ProviderGithub { command } => match command {
            ProviderGithubCommands::Show => {
                let resp = provider_github_show(
                    ctx.server.as_deref(),
                    ctx.token.as_deref(),
                    ctx.insecure,
                    ctx.request_timeout,
                )
                .await?;
                crate::output::print_output(ctx.format, &resp)?;
            }
            ProviderGithubCommands::Set {
                auth_token,
                api_base_url,
            } => {
                let resp = provider_github_set(
                    auth_token,
                    api_base_url,
                    ctx.server.as_deref(),
                    ctx.token.as_deref(),
                    ctx.insecure,
                    ctx.request_timeout,
                )
                .await?;
                crate::output::print_output(ctx.format, &resp)?;
            }
            ProviderGithubCommands::Clear => {
                let resp = provider_github_clear(
                    ctx.server.as_deref(),
                    ctx.token.as_deref(),
                    ctx.insecure,
                    ctx.request_timeout,
                )
                .await?;
                crate::output::print_output(ctx.format, &resp)?;
            }
        },
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
    use uptrakit_openapi_client::types::settings::RegistrationSettingsResponse;
    use uptrakit_openapi_client::types::settings_agent_certs::AgentCertificateSettingsResponse;
    use uptrakit_openapi_client::types::settings_auth::AuthenticationSettingsResponse;
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
            message: "Item abc deleted.".to_string(),
        };
        assert!(out.to_human_string().contains("abc deleted"));
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
            multi_tenancy_enabled: false,
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
}
