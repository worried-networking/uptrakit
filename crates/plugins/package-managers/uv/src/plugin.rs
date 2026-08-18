use std::collections::HashMap;

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

/// Parse `uv tool list` output into a tool name → version map.
///
/// Stdout format (verified on uv 0.11.29):
///
/// ```text
/// ruff v0.6.8
/// - ruff
/// ```
///
/// The input is the **merged stdout+stderr stream** (`CommandOutput.output`
/// concatenates both), so acceptance is hard-anchored. A line is accepted only
/// when, split at the first `" v"`:
/// - the name part starts at column 0, is non-empty, and passes the
///   PEP 503/508 identifier charset (leading whitespace, `- ` bullets,
///   `warning:`/`note:` prefixes all fail this);
/// - the version part starts with an ASCII digit (PEP 440 normalized forms
///   cannot start otherwise) and contains no whitespace — any trailing
///   content rejects the line.
///
/// Non-matching lines (entrypoint bullets, `No tools installed` on stderr,
/// uv warnings/notices) are skipped without error; there is no "malformed
/// payload" bail for this free-text format. The degenerate all-noise case
/// yields an empty map, indistinguishable from "no tools" by design.
pub fn parse_uv_tool_list(output: &str) -> HashMap<String, String> {
    let mut result = HashMap::new();
    for line in output.lines() {
        let Some((name, version)) = line.split_once(" v") else {
            continue;
        };
        if name.is_empty() || validate_identifier(name).is_err() {
            continue;
        }
        if !version.starts_with(|c: char| c.is_ascii_digit())
            || version.contains(char::is_whitespace)
        {
            continue;
        }
        result.insert(name.to_string(), version.to_string());
    }
    result
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

    // ── parse_uv_tool_list ────────────────────────────────────────────────

    #[test]
    fn parse_uv_tool_list_basic() {
        let output = "ruff v0.6.8\n- ruff\nblack v24.4.2\n- black\n- blackd\n";
        let map = parse_uv_tool_list(output);
        assert_eq!(map.get("ruff"), Some(&"0.6.8".to_string()));
        assert_eq!(map.get("black"), Some(&"24.4.2".to_string()));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn parse_uv_tool_list_skips_entrypoint_bullets_and_indented_lines() {
        let map = parse_uv_tool_list("- ruff\n    ruff v0.6.8\n");
        assert!(map.is_empty());
    }

    /// `CommandOutput.output` merges stdout and stderr; the empty state prints
    /// `No tools installed` on stderr with exit 0. The sentinel line must not
    /// become a phantom entry alongside real output.
    #[test]
    fn parse_uv_tool_list_no_tools_installed_stderr_merge() {
        let map = parse_uv_tool_list("No tools installed\nruff v0.6.8\n");
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("ruff"), Some(&"0.6.8".to_string()));
    }

    /// Merged-stream fixture: a stderr warning line is rejected by the
    /// identifier guard alone (its trailing `foo` fragment fails the PEP
    /// 503/508 charset even though the version part `2.0` is otherwise
    /// clean), isolating that guard from the version-whitespace guard.
    #[test]
    fn parse_uv_tool_list_interleaved_warning_line() {
        let output = "warning: foo v2.0\nruff v0.6.8\n- ruff\n";
        let map = parse_uv_tool_list(output);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("ruff"), Some(&"0.6.8".to_string()));
    }

    /// PEP 440 normalized forms are not semver; the parser must accept them
    /// verbatim as the map value (no semver-shaped validation is performed).
    #[test]
    fn parse_uv_tool_list_accepts_pep440_version_forms() {
        let output = "ruff v1.0.0rc1\nblack v2.0.0b3\nmypy v1.0.post1\npoetry v1.0.dev0\nhttpie v1.0+local\n";
        let map = parse_uv_tool_list(output);
        assert_eq!(map.get("ruff"), Some(&"1.0.0rc1".to_string()));
        assert_eq!(map.get("black"), Some(&"2.0.0b3".to_string()));
        assert_eq!(map.get("mypy"), Some(&"1.0.post1".to_string()));
        assert_eq!(map.get("poetry"), Some(&"1.0.dev0".to_string()));
        assert_eq!(map.get("httpie"), Some(&"1.0+local".to_string()));
        assert_eq!(map.len(), 5);
    }

    /// `str::lines` strips a trailing `\r` from CRLF-terminated input, so
    /// Windows-style line endings must yield clean version values.
    #[test]
    fn parse_uv_tool_list_handles_crlf_line_endings() {
        let output = "ruff v0.6.8\r\nblack v24.8.0\r\n";
        let map = parse_uv_tool_list(output);
        assert_eq!(map.get("ruff"), Some(&"0.6.8".to_string()));
        assert_eq!(map.get("black"), Some(&"24.8.0".to_string()));
        assert_eq!(map.len(), 2);
    }

    /// Two lines for the same tool name: `HashMap::insert` overwrites the
    /// prior value, so the later line's version wins deliberately.
    #[test]
    fn parse_uv_tool_list_duplicate_name_last_wins() {
        let output = "ruff v0.6.8\nruff v0.6.9\n";
        let map = parse_uv_tool_list(output);
        assert_eq!(map.get("ruff"), Some(&"0.6.9".to_string()));
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn parse_uv_tool_list_rejects_trailing_content_and_non_digit_versions() {
        assert!(parse_uv_tool_list("ruff v0.6.8 (extra)\n").is_empty());
        assert!(parse_uv_tool_list("ruff vlatest\n").is_empty());
        assert!(parse_uv_tool_list("ruff v\n").is_empty());
    }

    #[test]
    fn parse_uv_tool_list_empty_input() {
        assert!(parse_uv_tool_list("").is_empty());
        assert!(parse_uv_tool_list("   \n\t\n").is_empty());
    }
}
