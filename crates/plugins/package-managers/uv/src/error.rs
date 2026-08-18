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

    /// An outbound HTTP request to the PyPI Simple API failed at the
    /// transport level (connection, TLS, timeout, or decoding).
    #[error("request error: {0}")]
    Request(String),

    /// The PyPI Simple API returned a non-success HTTP status.
    #[error("API error (status {status}): {message}")]
    ApiError {
        status: reqwest::StatusCode,
        message: String,
    },

    /// A valid Simple-API project page always carries at least one version or
    /// file; zero extractable versions signals a wrong index URL or lossy
    /// (HTML) content negotiation — never a silent empty list.
    #[error("index returned no extractable versions: {0}")]
    EmptyIndex(String),
}

/// Result alias covering every fallible uv-plugin-internal function.
pub type Result<T> = std::result::Result<T, Report<UvError>>;

impl_report_conversion!(UvError => PluginError, |e| PluginError::PluginInternal(e.to_string()));
impl_report_conversion!(PluginError => UvError, |e| UvError::Configuration(e.to_string()));
impl_report_conversion!(reqwest::Error => UvError, |e| UvError::Request(e.to_string()));
