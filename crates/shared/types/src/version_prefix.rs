//! Version-prefix validation shared by release-source and shell plugins.
//!
//! `tag_prefix` (GitHub/Forgejo release plugins) and `version_strip_prefix`
//! (generic shell plugin) are short literal prefixes; this module bounds
//! their length so plugin configs and per-host overrides cannot store
//! unbounded strings. Placed in `shared-types` for the same reason as
//! [`crate::command_validation`]: every plugin crate already depends on it.

/// Maximum length of a version/tag prefix string.
pub const MAX_VERSION_PREFIX_LENGTH: usize = 256;

/// Validate a prefix string: non-empty, no surrounding whitespace or control
/// characters, and within [`MAX_VERSION_PREFIX_LENGTH`].
///
/// Returns `Ok(())` if the prefix is acceptable, or `Err` with a
/// human-readable message identifying the field that failed validation.
/// Empty prefixes are rejected — an absent optional field expresses "no
/// prefix"; `Some("")` is always a configuration mistake. Leading/trailing
/// whitespace and control characters are rejected because tags never carry
/// them: such a prefix can only silently match nothing.
pub fn validate_version_prefix(prefix: &str, field_name: &str) -> Result<(), String> {
    if prefix.is_empty() {
        return Err(format!(
            "{field_name} must not be empty (omit the field instead)"
        ));
    }
    if prefix != prefix.trim() || prefix.chars().any(char::is_control) {
        return Err(format!(
            "{field_name} must not contain leading/trailing whitespace or control characters"
        ));
    }
    if prefix.len() > MAX_VERSION_PREFIX_LENGTH {
        return Err(format!(
            "{field_name} exceeds maximum length of {MAX_VERSION_PREFIX_LENGTH} bytes ({} bytes)",
            prefix.len()
        ));
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
    fn validate_version_prefix_ok() {
        assert!(validate_version_prefix("uptrakit-controller-standalone-v", "tag_prefix").is_ok());
    }

    #[test]
    fn validate_version_prefix_at_limit() {
        let prefix = "x".repeat(MAX_VERSION_PREFIX_LENGTH);
        assert!(validate_version_prefix(&prefix, "tag_prefix").is_ok());
    }

    #[test]
    fn validate_version_prefix_over_limit() {
        let prefix = "x".repeat(MAX_VERSION_PREFIX_LENGTH + 1);
        let err = validate_version_prefix(&prefix, "tag_prefix").unwrap_err();
        assert!(err.contains("tag_prefix"));
        assert!(err.contains("exceeds maximum length"));
    }

    #[test]
    fn validate_version_prefix_empty() {
        let err = validate_version_prefix("", "version_strip_prefix").unwrap_err();
        assert!(err.contains("must not be empty"));
        assert!(err.contains("version_strip_prefix"));
    }

    #[test]
    fn validate_version_prefix_rejects_surrounding_whitespace_and_control() {
        let err = validate_version_prefix(" v", "tag_prefix").unwrap_err();
        assert!(err.contains("whitespace"));
        let err = validate_version_prefix("v ", "tag_prefix").unwrap_err();
        assert!(err.contains("whitespace"));
        let err = validate_version_prefix("v\u{7}x", "tag_prefix").unwrap_err();
        assert!(err.contains("control"));
        // Interior whitespace is fine — some projects use spaced tag schemes.
        assert!(validate_version_prefix("my app v", "tag_prefix").is_ok());
    }
}
