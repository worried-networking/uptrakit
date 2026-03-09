use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_plugin_infrastructure_core::PluginError;
use uptrakit_shared_macros::impl_report_conversion;

/// Errors specific to the Proxmox Helper Scripts discovery plugin.
#[derive(Debug, Error)]
pub enum ProxmoxHelperScriptsError {
    #[error("HTTP request failed: {0}")]
    Request(String),

    #[error("failed to parse response: {0}")]
    ParseResponse(String),

    #[error("configuration error: {0}")]
    Configuration(String),

    #[error("plugin error: {0}")]
    Plugin(String),

    #[error("script fetch failed for slug '{slug}': {reason}")]
    ScriptFetchFailed { slug: String, reason: String },

    #[error("command execution failed: {0}")]
    CommandFailed(String),
}

/// Result type alias for Proxmox Helper Scripts plugin operations.
pub type Result<T> = std::result::Result<T, Report<ProxmoxHelperScriptsError>>;

impl_report_conversion!(reqwest::Error => ProxmoxHelperScriptsError, |e| ProxmoxHelperScriptsError::Request(e.to_string()));
impl_report_conversion!(PluginError => ProxmoxHelperScriptsError, |e| ProxmoxHelperScriptsError::Configuration(e.to_string()));
impl_report_conversion!(ProxmoxHelperScriptsError => PluginError, |e| PluginError::PluginInternal(e.to_string()));
