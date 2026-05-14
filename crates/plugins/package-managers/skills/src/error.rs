use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_plugin_infrastructure_core::PluginError;
use uptrakit_shared_macros::impl_report_conversion;

/// Errors specific to the Agent Skills plugin.
#[derive(Debug, Error)]
pub(crate) enum SkillsError {
    #[error("lock file malformed: {0}")]
    LockFileMalformed(String),

    #[error("invalid identifier: {0}")]
    InvalidIdentifier(String),

    #[error("unsupported source type: {0}")]
    UnsupportedSource(String),

    #[error("plugin error: {0}")]
    Plugin(String),
}

/// Result type alias for the Skills plugin.
pub(crate) type Result<T> = std::result::Result<T, Report<SkillsError>>;

impl_report_conversion!(SkillsError => PluginError, |e| match &e {
    SkillsError::InvalidIdentifier(_) => PluginError::Configuration(e.to_string()),
    _ => PluginError::PluginInternal(e.to_string()),
});
impl_report_conversion!(PluginError => SkillsError, |e| SkillsError::Plugin(e.to_string()));
