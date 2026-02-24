use uptrakit_cli::{commands, error, output};

use clap::{CommandFactory, Parser, Subcommand};
use rootcause::prelude::*;
use tracing_subscriber::EnvFilter;
use uptrakit_build_info::BuildInfo;
use uptrakit_cli::output::OutputFormat;
use uptrakit_openapi_client::Uuid;
use uptrakit_shared_types::ProviderType;

#[derive(Debug, Parser)]
#[command(name = "uptrakit", about = "Uptrakit CLI")]
#[command(disable_version_flag = true)]
struct Cli {
    /// Show crate version and build metadata
    #[arg(long, global = true)]
    version: bool,

    /// Server URL (overrides stored config)
    #[arg(long, global = true, env = "UPTRAKIT_SERVER")]
    server: Option<String>,

    /// API token (overrides stored credentials)
    #[arg(long, global = true, env = "UPTRAKIT_TOKEN")]
    token: Option<String>,

    /// Skip TLS certificate verification (insecure, for development only)
    #[arg(long, global = true)]
    insecure: bool,

    /// API request timeout in seconds (default: 30)
    #[arg(long, global = true, value_name = "SECONDS", env = "UPTRAKIT_TIMEOUT")]
    timeout: Option<u64>,

    /// Output format
    #[arg(long, short, global = true, default_value_t, value_enum)]
    output: OutputFormat,

    /// Increase log verbosity (-v for warn, -vv for info, -vvv for debug, -vvvv for trace).
    /// Log output goes to stderr; command output stays on stdout.
    #[arg(short = 'v', long = "verbose", global = true, action = clap::ArgAction::Count)]
    verbose: u8,

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
    /// Manage provider configurations
    ProviderConfigs {
        #[command(subcommand)]
        command: ProviderConfigsCommands,
    },
    /// Autodiscovery management
    Autodiscovery {
        #[command(subcommand)]
        command: AutodiscoveryCommands,
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
        id: Uuid,
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
        id: Uuid,
    },
    /// Update a host's settings
    Update {
        /// Host UUID
        id: Uuid,
        /// New friendly name
        #[arg(long)]
        friendly_name: Option<String>,
    },
    /// Deactivate (remove) a host
    Deactivate {
        /// Host UUID
        id: Uuid,
    },
    /// Trigger autodiscovery on a host
    Discover {
        /// Host UUID
        id: Uuid,
    },
    /// Discard all pending discovered items for a host
    DiscardDiscovered {
        /// Host UUID
        id: Uuid,
        /// Optionally filter by provider config UUID
        #[arg(long)]
        provider_config: Option<Uuid>,
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
        id: Uuid,
    },
    /// Create a new software item
    Create {
        /// Item name
        #[arg(long)]
        name: String,
        /// Enable or disable on creation
        #[arg(long)]
        enabled: Option<bool>,
    },
    /// Update a software item
    Update {
        /// Software item UUID
        id: Uuid,
        /// New name
        #[arg(long)]
        name: Option<String>,
        /// Enable or disable
        #[arg(long)]
        enabled: Option<bool>,
    },
    /// Delete a software item
    Delete {
        /// Software item UUID
        id: Uuid,
    },
    /// Approve a pending discovered software item
    Approve {
        /// Software item UUID
        id: Uuid,
    },
    /// Assign a host to a software item
    Assign {
        /// Software item UUID
        id: Uuid,
        /// Host UUID
        #[arg(long)]
        host: Uuid,
        /// Provider config UUID
        #[arg(long)]
        provider_config: Option<Uuid>,
        /// Package identifier
        #[arg(long)]
        package: Option<String>,
    },
    /// Unassign a host from a software item
    Unassign {
        /// Software item UUID
        id: Uuid,
        /// Host UUID
        #[arg(long)]
        host: Uuid,
        /// Also create an autodiscovery ignore rule
        #[arg(long, default_value_t = false)]
        ignore: bool,
    },
    /// Trigger update to the latest known version for a software item on a host
    UpdateLatest {
        /// Software item UUID
        id: Uuid,
        /// Host UUID
        #[arg(long)]
        host: Uuid,
    },
}

#[derive(Debug, Subcommand)]
enum CheckCommands {
    /// Trigger bulk version check (all items, all hosts)
    All,
    /// Trigger version check for a software item
    Item {
        /// Software item UUID
        item_id: Uuid,
        /// Optionally scope to a specific host
        #[arg(long)]
        host: Option<Uuid>,
    },
}

#[derive(Debug, Subcommand)]
enum UpdateCommands {
    /// Trigger an update for a software item on a host
    Trigger {
        /// Software item UUID
        item_id: Uuid,
        /// Host UUID
        host_id: Uuid,
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
        host: Option<Uuid>,
        /// Filter by software item UUID
        #[arg(long)]
        software_item: Option<Uuid>,
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
        id: Uuid,
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
        id: Uuid,
    },
    /// Approve a pending service
    Approve {
        /// Service UUID
        id: Uuid,
    },
    /// Reject a pending service
    Reject {
        /// Service UUID
        id: Uuid,
    },
    /// Remove (deactivate) a service
    Remove {
        /// Service UUID
        id: Uuid,
    },
    /// Update a service's settings
    Update {
        /// Service UUID
        id: Uuid,
        /// Custom ping interval in seconds (0 to clear override)
        #[arg(long)]
        ping_interval: Option<u32>,
    },
    /// Merge a source service into a target service
    Merge {
        /// Target service UUID (approved)
        target_id: Uuid,
        /// Source service UUID (pending)
        source_id: Uuid,
    },
}

#[derive(Debug, Subcommand)]
enum SchedulerCommands {
    /// List scheduled tasks
    List,
    /// Show scheduled task details
    Show {
        /// Task UUID
        id: Uuid,
    },
    /// Trigger immediate execution of a scheduled task
    Trigger {
        /// Task UUID
        id: Uuid,
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
        /// Topic prefix (e.g. homeassistant)
        #[arg(long)]
        topic_prefix: Option<String>,
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
enum ProviderConfigsCommands {
    /// List provider configurations
    List {
        /// Page number
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
    },
    /// Show a provider configuration
    Show {
        /// Provider config UUID
        id: Uuid,
    },
    /// Create a new provider configuration
    Create {
        /// Config name
        #[arg(long)]
        name: String,
        /// Provider type (github_releases, docker_registry, homebrew, proxmox_helper_scripts)
        #[arg(long)]
        provider_type: String,
        /// Provider-specific config as JSON string
        #[arg(long)]
        config: Option<String>,
        /// Enable on creation
        #[arg(long)]
        enabled: Option<bool>,
    },
    /// Update a provider configuration
    Update {
        /// Provider config UUID
        id: Uuid,
        /// New name
        #[arg(long)]
        name: Option<String>,
        /// Updated config as JSON string
        #[arg(long)]
        config: Option<String>,
        /// Enable or disable
        #[arg(long)]
        enabled: Option<bool>,
    },
    /// Delete a provider configuration
    Delete {
        /// Provider config UUID
        id: Uuid,
    },
    /// Trigger autodiscovery for a provider config
    Discover {
        /// Provider config UUID
        id: Uuid,
    },
    /// Discard all pending discovered items for a provider config
    DiscardDiscovered {
        /// Provider config UUID
        id: Uuid,
    },
}

#[derive(Debug, Subcommand)]
enum AutodiscoveryCommands {
    /// Manage autodiscovery ignore rules
    Ignores {
        #[command(subcommand)]
        command: IgnoresCommands,
    },
}

#[derive(Debug, Subcommand)]
enum IgnoresCommands {
    /// List autodiscovery ignore rules
    List {
        /// Filter by provider config UUID
        #[arg(long)]
        provider_config: Option<Uuid>,
        /// Page number
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
    },
    /// Create an autodiscovery ignore rule
    Create {
        /// Provider config UUID
        #[arg(long)]
        provider_config: Uuid,
        /// Package identifier to suppress
        #[arg(long)]
        package: String,
    },
    /// Delete an autodiscovery ignore rule
    Delete {
        /// Ignore rule UUID
        id: Uuid,
    },
}

/// Parse a registration mode string into the typed enum.
fn parse_registration_mode(
    s: &str,
) -> std::result::Result<uptrakit_openapi_client::types::registration::RegistrationMode, String> {
    s.parse()
        .map_err(|_| format!("invalid registration mode: {s} (expected open, invite, or closed)"))
}

/// Resolve `--ca-pem` (inline string) or `--ca-pem-file` (file path) into a single
/// `Option<String>`. Clap's `conflicts_with` ensures at most one is provided.
fn resolve_ca_pem(
    ca_pem: Option<String>,
    ca_pem_file: Option<std::path::PathBuf>,
) -> error::Result<Option<String>> {
    match (ca_pem, ca_pem_file) {
        (Some(pem), None) => Ok(Some(pem)),
        (None, Some(path)) => {
            let contents = std::fs::read_to_string(&path).context_to()?;
            Ok(Some(contents))
        }
        _ => Ok(None),
    }
}

async fn run(cli: Cli) -> error::Result<()> {
    if cli.version {
        let build_info = BuildInfo::current(
            "uptrakit",
            env!("CARGO_PKG_VERSION"),
            option_env!("UPTRAKIT_BUILD_ENABLED_FEATURES"),
        );
        output::print_output(cli.output, &build_info)?;
        return Ok(());
    }

    if cli.verbose > 4 {
        eprintln!(
            "warning: -vvvvv or more has no additional effect; maximum verbosity is -vvvv (trace)"
        );
    }
    if cli.verbose > 0 {
        let level = match cli.verbose {
            1 => "warn",
            2 => "info",
            3 => "debug",
            _ => "trace",
        };
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_env_filter(
                EnvFilter::from_default_env()
                    .add_directive(level.parse().expect("valid level directive")),
            )
            .init();
    }

    if cli.insecure {
        eprintln!("WARNING: TLS certificate verification is disabled. Connection is insecure.");
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

    let format = cli.output;
    let insecure = cli.insecure;
    let request_timeout = cli.timeout.map(std::time::Duration::from_secs);

    match command {
        Commands::Auth { command } => match command {
            AuthCommands::Login => {
                commands::auth::login(cli.server.as_deref(), insecure).await?;
            }
            AuthCommands::Status => {
                let resp = commands::auth::status(
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    insecure,
                    request_timeout,
                )
                .await?;
                output::print_output(format, &resp)?;
            }
            AuthCommands::Token { command } => match command {
                TokenCommands::Create { name } => {
                    let resp = commands::auth::token_create(
                        &name,
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
                TokenCommands::List => {
                    let resp = commands::auth::token_list(
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
                TokenCommands::Revoke { id } => {
                    let resp = commands::auth::token_revoke(
                        &id,
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
            },
        },
        Commands::Api { method, path, data } => {
            commands::api::execute(commands::api::ExecuteParams {
                method: &method,
                path: &path,
                data: data.as_deref(),
                server: cli.server.as_deref(),
                token: cli.token.as_deref(),
                format: cli.output,
                insecure,
                request_timeout,
            })
            .await?;
        }
        Commands::Services { command } => match command {
            ServicesCommands::List {
                r#type,
                status,
                page,
                per_page,
            } => {
                let resp = commands::services::list(commands::services::ListParams {
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    service_type: r#type.as_deref(),
                    status: status.as_deref(),
                    page,
                    per_page,
                    request_timeout,
                })
                .await?;
                output::print_output(format, &resp)?;
            }
            ServicesCommands::Show { id } => {
                let resp = commands::services::show(
                    &id,
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    insecure,
                    request_timeout,
                )
                .await?;
                output::print_output(format, &resp)?;
            }
            ServicesCommands::Approve { id } => {
                let resp = commands::services::approve(
                    &id,
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    insecure,
                    request_timeout,
                )
                .await?;
                output::print_output(format, &resp)?;
            }
            ServicesCommands::Reject { id } => {
                let resp = commands::services::reject(
                    &id,
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    insecure,
                    request_timeout,
                )
                .await?;
                output::print_output(format, &resp)?;
            }
            ServicesCommands::Remove { id } => {
                let resp = commands::services::remove(
                    &id,
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    insecure,
                    request_timeout,
                )
                .await?;
                output::print_output(format, &resp)?;
            }
            ServicesCommands::Update { id, ping_interval } => {
                let resp = commands::services::update(
                    &id,
                    ping_interval,
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    insecure,
                    request_timeout,
                )
                .await?;
                output::print_output(format, &resp)?;
            }
            ServicesCommands::Merge {
                target_id,
                source_id,
            } => {
                let resp = commands::services::merge(
                    &target_id,
                    &source_id,
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    insecure,
                    request_timeout,
                )
                .await?;
                output::print_output(format, &resp)?;
            }
        },
        Commands::Hosts { command } => match command {
            HostsCommands::List { page, per_page } => {
                let resp = commands::hosts::list(commands::hosts::ListParams {
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    page,
                    per_page,
                    request_timeout,
                })
                .await?;
                output::print_output(format, &resp)?;
            }
            HostsCommands::Show { id } => {
                let resp = commands::hosts::show(commands::hosts::ShowParams {
                    id: &id,
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    request_timeout,
                })
                .await?;
                output::print_output(format, &resp)?;
            }
            HostsCommands::Update { id, friendly_name } => {
                let resp = commands::hosts::update(commands::hosts::UpdateParams {
                    id: &id,
                    friendly_name,
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    request_timeout,
                })
                .await?;
                output::print_output(format, &resp)?;
            }
            HostsCommands::Deactivate { id } => {
                let resp = commands::hosts::deactivate(commands::hosts::DeactivateParams {
                    id: &id,
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    request_timeout,
                })
                .await?;
                output::print_output(format, &resp)?;
            }
            HostsCommands::Discover { id } => {
                let resp = commands::hosts::discover(commands::hosts::DiscoverParams {
                    id: &id,
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    request_timeout,
                })
                .await?;
                output::print_output(format, &resp)?;
            }
            HostsCommands::DiscardDiscovered {
                id,
                provider_config,
            } => {
                let resp =
                    commands::hosts::discard_discovered(commands::hosts::DiscardDiscoveredParams {
                        id: &id,
                        provider_config_id: provider_config.as_ref(),
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    })
                    .await?;
                output::print_output(format, &resp)?;
            }
        },
        Commands::SoftwareItems { command } => match command {
            SoftwareItemsCommands::List { page, per_page } => {
                let resp = commands::software_items::list(commands::software_items::ListParams {
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    page,
                    per_page,
                    request_timeout,
                })
                .await?;
                output::print_output(format, &resp)?;
            }
            SoftwareItemsCommands::Show { id } => {
                let resp = commands::software_items::show(commands::software_items::ShowParams {
                    id: &id,
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    request_timeout,
                })
                .await?;
                output::print_output(format, &resp)?;
            }
            SoftwareItemsCommands::Create { name, enabled } => {
                let resp =
                    commands::software_items::create(commands::software_items::CreateParams {
                        name,
                        enabled,
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    })
                    .await?;
                output::print_output(format, &resp)?;
            }
            SoftwareItemsCommands::Update { id, name, enabled } => {
                let resp =
                    commands::software_items::update(commands::software_items::UpdateParams {
                        id: &id,
                        name,
                        enabled,
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    })
                    .await?;
                output::print_output(format, &resp)?;
            }
            SoftwareItemsCommands::Delete { id } => {
                let resp =
                    commands::software_items::delete(commands::software_items::DeleteParams {
                        id: &id,
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    })
                    .await?;
                output::print_output(format, &resp)?;
            }
            SoftwareItemsCommands::Approve { id } => {
                let resp =
                    commands::software_items::approve(commands::software_items::ApproveParams {
                        id: &id,
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    })
                    .await?;
                output::print_output(format, &resp)?;
            }
            SoftwareItemsCommands::Assign {
                id,
                host,
                provider_config,
                package,
            } => {
                let resp =
                    commands::software_items::assign(commands::software_items::AssignParams {
                        id: &id,
                        host_id: &host,
                        provider_config_id: provider_config.as_ref(),
                        package_identifier: package,
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    })
                    .await?;
                output::print_output(format, &resp)?;
            }
            SoftwareItemsCommands::Unassign { id, host, ignore } => {
                let resp =
                    commands::software_items::unassign(commands::software_items::UnassignParams {
                        id: &id,
                        host_id: &host,
                        ignore,
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    })
                    .await?;
                output::print_output(format, &resp)?;
            }
            SoftwareItemsCommands::UpdateLatest { id, host } => {
                let resp = commands::software_items::update_latest(
                    commands::software_items::UpdateLatestParams {
                        id: &id,
                        host_id: &host,
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    },
                )
                .await?;
                output::print_output(format, &resp)?;
            }
        },
        Commands::Check { command } => match command {
            CheckCommands::All => {
                let resp = commands::check::all(commands::check::AllParams {
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    request_timeout,
                })
                .await?;
                output::print_output(format, &resp)?;
            }
            CheckCommands::Item { item_id, host } => {
                let resp = commands::check::item(commands::check::ItemParams {
                    item_id: &item_id,
                    host_id: host.as_ref(),
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    request_timeout,
                })
                .await?;
                output::print_output(format, &resp)?;
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
                let resp = commands::update::trigger(commands::update::TriggerParams {
                    item_id: &item_id,
                    host_id: &host_id,
                    to_version: &to_version,
                    release_tag: release_tag.as_deref(),
                    release_url: release_url.as_deref(),
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    request_timeout,
                })
                .await?;
                output::print_output(format, &resp)?;
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
                let resp = commands::history::list(commands::history::ListParams {
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    host_id: host,
                    software_item_id: software_item,
                    status: status.as_deref(),
                    page,
                    per_page,
                    request_timeout,
                })
                .await?;
                output::print_output(format, &resp)?;
            }
            HistoryCommands::Show { id } => {
                let resp = commands::history::show(
                    &id,
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    insecure,
                    request_timeout,
                )
                .await?;
                output::print_output(format, &resp)?;
            }
        },
        Commands::Scheduler { command } => match command {
            SchedulerCommands::List => {
                let resp = commands::scheduler::list(commands::scheduler::ListParams {
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    request_timeout,
                })
                .await?;
                output::print_output(format, &resp)?;
            }
            SchedulerCommands::Show { id } => {
                let resp = commands::scheduler::show(commands::scheduler::ShowParams {
                    id: &id,
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    request_timeout,
                })
                .await?;
                output::print_output(format, &resp)?;
            }
            SchedulerCommands::Trigger { id } => {
                let resp = commands::scheduler::trigger(commands::scheduler::TriggerParams {
                    id: &id,
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    request_timeout,
                })
                .await?;
                output::print_output(format, &resp)?;
            }
        },
        Commands::Settings { command } => match command {
            SettingsCommands::Show => {
                let resp = commands::settings::show_combined(
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    insecure,
                    request_timeout,
                )
                .await?;
                output::print_output(format, &resp)?;
            }
            SettingsCommands::Registration { command } => match command {
                RegistrationCommands::Show => {
                    let resp = commands::settings::registration_show(
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
                RegistrationCommands::Update {
                    mode,
                    token,
                    require_token_for_oidc,
                } => {
                    let resp = commands::settings::registration_update(
                        commands::settings::RegistrationUpdateParams {
                            server: cli.server.as_deref(),
                            token: cli.token.as_deref(),
                            insecure,
                            mode,
                            reg_token: token,
                            require_token_for_oidc,
                            request_timeout,
                        },
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
            },
            SettingsCommands::Authentication { command } => match command {
                AuthenticationCommands::Show => {
                    let resp = commands::settings::authentication_show(
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
                AuthenticationCommands::Update {
                    password_auth_enabled,
                } => {
                    let resp = commands::settings::authentication_update(
                        password_auth_enabled,
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
            },
            SettingsCommands::Certificates { command } => match command {
                CertificateCommands::Show => {
                    let resp = commands::settings::certificates_show(
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
                CertificateCommands::Update {
                    lifetime_days,
                    renewal_window_hours,
                } => {
                    let resp = commands::settings::certificates_update(
                        lifetime_days,
                        renewal_window_hours,
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
            },
            SettingsCommands::Network { command } => match command {
                NetworkCommands::Show => {
                    let resp = commands::settings::network_show(
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    )
                    .await?;
                    output::print_output(format, &resp)?;
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
                    let resp = commands::settings::network_update(
                        commands::settings::NetworkUpdateParams {
                            server: cli.server.as_deref(),
                            token: cli.token.as_deref(),
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
                            request_timeout,
                        },
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
            },
            SettingsCommands::RotateCa => {
                let resp = commands::settings::rotate_ca(
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    insecure,
                    request_timeout,
                )
                .await?;
                output::print_output(format, &resp)?;
            }
            SettingsCommands::RenewServerCert => {
                let resp = commands::settings::renew_server_cert(
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    insecure,
                    request_timeout,
                )
                .await?;
                output::print_output(format, &resp)?;
            }
            SettingsCommands::Mqtt { command } => match command {
                MqttCommands::List => {
                    let resp = commands::settings::mqtt_list(
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
                MqttCommands::Show { id } => {
                    let resp = commands::settings::mqtt_show(
                        &id,
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    )
                    .await?;
                    output::print_output(format, &resp)?;
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
                } => {
                    let ca_pem = resolve_ca_pem(ca_pem, ca_pem_file)?;
                    let resp =
                        commands::settings::mqtt_create(commands::settings::MqttCreateParams {
                            server: cli.server.as_deref(),
                            token: cli.token.as_deref(),
                            insecure,
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
                            request_timeout,
                        })
                        .await?;
                    output::print_output(format, &resp)?;
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
                } => {
                    let ca_pem = resolve_ca_pem(ca_pem, ca_pem_file)?;
                    let resp =
                        commands::settings::mqtt_update(commands::settings::MqttUpdateParams {
                            server: cli.server.as_deref(),
                            token: cli.token.as_deref(),
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
                            ca_pem,
                            topic_prefix,
                            request_timeout,
                        })
                        .await?;
                    output::print_output(format, &resp)?;
                }
                MqttCommands::Delete { id } => {
                    let resp = commands::settings::mqtt_delete(
                        &id,
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
                MqttCommands::Limit { command } => match command {
                    MqttLimitCommands::Show => {
                        let resp = commands::settings::mqtt_limit_show(
                            cli.server.as_deref(),
                            cli.token.as_deref(),
                            insecure,
                            request_timeout,
                        )
                        .await?;
                        output::print_output(format, &resp)?;
                    }
                    MqttLimitCommands::Update { max } => {
                        let resp = commands::settings::mqtt_limit_update(
                            max,
                            cli.server.as_deref(),
                            cli.token.as_deref(),
                            insecure,
                            request_timeout,
                        )
                        .await?;
                        output::print_output(format, &resp)?;
                    }
                },
            },
            SettingsCommands::Oidc { command } => match command {
                OidcCommands::List => {
                    let resp = commands::settings::oidc_list(
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
                OidcCommands::Show { id } => {
                    let resp = commands::settings::oidc_show(
                        &id,
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    )
                    .await?;
                    output::print_output(format, &resp)?;
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
                    let resp =
                        commands::settings::oidc_create(commands::settings::OidcCreateParams {
                            server: cli.server.as_deref(),
                            token: cli.token.as_deref(),
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
                            request_timeout,
                        })
                        .await?;
                    output::print_output(format, &resp)?;
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
                    let resp =
                        commands::settings::oidc_update(commands::settings::OidcUpdateParams {
                            server: cli.server.as_deref(),
                            token: cli.token.as_deref(),
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
                            request_timeout,
                        })
                        .await?;
                    output::print_output(format, &resp)?;
                }
                OidcCommands::Delete { id } => {
                    let resp = commands::settings::oidc_delete(
                        &id,
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
                OidcCommands::Activate { id } => {
                    let resp = commands::settings::oidc_activate(
                        &id,
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
                OidcCommands::Deactivate { id } => {
                    let resp = commands::settings::oidc_deactivate(
                        &id,
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
            },
            SettingsCommands::Alerts => {
                let resp = commands::settings::alerts(
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    insecure,
                    request_timeout,
                )
                .await?;
                output::print_output(format, &resp)?;
            }
        },
        Commands::ProviderConfigs { command } => match command {
            ProviderConfigsCommands::List { page, per_page } => {
                let resp =
                    commands::provider_configs::list(commands::provider_configs::ListParams {
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        page,
                        per_page,
                        request_timeout,
                    })
                    .await?;
                output::print_output(format, &resp)?;
            }
            ProviderConfigsCommands::Show { id } => {
                let resp =
                    commands::provider_configs::show(commands::provider_configs::ShowParams {
                        id: &id,
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    })
                    .await?;
                output::print_output(format, &resp)?;
            }
            ProviderConfigsCommands::Create {
                name,
                provider_type,
                config,
                enabled,
            } => {
                let provider_type: ProviderType = provider_type.parse().map_err(|_| {
                    report!(error::CliError::Other(format!(
                        "unknown provider type: {provider_type}"
                    )))
                })?;
                let config_value: serde_json::Value = match config {
                    Some(s) => serde_json::from_str(&s).map_err(|e| {
                        report!(error::CliError::Other(format!(
                            "invalid JSON for --config: {e}"
                        )))
                    })?,
                    None => serde_json::Value::Object(serde_json::Map::new()),
                };
                let resp =
                    commands::provider_configs::create(commands::provider_configs::CreateParams {
                        name,
                        provider_type,
                        config: config_value,
                        enabled,
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    })
                    .await?;
                output::print_output(format, &resp)?;
            }
            ProviderConfigsCommands::Update {
                id,
                name,
                config,
                enabled,
            } => {
                let config_value: Option<serde_json::Value> = match config {
                    Some(s) => Some(serde_json::from_str(&s).map_err(|e| {
                        report!(error::CliError::Other(format!(
                            "invalid JSON for --config: {e}"
                        )))
                    })?),
                    None => None,
                };
                let resp =
                    commands::provider_configs::update(commands::provider_configs::UpdateParams {
                        id: &id,
                        name,
                        config: config_value,
                        enabled,
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    })
                    .await?;
                output::print_output(format, &resp)?;
            }
            ProviderConfigsCommands::Delete { id } => {
                let resp =
                    commands::provider_configs::delete(commands::provider_configs::DeleteParams {
                        id: &id,
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    })
                    .await?;
                output::print_output(format, &resp)?;
            }
            ProviderConfigsCommands::Discover { id } => {
                let resp = commands::provider_configs::discover(
                    commands::provider_configs::DiscoverParams {
                        id: &id,
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    },
                )
                .await?;
                output::print_output(format, &resp)?;
            }
            ProviderConfigsCommands::DiscardDiscovered { id } => {
                let resp = commands::provider_configs::discard_discovered(
                    commands::provider_configs::DiscardDiscoveredParams {
                        id: &id,
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    },
                )
                .await?;
                output::print_output(format, &resp)?;
            }
        },
        Commands::Autodiscovery { command } => match command {
            AutodiscoveryCommands::Ignores { command } => match command {
                IgnoresCommands::List {
                    provider_config,
                    page,
                    per_page,
                } => {
                    let resp = commands::autodiscovery::ignores_list(
                        commands::autodiscovery::IgnoresListParams {
                            provider_config_id: provider_config.as_ref(),
                            server: cli.server.as_deref(),
                            token: cli.token.as_deref(),
                            insecure,
                            page,
                            per_page,
                            request_timeout,
                        },
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
                IgnoresCommands::Create {
                    provider_config,
                    package,
                } => {
                    let resp = commands::autodiscovery::ignores_create(
                        commands::autodiscovery::IgnoresCreateParams {
                            provider_config_id: &provider_config,
                            package_identifier: package,
                            server: cli.server.as_deref(),
                            token: cli.token.as_deref(),
                            insecure,
                            request_timeout,
                        },
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
                IgnoresCommands::Delete { id } => {
                    let resp = commands::autodiscovery::ignores_delete(
                        commands::autodiscovery::IgnoresDeleteParams {
                            id: &id,
                            server: cli.server.as_deref(),
                            token: cli.token.as_deref(),
                            insecure,
                            request_timeout,
                        },
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
            },
        },
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli).await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Test UUID constants for readability.
    const HOST_UUID: &str = "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6";
    const ITEM_UUID: &str = "b1b2b3b4-c1c2-d1d2-e1e2-f1f2f3f4f5f6";
    const SVC_UUID: &str = "c1c2c3c4-d1d2-e1e2-f1f2-a1a2a3a4a5a6";
    const TASK_UUID: &str = "d1d2d3d4-e1e2-f1f2-a1a2-b1b2b3b4b5b6";
    const HIST_UUID: &str = "e1e2e3e4-f1f2-a1a2-b1b2-c1c2c3c4c5c6";
    const MQTT_UUID: &str = "01020304-0506-0708-090a-0b0c0d0e0f10";
    const OIDC_UUID: &str = "11121314-1516-1718-191a-1b1c1d1e1f20";
    const TARGET_UUID: &str = "aa000000-bb00-cc00-dd00-ee0000000001";
    const SOURCE_UUID: &str = "aa000000-bb00-cc00-dd00-ee0000000002";
    const PC_UUID: &str = "aa100000-bb00-cc00-dd00-ee0000000001";
    const IGNORE_UUID: &str = "aa200000-bb00-cc00-dd00-ee0000000001";

    /// Parse a UUID constant (safe in tests).
    fn uuid(s: &str) -> Uuid {
        s.parse().expect("test UUID constant should be valid")
    }

    #[test]
    fn verbose_flag_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "-v", "-v", "-v"]).expect("should parse -v flags");
        assert_eq!(args.verbose, 3);
    }

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
        let args =
            Cli::try_parse_from(["uptrakit", "hosts", "show", HOST_UUID]).expect("should parse");
        match args.command {
            Some(Commands::Hosts {
                command: HostsCommands::Show { id },
            }) => {
                assert_eq!(id, uuid(HOST_UUID));
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
        let args = Cli::try_parse_from(["uptrakit", "software-items", "show", ITEM_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::SoftwareItems {
                command: SoftwareItemsCommands::Show { id },
            }) => {
                assert_eq!(id, uuid(ITEM_UUID));
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
    fn check_item_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "check", "item", ITEM_UUID]).expect("should parse");
        match args.command {
            Some(Commands::Check {
                command: CheckCommands::Item { item_id, host },
            }) => {
                assert_eq!(item_id, uuid(ITEM_UUID));
                assert!(host.is_none());
            }
            _ => panic!("expected Check Item"),
        }
    }

    #[test]
    fn check_item_with_host_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "check", "item", ITEM_UUID, "--host", HOST_UUID])
                .expect("should parse");
        match args.command {
            Some(Commands::Check {
                command: CheckCommands::Item { item_id, host },
            }) => {
                assert_eq!(item_id, uuid(ITEM_UUID));
                assert_eq!(host, Some(uuid(HOST_UUID)));
            }
            _ => panic!("expected Check Item"),
        }
    }

    #[test]
    fn update_trigger_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "update",
            "trigger",
            ITEM_UUID,
            HOST_UUID,
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
                assert_eq!(item_id, uuid(ITEM_UUID));
                assert_eq!(host_id, uuid(HOST_UUID));
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
            ITEM_UUID,
            HOST_UUID,
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
            HOST_UUID,
            "--software-item",
            ITEM_UUID,
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
                assert_eq!(host, Some(uuid(HOST_UUID)));
                assert_eq!(software_item, Some(uuid(ITEM_UUID)));
                assert_eq!(status.as_deref(), Some("completed"));
            }
            _ => panic!("expected History List"),
        }
    }

    #[test]
    fn history_show_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "history", "show", HIST_UUID]).expect("should parse");
        match args.command {
            Some(Commands::History {
                command: HistoryCommands::Show { id },
            }) => {
                assert_eq!(id, uuid(HIST_UUID));
            }
            _ => panic!("expected History Show"),
        }
    }

    #[test]
    fn scheduler_list_parses() {
        let args = Cli::try_parse_from(["uptrakit", "scheduler", "list"]).expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::Scheduler {
                command: SchedulerCommands::List
            })
        ));
    }

    #[test]
    fn scheduler_show_parses() {
        let args = Cli::try_parse_from(["uptrakit", "scheduler", "show", TASK_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::Scheduler {
                command: SchedulerCommands::Show { id },
            }) => {
                assert_eq!(id, uuid(TASK_UUID));
            }
            _ => panic!("expected Scheduler Show"),
        }
    }

    #[test]
    fn scheduler_trigger_parses() {
        let args = Cli::try_parse_from(["uptrakit", "scheduler", "trigger", TASK_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::Scheduler {
                command: SchedulerCommands::Trigger { id },
            }) => {
                assert_eq!(id, uuid(TASK_UUID));
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
        let args =
            Cli::try_parse_from(["uptrakit", "services", "show", SVC_UUID]).expect("should parse");
        match args.command {
            Some(Commands::Services {
                command: ServicesCommands::Show { id },
            }) => {
                assert_eq!(id, uuid(SVC_UUID));
            }
            _ => panic!("expected Services Show"),
        }
    }

    #[test]
    fn services_approve_parses() {
        let args = Cli::try_parse_from(["uptrakit", "services", "approve", SVC_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::Services {
                command: ServicesCommands::Approve { id },
            }) => {
                assert_eq!(id, uuid(SVC_UUID));
            }
            _ => panic!("expected Services Approve"),
        }
    }

    #[test]
    fn services_reject_parses() {
        let args = Cli::try_parse_from(["uptrakit", "services", "reject", SVC_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::Services {
                command: ServicesCommands::Reject { id },
            }) => {
                assert_eq!(id, uuid(SVC_UUID));
            }
            _ => panic!("expected Services Reject"),
        }
    }

    #[test]
    fn services_remove_parses() {
        let args = Cli::try_parse_from(["uptrakit", "services", "remove", SVC_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::Services {
                command: ServicesCommands::Remove { id },
            }) => {
                assert_eq!(id, uuid(SVC_UUID));
            }
            _ => panic!("expected Services Remove"),
        }
    }

    #[test]
    fn services_merge_parses() {
        let args = Cli::try_parse_from(["uptrakit", "services", "merge", TARGET_UUID, SOURCE_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::Services {
                command:
                    ServicesCommands::Merge {
                        target_id,
                        source_id,
                    },
            }) => {
                assert_eq!(target_id, uuid(TARGET_UUID));
                assert_eq!(source_id, uuid(SOURCE_UUID));
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
        let args = Cli::try_parse_from(["uptrakit", "settings", "show"]).expect("should parse");
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
                        command:
                            AuthenticationCommands::Update {
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
        let args =
            Cli::try_parse_from(["uptrakit", "settings", "network", "show"]).expect("should parse");
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
                assert_eq!(trusted_proxies.as_deref(), Some("10.0.0.0/8,172.16.0.0/12"));
                assert_eq!(real_ip_header.as_deref(), Some("X-Real-IP"));
            }
            _ => panic!("expected Settings Network Update"),
        }
    }

    #[test]
    fn settings_rotate_ca_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "settings", "rotate-ca"]).expect("should parse");
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
        let args = Cli::try_parse_from(["uptrakit", "settings", "mqtt", "show", MQTT_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::Settings {
                command:
                    SettingsCommands::Mqtt {
                        command: MqttCommands::Show { id },
                    },
            }) => {
                assert_eq!(id, uuid(MQTT_UUID));
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
            MQTT_UUID,
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
                                id, enabled, host, ..
                            },
                    },
            }) => {
                assert_eq!(id, uuid(MQTT_UUID));
                assert_eq!(enabled, Some(false));
                assert_eq!(host.as_deref(), Some("new-broker"));
            }
            _ => panic!("expected Settings Mqtt Update"),
        }
    }

    #[test]
    fn settings_mqtt_delete_parses() {
        let args = Cli::try_parse_from(["uptrakit", "settings", "mqtt", "delete", MQTT_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::Settings {
                command:
                    SettingsCommands::Mqtt {
                        command: MqttCommands::Delete { id },
                    },
            }) => {
                assert_eq!(id, uuid(MQTT_UUID));
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
            "uptrakit", "settings", "mqtt", "limit", "update", "--max", "10",
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
        let args = Cli::try_parse_from(["uptrakit", "settings", "oidc", "show", OIDC_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::Settings {
                command:
                    SettingsCommands::Oidc {
                        command: OidcCommands::Show { id },
                    },
            }) => {
                assert_eq!(id, uuid(OIDC_UUID));
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
            OIDC_UUID,
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
                assert_eq!(id, uuid(OIDC_UUID));
                assert_eq!(name.as_deref(), Some("Google Workspace"));
                assert_eq!(auto_create_users, Some(false));
            }
            _ => panic!("expected Settings Oidc Update"),
        }
    }

    #[test]
    fn settings_oidc_delete_parses() {
        let args = Cli::try_parse_from(["uptrakit", "settings", "oidc", "delete", OIDC_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::Settings {
                command:
                    SettingsCommands::Oidc {
                        command: OidcCommands::Delete { id },
                    },
            }) => {
                assert_eq!(id, uuid(OIDC_UUID));
            }
            _ => panic!("expected Settings Oidc Delete"),
        }
    }

    #[test]
    fn settings_oidc_activate_parses() {
        let args = Cli::try_parse_from(["uptrakit", "settings", "oidc", "activate", OIDC_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::Settings {
                command:
                    SettingsCommands::Oidc {
                        command: OidcCommands::Activate { id },
                    },
            }) => {
                assert_eq!(id, uuid(OIDC_UUID));
            }
            _ => panic!("expected Settings Oidc Activate"),
        }
    }

    #[test]
    fn settings_oidc_deactivate_parses() {
        let args = Cli::try_parse_from(["uptrakit", "settings", "oidc", "deactivate", OIDC_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::Settings {
                command:
                    SettingsCommands::Oidc {
                        command: OidcCommands::Deactivate { id },
                    },
            }) => {
                assert_eq!(id, uuid(OIDC_UUID));
            }
            _ => panic!("expected Settings Oidc Deactivate"),
        }
    }

    #[test]
    fn settings_alerts_parses() {
        let args = Cli::try_parse_from(["uptrakit", "settings", "alerts"]).expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::Settings {
                command: SettingsCommands::Alerts
            })
        ));
    }

    #[test]
    fn hosts_update_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "hosts",
            "update",
            HOST_UUID,
            "--friendly-name",
            "My Server",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::Hosts {
                command: HostsCommands::Update { id, friendly_name },
            }) => {
                assert_eq!(id, uuid(HOST_UUID));
                assert_eq!(friendly_name.as_deref(), Some("My Server"));
            }
            _ => panic!("expected Hosts Update"),
        }
    }

    #[test]
    fn hosts_deactivate_parses() {
        let args = Cli::try_parse_from(["uptrakit", "hosts", "deactivate", HOST_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::Hosts {
                command: HostsCommands::Deactivate { id },
            }) => {
                assert_eq!(id, uuid(HOST_UUID));
            }
            _ => panic!("expected Hosts Deactivate"),
        }
    }

    #[test]
    fn hosts_discover_parses() {
        let args = Cli::try_parse_from(["uptrakit", "hosts", "discover", HOST_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::Hosts {
                command: HostsCommands::Discover { id },
            }) => {
                assert_eq!(id, uuid(HOST_UUID));
            }
            _ => panic!("expected Hosts Discover"),
        }
    }

    #[test]
    fn hosts_discard_discovered_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "hosts",
            "discard-discovered",
            HOST_UUID,
            "--provider-config",
            PC_UUID,
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::Hosts {
                command:
                    HostsCommands::DiscardDiscovered {
                        id,
                        provider_config,
                    },
            }) => {
                assert_eq!(id, uuid(HOST_UUID));
                assert_eq!(provider_config, Some(uuid(PC_UUID)));
            }
            _ => panic!("expected Hosts DiscardDiscovered"),
        }
    }

    #[test]
    fn software_items_create_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "software-items", "create", "--name", "My App"])
                .expect("should parse");
        match args.command {
            Some(Commands::SoftwareItems {
                command: SoftwareItemsCommands::Create { name, enabled },
            }) => {
                assert_eq!(name, "My App");
                assert!(enabled.is_none());
            }
            _ => panic!("expected SoftwareItems Create"),
        }
    }

    #[test]
    fn software_items_update_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "software-items",
            "update",
            ITEM_UUID,
            "--name",
            "Updated App",
            "--enabled",
            "false",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::SoftwareItems {
                command: SoftwareItemsCommands::Update { id, name, enabled },
            }) => {
                assert_eq!(id, uuid(ITEM_UUID));
                assert_eq!(name.as_deref(), Some("Updated App"));
                assert_eq!(enabled, Some(false));
            }
            _ => panic!("expected SoftwareItems Update"),
        }
    }

    #[test]
    fn software_items_delete_parses() {
        let args = Cli::try_parse_from(["uptrakit", "software-items", "delete", ITEM_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::SoftwareItems {
                command: SoftwareItemsCommands::Delete { id },
            }) => {
                assert_eq!(id, uuid(ITEM_UUID));
            }
            _ => panic!("expected SoftwareItems Delete"),
        }
    }

    #[test]
    fn software_items_approve_parses() {
        let args = Cli::try_parse_from(["uptrakit", "software-items", "approve", ITEM_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::SoftwareItems {
                command: SoftwareItemsCommands::Approve { id },
            }) => {
                assert_eq!(id, uuid(ITEM_UUID));
            }
            _ => panic!("expected SoftwareItems Approve"),
        }
    }

    #[test]
    fn software_items_assign_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "software-items",
            "assign",
            ITEM_UUID,
            "--host",
            HOST_UUID,
            "--provider-config",
            PC_UUID,
            "--package",
            "org/app",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::SoftwareItems {
                command:
                    SoftwareItemsCommands::Assign {
                        id,
                        host,
                        provider_config,
                        package,
                    },
            }) => {
                assert_eq!(id, uuid(ITEM_UUID));
                assert_eq!(host, uuid(HOST_UUID));
                assert_eq!(provider_config, Some(uuid(PC_UUID)));
                assert_eq!(package.as_deref(), Some("org/app"));
            }
            _ => panic!("expected SoftwareItems Assign"),
        }
    }

    #[test]
    fn software_items_unassign_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "software-items",
            "unassign",
            ITEM_UUID,
            "--host",
            HOST_UUID,
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::SoftwareItems {
                command: SoftwareItemsCommands::Unassign { id, host, ignore },
            }) => {
                assert_eq!(id, uuid(ITEM_UUID));
                assert_eq!(host, uuid(HOST_UUID));
                assert!(!ignore);
            }
            _ => panic!("expected SoftwareItems Unassign"),
        }
    }

    #[test]
    fn software_items_unassign_with_ignore_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "software-items",
            "unassign",
            ITEM_UUID,
            "--host",
            HOST_UUID,
            "--ignore",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::SoftwareItems {
                command: SoftwareItemsCommands::Unassign { ignore, .. },
            }) => {
                assert!(ignore);
            }
            _ => panic!("expected SoftwareItems Unassign"),
        }
    }

    #[test]
    fn provider_configs_list_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "provider-configs", "list"]).expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::ProviderConfigs {
                command: ProviderConfigsCommands::List { .. }
            })
        ));
    }

    #[test]
    fn provider_configs_show_parses() {
        let args = Cli::try_parse_from(["uptrakit", "provider-configs", "show", PC_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::ProviderConfigs {
                command: ProviderConfigsCommands::Show { id },
            }) => {
                assert_eq!(id, uuid(PC_UUID));
            }
            _ => panic!("expected ProviderConfigs Show"),
        }
    }

    #[test]
    fn provider_configs_create_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "provider-configs",
            "create",
            "--name",
            "My GitHub",
            "--provider-type",
            "github_releases",
            "--config",
            r#"{"owner":"org","repo":"app"}"#,
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::ProviderConfigs {
                command:
                    ProviderConfigsCommands::Create {
                        name,
                        provider_type,
                        ..
                    },
            }) => {
                assert_eq!(name, "My GitHub");
                assert_eq!(provider_type, "github_releases");
            }
            _ => panic!("expected ProviderConfigs Create"),
        }
    }

    #[test]
    fn provider_configs_delete_parses() {
        let args = Cli::try_parse_from(["uptrakit", "provider-configs", "delete", PC_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::ProviderConfigs {
                command: ProviderConfigsCommands::Delete { id },
            }) => {
                assert_eq!(id, uuid(PC_UUID));
            }
            _ => panic!("expected ProviderConfigs Delete"),
        }
    }

    #[test]
    fn provider_configs_discover_parses() {
        let args = Cli::try_parse_from(["uptrakit", "provider-configs", "discover", PC_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::ProviderConfigs {
                command: ProviderConfigsCommands::Discover { id },
            }) => {
                assert_eq!(id, uuid(PC_UUID));
            }
            _ => panic!("expected ProviderConfigs Discover"),
        }
    }

    #[test]
    fn provider_configs_discard_discovered_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "provider-configs",
            "discard-discovered",
            PC_UUID,
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::ProviderConfigs {
                command: ProviderConfigsCommands::DiscardDiscovered { id },
            }) => {
                assert_eq!(id, uuid(PC_UUID));
            }
            _ => panic!("expected ProviderConfigs DiscardDiscovered"),
        }
    }

    #[test]
    fn autodiscovery_ignores_list_parses() {
        let args = Cli::try_parse_from(["uptrakit", "autodiscovery", "ignores", "list"])
            .expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::Autodiscovery {
                command: AutodiscoveryCommands::Ignores {
                    command: IgnoresCommands::List { .. }
                }
            })
        ));
    }

    #[test]
    fn autodiscovery_ignores_create_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "autodiscovery",
            "ignores",
            "create",
            "--provider-config",
            PC_UUID,
            "--package",
            "org/app",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::Autodiscovery {
                command:
                    AutodiscoveryCommands::Ignores {
                        command:
                            IgnoresCommands::Create {
                                provider_config,
                                package,
                            },
                    },
            }) => {
                assert_eq!(provider_config, uuid(PC_UUID));
                assert_eq!(package, "org/app");
            }
            _ => panic!("expected Autodiscovery Ignores Create"),
        }
    }

    #[test]
    fn autodiscovery_ignores_delete_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "autodiscovery",
            "ignores",
            "delete",
            IGNORE_UUID,
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::Autodiscovery {
                command:
                    AutodiscoveryCommands::Ignores {
                        command: IgnoresCommands::Delete { id },
                    },
            }) => {
                assert_eq!(id, uuid(IGNORE_UUID));
            }
            _ => panic!("expected Autodiscovery Ignores Delete"),
        }
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

    #[test]
    fn rejects_invalid_uuid_for_id_arguments() {
        let result = Cli::try_parse_from(["uptrakit", "hosts", "show", "not-a-uuid"]);
        assert!(result.is_err());
    }

    #[test]
    fn global_timeout_parses() {
        let args = Cli::try_parse_from(["uptrakit", "--timeout", "60", "hosts", "list"])
            .expect("should parse");
        assert_eq!(args.timeout, Some(60));
    }

    #[test]
    fn global_timeout_defaults_to_none() {
        let args = Cli::try_parse_from(["uptrakit", "hosts", "list"]).expect("should parse");
        assert!(args.timeout.is_none());
    }
}
