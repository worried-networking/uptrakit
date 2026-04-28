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
/// Maximum number of hosts in a `ReportHosts` message.
pub const MAX_REPORT_HOSTS: usize = 500;
/// Maximum number of version check assignments in a single message.
pub const MAX_VERSION_CHECK_ASSIGNMENTS: usize = 2_000;
/// Maximum number of version check results in a single message.
pub const MAX_VERSION_CHECK_RESULTS: usize = 2_000;
/// Maximum number of pre/post-update hook plugins in an update message.
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
/// Maximum byte length of a `DiscoveredSoftware.qualifier` string.
pub const MAX_DISCOVERED_QUALIFIER_LEN: usize = 256;
/// Maximum number of capabilities in a capability set.
pub const MAX_CAPABILITIES: usize = 50;
/// Maximum number of MQTT tenants in an assignment message.
pub const MAX_MQTT_TENANTS: usize = 500;
/// Maximum number of software state items.
pub const MAX_SOFTWARE_STATE_ITEMS: usize = 2_000;
/// Maximum number of hosts per software state item.
pub const MAX_SOFTWARE_STATE_HOSTS: usize = 500;
/// Maximum number of host package host states.
pub const MAX_HOST_PACKAGE_HOST_STATES: usize = 2_000;
/// Maximum number of host metadata entries in a `SoftwareStates` message.
pub const MAX_MQTT_HOSTS: usize = 2_000;
/// Maximum number of tags per host in a `HostStateMetadata` entry.
pub const MAX_HOST_TAGS: usize = 100;
/// Maximum number of connectivity updates in a `HostConnectivityUpdated` message.
pub const MAX_CONNECTIVITY_UPDATES: usize = 500;
/// Maximum number of active MQTT client IDs.
pub const MAX_ACTIVE_MQTT_CLIENTS: usize = 50_000;
/// Maximum number of capabilities in a single `Register` message.
///
/// Bounds the `BTreeSet<Capability>` sent by services, accommodating all known
/// variants plus a reasonable number of forward-compatibility `Other(String)`
/// entries.
pub const MAX_CAPABILITIES_PER_SERVICE: usize = 64;
/// Maximum number of surfaces in a single `SurfaceRegistration` message.
pub const MAX_SURFACE_MANIFESTS: usize = 50;
/// Maximum number of columns in a `TableColumns` placement or `DataTable` UI.
pub const MAX_SURFACE_COLUMNS: usize = 50;
/// Maximum number of action ID references in a single surface node.
pub const MAX_SURFACE_ACTION_REFS: usize = 50;
/// Maximum number of interaction descriptors in a surface registration.
pub const MAX_SURFACE_ACTIONS: usize = 200;
/// Maximum number of fields in a single form.
pub const MAX_SURFACE_FIELDS: usize = 100;
/// Maximum number of steps in a wizard.
pub const MAX_SURFACE_WIZARD_STEPS: usize = 20;
/// Maximum number of options in a select field.
pub const MAX_SURFACE_SELECT_OPTIONS: usize = 200;
/// Maximum byte length of surface action params JSON.
pub const MAX_SURFACE_PARAMS_LEN: usize = 65_536;
/// Maximum byte length of surface action response JSON.
pub const MAX_SURFACE_RESPONSE_LEN: usize = 1_048_576;
/// Maximum nesting depth for JSON values carried in surface payloads.
pub const MAX_SURFACE_JSON_DEPTH: usize = 32;
/// Maximum number of nodes visited when traversing surface JSON values.
pub const MAX_SURFACE_JSON_NODES: usize = 20_000;
/// Maximum byte length of plugin config JSON in a `ReportPluginConfig` message.
pub const MAX_PLUGIN_CONFIG_JSON_LEN: usize = 65_536;
/// Maximum byte length of stdin data in an `UpdateStdinData` message (64 KB).
///
/// Base64-encoded bytes written to the process PTY. 64 KB is generous for
/// interactive input; typical keystrokes are a few bytes each.
pub const MAX_STDIN_DATA_LEN: usize = 65_536;
/// Maximum byte length of a software item icon URL.
pub const MAX_ICON_URL_LEN: usize = 2_048;
/// Maximum number of pages in a single paginated report.
pub const MAX_REPORT_PAGES: u32 = 50;
/// Number of active hosts processed per page in paginated MQTT software-states delivery.
pub const STATES_HOST_PAGE_SIZE: u64 = 100;
/// Maximum number of concurrent pending (incomplete) paginated reports per
/// WebSocket connection. Prevents memory exhaustion from abandoned reports.
pub const MAX_PENDING_REPORTS_PER_CONNECTION: usize = 10;
/// Total timeout for a paginated report from first page to completion (5 min).
///
/// If all pages have not arrived within this window, the report is discarded
/// and a warning is logged. Already-processed pages committed their DB writes
/// independently; only the finalization step (e.g. notification) is lost.
pub const REPORT_TOTAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);
/// Idle timeout after the last page of a paginated report (15 s).
///
/// If no new page for the same `report_id` arrives within this window, the
/// report is considered abandoned. Catches mid-report connection stalls.
pub const REPORT_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
/// Serialized JSON size threshold (768 KB) above which a payload is split into
/// pages. Well under the 1 MB WebSocket frame limit to leave headroom for the
/// envelope overhead (protocol_version, seq, trace_context, pagination, type tag).
pub const PAGINATION_SIZE_THRESHOLD: usize = 786_432;
/// Maximum length of a trace ID (32 hex chars for 128-bit W3C trace ID).
pub const MAX_TRACE_ID_LEN: usize = 32;
/// Maximum length of a span ID (16 hex chars for 64-bit W3C span ID).
pub const MAX_SPAN_ID_LEN: usize = 16;
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
/// Maximum byte length of a config test output string.
pub const MAX_CONFIG_TEST_OUTPUT_LEN: usize = 65_536;
/// Maximum number of assets in a `ReleaseInfo` message.
pub const MAX_RELEASE_ASSETS: usize = 500;
/// Maximum number of entries in a `ServiceConfigDelivery` or `ServiceConfigUpdated` message.
pub const MAX_SERVICE_CONFIG_ENTRIES: usize = 1_000_000;
/// Maximum byte length of a service config value (serialized JSON).
pub const MAX_SERVICE_CONFIG_VALUE_LEN: usize = 65_536;
/// Maximum number of config keys in a single `WorkloadClaim` message.
pub const MAX_WORKLOAD_CLAIM_KEYS: usize = 100_000;
/// Expected byte length of a SHA-256 hex digest string (64 hex characters).
pub const SHA256_DIGEST_LEN: usize = 64;
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
/// Check that a `BTreeMap` does not exceed the given length limit.
pub fn check_map_len<K, V>(
    items: &std::collections::BTreeMap<K, V>,
    max: usize,
    field: &'static str,
) -> Result<(), WireValidationError> {
    if items.len() > max {
        return Err(WireValidationError {
            field,
            message: format!("map has {} entries, max {max}", items.len()),
        });
    }
    Ok(())
}
/// Check that a `BTreeSet` does not exceed the given length limit.
pub fn check_set_len<T>(
    items: &std::collections::BTreeSet<T>,
    max: usize,
    field: &'static str,
) -> Result<(), WireValidationError> {
    if items.len() > max {
        return Err(WireValidationError {
            field,
            message: format!("set has {} items, max {max}", items.len()),
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
