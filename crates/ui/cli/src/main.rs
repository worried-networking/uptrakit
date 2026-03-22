use uptrakit_cli::{commands, error, output};

use clap::{CommandFactory, Parser, Subcommand};
use uptrakit_build_info::BuildInfo;
use uptrakit_cli::output::OutputFormat;

// Re-import subcommand enums from lib-crate modules so clap can derive on `Commands`.
use commands::audit_logs::AuditLogsCommands;
use commands::auth::AuthCommands;
use commands::autodiscovery::AutodiscoveryCommands;
use commands::batch_update::UpdateBatchesCommands;
use commands::check::CheckCommands;
use commands::discovery_allowlist::DiscoveryAllowlistCommands;
use commands::enrollment_tokens::EnrollmentTokensCommands;
use commands::extensions::ExtensionsCommands;
use commands::history::HistoryCommands;
use commands::host_tags::HostTagsCommands;
use commands::hosts::HostsCommands;
use commands::notifications::NotificationsCommands;
use commands::plugin_configs::PluginConfigsCommands;
use commands::plugin_type_settings::PluginTypeSettingsCommands;
use commands::scheduler::SchedulerCommands;
use commands::services::ServicesCommands;
use commands::settings::SettingsCommands;
use commands::software_items::SoftwareItemsCommands;
use commands::system_enrollment_tokens::SystemEnrollmentTokensCommands;
use commands::system_services::SystemServicesCommands;
use commands::update::UpdateCommands;
use commands::users::{AccessPresetsCommands, RolesCommands, UsersCommands};

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
    /// Manage per-plugin-type default settings
    PluginTypeSettings {
        #[command(subcommand)]
        command: PluginTypeSettingsCommands,
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
    /// Manage host tags
    HostTags {
        #[command(subcommand)]
        command: HostTagsCommands,
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
    /// Manage users, roles, and access presets
    Users {
        #[command(subcommand)]
        command: UsersCommands,
    },
    /// Manage roles and their permissions
    Roles {
        #[command(subcommand)]
        command: RolesCommands,
    },
    /// List access presets
    AccessPresets {
        #[command(subcommand)]
        command: AccessPresetsCommands,
    },
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

    uptrakit_service_sdk::init_cli_tracing(cli.verbose);

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

    let ctx = commands::CliContext {
        server: cli.server,
        token: cli.token,
        insecure: cli.insecure,
        request_timeout: cli.timeout.map(std::time::Duration::from_secs),
        format: cli.output,
    };

    match command {
        Commands::Auth { command } => commands::auth::dispatch(command, &ctx).await?,
        Commands::Api { method, path, data } => {
            commands::api::execute(commands::api::ExecuteParams {
                method: &method,
                path: &path,
                data: data.as_deref(),
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                format: ctx.format,
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
        }
        Commands::Services { command } => commands::services::dispatch(command, &ctx).await?,
        Commands::Hosts { command } => commands::hosts::dispatch(command, &ctx).await?,
        Commands::SoftwareItems { command } => {
            commands::software_items::dispatch(command, &ctx).await?;
        }
        Commands::Check { command } => commands::check::dispatch(command, &ctx).await?,
        Commands::Update { command } => commands::update::dispatch(command, &ctx).await?,
        Commands::History { command } => commands::history::dispatch(command, &ctx).await?,
        Commands::Scheduler { command } => commands::scheduler::dispatch(command, &ctx).await?,
        Commands::Settings { command } => commands::settings::dispatch(command, &ctx).await?,
        Commands::PluginConfigs { command } => {
            commands::plugin_configs::dispatch(command, &ctx).await?;
        }
        Commands::PluginTypeSettings { command } => {
            commands::plugin_type_settings::dispatch(command, &ctx).await?;
        }
        Commands::EnrollmentTokens { command } => {
            commands::enrollment_tokens::dispatch(command, &ctx).await?;
        }
        Commands::Autodiscovery { command } => {
            commands::autodiscovery::dispatch(command, &ctx).await?;
        }
        Commands::DiscoveryAllowlist { command } => {
            commands::discovery_allowlist::dispatch(command, &ctx).await?;
        }
        Commands::Notifications { command } => {
            commands::notifications::dispatch(command, &ctx).await?;
        }
        Commands::HostTags { command } => commands::host_tags::dispatch(command, &ctx).await?,
        Commands::UpdateBatches { command } => {
            commands::batch_update::dispatch(command, &ctx).await?;
        }
        Commands::SystemServices { command } => {
            commands::system_services::dispatch(command, &ctx).await?;
        }
        Commands::SystemEnrollmentTokens { command } => {
            commands::system_enrollment_tokens::dispatch(command, &ctx).await?;
        }
        Commands::AuditLogs { command } => commands::audit_logs::dispatch(command, &ctx).await?,
        Commands::Extensions { command } => commands::extensions::dispatch(command, &ctx).await?,
        Commands::Users { command } => commands::users::dispatch_users(command, &ctx).await?,
        Commands::Roles { command } => commands::users::dispatch_roles(command, &ctx).await?,
        Commands::AccessPresets { command } => {
            commands::users::dispatch_access_presets(command, &ctx).await?;
        }
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
#[path = "tests.rs"]
mod tests;
