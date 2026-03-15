//! Attribute payload builders and state string helpers for HA MQTT discovery.
//!
//! All functions are stateless and produce deterministic JSON or string results.

/// Build the JSON attributes payload for a `(software_item, host)` entity.
///
/// Returns a [`serde_json::Value`] that should be serialized and published
/// (retained) to [`super::json_attributes_topic`].
///
/// The `in_progress` attribute is recognized by Home Assistant's `update`
/// entity:
/// - `false` -- no update running (idle).
/// - `true` -- an update is pending dispatch or executing on the agent
///   (displays a spinner in the HA UI).
///
/// Optional fields are omitted from the JSON when `None`:
/// - `update_category` -- classification of the pending update
///   (`"security"`, `"bugfix"`, `"feature"`, or `"unknown"`).
/// - `release_date` -- ISO 8601 date string of the latest release (`"2025-01-15"`).
/// - `last_checked_at` -- ISO 8601 datetime when the version was last detected.
///
/// # Examples
///
/// ```
/// # use uptrakit_mqtt::ha_discovery::build_attributes_payload;
/// let payload = build_attributes_payload(true, None, None, None);
/// assert_eq!(payload["in_progress"], true);
/// assert!(payload.get("update_category").is_none());
///
/// let payload = build_attributes_payload(false, Some("security"), Some("2025-01-15"), None);
/// assert_eq!(payload["in_progress"], false);
/// assert_eq!(payload["update_category"], "security");
/// assert_eq!(payload["release_date"], "2025-01-15");
/// assert!(payload.get("last_checked_at").is_none());
/// ```
pub(crate) fn build_attributes_payload(
    in_progress: bool,
    update_category: Option<&str>,
    release_date: Option<&str>,
    last_checked_at: Option<&str>,
) -> serde_json::Value {
    let mut v = serde_json::json!({ "in_progress": in_progress });
    if let Some(cat) = update_category {
        v["update_category"] = serde_json::Value::String(cat.to_string());
    }
    if let Some(rd) = release_date {
        v["release_date"] = serde_json::Value::String(rd.to_string());
    }
    if let Some(lca) = last_checked_at {
        v["last_checked_at"] = serde_json::Value::String(lca.to_string());
    }
    v
}

/// Build the JSON attributes payload for a host's package entity.
///
/// Returns a [`serde_json::Value`] to be serialized and published (retained)
/// to [`super::host_packages_json_attributes_topic`].
///
/// Fields:
/// - `in_progress` -- whether a batch update is pending/running.
/// - `pending_count` -- number of packages with available updates.
/// - `total_count` -- total number of tracked packages.
/// - `bugfix_count` -- pending packages classified as `"bugfix"`.
/// - `feature_count` -- pending packages classified as `"feature"`.
///
/// # Examples
///
/// ```
/// # use uptrakit_mqtt::ha_discovery::build_host_packages_attributes_payload;
/// let payload = build_host_packages_attributes_payload(true, 3, 10, 1, 2);
/// assert_eq!(payload["in_progress"], true);
/// assert_eq!(payload["pending_count"], 3u32);
/// assert_eq!(payload["total_count"], 10u32);
/// assert_eq!(payload["bugfix_count"], 1u32);
/// assert_eq!(payload["feature_count"], 2u32);
/// ```
pub(crate) fn build_host_packages_attributes_payload(
    in_progress: bool,
    pending_count: u32,
    total_count: u32,
    bugfix_count: u32,
    feature_count: u32,
) -> serde_json::Value {
    serde_json::json!({
        "in_progress": in_progress,
        "pending_count": pending_count,
        "total_count": total_count,
        "bugfix_count": bugfix_count,
        "feature_count": feature_count,
    })
}

/// Build the JSON attributes payload for a host's security updates entity.
///
/// Returns a [`serde_json::Value`] to be serialized and published (retained)
/// to [`super::host_security_json_attributes_topic`].
///
/// # Examples
///
/// ```
/// # use uptrakit_mqtt::ha_discovery::build_host_security_attributes_payload;
/// let payload = build_host_security_attributes_payload(false, 2);
/// assert_eq!(payload["in_progress"], false);
/// assert_eq!(payload["pending_count"], 2u32);
/// ```
pub(crate) fn build_host_security_attributes_payload(
    in_progress: bool,
    security_pending_count: u32,
) -> serde_json::Value {
    serde_json::json!({
        "in_progress": in_progress,
        "pending_count": security_pending_count,
    })
}

/// Build the JSON attributes payload for a host's connectivity entity.
///
/// Returns a [`serde_json::Value`] to be serialized and published (retained)
/// to [`super::host_connectivity_attributes_topic`].
///
/// Both fields are optional:
/// - `last_seen` -- ISO 8601 datetime string of the agent's last contact.
/// - `version` -- agent version string.
///
/// # Examples
///
/// ```
/// # use uptrakit_mqtt::ha_discovery::build_host_connectivity_attributes_payload;
/// let payload = build_host_connectivity_attributes_payload(
///     Some("2025-01-15T12:00:00Z"),
///     Some("1.2.3"),
/// );
/// assert_eq!(payload["last_seen"], "2025-01-15T12:00:00Z");
/// assert_eq!(payload["version"], "1.2.3");
///
/// let payload = build_host_connectivity_attributes_payload(None, None);
/// assert!(payload["last_seen"].is_null());
/// assert!(payload["version"].is_null());
/// ```
pub(crate) fn build_host_connectivity_attributes_payload(
    last_seen: Option<&str>,
    version: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "last_seen": last_seen,
        "version": version,
    })
}

/// Build the JSON OS info payload for a host.
///
/// Returns a [`serde_json::Value`] to be serialized and published (retained)
/// to [`super::host_info_topic`].
///
/// All fields are optional and represented as JSON `null` when absent.
///
/// # Examples
///
/// ```
/// # use uptrakit_mqtt::ha_discovery::build_host_info_payload;
/// let payload = build_host_info_payload(
///     Some("Linux"),
///     Some("Ubuntu 24.04"),
///     Some("x86_64"),
/// );
/// assert_eq!(payload["os_type"], "Linux");
/// assert_eq!(payload["os_version"], "Ubuntu 24.04");
/// assert_eq!(payload["architecture"], "x86_64");
///
/// let payload = build_host_info_payload(None, None, None);
/// assert!(payload["os_type"].is_null());
/// ```
pub(crate) fn build_host_info_payload(
    os_type: Option<&str>,
    os_version: Option<&str>,
    architecture: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "os_type": os_type,
        "os_version": os_version,
        "architecture": architecture,
    })
}

/// Build the JSON agent info payload for a host.
///
/// Returns a [`serde_json::Value`] to be serialized and published (retained)
/// to [`super::host_agent_topic`].
///
/// Both fields are optional and represented as JSON `null` when absent.
///
/// # Examples
///
/// ```
/// # use uptrakit_mqtt::ha_discovery::build_host_agent_payload;
/// let payload = build_host_agent_payload(Some("2025-01-15T12:00:00Z"), Some("1.2.3"));
/// assert_eq!(payload["last_seen"], "2025-01-15T12:00:00Z");
/// assert_eq!(payload["version"], "1.2.3");
///
/// let payload = build_host_agent_payload(None, None);
/// assert!(payload["last_seen"].is_null());
/// assert!(payload["version"].is_null());
/// ```
pub(crate) fn build_host_agent_payload(
    last_seen: Option<&str>,
    version: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "last_seen": last_seen,
        "version": version,
    })
}

// =============================================================================
// State string helpers
// =============================================================================

/// Returns the state string published on the host packages state topic.
///
/// When `pending_count > 0`, returns `"unknown"` -- there is no single
/// "installed version" for an aggregate host entity. When `pending_count == 0`,
/// returns `"up-to-date"`.
///
/// # Examples
///
/// ```
/// # use uptrakit_mqtt::ha_discovery::host_packages_state_string;
/// assert_eq!(host_packages_state_string(0), "up-to-date");
/// assert_eq!(host_packages_state_string(3), "unknown");
/// ```
pub(crate) fn host_packages_state_string(pending_count: u32) -> String {
    if pending_count > 0 {
        "unknown".to_string()
    } else {
        "up-to-date".to_string()
    }
}

/// Returns the latest-version string published on the host packages
/// `latest_version` topic.
///
/// When `pending_count > 0`, returns `"{N} available"` so Home Assistant shows
/// an update badge with the count. When `pending_count == 0`, returns
/// `"up-to-date"` (matching state, so HA hides the badge).
///
/// # Examples
///
/// ```
/// # use uptrakit_mqtt::ha_discovery::host_packages_latest_version_string;
/// assert_eq!(host_packages_latest_version_string(0), "up-to-date");
/// assert_eq!(host_packages_latest_version_string(3), "3 available");
/// ```
pub(crate) fn host_packages_latest_version_string(pending_count: u32) -> String {
    if pending_count > 0 {
        format!("{pending_count} available")
    } else {
        "up-to-date".to_string()
    }
}

/// Returns the state string published on the host security state topic.
///
/// When `security_pending_count > 0`, returns `"unknown"`. When 0, returns
/// `"up-to-date"`.
///
/// # Examples
///
/// ```
/// # use uptrakit_mqtt::ha_discovery::host_security_state_string;
/// assert_eq!(host_security_state_string(0), "up-to-date");
/// assert_eq!(host_security_state_string(2), "unknown");
/// ```
pub(crate) fn host_security_state_string(security_pending_count: u32) -> String {
    if security_pending_count > 0 {
        "unknown".to_string()
    } else {
        "up-to-date".to_string()
    }
}

/// Returns the latest-version string published on the host security
/// `latest_version` topic.
///
/// When `security_pending_count > 0`, returns `"{N} available"`. When 0,
/// returns `"up-to-date"`.
///
/// # Examples
///
/// ```
/// # use uptrakit_mqtt::ha_discovery::host_security_latest_version_string;
/// assert_eq!(host_security_latest_version_string(0), "up-to-date");
/// assert_eq!(host_security_latest_version_string(2), "2 available");
/// ```
pub(crate) fn host_security_latest_version_string(security_pending_count: u32) -> String {
    if security_pending_count > 0 {
        format!("{security_pending_count} available")
    } else {
        "up-to-date".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // build_attributes_payload
    // -------------------------------------------------------------------------

    #[test]
    fn build_attributes_payload_in_progress_true() {
        let payload = build_attributes_payload(true, None, None, None);
        assert_eq!(payload["in_progress"], true);
    }

    #[test]
    fn build_attributes_payload_in_progress_false() {
        let payload = build_attributes_payload(false, None, None, None);
        assert_eq!(payload["in_progress"], false);
    }

    #[test]
    fn build_attributes_payload_is_valid_json() {
        let payload = build_attributes_payload(true, None, None, None);
        let s = payload.to_string();
        let reparsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(reparsed["in_progress"], true);
    }

    #[test]
    fn build_attributes_payload_with_all_optional_fields() {
        let payload = build_attributes_payload(
            false,
            Some("security"),
            Some("2025-01-15"),
            Some("2025-01-15T12:00:00Z"),
        );
        assert_eq!(payload["in_progress"], false);
        assert_eq!(payload["update_category"], "security");
        assert_eq!(payload["release_date"], "2025-01-15");
        assert_eq!(payload["last_checked_at"], "2025-01-15T12:00:00Z");
    }

    #[test]
    fn build_attributes_payload_omits_none_optional_fields() {
        let payload = build_attributes_payload(true, None, Some("2025-01-15"), None);
        assert!(payload.get("update_category").is_none());
        assert_eq!(payload["release_date"], "2025-01-15");
        assert!(payload.get("last_checked_at").is_none());
    }

    // -------------------------------------------------------------------------
    // build_host_packages_attributes_payload
    // -------------------------------------------------------------------------

    #[test]
    fn build_host_packages_attributes_payload_in_progress() {
        let payload = build_host_packages_attributes_payload(true, 5, 20, 3, 2);
        assert_eq!(payload["in_progress"], true);
        assert_eq!(payload["pending_count"], 5u32);
        assert_eq!(payload["total_count"], 20u32);
        assert_eq!(payload["bugfix_count"], 3u32);
        assert_eq!(payload["feature_count"], 2u32);
    }

    #[test]
    fn build_host_packages_attributes_payload_idle() {
        let payload = build_host_packages_attributes_payload(false, 0, 10, 0, 0);
        assert_eq!(payload["in_progress"], false);
        assert_eq!(payload["pending_count"], 0u32);
        assert_eq!(payload["total_count"], 10u32);
        assert_eq!(payload["bugfix_count"], 0u32);
        assert_eq!(payload["feature_count"], 0u32);
    }

    // -------------------------------------------------------------------------
    // build_host_security_attributes_payload
    // -------------------------------------------------------------------------

    #[test]
    fn build_host_security_attributes_payload_with_pending() {
        let payload = build_host_security_attributes_payload(false, 2);
        assert_eq!(payload["in_progress"], false);
        assert_eq!(payload["pending_count"], 2u32);
    }

    #[test]
    fn build_host_security_attributes_payload_in_progress() {
        let payload = build_host_security_attributes_payload(true, 1);
        assert_eq!(payload["in_progress"], true);
        assert_eq!(payload["pending_count"], 1u32);
    }

    // -------------------------------------------------------------------------
    // build_host_connectivity_attributes_payload
    // -------------------------------------------------------------------------

    #[test]
    fn build_host_connectivity_attributes_payload_with_values() {
        let p =
            build_host_connectivity_attributes_payload(Some("2025-01-15T12:00:00Z"), Some("1.2.3"));
        assert_eq!(p["last_seen"], "2025-01-15T12:00:00Z");
        assert_eq!(p["version"], "1.2.3");
    }

    #[test]
    fn build_host_connectivity_attributes_payload_nulls() {
        let p = build_host_connectivity_attributes_payload(None, None);
        assert!(p["last_seen"].is_null());
        assert!(p["version"].is_null());
    }

    // -------------------------------------------------------------------------
    // build_host_info_payload
    // -------------------------------------------------------------------------

    #[test]
    fn build_host_info_payload_all_fields() {
        let p = build_host_info_payload(Some("Linux"), Some("Ubuntu 24.04"), Some("x86_64"));
        assert_eq!(p["os_type"], "Linux");
        assert_eq!(p["os_version"], "Ubuntu 24.04");
        assert_eq!(p["architecture"], "x86_64");
    }

    #[test]
    fn build_host_info_payload_null_fields() {
        let p = build_host_info_payload(None, None, None);
        assert!(p["os_type"].is_null());
        assert!(p["os_version"].is_null());
        assert!(p["architecture"].is_null());
    }

    // -------------------------------------------------------------------------
    // build_host_agent_payload
    // -------------------------------------------------------------------------

    #[test]
    fn build_host_agent_payload_all_fields() {
        let p = build_host_agent_payload(Some("2025-01-15T12:00:00Z"), Some("1.2.3"));
        assert_eq!(p["last_seen"], "2025-01-15T12:00:00Z");
        assert_eq!(p["version"], "1.2.3");
    }

    #[test]
    fn build_host_agent_payload_null_fields() {
        let p = build_host_agent_payload(None, None);
        assert!(p["last_seen"].is_null());
        assert!(p["version"].is_null());
    }

    // -------------------------------------------------------------------------
    // host_packages_state_string
    // -------------------------------------------------------------------------

    #[test]
    fn host_packages_state_string_zero_pending() {
        assert_eq!(host_packages_state_string(0), "up-to-date");
    }

    #[test]
    fn host_packages_state_string_one_pending() {
        assert_eq!(host_packages_state_string(1), "unknown");
    }

    #[test]
    fn host_packages_state_string_many_pending() {
        assert_eq!(host_packages_state_string(3), "unknown");
    }

    // -------------------------------------------------------------------------
    // host_packages_latest_version_string
    // -------------------------------------------------------------------------

    #[test]
    fn host_packages_latest_version_string_zero_pending() {
        assert_eq!(host_packages_latest_version_string(0), "up-to-date");
    }

    #[test]
    fn host_packages_latest_version_string_one_pending() {
        assert_eq!(host_packages_latest_version_string(1), "1 available");
    }

    #[test]
    fn host_packages_latest_version_string_many_pending() {
        assert_eq!(host_packages_latest_version_string(3), "3 available");
    }

    // -------------------------------------------------------------------------
    // host_security_state_string
    // -------------------------------------------------------------------------

    #[test]
    fn host_security_state_string_zero_pending() {
        assert_eq!(host_security_state_string(0), "up-to-date");
    }

    #[test]
    fn host_security_state_string_one_pending() {
        assert_eq!(host_security_state_string(1), "unknown");
    }

    #[test]
    fn host_security_state_string_many_pending() {
        assert_eq!(host_security_state_string(3), "unknown");
    }

    // -------------------------------------------------------------------------
    // host_security_latest_version_string
    // -------------------------------------------------------------------------

    #[test]
    fn host_security_latest_version_string_zero_pending() {
        assert_eq!(host_security_latest_version_string(0), "up-to-date");
    }

    #[test]
    fn host_security_latest_version_string_one_pending() {
        assert_eq!(host_security_latest_version_string(1), "1 available");
    }

    #[test]
    fn host_security_latest_version_string_many_pending() {
        assert_eq!(host_security_latest_version_string(3), "3 available");
    }
}
