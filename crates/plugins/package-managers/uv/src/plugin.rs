use uptrakit_plugin_infrastructure_core::PluginConfigValidationError;
use uptrakit_shared_types::PackageIdentifierRules;

/// PEP 503/508 project-name charset: ASCII alphanumeric plus `.`, `_`, `-`;
/// must start (and per PEP 508 end, enforced loosely here) alphanumeric.
const IDENTIFIER_RULES: PackageIdentifierRules = PackageIdentifierRules {
    min_len: 1,
    max_len: 128,
    first_char_valid: |c| c.is_ascii_alphanumeric(),
    char_valid: |c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-',
    reject_double_dot: true,
};

/// Validate a uv tool package identifier (PEP 503/508 project-name charset).
///
/// uv prints PEP 503-normalized names (`ruamel.yaml.cmd` → `ruamel-yaml-cmd`),
/// but the raw charset still admits `.` and `_` so operator-entered
/// identifiers in either form validate. `..` and path separators are rejected
/// (the identifier is later used as a path segment under the uv tools dir).
pub fn validate_identifier(value: &str) -> std::result::Result<(), PluginConfigValidationError> {
    IDENTIFIER_RULES
        .validate(value)
        .map_err(PluginConfigValidationError::InvalidIdentifier)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_identifier_valid_names() {
        validate_identifier("ruff").unwrap();
        validate_identifier("ruamel-yaml-cmd").unwrap();
        validate_identifier("ruamel.yaml.cmd").unwrap();
        validate_identifier("typing_extensions").unwrap();
        validate_identifier("2to3").unwrap(); // digits are valid PEP 508 first chars
        validate_identifier("a").unwrap();
        validate_identifier(&"a".repeat(128)).unwrap();
    }

    #[test]
    fn validate_identifier_invalid_names() {
        validate_identifier("").unwrap_err();
        validate_identifier(&"a".repeat(129)).unwrap_err();
        validate_identifier("-ruff").unwrap_err();
        validate_identifier(".ruff").unwrap_err();
        validate_identifier("a..b").unwrap_err();
        validate_identifier("owner/pkg").unwrap_err();
        validate_identifier("pkg name").unwrap_err();
        validate_identifier("pkg==1.0").unwrap_err();
    }
}
