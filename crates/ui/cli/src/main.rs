use uptrakit_cli::{commands, error, output};

use clap::{CommandFactory, Parser, Subcommand};
use rootcause::prelude::*;
use std::ffi::OsString;
use tracing_subscriber::EnvFilter;
use uptrakit_build_info::BuildInfo;
use uptrakit_cli::output::OutputFormat;
use uptrakit_openapi_client::Uuid;
use uptrakit_shared_types::PluginType;

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

    /// Increase log verbosity (-v warn, -vv debug for CLI crate, -vvv debug for all uptrakit, -vvvv trace).
    /// Use RUST_LOG to enable logging for other crates.
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
    /// Manage plugin configurations
    PluginConfigs {
        #[command(subcommand)]
        command: PluginConfigsCommands,
    },
    /// Manage enrollment tokens
    EnrollmentTokens {
        #[command(subcommand)]
        command: EnrollmentTokensCommands,
    },
    /// Autodiscovery management
    Autodiscovery {
        #[command(subcommand)]
        command: AutodiscoveryCommands,
    },
    /// Manage the tenant-wide discovery plugin allowlist
    DiscoveryAllowlist {
        #[command(subcommand)]
        command: DiscoveryAllowlistCommands,
    },
    /// Manage notification channels, rules, and log
    Notifications {
        #[command(subcommand)]
        command: NotificationsCommands,
    },
    /// Manage host packages (system-level package updates)
    HostPackages {
        #[command(subcommand)]
        command: HostPackagesCommands,
    },
    /// Manage update batches
    UpdateBatches {
        #[command(subcommand)]
        command: UpdateBatchesCommands,
    },
    /// Manage system services (MQTT bridge, external scheduler)
    SystemServices {
        #[command(subcommand)]
        command: SystemServicesCommands,
    },
    /// Manage system enrollment tokens (for system service auto-approval)
    SystemEnrollmentTokens {
        #[command(subcommand)]
        command: SystemEnrollmentTokensCommands,
    },
    /// View audit logs (tenant and system)
    AuditLogs {
        #[command(subcommand)]
        command: AuditLogsCommands,
    },
    /// Manage UI extensions provided by connected services
    Extensions {
        #[command(subcommand)]
        command: ExtensionsCommands,
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
        /// Optionally filter by plugin config UUID
        #[arg(long)]
        plugin_config: Option<Uuid>,
    },
    /// Manage the host-specific discovery plugin allowlist
    DiscoveryAllowlist {
        /// Host UUID
        id: Uuid,
        #[command(subcommand)]
        command: HostDiscoveryAllowlistCommands,
    },
}

#[derive(Debug, Subcommand)]
enum HostPackagesCommands {
    /// List host packages
    List {
        /// Host UUID
        host_id: Uuid,
        /// Page number (1-indexed)
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
        /// Filter by enabled status
        #[arg(long)]
        enabled: Option<bool>,
        /// Filter packages that have an available update
        #[arg(long)]
        has_update: Option<bool>,
        /// Filter by update category (e.g. security, standard)
        #[arg(long)]
        category: Option<String>,
        /// Search by package name
        #[arg(long)]
        search: Option<String>,
    },
    /// Show host package details with update history
    Show {
        /// Host UUID
        host_id: Uuid,
        /// Package UUID
        package_id: Uuid,
    },
    /// Enable a host package
    Enable {
        /// Host UUID
        host_id: Uuid,
        /// Package UUID
        package_id: Uuid,
    },
    /// Disable a host package
    Disable {
        /// Host UUID
        host_id: Uuid,
        /// Package UUID
        package_id: Uuid,
    },
    /// Delete a host package
    Delete {
        /// Host UUID
        host_id: Uuid,
        /// Package UUID
        package_id: Uuid,
        /// Also create an ignore rule to prevent rediscovery
        #[arg(long)]
        ignore: bool,
    },
    /// Promote a host package to a tracked software item
    Promote {
        /// Host UUID
        host_id: Uuid,
        /// Package UUID
        package_id: Uuid,
        /// Display name for the new software item (defaults to package name)
        #[arg(long)]
        name: Option<String>,
        /// Promote into an existing software item instead of creating a new one
        #[arg(long)]
        software_item_id: Option<Uuid>,
    },
    /// Manage ignore rules
    Ignore {
        #[command(subcommand)]
        command: HostPackageIgnoreCommands,
    },
}

#[derive(Debug, Subcommand)]
enum HostPackageIgnoreCommands {
    /// List ignore rules for a host
    List {
        /// Host UUID
        host_id: Uuid,
    },
    /// Add an ignore rule
    Add {
        /// Host UUID
        host_id: Uuid,
        /// Plugin config UUID
        #[arg(long)]
        plugin_config: Uuid,
        /// Package identifier to ignore
        #[arg(long)]
        package: String,
    },
    /// Remove an ignore rule
    Remove {
        /// Host UUID
        host_id: Uuid,
        /// Ignore rule UUID
        ignore_id: Uuid,
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
        /// Plugin config UUID
        #[arg(long)]
        plugin_config: Option<Uuid>,
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
        /// Follow (tail) update output in real-time after triggering
        #[arg(long, short)]
        follow: bool,
    },
    /// Trigger a batch update for all outdated items on a host
    BatchHost {
        /// Host UUID
        host_id: Uuid,
        /// Only update items in this category (e.g. security)
        #[arg(long)]
        category: Option<String>,
        /// Exclude these software item UUIDs from the batch
        #[arg(long, value_delimiter = ',')]
        exclude: Vec<Uuid>,
        /// Follow batch progress in real-time after triggering
        #[arg(long, short)]
        follow: bool,
    },
    /// Trigger a batch update to roll out a software item to hosts
    BatchItem {
        /// Software item UUID
        item_id: Uuid,
        /// Target version to update to
        #[arg(long)]
        to_version: String,
        /// Limit to these host UUIDs (default: all assigned hosts)
        #[arg(long, value_delimiter = ',')]
        host: Vec<Uuid>,
        /// Follow batch progress in real-time after triggering
        #[arg(long, short)]
        follow: bool,
    },
}

#[derive(Debug, Subcommand)]
enum UpdateBatchesCommands {
    /// List update batches
    List {
        /// Filter by status
        #[arg(long, value_parser = ["in_progress", "completed", "partially_completed"])]
        status: Option<String>,
        /// Page number (1-indexed)
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
    },
    /// Show update batch details
    Show {
        /// Batch UUID
        id: Uuid,
    },
    /// Follow batch progress in real-time via SSE
    Follow {
        /// Batch UUID
        id: Uuid,
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
    /// Tail update output in real-time
    Tail {
        /// Update history UUID
        id: Uuid,
    },
}

#[derive(Debug, Subcommand)]
enum ServicesCommands {
    /// List all services
    List {
        /// Filter by capability (software_discovery, mqtt_bridge, ssh_remote)
        #[arg(long)]
        capability: Option<String>,
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
        /// Per-service certificate lifetime in hours (0 to clear override)
        #[arg(long)]
        cert_lifetime_hours: Option<u32>,
    },
    /// Merge a source service into a target service
    Merge {
        /// Target service UUID (approved)
        target_id: Uuid,
        /// Source service UUID (pending)
        source_id: Uuid,
    },
    /// Enable or disable the update freeze on a connected service
    UpdateFreeze {
        /// Service UUID
        id: Uuid,
        /// Enable the freeze (blocks updates on the agent)
        #[arg(long, group = "freeze_action")]
        enable: bool,
        /// Disable the freeze (allows updates on the agent)
        #[arg(long, group = "freeze_action")]
        disable: bool,
        /// Optional reason for the freeze
        #[arg(long)]
        reason: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum SystemServicesCommands {
    /// List all system services
    List {
        /// Filter by capability (mqtt_bridge, scheduler)
        #[arg(long)]
        capability: Option<String>,
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
    /// Show system service details
    Show {
        /// System service UUID
        id: Uuid,
    },
    /// Approve a pending system service
    Approve {
        /// System service UUID
        id: Uuid,
    },
    /// Reject a pending system service
    Reject {
        /// System service UUID
        id: Uuid,
    },
    /// Remove (deactivate) a system service
    Remove {
        /// System service UUID
        id: Uuid,
    },
    /// Update a system service's settings
    Update {
        /// System service UUID
        id: Uuid,
        /// Custom ping interval in seconds (0 to clear override)
        #[arg(long)]
        ping_interval: Option<u32>,
        /// Per-service certificate lifetime in hours (0 to clear override)
        #[arg(long)]
        cert_lifetime_hours: Option<u32>,
    },
}

#[derive(Debug, Subcommand)]
enum SystemEnrollmentTokensCommands {
    /// List system enrollment tokens
    List {
        /// Page number (1-indexed)
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
    },
    /// Create a new system enrollment token
    Create {
        /// Human-readable token name
        #[arg(long)]
        name: String,
        /// Maximum number of enrollments allowed
        #[arg(long)]
        max_uses: Option<u32>,
        /// Token lifetime in seconds (e.g. 86400 for 24 hours)
        #[arg(long)]
        expires_in: Option<u64>,
    },
    /// Show system enrollment token details
    Show {
        /// System enrollment token UUID
        id: Uuid,
    },
    /// Revoke a system enrollment token
    Revoke {
        /// System enrollment token UUID
        id: Uuid,
    },
}

#[derive(Debug, Subcommand)]
enum AuditLogsCommands {
    /// List tenant-scoped audit log entries
    List {
        /// Filter by actor type (user, api_token, oidc)
        #[arg(long)]
        actor_type: Option<String>,
        /// Filter by HTTP method (GET, POST, PUT, DELETE, PATCH)
        #[arg(long)]
        method: Option<String>,
        /// Filter by exact HTTP status code (e.g. 200, 403, 500)
        #[arg(long)]
        status: Option<u16>,
        /// Lower bound timestamp (inclusive), RFC 3339 format
        #[arg(long)]
        from: Option<String>,
        /// Upper bound timestamp (inclusive), RFC 3339 format
        #[arg(long)]
        to: Option<String>,
        /// Filter entries by a specific actor UUID
        #[arg(long)]
        actor_id: Option<Uuid>,
        /// Page number (1-indexed)
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
    },
    /// View system-level audit log entries (global settings, CA rotation, etc.)
    System {
        #[command(subcommand)]
        command: AuditLogsSystemCommands,
    },
}

#[derive(Debug, Subcommand)]
enum AuditLogsSystemCommands {
    /// List system-level audit log entries
    List {
        /// Filter by actor type (user, api_token, oidc)
        #[arg(long)]
        actor_type: Option<String>,
        /// Filter by HTTP method (GET, POST, PUT, DELETE, PATCH)
        #[arg(long)]
        method: Option<String>,
        /// Filter by exact HTTP status code (e.g. 200, 403, 500)
        #[arg(long)]
        status: Option<u16>,
        /// Lower bound timestamp (inclusive), RFC 3339 format
        #[arg(long)]
        from: Option<String>,
        /// Upper bound timestamp (inclusive), RFC 3339 format
        #[arg(long)]
        to: Option<String>,
        /// Filter entries by a specific actor UUID
        #[arg(long)]
        actor_id: Option<Uuid>,
        /// Page number (1-indexed)
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
    },
}

#[derive(Debug, Subcommand)]
enum ExtensionsCommands {
    /// List all registered UI extensions
    List,
    /// List connected service instances providing an extension
    Providers {
        /// Extension ID (e.g. "ssh-agent.host-management")
        extension_id: String,
    },
    /// Invoke an extension action (raw JSON params)
    Invoke {
        /// Extension ID
        extension_id: String,
        /// Action ID to invoke
        action_id: String,
        /// JSON parameters to pass to the action
        #[arg(long, default_value = "{}")]
        params: String,
        /// Route to a specific service instance (required for targeted extensions)
        #[arg(long)]
        service_id: Option<Uuid>,
    },
    /// Dynamic extension subcommand (e.g., `extensions ssh-agent.hosts list-hosts`)
    #[command(external_subcommand)]
    Dynamic(Vec<OsString>),
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
}

#[derive(Debug, Subcommand)]
enum SmtpCommands {
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
enum NatsCommands {
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
        /// Certificate lifetime in hours (max 17520)
        #[arg(long)]
        lifetime_hours: Option<u32>,
        /// Certificate renewal window in hours (use 0 to reset to automatic: min(14 days, lifetime/5))
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
enum PluginConfigsCommands {
    /// List plugin configurations
    List {
        /// Page number
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
    },
    /// Show a plugin configuration
    Show {
        /// Plugin config UUID
        id: Uuid,
    },
    /// Create a new plugin configuration
    Create {
        /// Config name
        #[arg(long)]
        name: String,
        /// Plugin type (github_releases, docker, homebrew, proxmox_helper_scripts)
        #[arg(long)]
        plugin_type: String,
        /// Plugin-specific config as JSON string
        #[arg(long)]
        config: Option<String>,
        /// Enable on creation
        #[arg(long)]
        enabled: Option<bool>,
    },
    /// Update a plugin configuration
    Update {
        /// Plugin config UUID
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
    /// Delete a plugin configuration
    Delete {
        /// Plugin config UUID
        id: Uuid,
    },
    /// Trigger autodiscovery for a plugin config
    Discover {
        /// Plugin config UUID
        id: Uuid,
    },
    /// Discard all pending discovered items for a plugin config
    DiscardDiscovered {
        /// Plugin config UUID
        id: Uuid,
    },
}

#[derive(Debug, Subcommand)]
enum EnrollmentTokensCommands {
    /// List enrollment tokens
    List {
        /// Page number (1-indexed)
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
    },
    /// Create a new enrollment token
    Create {
        /// Human-readable token name
        #[arg(long)]
        name: String,
        /// Comma-separated list of allowed capabilities (e.g. software_discovery,mqtt_bridge).
        /// Omit for a wildcard token that allows any service type.
        #[arg(long)]
        capabilities: Option<String>,
        /// Maximum number of enrollments allowed
        #[arg(long)]
        max_uses: Option<u32>,
        /// Token lifetime in seconds (e.g. 86400 for 24 hours)
        #[arg(long)]
        expires_in: Option<u64>,
    },
    /// Show enrollment token details
    Show {
        /// Enrollment token UUID
        id: Uuid,
    },
    /// Revoke an enrollment token
    Revoke {
        /// Enrollment token UUID
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
        /// Filter by plugin config UUID
        #[arg(long)]
        plugin_config: Option<Uuid>,
        /// Page number
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
    },
    /// Create an autodiscovery ignore rule
    Create {
        /// Plugin config UUID
        #[arg(long)]
        plugin_config: Uuid,
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

#[derive(Debug, Subcommand)]
enum DiscoveryAllowlistCommands {
    /// List tenant-wide discovery allowlist entries.
    ///
    /// An empty list means no restrictions — all discovery plugins run.
    List,
    /// Add a plugin type to the tenant-wide discovery allowlist
    Add {
        /// Plugin type (e.g. package_manager_homebrew)
        plugin_type: PluginType,
    },
    /// Remove a tenant-wide discovery allowlist entry
    Remove {
        /// Entry UUID
        id: Uuid,
    },
}

#[derive(Debug, Subcommand)]
enum HostDiscoveryAllowlistCommands {
    /// List host-specific discovery allowlist entries.
    ///
    /// An empty list means the host inherits the tenant allowlist.
    List,
    /// Add a plugin type to the host's discovery allowlist
    Add {
        /// Plugin type (e.g. package_manager_apt)
        plugin_type: PluginType,
    },
    /// Remove a host-specific discovery allowlist entry
    Remove {
        /// Entry UUID
        entry_id: Uuid,
    },
}

#[derive(Debug, Subcommand)]
enum NotificationsCommands {
    /// Manage notification channels
    Channels {
        #[command(subcommand)]
        command: ChannelsCommands,
    },
    /// Manage notification rules
    Rules {
        #[command(subcommand)]
        command: RulesCommands,
    },
    /// View notification delivery log
    Log {
        /// Page number (1-indexed)
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
    },
}

#[derive(Debug, Subcommand)]
enum ChannelsCommands {
    /// List notification channels
    List {
        /// Page number (1-indexed)
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
    },
    /// Show notification channel details
    Get {
        /// Channel UUID
        id: Uuid,
    },
    /// Create a new notification channel
    Create {
        /// Channel name
        #[arg(long)]
        name: String,
        /// Channel type (webhook, telegram)
        #[arg(long = "type")]
        channel_type: String,
        /// Channel-specific configuration as JSON string
        #[arg(long)]
        config: String,
    },
    /// Update a notification channel
    Update {
        /// Channel UUID
        id: Uuid,
        /// New name
        #[arg(long)]
        name: Option<String>,
        /// Updated configuration as JSON string
        #[arg(long)]
        config: Option<String>,
        /// Enable or disable
        #[arg(long)]
        enabled: Option<bool>,
    },
    /// Delete a notification channel
    Delete {
        /// Channel UUID
        id: Uuid,
    },
    /// Send a test notification through a channel
    Test {
        /// Channel UUID
        id: Uuid,
    },
}

#[derive(Debug, Subcommand)]
enum RulesCommands {
    /// List notification rules
    List {
        /// Page number (1-indexed)
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
    },
    /// Show notification rule details
    Get {
        /// Rule UUID
        id: Uuid,
    },
    /// Create a new notification rule
    Create {
        /// Channel UUID to deliver notifications through
        #[arg(long)]
        channel_id: Uuid,
        /// Event type (update_available, update_completed, update_failed, new_software_discovered, new_service_enrolled, ca_rotated)
        #[arg(long)]
        event_type: String,
        /// Optionally scope to a specific host
        #[arg(long)]
        host_id: Option<Uuid>,
        /// Optionally scope to a specific software item
        #[arg(long)]
        software_item_id: Option<Uuid>,
        /// Optionally scope to a specific plugin type
        #[arg(long)]
        plugin_type: Option<String>,
    },
    /// Update a notification rule
    Update {
        /// Rule UUID
        id: Uuid,
        /// New event type
        #[arg(long)]
        event_type: Option<String>,
        /// Enable or disable
        #[arg(long)]
        enabled: Option<bool>,
    },
    /// Delete a notification rule
    Delete {
        /// Rule UUID
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
        let directive = match cli.verbose {
            1 => "uptrakit_cli=warn",
            2 => "uptrakit_cli=debug",
            3 => "uptrakit=debug",
            _ => "uptrakit=trace",
        };
        let filter = EnvFilter::from_default_env();
        let filter = if let Ok(d) = directive.parse() {
            filter.add_directive(d)
        } else {
            filter
        };
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_env_filter(filter)
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
                capability,
                status,
                page,
                per_page,
            } => {
                let resp = commands::services::list(commands::services::ListParams {
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    capability: capability.as_deref(),
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
            ServicesCommands::Update {
                id,
                ping_interval,
                cert_lifetime_hours,
            } => {
                let resp = commands::services::update(
                    &id,
                    ping_interval,
                    cert_lifetime_hours,
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
            ServicesCommands::UpdateFreeze {
                id,
                enable,
                disable: _,
                reason,
            } => {
                let resp = commands::services::update_freeze(
                    &id,
                    enable,
                    reason.as_deref(),
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
            HostsCommands::DiscardDiscovered { id, plugin_config } => {
                let resp =
                    commands::hosts::discard_discovered(commands::hosts::DiscardDiscoveredParams {
                        id: &id,
                        plugin_config_id: plugin_config.as_ref(),
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    })
                    .await?;
                output::print_output(format, &resp)?;
            }
            HostsCommands::DiscoveryAllowlist { id, command } => match command {
                HostDiscoveryAllowlistCommands::List => {
                    let resp = commands::discovery_allowlist::host_list(
                        commands::discovery_allowlist::ListHostParams {
                            host_id: &id,
                            server: cli.server.as_deref(),
                            token: cli.token.as_deref(),
                            insecure,
                            request_timeout,
                        },
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
                HostDiscoveryAllowlistCommands::Add { plugin_type } => {
                    let resp = commands::discovery_allowlist::host_add(
                        commands::discovery_allowlist::AddHostParams {
                            host_id: &id,
                            plugin_type,
                            server: cli.server.as_deref(),
                            token: cli.token.as_deref(),
                            insecure,
                            request_timeout,
                        },
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
                HostDiscoveryAllowlistCommands::Remove { entry_id } => {
                    let resp = commands::discovery_allowlist::host_remove(
                        commands::discovery_allowlist::RemoveHostParams {
                            host_id: &id,
                            entry_id: &entry_id,
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
                plugin_config,
                package,
            } => {
                let resp =
                    commands::software_items::assign(commands::software_items::AssignParams {
                        id: &id,
                        host_id: &host,
                        plugin_config_id: plugin_config.as_ref(),
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
                follow,
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

                if follow {
                    let tail_result = commands::tail::tail(commands::tail::TailParams {
                        update_history_id: &resp.update_history_id,
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                    })
                    .await?;
                    std::process::exit(tail_result.exit_code());
                }
            }
            UpdateCommands::BatchHost {
                host_id,
                category,
                exclude,
                follow,
            } => {
                let resp = commands::batch_update::trigger_host_batch(
                    commands::batch_update::HostBatchParams {
                        host_id: &host_id,
                        category: category.as_deref(),
                        exclude: &exclude,
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    },
                )
                .await?;
                output::print_output(format, &resp)?;

                if follow && let Some(batch_id) = resp.batch_id {
                    let result = commands::batch_update::follow_batch(
                        commands::batch_update::FollowBatchParams {
                            batch_id: &batch_id,
                            server: cli.server.as_deref(),
                            token: cli.token.as_deref(),
                            insecure,
                        },
                    )
                    .await?;
                    std::process::exit(result.exit_code());
                }
            }
            UpdateCommands::BatchItem {
                item_id,
                to_version,
                host,
                follow,
            } => {
                let resp = commands::batch_update::trigger_item_batch(
                    commands::batch_update::ItemBatchParams {
                        item_id: &item_id,
                        to_version: &to_version,
                        host_ids: &host,
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    },
                )
                .await?;
                output::print_output(format, &resp)?;

                if follow && let Some(batch_id) = resp.batch_id {
                    let result = commands::batch_update::follow_batch(
                        commands::batch_update::FollowBatchParams {
                            batch_id: &batch_id,
                            server: cli.server.as_deref(),
                            token: cli.token.as_deref(),
                            insecure,
                        },
                    )
                    .await?;
                    std::process::exit(result.exit_code());
                }
            }
        },
        Commands::HostPackages { command } => match command {
            HostPackagesCommands::List {
                host_id,
                page,
                per_page,
                enabled,
                has_update,
                category,
                search,
            } => {
                let resp = commands::host_packages::list(commands::host_packages::ListParams {
                    host_id: &host_id,
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    request_timeout,
                    page,
                    per_page,
                    enabled,
                    has_update,
                    category,
                    search,
                })
                .await?;
                output::print_output(format, &resp)?;
            }
            HostPackagesCommands::Show {
                host_id,
                package_id,
            } => {
                let resp = commands::host_packages::show(commands::host_packages::ShowParams {
                    host_id: &host_id,
                    package_id: &package_id,
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    request_timeout,
                })
                .await?;
                output::print_output(format, &resp)?;
            }
            HostPackagesCommands::Enable {
                host_id,
                package_id,
            } => {
                let resp = commands::host_packages::update(commands::host_packages::UpdateParams {
                    host_id: &host_id,
                    package_id: &package_id,
                    enabled: true,
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    request_timeout,
                })
                .await?;
                output::print_output(format, &resp)?;
            }
            HostPackagesCommands::Disable {
                host_id,
                package_id,
            } => {
                let resp = commands::host_packages::update(commands::host_packages::UpdateParams {
                    host_id: &host_id,
                    package_id: &package_id,
                    enabled: false,
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    request_timeout,
                })
                .await?;
                output::print_output(format, &resp)?;
            }
            HostPackagesCommands::Delete {
                host_id,
                package_id,
                ignore,
            } => {
                let resp = commands::host_packages::delete(commands::host_packages::DeleteParams {
                    host_id: &host_id,
                    package_id: &package_id,
                    ignore,
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    request_timeout,
                })
                .await?;
                output::print_output(format, &resp)?;
            }
            HostPackagesCommands::Promote {
                host_id,
                package_id,
                name,
                software_item_id,
            } => {
                let resp =
                    commands::host_packages::promote(commands::host_packages::PromoteParams {
                        host_id: &host_id,
                        package_id: &package_id,
                        name,
                        software_item_id,
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    })
                    .await?;
                output::print_output(format, &resp)?;
            }
            HostPackagesCommands::Ignore { command } => match command {
                HostPackageIgnoreCommands::List { host_id } => {
                    let resp = commands::host_packages::list_ignores(
                        commands::host_packages::ListIgnoresParams {
                            host_id: &host_id,
                            server: cli.server.as_deref(),
                            token: cli.token.as_deref(),
                            insecure,
                            request_timeout,
                        },
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
                HostPackageIgnoreCommands::Add {
                    host_id,
                    plugin_config,
                    package,
                } => {
                    let resp = commands::host_packages::add_ignore(
                        commands::host_packages::AddIgnoreParams {
                            host_id: &host_id,
                            plugin_config_id: &plugin_config,
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
                HostPackageIgnoreCommands::Remove { host_id, ignore_id } => {
                    let resp = commands::host_packages::remove_ignore(
                        commands::host_packages::RemoveIgnoreParams {
                            host_id: &host_id,
                            ignore_id: &ignore_id,
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
        Commands::UpdateBatches { command } => match command {
            UpdateBatchesCommands::List {
                status,
                page,
                per_page,
            } => {
                let resp =
                    commands::batch_update::list_batches(commands::batch_update::ListBatchParams {
                        status: status.as_deref(),
                        page,
                        per_page,
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    })
                    .await?;
                output::print_output(format, &resp)?;
            }
            UpdateBatchesCommands::Show { id } => {
                let resp =
                    commands::batch_update::show_batch(commands::batch_update::ShowBatchParams {
                        batch_id: &id,
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    })
                    .await?;
                output::print_output(format, &resp)?;
            }
            UpdateBatchesCommands::Follow { id } => {
                let result = commands::batch_update::follow_batch(
                    commands::batch_update::FollowBatchParams {
                        batch_id: &id,
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                    },
                )
                .await?;
                std::process::exit(result.exit_code());
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
            HistoryCommands::Tail { id } => {
                let tail_result = commands::tail::tail(commands::tail::TailParams {
                    update_history_id: &id,
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                })
                .await?;
                std::process::exit(tail_result.exit_code());
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
                    lifetime_hours,
                    renewal_window_hours,
                } => {
                    let resp = commands::settings::certificates_update(
                        lifetime_hours,
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
                    ha_discovery,
                    no_ha_discovery,
                    ha_discovery_prefix,
                } => {
                    let ca_pem = resolve_ca_pem(ca_pem, ca_pem_file)?;
                    let ha_discovery_flag = if ha_discovery {
                        Some(true)
                    } else if no_ha_discovery {
                        Some(false)
                    } else {
                        None
                    };
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
                            ha_discovery: ha_discovery_flag,
                            ha_discovery_prefix,
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
                    ha_discovery,
                    no_ha_discovery,
                    ha_discovery_prefix,
                } => {
                    let ca_pem = resolve_ca_pem(ca_pem, ca_pem_file)?;
                    let ha_discovery_flag = if ha_discovery {
                        Some(true)
                    } else if no_ha_discovery {
                        Some(false)
                    } else {
                        None
                    };
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
                            ha_discovery: ha_discovery_flag,
                            ha_discovery_prefix,
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
            SettingsCommands::Smtp { command } => match command {
                SmtpCommands::Show => {
                    let resp = commands::settings::smtp_show(
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    )
                    .await?;
                    output::print_output(format, &resp)?;
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
                    let resp = commands::settings::smtp_set(commands::settings::SmtpSetParams {
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
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
            SettingsCommands::Nats { command } => match command {
                NatsCommands::Show => {
                    let resp = commands::settings::nats_show(
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
                NatsCommands::Set { url } => {
                    let resp = commands::settings::nats_set(
                        url,
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                    eprintln!(
                        "NATS URL updated. The change will take effect after the controller is restarted."
                    );
                }
                NatsCommands::Clear => {
                    let resp = commands::settings::nats_clear(
                        cli.server.as_deref(),
                        cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                    eprintln!(
                        "NATS URL cleared. The change will take effect after the controller is restarted."
                    );
                }
            },
        },
        Commands::PluginConfigs { command } => match command {
            PluginConfigsCommands::List { page, per_page } => {
                let resp = commands::plugin_configs::list(commands::plugin_configs::ListParams {
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
            PluginConfigsCommands::Show { id } => {
                let resp = commands::plugin_configs::show(commands::plugin_configs::ShowParams {
                    id: &id,
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    request_timeout,
                })
                .await?;
                output::print_output(format, &resp)?;
            }
            PluginConfigsCommands::Create {
                name,
                plugin_type,
                config,
                enabled,
            } => {
                let plugin_type: PluginType = plugin_type.parse().map_err(|_| {
                    report!(error::CliError::Other(format!(
                        "unknown plugin type: {plugin_type}"
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
                    commands::plugin_configs::create(commands::plugin_configs::CreateParams {
                        name,
                        plugin_type,
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
            PluginConfigsCommands::Update {
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
                    commands::plugin_configs::update(commands::plugin_configs::UpdateParams {
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
            PluginConfigsCommands::Delete { id } => {
                let resp =
                    commands::plugin_configs::delete(commands::plugin_configs::DeleteParams {
                        id: &id,
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    })
                    .await?;
                output::print_output(format, &resp)?;
            }
            PluginConfigsCommands::Discover { id } => {
                let resp =
                    commands::plugin_configs::discover(commands::plugin_configs::DiscoverParams {
                        id: &id,
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    })
                    .await?;
                output::print_output(format, &resp)?;
            }
            PluginConfigsCommands::DiscardDiscovered { id } => {
                let resp = commands::plugin_configs::discard_discovered(
                    commands::plugin_configs::DiscardDiscoveredParams {
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
        Commands::EnrollmentTokens { command } => match command {
            EnrollmentTokensCommands::List { page, per_page } => {
                let resp =
                    commands::enrollment_tokens::list(commands::enrollment_tokens::ListParams {
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                        page,
                        per_page,
                    })
                    .await?;
                output::print_output(format, &resp)?;
            }
            EnrollmentTokensCommands::Create {
                name,
                capabilities,
                max_uses,
                expires_in,
            } => {
                let allowed_capabilities = capabilities.map(|s| {
                    s.split(',')
                        .map(|c| c.trim().to_string())
                        .filter(|c| !c.is_empty())
                        .collect()
                });
                let resp = commands::enrollment_tokens::create(
                    commands::enrollment_tokens::CreateParams {
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                        name: &name,
                        allowed_capabilities,
                        max_uses,
                        expires_in_seconds: expires_in,
                    },
                )
                .await?;
                output::print_output(format, &resp)?;
            }
            EnrollmentTokensCommands::Show { id } => {
                let resp =
                    commands::enrollment_tokens::show(commands::enrollment_tokens::ShowParams {
                        id: &id,
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    })
                    .await?;
                output::print_output(format, &resp)?;
            }
            EnrollmentTokensCommands::Revoke { id } => {
                let resp = commands::enrollment_tokens::revoke(
                    commands::enrollment_tokens::RevokeParams {
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
                    plugin_config,
                    page,
                    per_page,
                } => {
                    let resp = commands::autodiscovery::ignores_list(
                        commands::autodiscovery::IgnoresListParams {
                            plugin_config_id: plugin_config.as_ref(),
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
                    plugin_config,
                    package,
                } => {
                    let resp = commands::autodiscovery::ignores_create(
                        commands::autodiscovery::IgnoresCreateParams {
                            plugin_config_id: &plugin_config,
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
        Commands::DiscoveryAllowlist { command } => match command {
            DiscoveryAllowlistCommands::List => {
                let resp = commands::discovery_allowlist::tenant_list(
                    commands::discovery_allowlist::ListTenantParams {
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    },
                )
                .await?;
                output::print_output(format, &resp)?;
            }
            DiscoveryAllowlistCommands::Add { plugin_type } => {
                let resp = commands::discovery_allowlist::tenant_add(
                    commands::discovery_allowlist::AddTenantParams {
                        plugin_type,
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                    },
                )
                .await?;
                output::print_output(format, &resp)?;
            }
            DiscoveryAllowlistCommands::Remove { id } => {
                let resp = commands::discovery_allowlist::tenant_remove(
                    commands::discovery_allowlist::RemoveTenantParams {
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
        Commands::Notifications { command } => match command {
            NotificationsCommands::Channels { command } => match command {
                ChannelsCommands::List { page, per_page } => {
                    let resp = commands::notifications::channel_list(
                        commands::notifications::ChannelListParams {
                            server: cli.server.as_deref(),
                            token: cli.token.as_deref(),
                            insecure,
                            request_timeout,
                            page,
                            per_page,
                        },
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
                ChannelsCommands::Get { id } => {
                    let resp = commands::notifications::channel_get(
                        commands::notifications::ChannelGetParams {
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
                ChannelsCommands::Create {
                    name,
                    channel_type,
                    config,
                } => {
                    let channel_type: uptrakit_openapi_client::types::notifications::NotificationChannelType =
                        channel_type.parse().map_err(|_| {
                            report!(error::CliError::Other(format!(
                                "unknown channel type: {channel_type} (expected webhook or telegram)"
                            )))
                        })?;
                    let config_value: serde_json::Value =
                        serde_json::from_str(&config).map_err(|e| {
                            report!(error::CliError::Other(format!(
                                "invalid JSON for --config: {e}"
                            )))
                        })?;
                    let resp = commands::notifications::channel_create(
                        commands::notifications::ChannelCreateParams {
                            name,
                            channel_type,
                            config: config_value,
                            server: cli.server.as_deref(),
                            token: cli.token.as_deref(),
                            insecure,
                            request_timeout,
                        },
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
                ChannelsCommands::Update {
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
                    let resp = commands::notifications::channel_update(
                        commands::notifications::ChannelUpdateParams {
                            id: &id,
                            name,
                            config: config_value,
                            enabled,
                            server: cli.server.as_deref(),
                            token: cli.token.as_deref(),
                            insecure,
                            request_timeout,
                        },
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
                ChannelsCommands::Delete { id } => {
                    let resp = commands::notifications::channel_delete(
                        commands::notifications::ChannelDeleteParams {
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
                ChannelsCommands::Test { id } => {
                    let resp = commands::notifications::channel_test(
                        commands::notifications::ChannelTestParams {
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
            NotificationsCommands::Rules { command } => match command {
                RulesCommands::List { page, per_page } => {
                    let resp = commands::notifications::rule_list(
                        commands::notifications::RuleListParams {
                            server: cli.server.as_deref(),
                            token: cli.token.as_deref(),
                            insecure,
                            request_timeout,
                            page,
                            per_page,
                        },
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
                RulesCommands::Get { id } => {
                    let resp =
                        commands::notifications::rule_get(commands::notifications::RuleGetParams {
                            id: &id,
                            server: cli.server.as_deref(),
                            token: cli.token.as_deref(),
                            insecure,
                            request_timeout,
                        })
                        .await?;
                    output::print_output(format, &resp)?;
                }
                RulesCommands::Create {
                    channel_id,
                    event_type,
                    host_id,
                    software_item_id,
                    plugin_type,
                } => {
                    let event_type: uptrakit_openapi_client::types::notifications::NotificationEventType =
                        event_type.parse().map_err(|_| {
                            report!(error::CliError::Other(format!(
                                "unknown event type: {event_type} (expected update_available, update_completed, update_failed, new_software_discovered, new_service_enrolled, or ca_rotated)"
                            )))
                        })?;
                    let resp = commands::notifications::rule_create(
                        commands::notifications::RuleCreateParams {
                            channel_id,
                            event_type,
                            host_id,
                            software_item_id,
                            plugin_type,
                            server: cli.server.as_deref(),
                            token: cli.token.as_deref(),
                            insecure,
                            request_timeout,
                        },
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
                RulesCommands::Update {
                    id,
                    event_type,
                    enabled,
                } => {
                    let event_type: Option<uptrakit_openapi_client::types::notifications::NotificationEventType> =
                        match event_type {
                            Some(s) => Some(s.parse().map_err(|_| {
                                report!(error::CliError::Other(format!(
                                    "unknown event type: {s} (expected update_available, update_completed, update_failed, new_software_discovered, new_service_enrolled, or ca_rotated)"
                                )))
                            })?),
                            None => None,
                        };
                    let resp = commands::notifications::rule_update(
                        commands::notifications::RuleUpdateParams {
                            id: &id,
                            event_type,
                            enabled,
                            server: cli.server.as_deref(),
                            token: cli.token.as_deref(),
                            insecure,
                            request_timeout,
                        },
                    )
                    .await?;
                    output::print_output(format, &resp)?;
                }
                RulesCommands::Delete { id } => {
                    let resp = commands::notifications::rule_delete(
                        commands::notifications::RuleDeleteParams {
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
            NotificationsCommands::Log { page, per_page } => {
                let resp =
                    commands::notifications::log_list(commands::notifications::LogListParams {
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                        page,
                        per_page,
                    })
                    .await?;
                output::print_output(format, &resp)?;
            }
        },
        Commands::SystemServices { command } => match command {
            SystemServicesCommands::List {
                capability,
                status,
                page,
                per_page,
            } => {
                let resp = commands::system_services::list(commands::system_services::ListParams {
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    capability: capability.as_deref(),
                    status: status.as_deref(),
                    page,
                    per_page,
                    request_timeout,
                })
                .await?;
                output::print_output(format, &resp)?;
            }
            SystemServicesCommands::Show { id } => {
                let resp = commands::system_services::show(
                    &id,
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    insecure,
                    request_timeout,
                )
                .await?;
                output::print_output(format, &resp)?;
            }
            SystemServicesCommands::Approve { id } => {
                let resp = commands::system_services::approve(
                    &id,
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    insecure,
                    request_timeout,
                )
                .await?;
                output::print_output(format, &resp)?;
            }
            SystemServicesCommands::Reject { id } => {
                let resp = commands::system_services::reject(
                    &id,
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    insecure,
                    request_timeout,
                )
                .await?;
                output::print_output(format, &resp)?;
            }
            SystemServicesCommands::Remove { id } => {
                let resp = commands::system_services::remove(
                    &id,
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    insecure,
                    request_timeout,
                )
                .await?;
                output::print_output(format, &resp)?;
            }
            SystemServicesCommands::Update {
                id,
                ping_interval,
                cert_lifetime_hours,
            } => {
                let resp = commands::system_services::update(
                    &id,
                    ping_interval,
                    cert_lifetime_hours,
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    insecure,
                    request_timeout,
                )
                .await?;
                output::print_output(format, &resp)?;
            }
        },
        Commands::SystemEnrollmentTokens { command } => match command {
            SystemEnrollmentTokensCommands::List { page, per_page } => {
                let resp = commands::system_enrollment_tokens::list(
                    commands::system_enrollment_tokens::ListParams {
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                        page,
                        per_page,
                    },
                )
                .await?;
                output::print_output(format, &resp)?;
            }
            SystemEnrollmentTokensCommands::Create {
                name,
                max_uses,
                expires_in,
            } => {
                let resp = commands::system_enrollment_tokens::create(
                    commands::system_enrollment_tokens::CreateParams {
                        server: cli.server.as_deref(),
                        token: cli.token.as_deref(),
                        insecure,
                        request_timeout,
                        name: &name,
                        max_uses,
                        expires_in_seconds: expires_in,
                    },
                )
                .await?;
                output::print_output(format, &resp)?;
            }
            SystemEnrollmentTokensCommands::Show { id } => {
                let resp = commands::system_enrollment_tokens::show(
                    commands::system_enrollment_tokens::ShowParams {
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
            SystemEnrollmentTokensCommands::Revoke { id } => {
                let resp = commands::system_enrollment_tokens::revoke(
                    commands::system_enrollment_tokens::RevokeParams {
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
        Commands::AuditLogs { command } => match command {
            AuditLogsCommands::List {
                actor_type,
                method,
                status,
                from,
                to,
                actor_id,
                page,
                per_page,
            } => {
                let resp = commands::audit_logs::list(commands::audit_logs::ListParams {
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    request_timeout,
                    actor_type: actor_type.as_deref(),
                    method: method.as_deref(),
                    status,
                    from: from.as_deref(),
                    to: to.as_deref(),
                    actor_id,
                    page,
                    per_page,
                })
                .await?;
                output::print_output(format, &resp)?;
            }
            AuditLogsCommands::System { command } => match command {
                AuditLogsSystemCommands::List {
                    actor_type,
                    method,
                    status,
                    from,
                    to,
                    actor_id,
                    page,
                    per_page,
                } => {
                    let resp =
                        commands::audit_logs::list_system(commands::audit_logs::ListParams {
                            server: cli.server.as_deref(),
                            token: cli.token.as_deref(),
                            insecure,
                            request_timeout,
                            actor_type: actor_type.as_deref(),
                            method: method.as_deref(),
                            status,
                            from: from.as_deref(),
                            to: to.as_deref(),
                            actor_id,
                            page,
                            per_page,
                        })
                        .await?;
                    output::print_output(format, &resp)?;
                }
            },
        },
        Commands::Extensions { command } => match command {
            ExtensionsCommands::List => {
                let resp = commands::extensions::list(commands::extensions::ListParams {
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    request_timeout,
                })
                .await?;
                output::print_output(format, &resp)?;
            }
            ExtensionsCommands::Providers { extension_id } => {
                let resp = commands::extensions::providers(commands::extensions::ProvidersParams {
                    extension_id,
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    request_timeout,
                })
                .await?;
                output::print_output(format, &resp)?;
            }
            ExtensionsCommands::Invoke {
                extension_id,
                action_id,
                params,
                service_id,
            } => {
                let params_value: serde_json::Value = serde_json::from_str(&params).context_to()?;
                let resp = commands::extensions::invoke(commands::extensions::InvokeParams {
                    extension_id,
                    action_id,
                    params: params_value,
                    service_id,
                    server: cli.server.as_deref(),
                    token: cli.token.as_deref(),
                    insecure,
                    request_timeout,
                })
                .await?;
                output::print_output(format, &resp)?;
            }
            ExtensionsCommands::Dynamic(args) => {
                let resp = commands::extensions::dynamic_invoke(
                    args,
                    cli.server.as_deref(),
                    cli.token.as_deref(),
                    insecure,
                    request_timeout,
                )
                .await?;
                output::print_output(format, &resp)?;
            }
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
    const ET_UUID: &str = "aa300000-bb00-cc00-dd00-ee0000000001";
    const PKG_UUID: &str = "aa400000-bb00-cc00-dd00-ee0000000001";
    const SYS_ET_UUID: &str = "aa500000-bb00-cc00-dd00-ee0000000001";

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
                        follow,
                    },
            }) => {
                assert_eq!(item_id, uuid(ITEM_UUID));
                assert_eq!(host_id, uuid(HOST_UUID));
                assert_eq!(to_version, "2.0.0");
                assert!(release_tag.is_none());
                assert!(release_url.is_none());
                assert!(!follow);
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
    fn history_tail_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "history", "tail", HIST_UUID]).expect("should parse");
        match args.command {
            Some(Commands::History {
                command: HistoryCommands::Tail { id },
            }) => {
                assert_eq!(id, uuid(HIST_UUID));
            }
            _ => panic!("expected History Tail"),
        }
    }

    #[test]
    fn update_trigger_follow_flag_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "update",
            "trigger",
            ITEM_UUID,
            HOST_UUID,
            "--to-version",
            "2.0.0",
            "--follow",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::Update {
                command: UpdateCommands::Trigger { follow, .. },
            }) => {
                assert!(follow);
            }
            _ => panic!("expected Update Trigger with follow"),
        }
    }

    #[test]
    fn update_batch_host_parses() {
        let args = Cli::try_parse_from(["uptrakit", "update", "batch-host", HOST_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::Update {
                command:
                    UpdateCommands::BatchHost {
                        host_id,
                        category,
                        exclude,
                        follow,
                    },
            }) => {
                assert_eq!(host_id, uuid(HOST_UUID));
                assert!(category.is_none());
                assert!(exclude.is_empty());
                assert!(!follow);
            }
            _ => panic!("expected Update BatchHost"),
        }
    }

    #[test]
    fn update_batch_host_with_options_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "update",
            "batch-host",
            HOST_UUID,
            "--category",
            "security",
            "--exclude",
            ITEM_UUID,
            "--follow",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::Update {
                command:
                    UpdateCommands::BatchHost {
                        category,
                        exclude,
                        follow,
                        ..
                    },
            }) => {
                assert_eq!(category.as_deref(), Some("security"));
                assert_eq!(exclude.len(), 1);
                assert_eq!(exclude[0], uuid(ITEM_UUID));
                assert!(follow);
            }
            _ => panic!("expected Update BatchHost"),
        }
    }

    #[test]
    fn update_batch_item_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "update",
            "batch-item",
            ITEM_UUID,
            "--to-version",
            "3.0.0",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::Update {
                command:
                    UpdateCommands::BatchItem {
                        item_id,
                        to_version,
                        host,
                        follow,
                    },
            }) => {
                assert_eq!(item_id, uuid(ITEM_UUID));
                assert_eq!(to_version, "3.0.0");
                assert!(host.is_empty());
                assert!(!follow);
            }
            _ => panic!("expected Update BatchItem"),
        }
    }

    #[test]
    fn update_batch_item_with_hosts_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "update",
            "batch-item",
            ITEM_UUID,
            "--to-version",
            "3.0.0",
            "--host",
            HOST_UUID,
            "--follow",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::Update {
                command: UpdateCommands::BatchItem { host, follow, .. },
            }) => {
                assert_eq!(host.len(), 1);
                assert_eq!(host[0], uuid(HOST_UUID));
                assert!(follow);
            }
            _ => panic!("expected Update BatchItem"),
        }
    }

    // ── host-packages ────────────────────────────────────────────────

    #[test]
    fn host_packages_list_parses() {
        let args = Cli::try_parse_from(["uptrakit", "host-packages", "list", HOST_UUID])
            .expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::HostPackages {
                command: HostPackagesCommands::List { .. }
            })
        ));
    }

    #[test]
    fn host_packages_list_with_filters() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "host-packages",
            "list",
            HOST_UUID,
            "--enabled",
            "true",
            "--has-update",
            "true",
            "--category",
            "security",
            "--search",
            "nginx",
            "--page",
            "2",
            "--per-page",
            "10",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::HostPackages {
                command:
                    HostPackagesCommands::List {
                        host_id,
                        page,
                        per_page,
                        enabled,
                        has_update,
                        category,
                        search,
                    },
            }) => {
                assert_eq!(host_id, uuid(HOST_UUID));
                assert_eq!(page, Some(2));
                assert_eq!(per_page, Some(10));
                assert_eq!(enabled, Some(true));
                assert_eq!(has_update, Some(true));
                assert_eq!(category.as_deref(), Some("security"));
                assert_eq!(search.as_deref(), Some("nginx"));
            }
            _ => panic!("expected HostPackages List"),
        }
    }

    #[test]
    fn host_packages_show_parses() {
        let args = Cli::try_parse_from(["uptrakit", "host-packages", "show", HOST_UUID, PKG_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::HostPackages {
                command:
                    HostPackagesCommands::Show {
                        host_id,
                        package_id,
                    },
            }) => {
                assert_eq!(host_id, uuid(HOST_UUID));
                assert_eq!(package_id, uuid(PKG_UUID));
            }
            _ => panic!("expected HostPackages Show"),
        }
    }

    #[test]
    fn host_packages_enable_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "host-packages", "enable", HOST_UUID, PKG_UUID])
                .expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::HostPackages {
                command: HostPackagesCommands::Enable { .. }
            })
        ));
    }

    #[test]
    fn host_packages_disable_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "host-packages", "disable", HOST_UUID, PKG_UUID])
                .expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::HostPackages {
                command: HostPackagesCommands::Disable { .. }
            })
        ));
    }

    #[test]
    fn host_packages_delete_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "host-packages",
            "delete",
            HOST_UUID,
            PKG_UUID,
            "--ignore",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::HostPackages {
                command:
                    HostPackagesCommands::Delete {
                        host_id,
                        package_id,
                        ignore,
                    },
            }) => {
                assert_eq!(host_id, uuid(HOST_UUID));
                assert_eq!(package_id, uuid(PKG_UUID));
                assert!(ignore);
            }
            _ => panic!("expected HostPackages Delete"),
        }
    }

    #[test]
    fn host_packages_ignore_list_parses() {
        let args = Cli::try_parse_from(["uptrakit", "host-packages", "ignore", "list", HOST_UUID])
            .expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::HostPackages {
                command: HostPackagesCommands::Ignore {
                    command: HostPackageIgnoreCommands::List { .. }
                }
            })
        ));
    }

    #[test]
    fn host_packages_ignore_add_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "host-packages",
            "ignore",
            "add",
            HOST_UUID,
            "--plugin-config",
            PC_UUID,
            "--package",
            "nginx",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::HostPackages {
                command:
                    HostPackagesCommands::Ignore {
                        command:
                            HostPackageIgnoreCommands::Add {
                                host_id,
                                plugin_config,
                                package,
                            },
                    },
            }) => {
                assert_eq!(host_id, uuid(HOST_UUID));
                assert_eq!(plugin_config, uuid(PC_UUID));
                assert_eq!(package, "nginx");
            }
            _ => panic!("expected HostPackages Ignore Add"),
        }
    }

    #[test]
    fn host_packages_ignore_remove_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "host-packages",
            "ignore",
            "remove",
            HOST_UUID,
            IGNORE_UUID,
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::HostPackages {
                command:
                    HostPackagesCommands::Ignore {
                        command: HostPackageIgnoreCommands::Remove { host_id, ignore_id },
                    },
            }) => {
                assert_eq!(host_id, uuid(HOST_UUID));
                assert_eq!(ignore_id, uuid(IGNORE_UUID));
            }
            _ => panic!("expected HostPackages Ignore Remove"),
        }
    }

    #[test]
    fn host_packages_promote_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "host-packages",
            "promote",
            HOST_UUID,
            PKG_UUID,
            "--name",
            "My App",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::HostPackages {
                command:
                    HostPackagesCommands::Promote {
                        host_id,
                        package_id,
                        name,
                        software_item_id,
                    },
            }) => {
                assert_eq!(host_id, uuid(HOST_UUID));
                assert_eq!(package_id, uuid(PKG_UUID));
                assert_eq!(name.as_deref(), Some("My App"));
                assert!(software_item_id.is_none());
            }
            _ => panic!("expected HostPackages Promote"),
        }
    }

    #[test]
    fn host_packages_promote_with_software_item_id_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "host-packages",
            "promote",
            HOST_UUID,
            PKG_UUID,
            "--software-item-id",
            IGNORE_UUID,
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::HostPackages {
                command:
                    HostPackagesCommands::Promote {
                        host_id,
                        package_id,
                        name,
                        software_item_id,
                    },
            }) => {
                assert_eq!(host_id, uuid(HOST_UUID));
                assert_eq!(package_id, uuid(PKG_UUID));
                assert!(name.is_none());
                assert_eq!(software_item_id, Some(uuid(IGNORE_UUID)));
            }
            _ => panic!("expected HostPackages Promote with software_item_id"),
        }
    }

    // ── update-batches ──────────────────────────────────────────────

    #[test]
    fn update_batches_list_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "update-batches", "list"]).expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::UpdateBatches {
                command: UpdateBatchesCommands::List { .. }
            })
        ));
    }

    #[test]
    fn update_batches_list_with_filters() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "update-batches",
            "list",
            "--status",
            "in_progress",
            "--page",
            "2",
            "--per-page",
            "10",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::UpdateBatches {
                command:
                    UpdateBatchesCommands::List {
                        status,
                        page,
                        per_page,
                    },
            }) => {
                assert_eq!(status.as_deref(), Some("in_progress"));
                assert_eq!(page, Some(2));
                assert_eq!(per_page, Some(10));
            }
            _ => panic!("expected UpdateBatches List"),
        }
    }

    #[test]
    fn update_batches_show_parses() {
        let args = Cli::try_parse_from(["uptrakit", "update-batches", "show", HIST_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::UpdateBatches {
                command: UpdateBatchesCommands::Show { id },
            }) => {
                assert_eq!(id, uuid(HIST_UUID));
            }
            _ => panic!("expected UpdateBatches Show"),
        }
    }

    #[test]
    fn update_batches_follow_parses() {
        let args = Cli::try_parse_from(["uptrakit", "update-batches", "follow", HIST_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::UpdateBatches {
                command: UpdateBatchesCommands::Follow { id },
            }) => {
                assert_eq!(id, uuid(HIST_UUID));
            }
            _ => panic!("expected UpdateBatches Follow"),
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
            "--capability",
            "software_discovery",
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
                        capability,
                        status,
                        page,
                        per_page,
                    },
            }) => {
                assert_eq!(capability.as_deref(), Some("software_discovery"));
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
            "--lifetime-hours",
            "8760",
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
                                lifetime_hours,
                                renewal_window_hours,
                            },
                    },
            }) => {
                assert_eq!(lifetime_hours, Some(8760));
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
    fn settings_nats_show_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "settings", "nats", "show"]).expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::Settings {
                command: SettingsCommands::Nats {
                    command: NatsCommands::Show
                }
            })
        ));
    }

    #[test]
    fn settings_nats_set_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "settings",
            "nats",
            "set",
            "--url",
            "nats://host:4222",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::Settings {
                command:
                    SettingsCommands::Nats {
                        command: NatsCommands::Set { url },
                    },
            }) => {
                assert_eq!(url, "nats://host:4222");
            }
            _ => panic!("expected Settings Nats Set"),
        }
    }

    #[test]
    fn settings_nats_clear_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "settings", "nats", "clear"]).expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::Settings {
                command: SettingsCommands::Nats {
                    command: NatsCommands::Clear
                }
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
            "--plugin-config",
            PC_UUID,
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::Hosts {
                command: HostsCommands::DiscardDiscovered { id, plugin_config },
            }) => {
                assert_eq!(id, uuid(HOST_UUID));
                assert_eq!(plugin_config, Some(uuid(PC_UUID)));
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
            "--plugin-config",
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
                        plugin_config,
                        package,
                    },
            }) => {
                assert_eq!(id, uuid(ITEM_UUID));
                assert_eq!(host, uuid(HOST_UUID));
                assert_eq!(plugin_config, Some(uuid(PC_UUID)));
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
    fn plugin_configs_list_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "plugin-configs", "list"]).expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::PluginConfigs {
                command: PluginConfigsCommands::List { .. }
            })
        ));
    }

    #[test]
    fn plugin_configs_show_parses() {
        let args = Cli::try_parse_from(["uptrakit", "plugin-configs", "show", PC_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::PluginConfigs {
                command: PluginConfigsCommands::Show { id },
            }) => {
                assert_eq!(id, uuid(PC_UUID));
            }
            _ => panic!("expected PluginConfigs Show"),
        }
    }

    #[test]
    fn plugin_configs_create_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "plugin-configs",
            "create",
            "--name",
            "My GitHub",
            "--plugin-type",
            "releases_github",
            "--config",
            r#"{"tag_strip_prefix":"v"}"#,
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::PluginConfigs {
                command:
                    PluginConfigsCommands::Create {
                        name, plugin_type, ..
                    },
            }) => {
                assert_eq!(name, "My GitHub");
                assert_eq!(plugin_type, "releases_github");
            }
            _ => panic!("expected PluginConfigs Create"),
        }
    }

    #[test]
    fn plugin_configs_delete_parses() {
        let args = Cli::try_parse_from(["uptrakit", "plugin-configs", "delete", PC_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::PluginConfigs {
                command: PluginConfigsCommands::Delete { id },
            }) => {
                assert_eq!(id, uuid(PC_UUID));
            }
            _ => panic!("expected PluginConfigs Delete"),
        }
    }

    #[test]
    fn plugin_configs_discover_parses() {
        let args = Cli::try_parse_from(["uptrakit", "plugin-configs", "discover", PC_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::PluginConfigs {
                command: PluginConfigsCommands::Discover { id },
            }) => {
                assert_eq!(id, uuid(PC_UUID));
            }
            _ => panic!("expected PluginConfigs Discover"),
        }
    }

    #[test]
    fn plugin_configs_discard_discovered_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "plugin-configs", "discard-discovered", PC_UUID])
                .expect("should parse");
        match args.command {
            Some(Commands::PluginConfigs {
                command: PluginConfigsCommands::DiscardDiscovered { id },
            }) => {
                assert_eq!(id, uuid(PC_UUID));
            }
            _ => panic!("expected PluginConfigs DiscardDiscovered"),
        }
    }

    #[test]
    fn enrollment_tokens_list_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "enrollment-tokens", "list"]).expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::EnrollmentTokens {
                command: EnrollmentTokensCommands::List {
                    page: None,
                    per_page: None
                }
            })
        ));
    }

    #[test]
    fn enrollment_tokens_list_with_pagination() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "enrollment-tokens",
            "list",
            "--page",
            "2",
            "--per-page",
            "50",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::EnrollmentTokens {
                command: EnrollmentTokensCommands::List { page, per_page },
            }) => {
                assert_eq!(page, Some(2));
                assert_eq!(per_page, Some(50));
            }
            _ => panic!("expected EnrollmentTokens List"),
        }
    }

    #[test]
    fn enrollment_tokens_create_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "enrollment-tokens",
            "create",
            "--name",
            "CI Token",
            "--capabilities",
            "software_discovery,mqtt_bridge",
            "--max-uses",
            "10",
            "--expires-in",
            "86400",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::EnrollmentTokens {
                command:
                    EnrollmentTokensCommands::Create {
                        name,
                        capabilities,
                        max_uses,
                        expires_in,
                    },
            }) => {
                assert_eq!(name, "CI Token");
                assert_eq!(
                    capabilities.as_deref(),
                    Some("software_discovery,mqtt_bridge")
                );
                assert_eq!(max_uses, Some(10));
                assert_eq!(expires_in, Some(86400));
            }
            _ => panic!("expected EnrollmentTokens Create"),
        }
    }

    #[test]
    fn enrollment_tokens_create_minimal() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "enrollment-tokens",
            "create",
            "--name",
            "Wildcard",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::EnrollmentTokens {
                command:
                    EnrollmentTokensCommands::Create {
                        name,
                        capabilities,
                        max_uses,
                        expires_in,
                    },
            }) => {
                assert_eq!(name, "Wildcard");
                assert!(capabilities.is_none());
                assert!(max_uses.is_none());
                assert!(expires_in.is_none());
            }
            _ => panic!("expected EnrollmentTokens Create"),
        }
    }

    #[test]
    fn enrollment_tokens_show_parses() {
        let args = Cli::try_parse_from(["uptrakit", "enrollment-tokens", "show", ET_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::EnrollmentTokens {
                command: EnrollmentTokensCommands::Show { id },
            }) => {
                assert_eq!(id, uuid(ET_UUID));
            }
            _ => panic!("expected EnrollmentTokens Show"),
        }
    }

    #[test]
    fn enrollment_tokens_revoke_parses() {
        let args = Cli::try_parse_from(["uptrakit", "enrollment-tokens", "revoke", ET_UUID])
            .expect("should parse");
        match args.command {
            Some(Commands::EnrollmentTokens {
                command: EnrollmentTokensCommands::Revoke { id },
            }) => {
                assert_eq!(id, uuid(ET_UUID));
            }
            _ => panic!("expected EnrollmentTokens Revoke"),
        }
    }

    #[test]
    fn system_enrollment_tokens_list_parses() {
        let args = Cli::try_parse_from(["uptrakit", "system-enrollment-tokens", "list"])
            .expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::SystemEnrollmentTokens {
                command: SystemEnrollmentTokensCommands::List {
                    page: None,
                    per_page: None
                }
            })
        ));
    }

    #[test]
    fn system_enrollment_tokens_create_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "system-enrollment-tokens",
            "create",
            "--name",
            "MQTT Bridge Token",
            "--max-uses",
            "5",
            "--expires-in",
            "86400",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::SystemEnrollmentTokens {
                command:
                    SystemEnrollmentTokensCommands::Create {
                        name,
                        max_uses,
                        expires_in,
                    },
            }) => {
                assert_eq!(name, "MQTT Bridge Token");
                assert_eq!(max_uses, Some(5));
                assert_eq!(expires_in, Some(86400));
            }
            _ => panic!("expected SystemEnrollmentTokens Create"),
        }
    }

    #[test]
    fn system_enrollment_tokens_create_minimal() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "system-enrollment-tokens",
            "create",
            "--name",
            "Unlimited",
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::SystemEnrollmentTokens {
                command:
                    SystemEnrollmentTokensCommands::Create {
                        name,
                        max_uses,
                        expires_in,
                    },
            }) => {
                assert_eq!(name, "Unlimited");
                assert!(max_uses.is_none());
                assert!(expires_in.is_none());
            }
            _ => panic!("expected SystemEnrollmentTokens Create"),
        }
    }

    #[test]
    fn system_enrollment_tokens_show_parses() {
        let args =
            Cli::try_parse_from(["uptrakit", "system-enrollment-tokens", "show", SYS_ET_UUID])
                .expect("should parse");
        match args.command {
            Some(Commands::SystemEnrollmentTokens {
                command: SystemEnrollmentTokensCommands::Show { id },
            }) => {
                assert_eq!(id, uuid(SYS_ET_UUID));
            }
            _ => panic!("expected SystemEnrollmentTokens Show"),
        }
    }

    #[test]
    fn system_enrollment_tokens_revoke_parses() {
        let args = Cli::try_parse_from([
            "uptrakit",
            "system-enrollment-tokens",
            "revoke",
            SYS_ET_UUID,
        ])
        .expect("should parse");
        match args.command {
            Some(Commands::SystemEnrollmentTokens {
                command: SystemEnrollmentTokensCommands::Revoke { id },
            }) => {
                assert_eq!(id, uuid(SYS_ET_UUID));
            }
            _ => panic!("expected SystemEnrollmentTokens Revoke"),
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
            "--plugin-config",
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
                                plugin_config,
                                package,
                            },
                    },
            }) => {
                assert_eq!(plugin_config, uuid(PC_UUID));
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
