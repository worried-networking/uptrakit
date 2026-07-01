//! `xtask` — repo dev-tooling entry point. `cargo xtask <command>`.

use clap::{Parser, Subcommand};
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "xtask", about = "Repo dev-tooling gates")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

// Variant idents deliberately avoid a shared `…Check` suffix to future-proof against
// `clippy::enum_variant_names` as more subcommands land (it triggers at 3+ variants with a
// common affix); the CLI names are pinned with `#[command(name = …)]`.
#[derive(Subcommand)]
enum Command {
    /// Assert uptrakit-openapi-client tracks the OpenAPI spec.
    #[command(name = "openapi-client-check")]
    OpenapiClient,
    /// Ensure every state-changing site has an audit-catalog decision.
    #[command(name = "audit-coverage-check")]
    AuditCoverage,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let root = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("failed to get cwd: {e}");
            return ExitCode::from(2);
        }
    };
    match cli.command {
        Command::OpenapiClient => xtask::openapi_client_check::cli(&root),
        Command::AuditCoverage => xtask::audit_coverage_check::cli(&root),
    }
}
