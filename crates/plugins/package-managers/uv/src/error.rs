use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_plugin_infrastructure_core::PluginError;
use uptrakit_shared_macros::impl_report_conversion;

/// Errors specific to the uv package-manager plugin.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum UvError {
    /// `uv tool list` parsed to zero tools while its output still carried
    /// evidence that the output format itself changed — see
    /// [`crate::plugin::parse_uv_tool_list`] and the format-drift guard in
    /// `Discoverer::discover_software`.
    ///
    /// Carries the raw output length only: the output is host state, not a
    /// diagnostic worth copying into an error string.
    #[error(
        "uv tool list output ({0} bytes) did not match the expected format \
         (possible uv output format change)"
    )]
    OutputFormatDrift(usize),

    /// Invalid or unusable plugin configuration.
    #[error("configuration error: {0}")]
    Configuration(String),
}

/// Result alias covering every fallible uv-plugin-internal function.
pub type Result<T> = std::result::Result<T, Report<UvError>>;

impl_report_conversion!(UvError => PluginError, |e| PluginError::PluginInternal(e.to_string()));
impl_report_conversion!(PluginError => UvError, |e| UvError::Configuration(e.to_string()));
