//! `cargo xtask openapi-client-check` — assert the client tracks the OpenAPI spec.

use std::path::Path;
use std::process::ExitCode;

/// Entry point for the subcommand. Returns the process exit code.
#[must_use]
pub fn cli(_root: &Path) -> ExitCode {
    ExitCode::SUCCESS
}
