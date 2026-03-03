//! Per-field and per-collection size limits for wire protocol payloads.
//!
//! Post-deserialization validation prevents O(N) or O(N*M) processing attacks
//! within the 1 MB WebSocket frame limit. All limits are set above real-world
//! maximums with generous headroom to avoid breaking legitimate payloads.
//!
//! ## Design decision: post-deserialization validation
//!
//! We use a `WireValidate` trait (not custom serde deserializers) because:
//! - Custom deserializers are verbose and fragile for dozens of fields
//! - The 1 MB frame limit already caps total memory; the concern is processing cost
//! - Consistent with the existing `Validate` pattern in `web-api-types`
//! - Trivially backward-compatible (limits set far above real-world maximums)

use std::fmt;

/// Error returned when a wire payload field exceeds its size limit.
#[derive(Debug, Clone)]
pub struct WireValidationError {
    /// The field path that failed validation (e.g. `"hosts"`, `"results[0].error"`).
    pub field: &'static str,
    /// Human-readable description of the violation.
    pub message: String,
}

impl fmt::Display for WireValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "wire validation error: {}: {}", self.field, self.message)
    }
}

impl std::error::Error for WireValidationError {}

/// Trait for post-deserialization validation of wire protocol payloads.
///
/// Implementors check that all `Vec<T>` and `String` fields are within
/// bounds. Returns `Ok(())` when all fields pass, or the first violation
/// encountered.
pub trait WireValidate {
    /// Validate that all fields are within wire protocol size limits.
    fn wire_validate(&self) -> Result<(), WireValidationError>;
}

// ── Collection size limits ────────────────────────────────────────────────────

/// Maximum number of hosts in a `ReportHosts` message.
pub const MAX_REPORT_HOSTS: usize = 500;

/// Maximum number of version check assignments in a single message.
pub const MAX_VERSION_CHECK_ASSIGNMENTS: usize = 2_000;

/// Maximum number of version check results in a single message.
pub const MAX_VERSION_CHECK_RESULTS: usize = 2_000;

/// Maximum number of pre/post-update hooks in an update message.
pub const MAX_UPDATE_HOOKS: usize = 50;

/// Maximum number of packages in a batch update.
pub const MAX_BATCH_UPDATES: usize = 500;

/// Maximum number of results in a batch update result.
pub const MAX_BATCH_UPDATE_RESULTS: usize = 500;

/// Maximum number of discovery plugins in a single message.
pub const MAX_DISCOVERY_PLUGINS: usize = 50;

/// Maximum number of discovery plugin results in a single message.
pub const MAX_DISCOVERY_PLUGIN_RESULTS: usize = 50;

/// Maximum number of discoveries per plugin result.
pub const MAX_DISCOVERIES_PER_PLUGIN: usize = 1_000;

/// Maximum number of capabilities in a capability set.
pub const MAX_CAPABILITIES: usize = 50;

/// Maximum number of args in a `HookCommand::Exec` variant.
pub const MAX_HOOK_ARGS: usize = 100;

/// Maximum number of MQTT tenants in an assignment message.
pub const MAX_MQTT_TENANTS: usize = 500;

/// Maximum number of software state items.
pub const MAX_SOFTWARE_STATE_ITEMS: usize = 2_000;

/// Maximum number of hosts per software state item.
pub const MAX_SOFTWARE_STATE_HOSTS: usize = 500;

/// Maximum number of host package host states.
pub const MAX_HOST_PACKAGE_HOST_STATES: usize = 2_000;

/// Maximum number of active MQTT client IDs.
pub const MAX_ACTIVE_MQTT_CLIENTS: usize = 500;

// ── String length limits ──────────────────────────────────────────────────────

/// Maximum length for short strings (identifiers, names, versions).
pub const MAX_SHORT_STRING_LEN: usize = 1_024;

/// Maximum length for medium strings (hostnames, error messages).
pub const MAX_MEDIUM_STRING_LEN: usize = 4_096;

/// Maximum length for long strings (PEM certificates, CSRs, release notes).
pub const MAX_LONG_STRING_LEN: usize = 65_536;

/// Maximum length for output strings (command output, update output).
/// Matches the 1 MB frame limit — output is already bounded by `MAX_OUTPUT_BYTES`
/// in `agent-core/src/update.rs`.
pub const MAX_OUTPUT_STRING_LEN: usize = 1_048_576;

// ── Helper functions ──────────────────────────────────────────────────────────

/// Check that a `Vec` does not exceed the given length limit.
pub fn check_vec_len<T>(
    items: &[T],
    max: usize,
    field: &'static str,
) -> Result<(), WireValidationError> {
    if items.len() > max {
        return Err(WireValidationError {
            field,
            message: format!("collection has {} items, max {max}", items.len()),
        });
    }
    Ok(())
}

/// Check that a `String` does not exceed the given byte length limit.
pub fn check_string_len(
    s: &str,
    max: usize,
    field: &'static str,
) -> Result<(), WireValidationError> {
    if s.len() > max {
        return Err(WireValidationError {
            field,
            message: format!("string is {} bytes, max {max}", s.len()),
        });
    }
    Ok(())
}

/// Check that an `Option<String>` does not exceed the given byte length limit.
pub fn check_opt_string_len(
    s: &Option<String>,
    max: usize,
    field: &'static str,
) -> Result<(), WireValidationError> {
    if let Some(s) = s {
        check_string_len(s, max, field)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_vec_len_at_limit() {
        let items = vec![0u8; MAX_REPORT_HOSTS];
        assert!(check_vec_len(&items, MAX_REPORT_HOSTS, "test").is_ok());
    }

    #[test]
    fn check_vec_len_over_limit() {
        let items = vec![0u8; MAX_REPORT_HOSTS + 1];
        let err = check_vec_len(&items, MAX_REPORT_HOSTS, "test").unwrap_err();
        assert_eq!(err.field, "test");
        assert!(err.message.contains("501"));
    }

    #[test]
    fn check_vec_len_empty() {
        let items: Vec<u8> = vec![];
        assert!(check_vec_len(&items, MAX_REPORT_HOSTS, "test").is_ok());
    }

    #[test]
    fn check_string_len_at_limit() {
        let s = "a".repeat(MAX_SHORT_STRING_LEN);
        assert!(check_string_len(&s, MAX_SHORT_STRING_LEN, "test").is_ok());
    }

    #[test]
    fn check_string_len_over_limit() {
        let s = "a".repeat(MAX_SHORT_STRING_LEN + 1);
        let err = check_string_len(&s, MAX_SHORT_STRING_LEN, "test").unwrap_err();
        assert_eq!(err.field, "test");
        assert!(err.message.contains("1025"));
    }

    #[test]
    fn check_opt_string_len_none() {
        assert!(check_opt_string_len(&None, MAX_SHORT_STRING_LEN, "test").is_ok());
    }

    #[test]
    fn check_opt_string_len_some_over() {
        let s = Some("a".repeat(MAX_SHORT_STRING_LEN + 1));
        assert!(check_opt_string_len(&s, MAX_SHORT_STRING_LEN, "test").is_err());
    }

    #[test]
    fn wire_validation_error_display() {
        let err = WireValidationError {
            field: "hosts",
            message: "too many".to_string(),
        };
        let display = err.to_string();
        assert!(display.contains("hosts"));
        assert!(display.contains("too many"));
    }
}
