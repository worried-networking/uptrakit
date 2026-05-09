//! RouterOS output parsing helpers.
//!
//! RouterOS CLI commands return key-value output of the form:
//! ```text
//!                    version: 7.14 (stable)
//!              latest-version: 7.15
//! ```
//!
//! [`parse_ros_field`] extracts a named field value by scanning each line.
//! [`parse_resource_version`] wraps it for the `version` field from
//! `/system resource print`, stripping the `(channel)` suffix.
//! [`parse_latest_version`] wraps it for the `latest-version` field from
//! `/system package update print`.

/// Parse a key-value field from RouterOS CLI output.
///
/// RouterOS output lines look like `   key: value`.  The function searches
/// for the first line whose trimmed form starts with `<key>:`, then returns
/// the part after the colon, trimmed of surrounding whitespace.
///
/// Returns `None` when the field is not present or has no value after the colon.
pub(crate) fn parse_ros_field<'a>(output: &'a str, key: &str) -> Option<&'a str> {
    for line in output.lines() {
        let trimmed = line.trim();
        // Match `key: value` — key must be followed immediately by `:`.
        if let Some(rest) = trimmed.strip_prefix(key)
            && let Some(value) = rest.strip_prefix(':')
        {
            let v = value.trim();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

/// Extract the installed RouterOS version from `/system resource print` output.
///
/// RouterOS reports the version as `7.14 (stable)` — the channel suffix is
/// stripped so the result is a bare semver-like string (`7.14`).
///
/// Returns `None` when the `version` field is absent from the output.
pub(crate) fn parse_resource_version(output: &str) -> Option<String> {
    parse_resource_version_with_display(output).map(|(stripped, _)| stripped)
}

/// Extract both the bare and channel-suffixed forms of the installed
/// RouterOS version from `/system resource print` output.
///
/// Returns `(stripped, display)` where `stripped` is the bare semver-like
/// version (e.g. `"7.14.2"`) and `display` is the raw value as RouterOS
/// reported it including the channel suffix (e.g. `"7.14.2 (stable)"`).
/// Returns `None` when the `version` field is absent or blank.
///
/// The `Discoverer` impl uses this to populate
/// `DiscoveredSoftware.installed_display_version` so the channel info
/// stays visible in the dashboard despite `installed_version` being a bare
/// semver string for downstream comparison.
pub(crate) fn parse_resource_version_with_display(output: &str) -> Option<(String, String)> {
    let raw = parse_ros_field(output, "version")?;
    // Strip optional ` (channel)` suffix by splitting before the first `(`.
    // `split_once` is safe for char boundaries on ASCII `(`.
    let stripped = raw.split_once('(').map_or(raw, |(before, _)| before).trim();
    if stripped.is_empty() {
        None
    } else {
        Some((stripped.to_owned(), raw.to_owned()))
    }
}

/// Extract the latest available version from `/system package update print` output.
///
/// RouterOS reports the latest version as the `latest-version` field value.
/// Returns `None` when the field is absent or blank.
pub(crate) fn parse_latest_version(output: &str) -> Option<String> {
    parse_ros_field(output, "latest-version").map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_ros_field ───────────────────────────────────────────────────────

    #[test]
    fn parse_ros_field_finds_key() {
        let output = "                   version: 7.14 (stable)\n";
        assert_eq!(parse_ros_field(output, "version"), Some("7.14 (stable)"));
    }

    #[test]
    fn parse_ros_field_multi_line() {
        let output =
            "     uptime: 3d2h\n                   version: 7.14 (stable)\n  cpu-load: 5\n";
        assert_eq!(parse_ros_field(output, "version"), Some("7.14 (stable)"));
    }

    #[test]
    fn parse_ros_field_missing_key_returns_none() {
        let output = "     uptime: 3d2h\n  cpu-load: 5\n";
        assert_eq!(parse_ros_field(output, "version"), None);
    }

    #[test]
    fn parse_ros_field_empty_value_returns_none() {
        let output = "version: \n";
        assert_eq!(parse_ros_field(output, "version"), None);
    }

    #[test]
    fn parse_ros_field_does_not_match_partial_key() {
        // "versions:" must not match the key "version"
        let output = "versions: many\n";
        // "versions" does not start with "version:" — prefix match requires
        // the colon to follow immediately. strip_prefix("version") leaves "s: many",
        // then strip_prefix(':') fails, so this correctly returns None.
        assert_eq!(parse_ros_field(output, "version"), None);
    }

    #[test]
    fn parse_ros_field_latest_version() {
        let output = "  channel: stable\n  installed-version: 7.14\n  latest-version: 7.15\n";
        assert_eq!(parse_ros_field(output, "latest-version"), Some("7.15"));
    }

    // ── parse_resource_version ────────────────────────────────────────────────

    #[test]
    fn parse_resource_version_strips_channel_suffix() {
        let output = "                   version: 7.14 (stable)\n";
        assert_eq!(parse_resource_version(output), Some("7.14".to_string()));
    }

    #[test]
    fn parse_resource_version_no_channel_suffix() {
        let output = "                   version: 7.14\n";
        assert_eq!(parse_resource_version(output), Some("7.14".to_string()));
    }

    #[test]
    fn parse_resource_version_long_term_channel() {
        let output = "                   version: 7.14.3 (long-term)\n";
        assert_eq!(parse_resource_version(output), Some("7.14.3".to_string()));
    }

    #[test]
    fn parse_resource_version_missing_returns_none() {
        let output = "     uptime: 3d\n";
        assert_eq!(parse_resource_version(output), None);
    }

    #[test]
    fn parse_resource_version_full_resource_print() {
        // Simulate real RouterOS `/system resource print` output
        let output = "\
                   uptime: 3d2h15m\n\
                  version: 7.14.2 (stable)\n\
             build-time: 2024-03-01 12:00:00\n\
         free-memory: 128.0MiB\n\
        total-memory: 256.0MiB\n\
                      cpu: ARM\n\
               cpu-count: 4\n\
           cpu-frequency: 650MHz\n\
                cpu-load: 2\n\
          free-hdd-space: 50.0MiB\n\
         total-hdd-space: 128.0MiB\n";
        assert_eq!(parse_resource_version(output), Some("7.14.2".to_string()));
    }

    // ── parse_latest_version ──────────────────────────────────────────────────

    #[test]
    fn parse_latest_version_found() {
        let output = "  channel: stable\n  installed-version: 7.14\n  latest-version: 7.15\n";
        assert_eq!(parse_latest_version(output), Some("7.15".to_string()));
    }

    #[test]
    fn parse_latest_version_missing_returns_none() {
        let output = "  channel: stable\n  installed-version: 7.14\n";
        assert_eq!(parse_latest_version(output), None);
    }

    #[test]
    fn parse_latest_version_full_package_update_print() {
        // Simulate real RouterOS `/system package update print` output
        let output = "\
              channel: stable\n\
  installed-version: 7.14.2\n\
       status: New version is available\n\
   latest-version: 7.15\n";
        assert_eq!(parse_latest_version(output), Some("7.15".to_string()));
    }
}
