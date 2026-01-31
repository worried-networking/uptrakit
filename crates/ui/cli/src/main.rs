mod client;
mod commands;
mod config;
mod error;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "uptrakit-cli", about = "Uptrakit CLI")]
struct Cli {
    /// Server URL (overrides stored config)
    #[arg(long, global = true)]
    server: Option<String>,

    /// API token (overrides stored credentials)
    #[arg(long, global = true)]
    token: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
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

#[derive(Subcommand)]
enum AuthCommands {
    /// Login with email and password
    Login,
    /// Show current authentication status
    Status,
    /// API token management
    Token {
        #[command(subcommand)]
        command: TokenCommands,
    },
}

#[derive(Subcommand)]
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

    let result = match cli.command {
        Commands::Auth { command } => match command {
            AuthCommands::Login => commands::auth::login(cli.server.as_deref()).await,
            AuthCommands::Status => {
                commands::auth::status(cli.server.as_deref(), cli.token.as_deref()).await
            }
            AuthCommands::Token { command } => match command {
                TokenCommands::Create { name } => {
                    commands::auth::token_create(&name, cli.server.as_deref(), cli.token.as_deref())
                        .await
                }
                TokenCommands::List => {
                    commands::auth::token_list(cli.server.as_deref(), cli.token.as_deref()).await
                }
                TokenCommands::Revoke { id } => {
                    commands::auth::token_revoke(&id, cli.server.as_deref(), cli.token.as_deref())
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
            )
            .await
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
