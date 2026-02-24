use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_provider_core::ProviderError;
use uptrakit_shared_macros::impl_report_conversion;

#[derive(Debug, Error)]
pub enum AptError {
    #[error("command execution failed with exit code {0}")]
    CommandFailed(i32),
    #[error("failed to parse apt output: {0}")]
    ParseOutput(String),
    #[error("configuration error: {0}")]
    Configuration(String),
}

pub type Result<T> = std::result::Result<T, Report<AptError>>;

impl_report_conversion!(ProviderError => AptError, |e| AptError::Configuration(e.to_string()));
impl_report_conversion!(AptError => ProviderError, |e| ProviderError::ProviderInternal(e.to_string()));
