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
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser;

    #[test]
    fn version_parses_without_subcommand() {
        let args = Cli::try_parse_from(["uptrakit-cli", "--version"]).expect("should parse");
        assert!(args.version);
        assert!(args.command.is_none());
    }
}
