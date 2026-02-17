mod client;
mod commands;
mod config;
mod error;
mod output;

use clap::{CommandFactory, Parser, Subcommand};
use output::OutputFormat;
use uptrakit_build_info::BuildInfo;

#[derive(Debug, Parser)]
#[command(name = "uptrakit", about = "Uptrakit CLI")]
#[command(disable_version_flag = true)]
struct Cli {
    /// Show crate version and build metadata
    #[arg(long, global = true)]
    version: bool,

    /// Server URL (overrides stored config)
    #[arg(long, global = true)]
    server: Option<String>,

    /// API token (overrides stored credentials)
    #[arg(long, global = true)]
    token: Option<String>,

    /// Skip TLS certificate verification (insecure, for development only)
    #[arg(long, global = true)]
    insecure: bool,

    /// Output format
    #[arg(long, short, global = true, default_value_t, value_enum)]
    output: OutputFormat,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Authentication and token management
    Auth {
        #[command(subcommand)]
        command: AuthCommands,
    },
    /// Execute raw API requests
    Api {
        /// HTTP method (GET, POST, PUT, DELETE, PATCH)
        method: String,

        /// API path (e.g. /api/v1/auth/me)
        path: String,

        /// JSON request body
        #[arg(long)]
        data: Option<String>,
    },
    /// Manage services (agents, MQTT, SSH)
    Services {
        #[command(subcommand)]
        command: ServicesCommands,
    },
    /// Manage hosts
    Hosts {
        #[command(subcommand)]
        command: HostsCommands,
    },
    /// Manage software items
    SoftwareItems {
        #[command(subcommand)]
        command: SoftwareItemsCommands,
    },
    /// Trigger version checks
    Check {
        #[command(subcommand)]
        command: CheckCommands,
    },
    /// Trigger software updates
    Update {
        #[command(subcommand)]
        command: UpdateCommands,
    },
    /// View update history
    History {
        #[command(subcommand)]
        command: HistoryCommands,
    },
    /// Manage scheduled tasks
    Scheduler {
        #[command(subcommand)]
        command: SchedulerCommands,
    },
    /// Manage server settings
    Settings {
        #[command(subcommand)]
        command: SettingsCommands,
    },
}

#[derive(Debug, Subcommand)]
enum AuthCommands {
    /// Login to the server via browser authorization
    Login,
    /// Show current authentication status
    Status,
    /// API token management
    Token {
        #[command(subcommand)]
        command: TokenCommands,
    },
}

#[derive(Debug, Subcommand)]
enum TokenCommands {
    /// Create a new API token
    Create {
        /// Token name
        #[arg(long)]
        name: String,
    },
    /// List API tokens
    List,
    /// Revoke an API token
    Revoke {
        /// Token ID to revoke
        id: String,
    },
}

#[derive(Debug, Subcommand)]
enum HostsCommands {
    /// List all hosts
    List {
        /// Page number (1-indexed)
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
    },
    /// Show host details
    Show {
        /// Host UUID
        id: String,
    },
}

#[derive(Debug, Subcommand)]
enum SoftwareItemsCommands {
    /// List all software items
    List {
        /// Page number (1-indexed)
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
    },
    /// Show software item details
    Show {
        /// Software item UUID
        id: String,
    },
}

#[derive(Debug, Subcommand)]
enum CheckCommands {
    /// Trigger bulk version check (all items, all hosts)
    All,
    /// Trigger installed version check for a software item
    Installed {
        /// Software item UUID
        item_id: String,
        /// Optionally scope to a specific host
        #[arg(long)]
        host: Option<String>,
    },
    /// Trigger available version check for a software item
    Available {
        /// Software item UUID
        item_id: String,
        /// Optionally scope to a specific host
        #[arg(long)]
        host: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum UpdateCommands {
    /// Trigger an update for a software item on a host
    Trigger {
        /// Software item UUID
        item_id: String,
        /// Host UUID
        host_id: String,
        /// Target version to update to
        #[arg(long)]
        to_version: String,
        /// Release tag (defaults to to_version)
        #[arg(long)]
        release_tag: Option<String>,
        /// Release URL
        #[arg(long)]
        release_url: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum HistoryCommands {
    /// List update history
    List {
        /// Filter by host UUID
        #[arg(long)]
        host: Option<String>,
        /// Filter by software item UUID
        #[arg(long)]
        software_item: Option<String>,
        /// Filter by status (pending, in_progress, completed, failed)
        #[arg(long)]
        status: Option<String>,
        /// Page number (1-indexed)
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
    },
    /// Show update history details
    Show {
        /// Update history UUID
        id: String,
    },
}

#[derive(Debug, Subcommand)]
enum ServicesCommands {
    /// List all services
    List {
        /// Filter by type (agent, mqtt, ssh_agent)
        #[arg(long)]
        r#type: Option<String>,
        /// Filter by status (pending, approved, rejected, deactivated)
        #[arg(long)]
        status: Option<String>,
        /// Page number (1-indexed)
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
    },
    /// Show service details
    Show {
        /// Service UUID
        id: String,
    },
    /// Approve a pending service
    Approve {
        /// Service UUID
        id: String,
    },
    /// Reject a pending service
    Reject {
        /// Service UUID
        id: String,
    },
    /// Remove (deactivate) a service
    Remove {
        /// Service UUID
        id: String,
    },
    /// Merge a source service into a target service
    Merge {
        /// Target service UUID (approved)
        target_id: String,
        /// Source service UUID (pending)
        source_id: String,
    },
}

#[derive(Debug, Subcommand)]
enum SchedulerCommands {
    /// List scheduled tasks
    List,
    /// Show scheduled task details
    Show {
        /// Task UUID
        id: String,
    },
    /// Trigger immediate execution of a scheduled task
    Trigger {
        /// Task UUID
        id: String,
    },
}

#[derive(Debug, Subcommand)]
enum SettingsCommands {
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
}

#[derive(Debug, Subcommand)]
enum RegistrationCommands {
    /// Show registration settings
    Show,
    /// Update registration settings
    Update {
        /// Registration mode (open, invite, closed)
        #[arg(long, value_parser = parse_registration_mode)]
        mode: uptrakit_openapi_client::types::registration::RegistrationMode,
        /// Registration token (required for invite mode)
        #[arg(long)]
        token: Option<String>,
        /// Whether OIDC users also need a registration token
        #[arg(long)]
        require_token_for_oidc: Option<bool>,
    },
}

#[derive(Debug, Subcommand)]
enum AuthenticationCommands {
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
enum CertificateCommands {
    /// Show agent certificate settings
    Show,
    /// Update agent certificate settings
    Update {
        /// Certificate lifetime in days (max 730)
        #[arg(long)]
        lifetime_days: Option<u16>,
        /// Certificate renewal window in hours
        #[arg(long)]
        renewal_window_hours: Option<u16>,
    },
}

#[derive(Debug, Subcommand)]
enum NetworkCommands {
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
        /// Comma-separated extra Subject Alternative Names
        #[arg(long)]
        extra_sans: Option<String>,
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
enum MqttCommands {
    /// List MQTT client configurations
    List,
    /// Show MQTT client configuration details
    Show {
        /// MQTT configuration ID
        id: String,
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
        /// Topic prefix (e.g. homeassistant)
        #[arg(long)]
        topic_prefix: Option<String>,
    },
    /// Update an MQTT client configuration
    Update {
        /// MQTT configuration ID
        id: String,
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
        /// Topic prefix
        #[arg(long)]
        topic_prefix: Option<String>,
    },
    /// Delete an MQTT client configuration
    Delete {
        /// MQTT configuration ID
        id: String,
    },
    /// MQTT client limit management
    Limit {
        #[command(subcommand)]
        command: MqttLimitCommands,
    },
}

#[derive(Debug, Subcommand)]
enum MqttLimitCommands {
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
enum OidcCommands {
    /// List OIDC providers
    List,
    /// Show OIDC provider details
    Show {
        /// OIDC provider ID
        id: String,
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
        /// OIDC provider ID
        id: String,
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
        /// OIDC provider ID
        id: String,
    },
    /// Activate an OIDC provider
    Activate {
        /// OIDC provider ID
        id: String,
    },
    /// Deactivate an OIDC provider
    Deactivate {
        /// OIDC provider ID
        id: String,
    },
}

/// Parse a registration mode string into the typed enum.
fn parse_registration_mode(
    s: &str,
) -> std::result::Result<uptrakit_openapi_client::types::registration::RegistrationMode, String> {
    s.parse()
        .map_err(|_| format!("invalid registration mode: {s} (expected open, invite, or closed)"))
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if cli.version {
        let build_info = BuildInfo::current(
            "uptrakit",
            env!("CARGO_PKG_VERSION"),
            option_env!("UPTRAKIT_BUILD_ENABLED_FEATURES"),
        );
        let human = build_info.render_human();
        if let Err(e) = output::print_output(cli.output, &human, &build_info) {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
        return;
    }

    let command = match cli.command {
        Some(command) => command,
        None => {
            let mut cmd = Cli::command();
            cmd.error(
                clap::error::ErrorKind::MissingSubcommand,
                "a subcommand is required",
            )
            .exit();
        }
    };

    let insecure = cli.insecure;
    let result = match command {
        Commands::Auth { command } => match command {
            AuthCommands::Login => commands::auth::login(cli.server.as_deref(), insecure).await,
            AuthCommands::Status => {
                commands::auth::status(
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    cli.output,
                    insecure,
                )
                .await
            }
            AuthCommands::Token { command } => match command {
                TokenCommands::Create { name } => {
                    commands::auth::token_create(
                        &name,
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        cli.output,
                        insecure,
                    )
                    .await
                }
                TokenCommands::List => {
                    commands::auth::token_list(
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        cli.output,
                        insecure,
                    )
                    .await
                }
                TokenCommands::Revoke { id } => {
                    commands::auth::token_revoke(
                        &id,
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        cli.output,
                        insecure,
                    )
                    .await
                }
            },
        },
        Commands::Api { method, path, data } => {
            commands::api::execute(
                &method,
                &path,
                data.as_deref(),
                cli.server.as_deref(),
                cli.token.as_deref(),
                cli.output,
                insecure,
            )
            .await
        }
        Commands::Services { command } => match command {
            ServicesCommands::List {
                r#type,
                status,
                page,
                per_page,
            } => {
                commands::services::list(commands::services::ListParams {
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    format: cli.output,
                    insecure,
                    service_type: r#type.as_deref(),
                    status: status.as_deref(),
                    page,
                    per_page,
                })
                .await
            }
            ServicesCommands::Show { id } => {
                commands::services::show(
                    &id,
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    cli.output,
                    insecure,
                )
                .await
            }
            ServicesCommands::Approve { id } => {
                commands::services::approve(
                    &id,
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    cli.output,
                    insecure,
                )
                .await
            }
            ServicesCommands::Reject { id } => {
                commands::services::reject(
                    &id,
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    cli.output,
                    insecure,
                )
                .await
            }
            ServicesCommands::Remove { id } => {
                commands::services::remove(
                    &id,
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    cli.output,
                    insecure,
                )
                .await
            }
            ServicesCommands::Merge {
                target_id,
                source_id,
            } => {
                commands::services::merge(
                    &target_id,
                    &source_id,
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    cli.output,
                    insecure,
                )
                .await
            }
        },
        Commands::Hosts { command } => match command {
            HostsCommands::List { page, per_page } => {
                commands::hosts::list(
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    cli.output,
                    insecure,
                    page,
                    per_page,
                )
                .await
            }
            HostsCommands::Show { id } => {
                commands::hosts::show(
                    &id,
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    cli.output,
                    insecure,
                )
                .await
            }
        },
        Commands::SoftwareItems { command } => match command {
            SoftwareItemsCommands::List { page, per_page } => {
                commands::software_items::list(
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    cli.output,
                    insecure,
                    page,
                    per_page,
                )
                .await
            }
            SoftwareItemsCommands::Show { id } => {
                commands::software_items::show(
                    &id,
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    cli.output,
                    insecure,
                )
                .await
            }
        },
        Commands::Check { command } => match command {
            CheckCommands::All => {
                commands::check::all(
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    cli.output,
                    insecure,
                )
                .await
            }
            CheckCommands::Installed { item_id, host } => {
                commands::check::installed(
                    &item_id,
                    host.as_deref(),
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    cli.output,
                    insecure,
                )
                .await
            }
            CheckCommands::Available { item_id, host } => {
                commands::check::available(
                    &item_id,
                    host.as_deref(),
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    cli.output,
                    insecure,
                )
                .await
            }
        },
        Commands::Update { command } => match command {
            UpdateCommands::Trigger {
                item_id,
                host_id,
                to_version,
                release_tag,
                release_url,
            } => {
                commands::update::trigger(commands::update::TriggerParams {
                    item_id: &item_id,
                    host_id: &host_id,
                    to_version: &to_version,
                    release_tag: release_tag.as_deref(),
                    release_url: release_url.as_deref(),
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    format: cli.output,
                    insecure,
                })
                .await
            }
        },
        Commands::History { command } => match command {
            HistoryCommands::List {
                host,
                software_item,
                status,
                page,
                per_page,
            } => {
                commands::history::list(commands::history::ListParams {
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    format: cli.output,
                    insecure,
                    host_id: host.as_deref(),
                    software_item_id: software_item.as_deref(),
                    status: status.as_deref(),
                    page,
                    per_page,
                })
                .await
            }
            HistoryCommands::Show { id } => {
                commands::history::show(
                    &id,
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    cli.output,
                    insecure,
                )
                .await
            }
        },
        Commands::Scheduler { command } => match command {
            SchedulerCommands::List => {
                commands::scheduler::list(
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    cli.output,
                    insecure,
                )
                .await
            }
            SchedulerCommands::Show { id } => {
                commands::scheduler::show(
                    &id,
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    cli.output,
                    insecure,
                )
                .await
            }
            SchedulerCommands::Trigger { id } => {
                commands::scheduler::trigger(
                    &id,
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    cli.output,
                    insecure,
                )
                .await
            }
        },
        Commands::Settings { command } => match command {
            SettingsCommands::Show => {
                commands::settings::show_combined(
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    cli.output,
                    insecure,
                )
                .await
            }
            SettingsCommands::Registration { command } => match command {
                RegistrationCommands::Show => {
                    commands::settings::registration_show(
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        cli.output,
                        insecure,
                    )
                    .await
                }
                RegistrationCommands::Update {
                    mode,
                    token,
                    require_token_for_oidc,
                } => {
                    commands::settings::registration_update(
                        commands::settings::RegistrationUpdateParams {
                            server: cli.server.as_deref(),
                            token: cli.token.as_deref(),
                            format: cli.output,
                            insecure,
                            mode,
                            reg_token: token,
                            require_token_for_oidc,
                        },
                    )
                    .await
                }
            },
            SettingsCommands::Authentication { command } => match command {
                AuthenticationCommands::Show => {
                    commands::settings::authentication_show(
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        cli.output,
                        insecure,
                    )
                    .await
                }
                AuthenticationCommands::Update {
                    password_auth_enabled,
                } => {
                    commands::settings::authentication_update(
                        password_auth_enabled,
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        cli.output,
                        insecure,
                    )
                    .await
                }
            },
            SettingsCommands::Certificates { command } => match command {
                CertificateCommands::Show => {
                    commands::settings::certificates_show(
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        cli.output,
                        insecure,
                    )
                    .await
                }
                CertificateCommands::Update {
                    lifetime_days,
                    renewal_window_hours,
                } => {
                    commands::settings::certificates_update(
                        lifetime_days,
                        renewal_window_hours,
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        cli.output,
                        insecure,
                    )
                    .await
                }
            },
            SettingsCommands::Network { command } => match command {
                NetworkCommands::Show => {
                    commands::settings::network_show(
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        cli.output,
                        insecure,
                    )
                    .await
                }
                NetworkCommands::Update {
                    trusted_proxies,
                    real_ip_header,
                    extra_sans,
                    https_addr,
                    fwd_cert_info_header,
                    fwd_cert_pem_header,
                    pki_addr,
                } => {
                    commands::settings::network_update(
                        commands::settings::NetworkUpdateParams {
                            server: cli.server.as_deref(),
                            token: cli.token.as_deref(),
                            format: cli.output,
                            insecure,
                            trusted_proxies: trusted_proxies
                                .map(|s| s.split(',').map(|v| v.trim().to_string()).collect()),
                            real_ip_header,
                            extra_sans: extra_sans
                                .map(|s| s.split(',').map(|v| v.trim().to_string()).collect()),
                            https_addr,
                            fwd_cert_info_header,
                            fwd_cert_pem_header,
                            pki_addr,
                        },
                    )
                    .await
                }
            },
            SettingsCommands::RotateCa => {
                commands::settings::rotate_ca(
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    cli.output,
                    insecure,
                )
                .await
            }
            SettingsCommands::RenewServerCert => {
                commands::settings::renew_server_cert(
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    cli.output,
                    insecure,
                )
                .await
            }
            SettingsCommands::Mqtt { command } => match command {
                MqttCommands::List => {
                    commands::settings::mqtt_list(
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        cli.output,
                        insecure,
                    )
                    .await
                }
                MqttCommands::Show { id } => {
                    commands::settings::mqtt_show(
                        &id,
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        cli.output,
                        insecure,
                    )
                    .await
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
                    topic_prefix,
                } => {
                    commands::settings::mqtt_create(commands::settings::MqttCreateParams {
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        format: cli.output,
                        insecure,
                        url,
                        transport,
                        host,
                        port,
                        enabled,
                        client_id,
                        username,
                        password,
                        topic_prefix,
                    })
                    .await
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
                    topic_prefix,
                } => {
                    commands::settings::mqtt_update(commands::settings::MqttUpdateParams {
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        format: cli.output,
                        insecure,
                        id,
                        url,
                        transport,
                        host,
                        port,
                        enabled,
                        client_id,
                        username,
                        password,
                        topic_prefix,
                    })
                    .await
                }
                MqttCommands::Delete { id } => {
                    commands::settings::mqtt_delete(
                        &id,
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        cli.output,
                        insecure,
                    )
                    .await
                }
                MqttCommands::Limit { command } => match command {
                    MqttLimitCommands::Show => {
                        commands::settings::mqtt_limit_show(
                            cli.server.as_deref(),
                            cli.token.as_deref(),
                            cli.output,
                            insecure,
                        )
                        .await
                    }
                    MqttLimitCommands::Update { max } => {
                        commands::settings::mqtt_limit_update(
                            max,
                            cli.server.as_deref(),
                            cli.token.as_deref(),
                            cli.output,
                            insecure,
                        )
                        .await
                    }
                },
            },
            SettingsCommands::Oidc { command } => match command {
                OidcCommands::List => {
                    commands::settings::oidc_list(
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        cli.output,
                        insecure,
                    )
                    .await
                }
                OidcCommands::Show { id } => {
                    commands::settings::oidc_show(
                        &id,
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        cli.output,
                        insecure,
                    )
                    .await
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
                    commands::settings::oidc_create(commands::settings::OidcCreateParams {
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        format: cli.output,
                        insecure,
                        name,
                        slug,
                        logo_url,
                        issuer_url,
                        client_id,
                        client_secret,
                        scopes,
                        auto_create_users,
                        role_claim_path,
                    })
                    .await
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
                    commands::settings::oidc_update(commands::settings::OidcUpdateParams {
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        format: cli.output,
                        insecure,
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
                    })
                    .await
                }
                OidcCommands::Delete { id } => {
                    commands::settings::oidc_delete(
                        &id,
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        cli.output,
                        insecure,
                    )
                    .await
                }
                OidcCommands::Activate { id } => {
                    commands::settings::oidc_activate(
                        &id,
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        cli.output,
                        insecure,
                    )
                    .await
                }
                OidcCommands::Deactivate { id } => {
                    commands::settings::oidc_deactivate(
                        &id,
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        cli.output,
                        insecure,
                    )
                    .await
                }
            },
            SettingsCommands::Alerts => {
                commands::settings::alerts(
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    cli.output,
                    insecure,
                )
                .await
            }
        },
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn version_parses_without_subcommand() {
        let args = Cli::try_parse_from(["uptrakit", "--version"]).expect("should parse");
        assert!(args.version);
        assert!(args.command.is_none());
    }

    #[test]
    fn hosts_list_parses() {
        let args = Cli::try_parse_from(["uptrakit", "hosts", "list"]).expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::Hosts {
                command: HostsCommands::List { .. }
            })
        ));
    }

    #[test]
    fn hosts_list_with_pagination() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "hosts",
            "list",
            "--page",
            "2",
            "--per-page",
            "50",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::Hosts {
                command: HostsCommands::List { page, per_page },
            }) => {
                assert_eq!(page, Some(2));
                assert_eq!(per_page, Some(50));
            }
            _ => panic!("expected Hosts List"),
        }
    }

    #[test]
    fn hosts_show_parses() {
        let args = Cli::try_parse_from(["uptrakit", "hosts", "show", "abc-123"])
            .expect("should parse");
        match args.command {
            Some(Commands::Hosts {
                command: HostsCommands::Show { id },
            }) => {
                assert_eq!(id, "abc-123");
            }
            _ => panic!("expected Hosts Show"),
        }
    }

    #[test]
    fn software_items_list_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "software-items", "list"]).expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::SoftwareItems {
                command: SoftwareItemsCommands::List { .. }
            })
        ));
    }

    #[test]
    fn software_items_show_parses() {
        let args = Cli::try_parse_from(["uptrakit", "software-items", "show", "item-123"])
            .expect("should parse");
        match args.command {
            Some(Commands::SoftwareItems {
                command: SoftwareItemsCommands::Show { id },
            }) => {
                assert_eq!(id, "item-123");
            }
            _ => panic!("expected SoftwareItems Show"),
        }
    }

    #[test]
    fn check_all_parses() {
        let args = Cli::try_parse_from(["uptrakit", "check", "all"]).expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::Check {
                command: CheckCommands::All
            })
        ));
    }

    #[test]
    fn check_installed_parses() {
        let args = Cli::try_parse_from(["uptrakit", "check", "installed", "item-1"])
            .expect("should parse");
        match args.command {
            Some(Commands::Check {
                command: CheckCommands::Installed { item_id, host },
            }) => {
                assert_eq!(item_id, "item-1");
                assert!(host.is_none());
            }
            _ => panic!("expected Check Installed"),
        }
    }

    #[test]
    fn check_installed_with_host_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "check",
            "installed",
            "item-1",
            "--host",
            "host-1",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::Check {
                command: CheckCommands::Installed { item_id, host },
            }) => {
                assert_eq!(item_id, "item-1");
                assert_eq!(host.as_deref(), Some("host-1"));
            }
            _ => panic!("expected Check Installed"),
        }
    }

    #[test]
    fn check_available_parses() {
        let args = Cli::try_parse_from(["uptrakit", "check", "available", "item-2"])
            .expect("should parse");
        match args.command {
            Some(Commands::Check {
                command: CheckCommands::Available { item_id, host },
            }) => {
                assert_eq!(item_id, "item-2");
                assert!(host.is_none());
            }
            _ => panic!("expected Check Available"),
        }
    }

    #[test]
    fn update_trigger_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "update",
            "trigger",
            "item-1",
            "host-1",
            "--to-version",
            "2.0.0",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::Update {
                command:
                    UpdateCommands::Trigger {
                        item_id,
                        host_id,
                        to_version,
                        release_tag,
                        release_url,
                    },
            }) => {
                assert_eq!(item_id, "item-1");
                assert_eq!(host_id, "host-1");
                assert_eq!(to_version, "2.0.0");
                assert!(release_tag.is_none());
                assert!(release_url.is_none());
            }
            _ => panic!("expected Update Trigger"),
        }
    }

    #[test]
    fn update_trigger_with_release_info_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "update",
            "trigger",
            "item-1",
            "host-1",
            "--to-version",
            "2.0.0",
            "--release-tag",
            "v2.0.0",
            "--release-url",
            "https://example.com/releases/v2.0.0",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::Update {
                command:
                    UpdateCommands::Trigger {
                        release_tag,
                        release_url,
                        ..
                    },
            }) => {
                assert_eq!(release_tag.as_deref(), Some("v2.0.0"));
                assert_eq!(
                    release_url.as_deref(),
                    Some("https://example.com/releases/v2.0.0")
                );
            }
            _ => panic!("expected Update Trigger"),
        }
    }

    #[test]
    fn history_list_parses() {
        let args = Cli::try_parse_from(["uptrakit", "history", "list"]).expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::History {
                command: HistoryCommands::List { .. }
            })
        ));
    }

    #[test]
    fn history_list_with_filters() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "history",
            "list",
            "--host",
            "host-1",
            "--software-item",
            "item-1",
            "--status",
            "completed",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::History {
                command:
                    HistoryCommands::List {
                        host,
                        software_item,
                        status,
                        ..
                    },
            }) => {
                assert_eq!(host.as_deref(), Some("host-1"));
                assert_eq!(software_item.as_deref(), Some("item-1"));
                assert_eq!(status.as_deref(), Some("completed"));
            }
            _ => panic!("expected History List"),
        }
    }

    #[test]
    fn history_show_parses() {
        let args = Cli::try_parse_from(["uptrakit", "history", "show", "hist-123"])
            .expect("should parse");
        match args.command {
            Some(Commands::History {
                command: HistoryCommands::Show { id },
            }) => {
                assert_eq!(id, "hist-123");
            }
            _ => panic!("expected History Show"),
        }
    }

    #[test]
    fn scheduler_list_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "scheduler", "list"]).expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::Scheduler {
                command: SchedulerCommands::List
            })
        ));
    }

    #[test]
    fn scheduler_show_parses() {
        let args = Cli::try_parse_from(["uptrakit", "scheduler", "show", "task-123"])
            .expect("should parse");
        match args.command {
            Some(Commands::Scheduler {
                command: SchedulerCommands::Show { id },
            }) => {
                assert_eq!(id, "task-123");
            }
            _ => panic!("expected Scheduler Show"),
        }
    }

    #[test]
    fn scheduler_trigger_parses() {
        let args = Cli::try_parse_from(["uptrakit", "scheduler", "trigger", "task-123"])
            .expect("should parse");
        match args.command {
            Some(Commands::Scheduler {
                command: SchedulerCommands::Trigger { id },
            }) => {
                assert_eq!(id, "task-123");
            }
            _ => panic!("expected Scheduler Trigger"),
        }
    }

    #[test]
    fn services_list_parses() {
        let args = Cli::try_parse_from(["uptrakit", "services", "list"]).expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::Services {
                command: ServicesCommands::List { .. }
            })
        ));
    }

    #[test]
    fn services_list_with_filters() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "services",
            "list",
            "--type",
            "agent",
            "--status",
            "pending",
            "--page",
            "2",
            "--per-page",
            "50",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::Services {
                command:
                    ServicesCommands::List {
                        r#type,
                        status,
                        page,
                        per_page,
                    },
            }) => {
                assert_eq!(r#type.as_deref(), Some("agent"));
                assert_eq!(status.as_deref(), Some("pending"));
                assert_eq!(page, Some(2));
                assert_eq!(per_page, Some(50));
            }
            _ => panic!("expected Services List"),
        }
    }

    #[test]
    fn services_show_parses() {
        let args = Cli::try_parse_from(["uptrakit", "services", "show", "svc-123"])
            .expect("should parse");
        match args.command {
            Some(Commands::Services {
                command: ServicesCommands::Show { id },
            }) => {
                assert_eq!(id, "svc-123");
            }
            _ => panic!("expected Services Show"),
        }
    }

    #[test]
    fn services_approve_parses() {
        let args = Cli::try_parse_from(["uptrakit", "services", "approve", "svc-123"])
            .expect("should parse");
        match args.command {
            Some(Commands::Services {
                command: ServicesCommands::Approve { id },
            }) => {
                assert_eq!(id, "svc-123");
            }
            _ => panic!("expected Services Approve"),
        }
    }

    #[test]
    fn services_reject_parses() {
        let args = Cli::try_parse_from(["uptrakit", "services", "reject", "svc-123"])
            .expect("should parse");
        match args.command {
            Some(Commands::Services {
                command: ServicesCommands::Reject { id },
            }) => {
                assert_eq!(id, "svc-123");
            }
            _ => panic!("expected Services Reject"),
        }
    }

    #[test]
    fn services_remove_parses() {
        let args = Cli::try_parse_from(["uptrakit", "services", "remove", "svc-123"])
            .expect("should parse");
        match args.command {
            Some(Commands::Services {
                command: ServicesCommands::Remove { id },
            }) => {
                assert_eq!(id, "svc-123");
            }
            _ => panic!("expected Services Remove"),
        }
    }

    #[test]
    fn services_merge_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "services",
            "merge",
            "target-123",
            "source-456",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::Services {
                command:
                    ServicesCommands::Merge {
                        target_id,
                        source_id,
                    },
            }) => {
                assert_eq!(target_id, "target-123");
                assert_eq!(source_id, "source-456");
            }
            _ => panic!("expected Services Merge"),
        }
    }

    #[test]
    fn global_options_parse_with_commands() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "--server",
            "https://example.com",
            "--token",
            "my-token",
            "--insecure",
            "--output",
            "json",
            "hosts",
            "list",
        ])
        .expect("should parse");
        assert_eq!(args.server.as_deref(), Some("https://example.com"));
        assert_eq!(args.token.as_deref(), Some("my-token"));
        assert!(args.insecure);
        assert_eq!(args.output, OutputFormat::Json);
    }

    // ── Settings tests ──────────────────────────────────────────────

    #[test]
    fn settings_show_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "settings", "show"]).expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::Settings {
                command: SettingsCommands::Show
            })
        ));
    }

    #[test]
    fn settings_registration_show_parses() {
        let args = Cli::try_parse_from(["uptrakit", "settings", "registration", "show"])
            .expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::Settings {
                command: SettingsCommands::Registration {
                    command: RegistrationCommands::Show
                }
            })
        ));
    }

    #[test]
    fn settings_registration_update_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "settings",
            "registration",
            "update",
            "--mode",
            "invite",
            "--token",
            "my-token",
            "--require-token-for-oidc",
            "true",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::Settings {
                command:
                    SettingsCommands::Registration {
                        command:
                            RegistrationCommands::Update {
                                mode,
                                token,
                                require_token_for_oidc,
                            },
                    },
            }) => {
                assert_eq!(
                    mode,
                    uptrakit_openapi_client::types::registration::RegistrationMode::Invite
                );
                assert_eq!(token.as_deref(), Some("my-token"));
                assert_eq!(require_token_for_oidc, Some(true));
            }
            _ => panic!("expected Settings Registration Update"),
        }
    }

    #[test]
    fn settings_authentication_show_parses() {
        let args = Cli::try_parse_from(["uptrakit", "settings", "authentication", "show"])
            .expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::Settings {
                command: SettingsCommands::Authentication {
                    command: AuthenticationCommands::Show
                }
            })
        ));
    }

    #[test]
    fn settings_authentication_update_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "settings",
            "authentication",
            "update",
            "--password-auth-enabled",
            "false",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::Settings {
                command:
                    SettingsCommands::Authentication {
                        command: AuthenticationCommands::Update {
                            password_auth_enabled,
                        },
                    },
            }) => {
                assert_eq!(password_auth_enabled, Some(false));
            }
            _ => panic!("expected Settings Authentication Update"),
        }
    }

    #[test]
    fn settings_certificates_show_parses() {
        let args = Cli::try_parse_from(["uptrakit", "settings", "certificates", "show"])
            .expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::Settings {
                command: SettingsCommands::Certificates {
                    command: CertificateCommands::Show
                }
            })
        ));
    }

    #[test]
    fn settings_certificates_update_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "settings",
            "certificates",
            "update",
            "--lifetime-days",
            "365",
            "--renewal-window-hours",
            "72",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::Settings {
                command:
                    SettingsCommands::Certificates {
                        command:
                            CertificateCommands::Update {
                                lifetime_days,
                                renewal_window_hours,
                            },
                    },
            }) => {
                assert_eq!(lifetime_days, Some(365));
                assert_eq!(renewal_window_hours, Some(72));
            }
            _ => panic!("expected Settings Certificates Update"),
        }
    }

    #[test]
    fn settings_network_show_parses() {
        let args = Cli::try_parse_from(["uptrakit", "settings", "network", "show"])
            .expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::Settings {
                command: SettingsCommands::Network {
                    command: NetworkCommands::Show
                }
            })
        ));
    }

    #[test]
    fn settings_network_update_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "settings",
            "network",
            "update",
            "--trusted-proxies",
            "10.0.0.0/8,172.16.0.0/12",
            "--real-ip-header",
            "X-Real-IP",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::Settings {
                command:
                    SettingsCommands::Network {
                        command:
                            NetworkCommands::Update {
                                trusted_proxies,
                                real_ip_header,
                                ..
                            },
                    },
            }) => {
                assert_eq!(
                    trusted_proxies.as_deref(),
                    Some("10.0.0.0/8,172.16.0.0/12")
                );
                assert_eq!(real_ip_header.as_deref(), Some("X-Real-IP"));
            }
            _ => panic!("expected Settings Network Update"),
        }
    }

    #[test]
    fn settings_rotate_ca_parses() {
        let args = Cli::try_parse_from(["uptrakit", "settings", "rotate-ca"])
            .expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::Settings {
                command: SettingsCommands::RotateCa
            })
        ));
    }

    #[test]
    fn settings_renew_server_cert_parses() {
        let args = Cli::try_parse_from(["uptrakit", "settings", "renew-server-cert"])
            .expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::Settings {
                command: SettingsCommands::RenewServerCert
            })
        ));
    }

    #[test]
    fn settings_mqtt_list_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "settings", "mqtt", "list"]).expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::Settings {
                command: SettingsCommands::Mqtt {
                    command: MqttCommands::List
                }
            })
        ));
    }

    #[test]
    fn settings_mqtt_show_parses() {
        let args = Cli::try_parse_from(["uptrakit", "settings", "mqtt", "show", "mqtt-123"])
            .expect("should parse");
        match args.command {
            Some(Commands::Settings {
                command:
                    SettingsCommands::Mqtt {
                        command: MqttCommands::Show { id },
                    },
            }) => {
                assert_eq!(id, "mqtt-123");
            }
            _ => panic!("expected Settings Mqtt Show"),
        }
    }

    #[test]
    fn settings_mqtt_create_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "settings",
            "mqtt",
            "create",
            "--url",
            "mqtt://broker:1883",
            "--enabled",
            "true",
            "--client-id",
            "uptrakit-1",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::Settings {
                command:
                    SettingsCommands::Mqtt {
                        command:
                            MqttCommands::Create {
                                url,
                                enabled,
                                client_id,
                                ..
                            },
                    },
            }) => {
                assert_eq!(url.as_deref(), Some("mqtt://broker:1883"));
                assert_eq!(enabled, Some(true));
                assert_eq!(client_id.as_deref(), Some("uptrakit-1"));
            }
            _ => panic!("expected Settings Mqtt Create"),
        }
    }

    #[test]
    fn settings_mqtt_update_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "settings",
            "mqtt",
            "update",
            "mqtt-123",
            "--enabled",
            "false",
            "--host",
            "new-broker",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::Settings {
                command:
                    SettingsCommands::Mqtt {
                        command:
                            MqttCommands::Update {
                                id,
                                enabled,
                                host,
                                ..
                            },
                    },
            }) => {
                assert_eq!(id, "mqtt-123");
                assert_eq!(enabled, Some(false));
                assert_eq!(host.as_deref(), Some("new-broker"));
            }
            _ => panic!("expected Settings Mqtt Update"),
        }
    }

    #[test]
    fn settings_mqtt_delete_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "settings", "mqtt", "delete", "mqtt-123"])
                .expect("should parse");
        match args.command {
            Some(Commands::Settings {
                command:
                    SettingsCommands::Mqtt {
                        command: MqttCommands::Delete { id },
                    },
            }) => {
                assert_eq!(id, "mqtt-123");
            }
            _ => panic!("expected Settings Mqtt Delete"),
        }
    }

    #[test]
    fn settings_mqtt_limit_show_parses() {
        let args = Cli::try_parse_from(["uptrakit", "settings", "mqtt", "limit", "show"])
            .expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::Settings {
                command: SettingsCommands::Mqtt {
                    command: MqttCommands::Limit {
                        command: MqttLimitCommands::Show
                    }
                }
            })
        ));
    }

    #[test]
    fn settings_mqtt_limit_update_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "settings",
            "mqtt",
            "limit",
            "update",
            "--max",
            "10",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::Settings {
                command:
                    SettingsCommands::Mqtt {
                        command:
                            MqttCommands::Limit {
                                command: MqttLimitCommands::Update { max },
                            },
                    },
            }) => {
                assert_eq!(max, 10);
            }
            _ => panic!("expected Settings Mqtt Limit Update"),
        }
    }

    #[test]
    fn settings_oidc_list_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "settings", "oidc", "list"]).expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::Settings {
                command: SettingsCommands::Oidc {
                    command: OidcCommands::List
                }
            })
        ));
    }

    #[test]
    fn settings_oidc_show_parses() {
        let args = Cli::try_parse_from(["uptrakit", "settings", "oidc", "show", "oidc-123"])
            .expect("should parse");
        match args.command {
            Some(Commands::Settings {
                command:
                    SettingsCommands::Oidc {
                        command: OidcCommands::Show { id },
                    },
            }) => {
                assert_eq!(id, "oidc-123");
            }
            _ => panic!("expected Settings Oidc Show"),
        }
    }

    #[test]
    fn settings_oidc_create_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "settings",
            "oidc",
            "create",
            "--name",
            "Google",
            "--slug",
            "google",
            "--issuer-url",
            "https://accounts.google.com",
            "--client-id",
            "cid-123",
            "--client-secret",
            "cs-456",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::Settings {
                command:
                    SettingsCommands::Oidc {
                        command:
                            OidcCommands::Create {
                                name,
                                slug,
                                issuer_url,
                                client_id,
                                client_secret,
                                ..
                            },
                    },
            }) => {
                assert_eq!(name, "Google");
                assert_eq!(slug, "google");
                assert_eq!(issuer_url, "https://accounts.google.com");
                assert_eq!(client_id, "cid-123");
                assert_eq!(client_secret, "cs-456");
            }
            _ => panic!("expected Settings Oidc Create"),
        }
    }

    #[test]
    fn settings_oidc_update_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "settings",
            "oidc",
            "update",
            "oidc-123",
            "--name",
            "Google Workspace",
            "--auto-create-users",
            "false",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::Settings {
                command:
                    SettingsCommands::Oidc {
                        command:
                            OidcCommands::Update {
                                id,
                                name,
                                auto_create_users,
                                ..
                            },
                    },
            }) => {
                assert_eq!(id, "oidc-123");
                assert_eq!(name.as_deref(), Some("Google Workspace"));
                assert_eq!(auto_create_users, Some(false));
            }
            _ => panic!("expected Settings Oidc Update"),
        }
    }

    #[test]
    fn settings_oidc_delete_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "settings", "oidc", "delete", "oidc-123"])
                .expect("should parse");
        match args.command {
            Some(Commands::Settings {
                command:
                    SettingsCommands::Oidc {
                        command: OidcCommands::Delete { id },
                    },
            }) => {
                assert_eq!(id, "oidc-123");
            }
            _ => panic!("expected Settings Oidc Delete"),
        }
    }

    #[test]
    fn settings_oidc_activate_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "settings", "oidc", "activate", "oidc-123"])
                .expect("should parse");
        match args.command {
            Some(Commands::Settings {
                command:
                    SettingsCommands::Oidc {
                        command: OidcCommands::Activate { id },
                    },
            }) => {
                assert_eq!(id, "oidc-123");
            }
            _ => panic!("expected Settings Oidc Activate"),
        }
    }

    #[test]
    fn settings_oidc_deactivate_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "settings", "oidc", "deactivate", "oidc-123"])
                .expect("should parse");
        match args.command {
            Some(Commands::Settings {
                command:
                    SettingsCommands::Oidc {
                        command: OidcCommands::Deactivate { id },
                    },
            }) => {
                assert_eq!(id, "oidc-123");
            }
            _ => panic!("expected Settings Oidc Deactivate"),
        }
    }

    #[test]
    fn settings_alerts_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "settings", "alerts"]).expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::Settings {
                command: SettingsCommands::Alerts
            })
        ));
    }

    #[test]
    fn settings_registration_update_rejects_invalid_mode() {
        let result = Cli::try_parse_from([
            "uptrakit",
            "settings",
            "registration",
            "update",
            "--mode",
            "invalid",
        ]);
        assert!(result.is_err());
    }
}
