mod client;
mod commands;
mod config;
mod error;
mod output;

use clap::{CommandFactory, Parser, Subcommand};
use output::OutputFormat;
use uptrakit_build_info::BuildInfo;

#[derive(Debug, Parser)]
#[command(name = "uptrakit-cli", about = "Uptrakit CLI")]
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

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if cli.version {
        let build_info = BuildInfo::current(
            "uptrakit-cli",
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
        let args = Cli::try_parse_from(["uptrakit-cli", "--version"]).expect("should parse");
        assert!(args.version);
        assert!(args.command.is_none());
    }

    #[test]
    fn hosts_list_parses() {
        let args = Cli::try_parse_from(["uptrakit-cli", "hosts", "list"]).expect("should parse");
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
            "uptrakit-cli",
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
        let args = Cli::try_parse_from(["uptrakit-cli", "hosts", "show", "abc-123"])
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
            Cli::try_parse_from(["uptrakit-cli", "software-items", "list"]).expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::SoftwareItems {
                command: SoftwareItemsCommands::List { .. }
            })
        ));
    }

    #[test]
    fn software_items_show_parses() {
        let args = Cli::try_parse_from(["uptrakit-cli", "software-items", "show", "item-123"])
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
        let args = Cli::try_parse_from(["uptrakit-cli", "check", "all"]).expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::Check {
                command: CheckCommands::All
            })
        ));
    }

    #[test]
    fn check_installed_parses() {
        let args = Cli::try_parse_from(["uptrakit-cli", "check", "installed", "item-1"])
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
            "uptrakit-cli",
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
        let args = Cli::try_parse_from(["uptrakit-cli", "check", "available", "item-2"])
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
            "uptrakit-cli",
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
            "uptrakit-cli",
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
        let args = Cli::try_parse_from(["uptrakit-cli", "history", "list"]).expect("should parse");
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
            "uptrakit-cli",
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
        let args = Cli::try_parse_from(["uptrakit-cli", "history", "show", "hist-123"])
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
            Cli::try_parse_from(["uptrakit-cli", "scheduler", "list"]).expect("should parse");
        assert!(matches!(
            args.command,
            Some(Commands::Scheduler {
                command: SchedulerCommands::List
            })
        ));
    }

    #[test]
    fn scheduler_show_parses() {
        let args = Cli::try_parse_from(["uptrakit-cli", "scheduler", "show", "task-123"])
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
        let args = Cli::try_parse_from(["uptrakit-cli", "scheduler", "trigger", "task-123"])
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
        let args = Cli::try_parse_from(["uptrakit-cli", "services", "list"]).expect("should parse");
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
            "uptrakit-cli",
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
        let args = Cli::try_parse_from(["uptrakit-cli", "services", "show", "svc-123"])
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
        let args = Cli::try_parse_from(["uptrakit-cli", "services", "approve", "svc-123"])
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
        let args = Cli::try_parse_from(["uptrakit-cli", "services", "reject", "svc-123"])
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
        let args = Cli::try_parse_from(["uptrakit-cli", "services", "remove", "svc-123"])
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
            "uptrakit-cli",
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
            "uptrakit-cli",
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
}
