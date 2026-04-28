// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use thiserror::Error;
/// Error returned when provider URL validation fails.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProviderValidationError {
    #[error("invalid URL")]
    InvalidUrl,
    #[error("must use https")]
    MustUseHttps,
    #[error("must include a host")]
    MissingHost,
    #[error("must not point to private/loopback addresses")]
    PrivateHost,
}
/// Validate provider API base URL.
pub fn validate_provider_api_base_url(value: &str) -> Result<(), ProviderValidationError> {
    let parsed = url::Url::parse(value).map_err(|_| ProviderValidationError::InvalidUrl)?;
    if parsed.scheme() != "https" {
        return Err(ProviderValidationError::MustUseHttps);
    }
    let host = parsed
        .host_str()
        .ok_or(ProviderValidationError::MissingHost)?;
    if crate::generated::shared_types::network::is_private_host(host) {
        return Err(ProviderValidationError::PrivateHost);
    }
    Ok(())
}
