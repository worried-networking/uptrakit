mod sync_openapi_client;
mod sync_sdk;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Regenerate service-sdk src/generated/ from wire + surfaces source.
    SyncSdk {
        /// Exit with error if any file would change (for CI / pre-commit).
        #[arg(long)]
        check: bool,
        /// Regenerate and commit in one shot.
        #[arg(long)]
        commit: bool,
    },
    /// Copy web-api-types + internal deps into openapi-client src/generated/.
    SyncOpenapiClient {
        /// Exit non-zero if any generated file would change (CI / pre-commit).
        #[arg(long)]
        check: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let workspace_root = workspace_root()?;

    match cli.command {
        Command::SyncSdk { check, commit } => sync_sdk::run(&workspace_root, check, commit)?,
        Command::SyncOpenapiClient { check } => {
            sync_openapi_client::run(&workspace_root, check)?
        }
    }
    Ok(())
}

fn workspace_root() -> Result<PathBuf> {
    let output = std::process::Command::new("cargo")
        .args(["locate-project", "--workspace", "--message-format=plain"])
        .output()?;
    let manifest = String::from_utf8(output.stdout)?.trim().to_string();
    Ok(PathBuf::from(manifest)
        .parent()
        .expect("Cargo.toml has parent")
        .to_path_buf())
}
