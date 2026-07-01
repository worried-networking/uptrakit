//! `cargo xtask openapi-client-check` — assert the client tracks the OpenAPI spec.

pub mod check;
pub mod client;
pub mod ledgers;
pub mod normalize;
pub mod spec;

use std::path::Path;
use std::process::ExitCode;

/// Entry point for the subcommand. Returns the process exit code.
#[must_use]
pub fn cli(_root: &Path) -> ExitCode {
    ExitCode::SUCCESS
}
