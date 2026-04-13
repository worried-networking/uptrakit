//! Pure helper functions for Home Assistant MQTT discovery.
//!
//! All functions are stateless and produce deterministic results from their
//! inputs. No async, no I/O.

mod attributes;
mod device;
mod parsers;
mod topics;

use uuid::Uuid;

// Re-export everything so that `crate::ha_discovery::*` paths used by
// tenant_manager.rs continue to resolve without changes.
pub(crate) use attributes::{
    build_attributes_payload, build_host_agent_payload, build_host_connectivity_attributes_payload,
    build_host_info_payload, build_host_packages_attributes_payload,
    build_host_security_attributes_payload, host_packages_latest_version_string,
    host_packages_state_string, host_security_latest_version_string, host_security_state_string,
};
pub(crate) use device::{
    build_discovery_config, build_host_connectivity_discovery_config,
    build_host_packages_discovery_config, build_host_security_discovery_config,
};
pub(crate) use parsers::{
    parse_command_topic, parse_host_packages_command_topic, parse_host_security_command_topic,
};
pub(crate) use topics::{
    command_topic, discovery_config_topic, friendly_name_topic, host_agent_topic,
    host_connectivity_attributes_topic, host_connectivity_discovery_config_topic,
    host_connectivity_state_topic, host_info_topic, host_packages_command_topic,
    host_packages_discovery_config_topic, host_packages_json_attributes_topic,
    host_packages_latest_version_topic, host_packages_state_topic, host_packages_unique_id,
    host_security_command_topic, host_security_discovery_config_topic,
    host_security_json_attributes_topic, host_security_latest_version_topic,
    host_security_state_topic, host_tags_topic, hostname_topic, json_attributes_topic,
    latest_version_topic, state_topic,
};

/// OS information for enriching HA device blocks.
///
/// All fields are optional. When `None`, the corresponding device block field
/// is omitted so that Home Assistant merges the info from whichever entity
/// provides it first.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct HostOsInfo<'a> {
    /// OS family / type string (maps to `model` in the HA device block).
    pub os_type: Option<&'a str>,
    /// OS version string (maps to `sw_version` in the HA device block).
    pub os_version: Option<&'a str>,
    /// CPU architecture string (maps to `hw_version` in the HA device block).
    pub architecture: Option<&'a str>,
}

/// Optional upstream release metadata included in a discovery config.
///
/// Passed to [`build_discovery_config`] to include release page links and
/// changelog snippets in the HA MQTT discovery payload.
#[derive(Debug, Default, Clone)]
pub(crate) struct ReleaseInfo<'a> {
    /// URL to the upstream release page (e.g. a GitHub release).
    pub url: Option<&'a str>,
    /// Full release notes or changelog text.
    ///
    /// Truncated to 500 Unicode characters when written to the discovery
    /// config (`release_summary`).
    pub notes: Option<&'a str>,
    /// Optional HTTPS URL to an icon/logo image.
    pub icon_url: Option<&'a str>,
}

/// Returns a unique ID string for this `(tenant, software_item, host)` triple.
///
/// Format: `uptrakit_{tenant_id_no_dashes}_{host_id_no_dashes}_{item_id_no_dashes}`
///
/// The host comes before the item so that all entities for a single host share
/// the same UUID prefix, aligning with the host-centric device model.
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::unique_id;
/// let tenant_id = Uuid::nil();
/// let item_id   = Uuid::nil();
/// let host_id   = Uuid::nil();
/// let uid = unique_id(tenant_id, item_id, host_id);
/// assert!(uid.starts_with("uptrakit_"));
/// assert!(!uid.contains('-'));
/// ```
pub(crate) fn unique_id(tenant_id: Uuid, item_id: Uuid, host_id: Uuid) -> String {
    let t = tenant_id.simple();
    let h = host_id.simple();
    let i = item_id.simple();
    format!("uptrakit_{t}_{h}_{i}")
}

/// Truncate `s` to at most `max_chars` Unicode scalar values at a character boundary.
fn truncate_str(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => &s[..byte_idx],
        None => s,
    }
}

/// Convert a string to a slug suitable for use in HA entity IDs.
///
/// Rules:
/// - ASCII alphanumeric characters are lowercased and kept as-is.
/// - All other characters (spaces, dots, hyphens, etc.) are replaced with `_`.
/// - Consecutive underscores are collapsed into one.
/// - Leading and trailing underscores are stripped.
///
/// # Examples
///
/// ```
/// # use uptrakit_mqtt_runtime::ha_discovery::slugify;
/// assert_eq!(slugify("My App"), "my_app");
/// assert_eq!(slugify("pangolin.uk.home.yantsen.su"), "pangolin_uk_home_yantsen_su");
/// assert_eq!(slugify("foo--bar"), "foo_bar");
/// assert_eq!(slugify("  leading"), "leading");
/// ```
pub(crate) fn slugify(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut prev_underscore = false;

    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            result.push(c.to_ascii_lowercase());
            prev_underscore = false;
        } else if !result.is_empty() && !prev_underscore {
            result.push('_');
            prev_underscore = true;
        }
    }

    // Strip trailing underscore (produced when the input ends with a
    // non-alphanumeric character).
    if result.ends_with('_') {
        result.pop();
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tenant() -> Uuid {
        Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap()
    }
    fn item() -> Uuid {
        Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap()
    }
    fn host() -> Uuid {
        Uuid::parse_str("33333333-3333-3333-3333-333333333333").unwrap()
    }

    // -------------------------------------------------------------------------
    // unique_id
    // -------------------------------------------------------------------------

    #[test]
    fn unique_id_starts_with_uptrakit() {
        assert!(unique_id(tenant(), item(), host()).starts_with("uptrakit_"));
    }

    #[test]
    fn unique_id_no_dashes() {
        let uid = unique_id(tenant(), item(), host());
        assert!(!uid.contains('-'));
    }

    #[test]
    fn unique_id_deterministic() {
        assert_eq!(
            unique_id(tenant(), item(), host()),
            unique_id(tenant(), item(), host())
        );
    }

    #[test]
    fn unique_id_different_for_different_inputs() {
        let uid1 = unique_id(tenant(), item(), host());
        let uid2 = unique_id(tenant(), host(), item()); // item/host swapped
        assert_ne!(uid1, uid2);
    }

    #[test]
    fn unique_id_format_exact() {
        let zero = Uuid::nil();
        let uid = unique_id(zero, zero, zero);
        assert_eq!(
            uid,
            "uptrakit_00000000000000000000000000000000_00000000000000000000000000000000_00000000000000000000000000000000"
        );
    }

    #[test]
    fn unique_id_host_before_item() {
        let uid = unique_id(tenant(), item(), host());
        let host_simple = host().simple().to_string();
        let item_simple = item().simple().to_string();
        let host_pos = uid.find(&host_simple).unwrap();
        let item_pos = uid.find(&item_simple).unwrap();
        assert!(host_pos < item_pos);
    }

    // -------------------------------------------------------------------------
    // slugify
    // -------------------------------------------------------------------------

    #[test]
    fn slugify_plain_lowercase() {
        assert_eq!(slugify("nginx"), "nginx");
    }

    #[test]
    fn slugify_uppercase_lowercased() {
        assert_eq!(slugify("MyApp"), "myapp");
    }

    #[test]
    fn slugify_spaces_become_underscores() {
        assert_eq!(slugify("My App"), "my_app");
    }

    #[test]
    fn slugify_dots_become_underscores() {
        assert_eq!(
            slugify("pangolin.uk.home.yantsen.su"),
            "pangolin_uk_home_yantsen_su"
        );
    }

    #[test]
    fn slugify_hyphens_become_underscores() {
        assert_eq!(slugify("my-service"), "my_service");
    }

    #[test]
    fn slugify_consecutive_separators_collapsed() {
        assert_eq!(slugify("foo--bar"), "foo_bar");
        assert_eq!(slugify("foo  bar"), "foo_bar");
        assert_eq!(slugify("foo.-bar"), "foo_bar");
    }

    #[test]
    fn slugify_leading_separators_stripped() {
        assert_eq!(slugify("  leading"), "leading");
        assert_eq!(slugify(".leading"), "leading");
    }

    #[test]
    fn slugify_trailing_separators_stripped() {
        assert_eq!(slugify("trailing "), "trailing");
        assert_eq!(slugify("trailing."), "trailing");
    }

    #[test]
    fn slugify_digits_kept() {
        assert_eq!(slugify("app2"), "app2");
        assert_eq!(slugify("v1.2.3"), "v1_2_3");
    }

    #[test]
    fn slugify_empty_string() {
        assert_eq!(slugify(""), "");
    }

    #[test]
    fn slugify_all_separators() {
        assert_eq!(slugify("..."), "");
    }

    #[test]
    fn slugify_real_world_example() {
        assert_eq!(slugify("uptrakit pangolin"), "uptrakit_pangolin");
        assert_eq!(
            slugify("pangolin.uk.home.yantsen.su"),
            "pangolin_uk_home_yantsen_su"
        );
    }
}
