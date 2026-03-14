pub mod api;
pub mod audit_logs;
pub mod auth;
pub mod autodiscovery;
pub mod batch_update;
pub mod check;
pub mod discovery_allowlist;
pub mod enrollment_tokens;
pub mod extensions;
pub mod history;
pub mod host_tags;
pub mod hosts;
pub mod notifications;
pub mod plugin_configs;
pub mod plugin_type_settings;
pub mod scheduler;
pub mod services;
pub mod settings;
pub mod software_items;
pub mod system_enrollment_tokens;
pub mod system_services;
pub mod tail;
pub mod update;
pub mod users;

use crate::output::OutputFormat;

/// Shared CLI state passed to every subcommand dispatch function.
pub struct CliContext {
    pub server: Option<String>,
    pub token: Option<String>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub format: OutputFormat,
}

/// Resolve `--ca-pem` (inline string) or `--ca-pem-file` (file path) into a single
/// `Option<String>`. Clap's `conflicts_with` ensures at most one is provided.
pub fn resolve_ca_pem(
    ca_pem: Option<String>,
    ca_pem_file: Option<std::path::PathBuf>,
) -> crate::error::Result<Option<String>> {
    use rootcause::prelude::*;
    match (ca_pem, ca_pem_file) {
        (Some(pem), None) => Ok(Some(pem)),
        (None, Some(path)) => {
            let contents = std::fs::read_to_string(&path).context_to()?;
            Ok(Some(contents))
        }
        _ => Ok(None),
    }
}

/// Parse a registration mode string into the typed enum.
pub fn parse_registration_mode(
    s: &str,
) -> std::result::Result<uptrakit_openapi_client::types::registration::RegistrationMode, String> {
    s.parse()
        .map_err(|_| format!("invalid registration mode: {s} (expected open, invite, or closed)"))
}
