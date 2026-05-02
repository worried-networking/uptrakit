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
    #[expect(
        clippy::map_err_ignore,
        reason = "url::ParseError details are not useful to callers; InvalidUrl conveys the same information"
    )]
    let parsed = url::Url::parse(value).map_err(|_| ProviderValidationError::InvalidUrl)?;
    if parsed.scheme() != "https" {
        return Err(ProviderValidationError::MustUseHttps);
    }
    let host = parsed
        .host_str()
        .ok_or(ProviderValidationError::MissingHost)?;
    if crate::network::is_private_host(host) {
        return Err(ProviderValidationError::PrivateHost);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — is_ok/is_err provides readable failure messages"
    )]
    use super::*;

    #[test]
    fn provider_validation_accepts_public_https_url() {
        let result = validate_provider_api_base_url("https://api.github.com");
        assert!(result.is_ok());
    }

    #[test]
    fn provider_validation_rejects_non_https_url() {
        let result = validate_provider_api_base_url("http://api.github.com");
        assert_eq!(result, Err(ProviderValidationError::MustUseHttps));
    }

    #[test]
    fn provider_validation_rejects_private_host() {
        let result = validate_provider_api_base_url("https://localhost/api/v3");
        assert_eq!(result, Err(ProviderValidationError::PrivateHost));
    }

    #[test]
    fn provider_validation_rejects_invalid_url() {
        let result = validate_provider_api_base_url("not-a-url");
        assert_eq!(result, Err(ProviderValidationError::InvalidUrl));
    }
}
