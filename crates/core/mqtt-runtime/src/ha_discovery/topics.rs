//! MQTT topic generation functions for Home Assistant discovery.
//!
//! All functions are pure and deterministic: given the same inputs they always
//! produce the same topic string.

use uuid::Uuid;

/// Returns the HA MQTT discovery config topic for an `update` entity.
///
/// Format: `{ha_prefix}/update/uptrakit/{object_id}/config`
///
/// where `{object_id}` is `unique_id` with any leading `"uptrakit_"` prefix
/// stripped.  The static `uptrakit` node_id level groups all Uptrakit discovery
/// configs under a single MQTT namespace, allowing HA or a broker to subscribe
/// to `{ha_prefix}/update/uptrakit/#` to receive all Uptrakit entities at once.
/// Since the namespace already carries the `uptrakit` identity, the redundant
/// prefix is dropped from the object_id to keep topics concise.
///
/// # Examples
///
/// ```
/// # use uptrakit_mqtt_runtime::ha_discovery::discovery_config_topic;
/// let topic = discovery_config_topic("homeassistant", "uptrakit_abc_def_ghi");
/// assert_eq!(topic, "homeassistant/update/uptrakit/abc_def_ghi/config");
/// ```
pub(crate) fn discovery_config_topic(ha_prefix: &str, unique_id: &str) -> String {
    let object_id = unique_id.strip_prefix("uptrakit_").unwrap_or(unique_id);
    format!("{ha_prefix}/update/uptrakit/{object_id}/config")
}

/// Returns the per-host topic prefix.
///
/// Format: `{topic_prefix}/hosts/{host_id}`
///
/// All host-scoped topics (software items, packages, security, hostname,
/// friendly_name) are nested under this prefix.
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::host_topic_prefix;
/// let prefix = host_topic_prefix("uptrakit", Uuid::nil());
/// assert!(prefix.starts_with("uptrakit/hosts/"));
/// ```
pub(crate) fn host_topic_prefix(topic_prefix: &str, host_id: Uuid) -> String {
    format!("{topic_prefix}/hosts/{host_id}")
}

/// Returns the topic carrying the installed version of a `(software_item, host)` pair.
///
/// Format: `{topic_prefix}/hosts/{host_id}/items/{item_id}/state`
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::state_topic;
/// let item_id = Uuid::nil();
/// let host_id = Uuid::nil();
/// let topic = state_topic("uptrakit", item_id, host_id);
/// assert!(topic.ends_with("/state"));
/// ```
pub(crate) fn state_topic(topic_prefix: &str, item_id: Uuid, host_id: Uuid) -> String {
    let hp = host_topic_prefix(topic_prefix, host_id);
    format!("{hp}/items/{item_id}/state")
}

/// Returns the topic carrying the latest available version.
///
/// Format: `{topic_prefix}/hosts/{host_id}/items/{item_id}/latest_version`
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::latest_version_topic;
/// let item_id = Uuid::nil();
/// let host_id = Uuid::nil();
/// let topic = latest_version_topic("uptrakit", item_id, host_id);
/// assert!(topic.ends_with("/latest_version"));
/// ```
pub(crate) fn latest_version_topic(topic_prefix: &str, item_id: Uuid, host_id: Uuid) -> String {
    let hp = host_topic_prefix(topic_prefix, host_id);
    format!("{hp}/items/{item_id}/latest_version")
}

/// Returns the MQTT command topic that HA publishes `"install"` to.
///
/// Format: `{topic_prefix}/hosts/{host_id}/items/{item_id}/set`
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::command_topic;
/// let item_id = Uuid::nil();
/// let host_id = Uuid::nil();
/// let topic = command_topic("uptrakit", item_id, host_id);
/// assert!(topic.ends_with("/set"));
/// ```
pub(crate) fn command_topic(topic_prefix: &str, item_id: Uuid, host_id: Uuid) -> String {
    let hp = host_topic_prefix(topic_prefix, host_id);
    format!("{hp}/items/{item_id}/set")
}

/// Returns the MQTT topic for HA JSON attributes of a `(software_item, host)` pair.
///
/// Published as a retained JSON payload. The recognized attribute is
/// `"in_progress"` (bool): `true` while an update is pending or running,
/// `false` when idle.
///
/// Format: `{topic_prefix}/hosts/{host_id}/items/{item_id}/attributes`
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::json_attributes_topic;
/// let item_id = Uuid::nil();
/// let host_id = Uuid::nil();
/// let topic = json_attributes_topic("uptrakit", item_id, host_id);
/// assert!(topic.ends_with("/attributes"));
/// ```
pub(crate) fn json_attributes_topic(topic_prefix: &str, item_id: Uuid, host_id: Uuid) -> String {
    let hp = host_topic_prefix(topic_prefix, host_id);
    format!("{hp}/items/{item_id}/attributes")
}

/// Returns the MQTT topic carrying the hostname string for a host.
///
/// Format: `{topic_prefix}/hosts/{host_id}/hostname`
///
/// Published as a retained plain-text payload for MQTT explorer visibility.
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::hostname_topic;
/// let topic = hostname_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/hostname"));
/// ```
pub(crate) fn hostname_topic(topic_prefix: &str, host_id: Uuid) -> String {
    let hp = host_topic_prefix(topic_prefix, host_id);
    format!("{hp}/hostname")
}

/// Returns the MQTT topic carrying the friendly name string for a host.
///
/// Format: `{topic_prefix}/hosts/{host_id}/friendly_name`
///
/// Published as a retained plain-text payload for MQTT explorer visibility.
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::friendly_name_topic;
/// let topic = friendly_name_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/friendly_name"));
/// ```
pub(crate) fn friendly_name_topic(topic_prefix: &str, host_id: Uuid) -> String {
    let hp = host_topic_prefix(topic_prefix, host_id);
    format!("{hp}/friendly_name")
}

// =============================================================================
// Host package entity topics
// =============================================================================

/// Returns the MQTT topic carrying the installed-version string (state) for a
/// host's package entity.
///
/// Format: `{prefix}/hosts/{host_id}/state`
///
/// The published value is `"unknown"` when `pending_count > 0`, or
/// `"up-to-date"` when all packages are current.
/// See [`super::host_packages_state_string`].
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::host_packages_state_topic;
/// let topic = host_packages_state_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/state"));
/// ```
pub(crate) fn host_packages_state_topic(prefix: &str, host_id: Uuid) -> String {
    let hp = host_topic_prefix(prefix, host_id);
    format!("{hp}/state")
}

/// Returns the MQTT topic carrying the latest-version string for a host's
/// package entity.
///
/// Format: `{prefix}/hosts/{host_id}/latest_version`
///
/// The published value is `"{N} available"` when `pending_count > 0`, or
/// `"up-to-date"` when all packages are current. Home Assistant compares this
/// against the state topic to determine whether an update badge is shown.
/// See [`super::host_packages_latest_version_string`].
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::host_packages_latest_version_topic;
/// let topic = host_packages_latest_version_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/latest_version"));
/// ```
pub(crate) fn host_packages_latest_version_topic(prefix: &str, host_id: Uuid) -> String {
    let hp = host_topic_prefix(prefix, host_id);
    format!("{hp}/latest_version")
}

/// Returns the MQTT topic for HA JSON attributes of a host's package entity.
///
/// Format: `{prefix}/hosts/{host_id}/attributes`
///
/// Published as a retained JSON payload with fields:
/// - `"in_progress"` (bool) -- whether a batch update is pending/running
/// - `"pending_count"` (u32) -- number of packages with available updates
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::host_packages_json_attributes_topic;
/// let topic = host_packages_json_attributes_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/attributes"));
/// ```
pub(crate) fn host_packages_json_attributes_topic(prefix: &str, host_id: Uuid) -> String {
    let hp = host_topic_prefix(prefix, host_id);
    format!("{hp}/attributes")
}

/// Returns the MQTT command topic that HA publishes `"install"` to for a host's
/// package entity.
///
/// Format: `{prefix}/hosts/{host_id}/set`
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::host_packages_command_topic;
/// let topic = host_packages_command_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/set"));
/// ```
pub(crate) fn host_packages_command_topic(prefix: &str, host_id: Uuid) -> String {
    let hp = host_topic_prefix(prefix, host_id);
    format!("{hp}/set")
}

/// Returns the HA MQTT discovery config topic for a host's package `update`
/// entity.
///
/// Format: `{ha_prefix}/update/uptrakit/pkgs_{t}_{h}/config`
///
/// The static `uptrakit` node_id groups all Uptrakit configs under a shared
/// namespace (see [`discovery_config_topic`]). The `uptrakit_` prefix is
/// stripped from the object_id because the node_id already provides that
/// context.
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::host_packages_discovery_config_topic;
/// let topic = host_packages_discovery_config_topic("homeassistant", Uuid::nil(), Uuid::nil());
/// assert!(topic.starts_with("homeassistant/update/uptrakit/pkgs_"));
/// assert!(topic.ends_with("/config"));
/// ```
pub(crate) fn host_packages_discovery_config_topic(
    ha_prefix: &str,
    tenant_id: Uuid,
    host_id: Uuid,
) -> String {
    let uid = super::host_packages_unique_id(tenant_id, host_id);
    let object_id = uid.strip_prefix("uptrakit_").unwrap_or(&uid);
    format!("{ha_prefix}/update/uptrakit/{object_id}/config")
}

/// Returns a unique ID string for a host's package entity.
///
/// Format: `uptrakit_{tenant_id_no_dashes}_{host_id_no_dashes}_pkgs`
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::host_packages_unique_id;
/// let uid = host_packages_unique_id(Uuid::nil(), Uuid::nil());
/// assert!(uid.starts_with("uptrakit_"));
/// assert!(uid.ends_with("_pkgs"));
/// assert!(!uid.contains('-'));
/// ```
pub(crate) fn host_packages_unique_id(tenant_id: Uuid, host_id: Uuid) -> String {
    let t = tenant_id.simple();
    let h = host_id.simple();
    format!("uptrakit_{t}_{h}_pkgs")
}

// =============================================================================
// Security-only host package entity topics
// =============================================================================

/// Returns the MQTT topic carrying the state string for a host's security
/// updates entity.
///
/// Format: `{prefix}/hosts/{host_id}/security/state`
///
/// The published value is `"unknown"` when `security_pending_count > 0`, or
/// `"up-to-date"` when all security packages are current.
/// See [`super::host_security_state_string`].
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::host_security_state_topic;
/// let topic = host_security_state_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/security/state"));
/// ```
pub(crate) fn host_security_state_topic(prefix: &str, host_id: Uuid) -> String {
    let hp = host_topic_prefix(prefix, host_id);
    format!("{hp}/security/state")
}

/// Returns the MQTT topic carrying the latest-version string for a host's
/// security updates entity.
///
/// Format: `{prefix}/hosts/{host_id}/security/latest_version`
///
/// The published value is `"{N} available"` when `security_pending_count > 0`,
/// or `"up-to-date"` when all security packages are current.
/// See [`super::host_security_latest_version_string`].
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::host_security_latest_version_topic;
/// let topic = host_security_latest_version_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/security/latest_version"));
/// ```
pub(crate) fn host_security_latest_version_topic(prefix: &str, host_id: Uuid) -> String {
    let hp = host_topic_prefix(prefix, host_id);
    format!("{hp}/security/latest_version")
}

/// Returns the MQTT topic for HA JSON attributes of a host's security updates
/// entity.
///
/// Format: `{prefix}/hosts/{host_id}/security/attributes`
///
/// Published as a retained JSON payload with fields:
/// - `"in_progress"` (bool) -- whether a security-only batch is pending/running
/// - `"pending_count"` (u32) -- number of security packages with available updates
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::host_security_json_attributes_topic;
/// let topic = host_security_json_attributes_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/security/attributes"));
/// ```
pub(crate) fn host_security_json_attributes_topic(prefix: &str, host_id: Uuid) -> String {
    let hp = host_topic_prefix(prefix, host_id);
    format!("{hp}/security/attributes")
}

/// Returns the MQTT command topic that HA publishes `"install"` to for a host's
/// security updates entity.
///
/// Format: `{prefix}/hosts/{host_id}/security/set`
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::host_security_command_topic;
/// let topic = host_security_command_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/security/set"));
/// ```
pub(crate) fn host_security_command_topic(prefix: &str, host_id: Uuid) -> String {
    let hp = host_topic_prefix(prefix, host_id);
    format!("{hp}/security/set")
}

/// Returns a unique ID string for a host's security updates entity.
///
/// Format: `uptrakit_{tenant_id_no_dashes}_{host_id_no_dashes}_sec`
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::host_security_unique_id;
/// let uid = host_security_unique_id(Uuid::nil(), Uuid::nil());
/// assert!(uid.starts_with("uptrakit_"));
/// assert!(uid.ends_with("_sec"));
/// assert!(!uid.contains('-'));
/// ```
pub(crate) fn host_security_unique_id(tenant_id: Uuid, host_id: Uuid) -> String {
    let t = tenant_id.simple();
    let h = host_id.simple();
    format!("uptrakit_{t}_{h}_sec")
}

/// Returns the HA MQTT discovery config topic for a host's security updates
/// `update` entity.
///
/// Format: `{ha_prefix}/update/uptrakit/sec_{t}_{h}/config`
///
/// The static `uptrakit` node_id groups all Uptrakit configs under a shared
/// namespace (see [`discovery_config_topic`]). The `uptrakit_` prefix is
/// stripped from the object_id because the node_id already provides that
/// context.
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::host_security_discovery_config_topic;
/// let topic = host_security_discovery_config_topic("homeassistant", Uuid::nil(), Uuid::nil());
/// assert!(topic.starts_with("homeassistant/update/uptrakit/sec_"));
/// assert!(topic.ends_with("/config"));
/// ```
pub(crate) fn host_security_discovery_config_topic(
    ha_prefix: &str,
    tenant_id: Uuid,
    host_id: Uuid,
) -> String {
    let uid = host_security_unique_id(tenant_id, host_id);
    let object_id = uid.strip_prefix("uptrakit_").unwrap_or(&uid);
    format!("{ha_prefix}/update/uptrakit/{object_id}/config")
}

// =============================================================================
// Host metadata and connectivity entity topics
// =============================================================================

/// Returns the MQTT topic carrying the OS info JSON for a host.
///
/// Format: `{prefix}/hosts/{host_id}/info`
///
/// Published as a retained JSON payload. See [`super::build_host_info_payload`].
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::host_info_topic;
/// let topic = host_info_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/info"));
/// ```
pub(crate) fn host_info_topic(prefix: &str, host_id: Uuid) -> String {
    let hp = host_topic_prefix(prefix, host_id);
    format!("{hp}/info")
}

/// Returns the MQTT topic carrying the tag list JSON for a host.
///
/// Format: `{prefix}/hosts/{host_id}/tags`
///
/// Published as a retained JSON array of tag name strings.
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::host_tags_topic;
/// let topic = host_tags_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/tags"));
/// ```
pub(crate) fn host_tags_topic(prefix: &str, host_id: Uuid) -> String {
    let hp = host_topic_prefix(prefix, host_id);
    format!("{hp}/tags")
}

/// Returns the MQTT topic carrying the agent info JSON for a host.
///
/// Format: `{prefix}/hosts/{host_id}/agent`
///
/// Published as a retained JSON payload. See [`super::build_host_agent_payload`].
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::host_agent_topic;
/// let topic = host_agent_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/agent"));
/// ```
pub(crate) fn host_agent_topic(prefix: &str, host_id: Uuid) -> String {
    let hp = host_topic_prefix(prefix, host_id);
    format!("{hp}/agent")
}

/// Returns the MQTT topic carrying the connectivity state string for a host.
///
/// Format: `{prefix}/hosts/{host_id}/connectivity/state`
///
/// Published as a retained `"online"` or `"offline"` string.
/// Updated by the `HostConnectivityUpdated` event from the controller.
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::host_connectivity_state_topic;
/// let topic = host_connectivity_state_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/connectivity/state"));
/// ```
pub(crate) fn host_connectivity_state_topic(prefix: &str, host_id: Uuid) -> String {
    let hp = host_topic_prefix(prefix, host_id);
    format!("{hp}/connectivity/state")
}

/// Returns the MQTT topic carrying the connectivity attributes JSON for a host.
///
/// Format: `{prefix}/hosts/{host_id}/connectivity/attributes`
///
/// Published as a retained JSON payload with `last_seen` (ISO 8601 string or
/// `null`) and `version` (agent version string or `null`).
/// Updated by the `HostConnectivityUpdated` event from the controller.
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::host_connectivity_attributes_topic;
/// let topic = host_connectivity_attributes_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/connectivity/attributes"));
/// ```
pub(crate) fn host_connectivity_attributes_topic(prefix: &str, host_id: Uuid) -> String {
    let hp = host_topic_prefix(prefix, host_id);
    format!("{hp}/connectivity/attributes")
}

/// Returns a unique ID string for a host's connectivity `binary_sensor` entity.
///
/// Format: `uptrakit_{tenant_id_no_dashes}_{host_id_no_dashes}_conn`
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::host_connectivity_unique_id;
/// let uid = host_connectivity_unique_id(Uuid::nil(), Uuid::nil());
/// assert!(uid.starts_with("uptrakit_"));
/// assert!(uid.ends_with("_conn"));
/// assert!(!uid.contains('-'));
/// ```
pub(crate) fn host_connectivity_unique_id(tenant_id: Uuid, host_id: Uuid) -> String {
    let t = tenant_id.simple();
    let h = host_id.simple();
    format!("uptrakit_{t}_{h}_conn")
}

/// Returns the HA MQTT discovery config topic for a host's connectivity
/// `binary_sensor` entity.
///
/// Format: `{ha_prefix}/binary_sensor/uptrakit/{t}_{h}_conn/config`
///
/// Uses the `binary_sensor` platform namespace (not `update`) so that HA
/// creates a connectivity sensor rather than an update entity. The `uptrakit`
/// node_id groups all Uptrakit configs under a single namespace.
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::host_connectivity_discovery_config_topic;
/// let topic = host_connectivity_discovery_config_topic("homeassistant", Uuid::nil(), Uuid::nil());
/// assert!(topic.starts_with("homeassistant/binary_sensor/uptrakit/"));
/// assert!(topic.ends_with("_conn/config"));
/// ```
pub(crate) fn host_connectivity_discovery_config_topic(
    ha_prefix: &str,
    tenant_id: Uuid,
    host_id: Uuid,
) -> String {
    let uid = host_connectivity_unique_id(tenant_id, host_id);
    let object_id = uid.strip_prefix("uptrakit_").unwrap_or(&uid);
    format!("{ha_prefix}/binary_sensor/uptrakit/{object_id}/config")
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
    // discovery_config_topic
    // -------------------------------------------------------------------------

    #[test]
    fn discovery_config_topic_format() {
        let uid = "uptrakit_abc_def_ghi";
        assert_eq!(
            discovery_config_topic("homeassistant", uid),
            "homeassistant/update/uptrakit/abc_def_ghi/config"
        );
    }

    #[test]
    fn discovery_config_topic_no_uptrakit_prefix_passthrough() {
        assert_eq!(
            discovery_config_topic("ha", "uid123"),
            "ha/update/uptrakit/uid123/config"
        );
    }

    #[test]
    fn discovery_config_topic_custom_prefix() {
        let uid = super::super::unique_id(tenant(), item(), host());
        let topic = discovery_config_topic("ha", &uid);
        assert!(topic.starts_with("ha/update/uptrakit/"));
        assert!(!topic.contains("uptrakit_uptrakit_"));
        assert!(topic.ends_with("/config"));
    }

    // -------------------------------------------------------------------------
    // host_topic_prefix
    // -------------------------------------------------------------------------

    #[test]
    fn host_topic_prefix_format() {
        let p = host_topic_prefix("uptrakit", host());
        assert_eq!(p, "uptrakit/hosts/33333333-3333-3333-3333-333333333333");
    }

    // -------------------------------------------------------------------------
    // state_topic
    // -------------------------------------------------------------------------

    #[test]
    fn state_topic_format() {
        let t = state_topic("uptrakit", item(), host());
        assert_eq!(
            t,
            "uptrakit/hosts/33333333-3333-3333-3333-333333333333/items/22222222-2222-2222-2222-222222222222/state"
        );
    }

    #[test]
    fn state_topic_ends_with_state() {
        assert!(state_topic("pfx", item(), host()).ends_with("/state"));
    }

    // -------------------------------------------------------------------------
    // latest_version_topic
    // -------------------------------------------------------------------------

    #[test]
    fn latest_version_topic_format() {
        let t = latest_version_topic("uptrakit", item(), host());
        assert_eq!(
            t,
            "uptrakit/hosts/33333333-3333-3333-3333-333333333333/items/22222222-2222-2222-2222-222222222222/latest_version"
        );
    }

    #[test]
    fn latest_version_topic_ends_with_latest_version() {
        assert!(latest_version_topic("pfx", item(), host()).ends_with("/latest_version"));
    }

    // -------------------------------------------------------------------------
    // command_topic
    // -------------------------------------------------------------------------

    #[test]
    fn command_topic_format() {
        let t = command_topic("uptrakit", item(), host());
        assert_eq!(
            t,
            "uptrakit/hosts/33333333-3333-3333-3333-333333333333/items/22222222-2222-2222-2222-222222222222/set"
        );
    }

    #[test]
    fn command_topic_ends_with_set() {
        assert!(command_topic("pfx", item(), host()).ends_with("/set"));
    }

    // -------------------------------------------------------------------------
    // json_attributes_topic
    // -------------------------------------------------------------------------

    #[test]
    fn json_attributes_topic_format() {
        let t = json_attributes_topic("uptrakit", item(), host());
        assert_eq!(
            t,
            "uptrakit/hosts/33333333-3333-3333-3333-333333333333/items/22222222-2222-2222-2222-222222222222/attributes"
        );
    }

    #[test]
    fn json_attributes_topic_ends_with_attributes() {
        assert!(json_attributes_topic("pfx", item(), host()).ends_with("/attributes"));
    }

    #[test]
    fn json_attributes_topic_custom_prefix() {
        let t = json_attributes_topic("home/uptrakit", item(), host());
        assert!(t.starts_with("home/uptrakit/hosts/"));
        assert!(t.ends_with("/attributes"));
    }

    // -------------------------------------------------------------------------
    // hostname_topic / friendly_name_topic
    // -------------------------------------------------------------------------

    #[test]
    fn hostname_topic_format() {
        let t = hostname_topic("uptrakit", host());
        assert_eq!(
            t,
            "uptrakit/hosts/33333333-3333-3333-3333-333333333333/hostname"
        );
    }

    #[test]
    fn friendly_name_topic_format() {
        let t = friendly_name_topic("uptrakit", host());
        assert_eq!(
            t,
            "uptrakit/hosts/33333333-3333-3333-3333-333333333333/friendly_name"
        );
    }

    // -------------------------------------------------------------------------
    // host_packages_state_topic
    // -------------------------------------------------------------------------

    #[test]
    fn host_packages_state_topic_format() {
        let t = host_packages_state_topic("uptrakit", host());
        assert_eq!(
            t,
            "uptrakit/hosts/33333333-3333-3333-3333-333333333333/state"
        );
    }

    #[test]
    fn host_packages_state_topic_ends_with_state() {
        assert!(host_packages_state_topic("pfx", host()).ends_with("/state"));
    }

    // -------------------------------------------------------------------------
    // host_packages_latest_version_topic
    // -------------------------------------------------------------------------

    #[test]
    fn host_packages_latest_version_topic_format() {
        let t = host_packages_latest_version_topic("uptrakit", host());
        assert_eq!(
            t,
            "uptrakit/hosts/33333333-3333-3333-3333-333333333333/latest_version"
        );
    }

    #[test]
    fn host_packages_latest_version_topic_ends_with_latest_version() {
        assert!(host_packages_latest_version_topic("pfx", host()).ends_with("/latest_version"));
    }

    // -------------------------------------------------------------------------
    // host_packages_json_attributes_topic
    // -------------------------------------------------------------------------

    #[test]
    fn host_packages_json_attributes_topic_format() {
        let t = host_packages_json_attributes_topic("uptrakit", host());
        assert_eq!(
            t,
            "uptrakit/hosts/33333333-3333-3333-3333-333333333333/attributes"
        );
    }

    // -------------------------------------------------------------------------
    // host_packages_command_topic
    // -------------------------------------------------------------------------

    #[test]
    fn host_packages_command_topic_format() {
        let t = host_packages_command_topic("uptrakit", host());
        assert_eq!(t, "uptrakit/hosts/33333333-3333-3333-3333-333333333333/set");
    }

    // -------------------------------------------------------------------------
    // host_packages_unique_id
    // -------------------------------------------------------------------------

    #[test]
    fn host_packages_unique_id_starts_with_uptrakit_ends_with_pkgs() {
        let uid = host_packages_unique_id(tenant(), host());
        assert!(uid.starts_with("uptrakit_"));
        assert!(uid.ends_with("_pkgs"));
    }

    #[test]
    fn host_packages_unique_id_no_dashes() {
        let uid = host_packages_unique_id(tenant(), host());
        assert!(!uid.contains('-'));
    }

    #[test]
    fn host_packages_unique_id_deterministic() {
        assert_eq!(
            host_packages_unique_id(tenant(), host()),
            host_packages_unique_id(tenant(), host())
        );
    }

    // -------------------------------------------------------------------------
    // host_packages_discovery_config_topic
    // -------------------------------------------------------------------------

    #[test]
    fn host_packages_discovery_config_topic_format() {
        let topic = host_packages_discovery_config_topic("homeassistant", tenant(), host());
        assert!(topic.starts_with("homeassistant/update/uptrakit/"));
        assert!(topic.ends_with("_pkgs/config"));
    }

    // -------------------------------------------------------------------------
    // host_security_state_topic
    // -------------------------------------------------------------------------

    #[test]
    fn host_security_state_topic_format() {
        let t = host_security_state_topic("uptrakit", host());
        assert_eq!(
            t,
            "uptrakit/hosts/33333333-3333-3333-3333-333333333333/security/state"
        );
    }

    #[test]
    fn host_security_state_topic_ends_with_security_state() {
        assert!(host_security_state_topic("pfx", host()).ends_with("/security/state"));
    }

    // -------------------------------------------------------------------------
    // host_security_latest_version_topic
    // -------------------------------------------------------------------------

    #[test]
    fn host_security_latest_version_topic_format() {
        let t = host_security_latest_version_topic("uptrakit", host());
        assert_eq!(
            t,
            "uptrakit/hosts/33333333-3333-3333-3333-333333333333/security/latest_version"
        );
    }

    // -------------------------------------------------------------------------
    // host_security_json_attributes_topic
    // -------------------------------------------------------------------------

    #[test]
    fn host_security_json_attributes_topic_format() {
        let t = host_security_json_attributes_topic("uptrakit", host());
        assert_eq!(
            t,
            "uptrakit/hosts/33333333-3333-3333-3333-333333333333/security/attributes"
        );
    }

    // -------------------------------------------------------------------------
    // host_security_command_topic
    // -------------------------------------------------------------------------

    #[test]
    fn host_security_command_topic_format() {
        let t = host_security_command_topic("uptrakit", host());
        assert_eq!(
            t,
            "uptrakit/hosts/33333333-3333-3333-3333-333333333333/security/set"
        );
    }

    // -------------------------------------------------------------------------
    // host_security_unique_id
    // -------------------------------------------------------------------------

    #[test]
    fn host_security_unique_id_starts_with_uptrakit_ends_with_sec() {
        let uid = host_security_unique_id(tenant(), host());
        assert!(uid.starts_with("uptrakit_"));
        assert!(uid.ends_with("_sec"));
    }

    #[test]
    fn host_security_unique_id_no_dashes() {
        let uid = host_security_unique_id(tenant(), host());
        assert!(!uid.contains('-'));
    }

    #[test]
    fn host_security_unique_id_differs_from_packages_unique_id() {
        assert_ne!(
            host_security_unique_id(tenant(), host()),
            host_packages_unique_id(tenant(), host())
        );
    }

    // -------------------------------------------------------------------------
    // host_security_discovery_config_topic
    // -------------------------------------------------------------------------

    #[test]
    fn host_security_discovery_config_topic_format() {
        let topic = host_security_discovery_config_topic("homeassistant", tenant(), host());
        assert!(topic.starts_with("homeassistant/update/uptrakit/"));
        assert!(topic.ends_with("_sec/config"));
    }

    // -------------------------------------------------------------------------
    // host_info_topic
    // -------------------------------------------------------------------------

    #[test]
    fn host_info_topic_format() {
        let t = host_info_topic("uptrakit", host());
        assert_eq!(
            t,
            "uptrakit/hosts/33333333-3333-3333-3333-333333333333/info"
        );
    }

    #[test]
    fn host_info_topic_ends_with_info() {
        assert!(host_info_topic("pfx", host()).ends_with("/info"));
    }

    // -------------------------------------------------------------------------
    // host_tags_topic
    // -------------------------------------------------------------------------

    #[test]
    fn host_tags_topic_format() {
        let t = host_tags_topic("uptrakit", host());
        assert_eq!(
            t,
            "uptrakit/hosts/33333333-3333-3333-3333-333333333333/tags"
        );
    }

    #[test]
    fn host_tags_topic_ends_with_tags() {
        assert!(host_tags_topic("pfx", host()).ends_with("/tags"));
    }

    // -------------------------------------------------------------------------
    // host_agent_topic
    // -------------------------------------------------------------------------

    #[test]
    fn host_agent_topic_format() {
        let t = host_agent_topic("uptrakit", host());
        assert_eq!(
            t,
            "uptrakit/hosts/33333333-3333-3333-3333-333333333333/agent"
        );
    }

    #[test]
    fn host_agent_topic_ends_with_agent() {
        assert!(host_agent_topic("pfx", host()).ends_with("/agent"));
    }

    // -------------------------------------------------------------------------
    // host_connectivity_state_topic
    // -------------------------------------------------------------------------

    #[test]
    fn host_connectivity_state_topic_format() {
        let t = host_connectivity_state_topic("uptrakit", host());
        assert_eq!(
            t,
            "uptrakit/hosts/33333333-3333-3333-3333-333333333333/connectivity/state"
        );
    }

    #[test]
    fn host_connectivity_state_topic_ends_correctly() {
        assert!(host_connectivity_state_topic("pfx", host()).ends_with("/connectivity/state"));
    }

    // -------------------------------------------------------------------------
    // host_connectivity_attributes_topic
    // -------------------------------------------------------------------------

    #[test]
    fn host_connectivity_attributes_topic_format() {
        let t = host_connectivity_attributes_topic("uptrakit", host());
        assert_eq!(
            t,
            "uptrakit/hosts/33333333-3333-3333-3333-333333333333/connectivity/attributes"
        );
    }

    #[test]
    fn host_connectivity_attributes_topic_ends_correctly() {
        assert!(
            host_connectivity_attributes_topic("pfx", host()).ends_with("/connectivity/attributes")
        );
    }

    // -------------------------------------------------------------------------
    // host_connectivity_unique_id
    // -------------------------------------------------------------------------

    #[test]
    fn host_connectivity_unique_id_starts_and_ends_correctly() {
        let uid = host_connectivity_unique_id(tenant(), host());
        assert!(uid.starts_with("uptrakit_"));
        assert!(uid.ends_with("_conn"));
    }

    #[test]
    fn host_connectivity_unique_id_no_dashes() {
        assert!(!host_connectivity_unique_id(tenant(), host()).contains('-'));
    }

    #[test]
    fn host_connectivity_unique_id_differs_from_packages() {
        assert_ne!(
            host_connectivity_unique_id(tenant(), host()),
            host_packages_unique_id(tenant(), host())
        );
    }

    // -------------------------------------------------------------------------
    // host_connectivity_discovery_config_topic
    // -------------------------------------------------------------------------

    #[test]
    fn host_connectivity_discovery_config_topic_format() {
        let topic = host_connectivity_discovery_config_topic("homeassistant", tenant(), host());
        assert!(topic.starts_with("homeassistant/binary_sensor/uptrakit/"));
        assert!(topic.ends_with("_conn/config"));
    }
}
