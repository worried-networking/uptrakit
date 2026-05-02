//! HA device block builder and discovery config JSON builders.
//!
//! All functions are stateless and produce deterministic results from their
//! inputs. No async, no I/O.

use uuid::Uuid;

use super::topics::{
    command_topic, host_connectivity_attributes_topic, host_connectivity_state_topic,
    host_connectivity_unique_id, host_packages_command_topic, host_packages_json_attributes_topic,
    host_packages_latest_version_topic, host_packages_state_topic, host_packages_unique_id,
    host_security_command_topic, host_security_json_attributes_topic,
    host_security_latest_version_topic, host_security_state_topic, host_security_unique_id,
    json_attributes_topic, latest_version_topic, state_topic,
};
use super::{HostOsInfo, ReleaseInfo, unique_id};

/// Build the HA device block JSON shared across all entity builders.
///
/// Uses the host-centric device identifier `uptrakit_host_{tenant}_{host}` so
/// every entity for a host groups under a single HA device. When `os_info`
/// fields are present the `model`, `sw_version`, and `hw_version` fields are
/// included.
#[expect(
    clippy::indexing_slicing,
    reason = "device is always a serde_json::Value::Object (constructed via json!({...})); string indexing on Object inserts/updates and never panics"
)]
fn build_device_block(
    tenant_id: Uuid,
    host_id: Uuid,
    friendly_name: &str,
    os_info: HostOsInfo<'_>,
) -> serde_json::Value {
    let tenant_simple = tenant_id.simple().to_string();
    let host_simple = host_id.simple().to_string();
    let mut device = serde_json::json!({
        "identifiers": [format!("uptrakit_host_{tenant_simple}_{host_simple}")],
        "name": friendly_name,
        "manufacturer": "Uptrakit"
    });
    if let Some(v) = os_info.os_type {
        device["model"] = serde_json::json!(v);
    }
    if let Some(v) = os_info.os_version {
        device["sw_version"] = serde_json::json!(v);
    }
    if let Some(v) = os_info.architecture {
        device["hw_version"] = serde_json::json!(v);
    }
    device
}

/// Build the HA MQTT discovery JSON for an `update` entity.
///
/// All entities for a given host are grouped under a single HA device
/// (identified by `uptrakit_host_{tenant_id}_{host_id}`), named after the
/// host's `friendly_name`. The entity itself is named after the software item.
/// An explicit `default_entity_id` in the form
/// `update.uptrakit_{friendly_name_slug}_{item_slug}` is included so that HA
/// uses a stable, human-readable entity ID on first registration.
///
/// Returns a [`serde_json::Value`] that should be serialized with `to_string()`
/// and published (retained) on `discovery_config_topic(...)`.
///
/// When `release.url` is provided it is included verbatim as `release_url`.
///
/// Pass `ReleaseInfo::default()` when no release metadata is available.
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::{build_discovery_config, ReleaseInfo, HostOsInfo};
/// let v = build_discovery_config(
///     "uptrakit",
///     Uuid::nil(),
///     Uuid::nil(),
///     Uuid::nil(),
///     "My App",
///     "myhost",
///     ReleaseInfo::default(),
///     HostOsInfo::default(),
/// );
/// assert_eq!(v["platform"], "mqtt");
/// assert_eq!(v["payload_install"], "install");
/// assert_eq!(v["title"], "Software Update (My App)");
/// ```
#[expect(
    clippy::too_many_arguments,
    clippy::indexing_slicing,
    reason = "all parameters are required to build the HA discovery config; config is a serde_json::Value::Object so string indexing is safe"
)]
pub(crate) fn build_discovery_config(
    topic_prefix: &str,
    tenant_id: Uuid,
    item_id: Uuid,
    host_id: Uuid,
    item_name: &str,
    friendly_name: &str,
    release: ReleaseInfo<'_>,
    os_info: HostOsInfo<'_>,
) -> serde_json::Value {
    let uid = unique_id(tenant_id, item_id, host_id);
    let default_entity_id = format!(
        "update.uptrakit_{}_{}",
        super::slugify(friendly_name),
        super::slugify(item_name)
    );

    let mut config = serde_json::json!({
        "platform": "mqtt",
        "unique_id": uid,
        "name": item_name,
        "title": format!("Software Update ({})", item_name),
        "default_entity_id": default_entity_id,
        "state_topic": state_topic(topic_prefix, item_id, host_id),
        "latest_version_topic": latest_version_topic(topic_prefix, item_id, host_id),
        "command_topic": command_topic(topic_prefix, item_id, host_id),
        "payload_install": "install",
        "json_attributes_topic": json_attributes_topic(topic_prefix, item_id, host_id),
        "availability": [
            {
                "topic": format!("{topic_prefix}/status"),
                "payload_available": "online",
                "payload_not_available": "offline"
            },
            {
                "topic": format!("{topic_prefix}/hosts/{host_id}/connectivity/state"),
                "payload_available": "online",
                "payload_not_available": "offline"
            }
        ],
        "availability_mode": "all",
        "device": build_device_block(tenant_id, host_id, friendly_name, os_info)
    });

    if let Some(url) = release.url {
        config["release_url"] = serde_json::Value::String(url.to_string());
    }
    if let Some(url) = release.icon_url {
        config["entity_picture"] = serde_json::Value::String(url.to_string());
    }

    config
}

/// Build the HA MQTT discovery JSON for a host's package `update` entity.
///
/// Publishes a single entity per host that represents the overall package update
/// status. The device is identified by the host (not the software item). Home
/// Assistant displays an update badge when `installed_version != latest_version`,
/// i.e. when `pending_count > 0`.
///
/// The entity is **disabled by default** (`"enabled_by_default": false`). Users
/// must explicitly enable it in Home Assistant to see it. This avoids noise for
/// users who do not want package-level tracking.
///
/// The returned JSON should be published (retained) on
/// [`super::host_packages_discovery_config_topic`].
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::{build_host_packages_discovery_config, HostOsInfo};
/// let v = build_host_packages_discovery_config(
///     "uptrakit",
///     Uuid::nil(),
///     Uuid::nil(),
///     "myserver",
///     HostOsInfo::default(),
/// );
/// assert!(v["name"].is_null());
/// assert_eq!(v["platform"], "mqtt");
/// assert_eq!(v["payload_install"], "install");
/// assert_eq!(v["enabled_by_default"], false);
/// ```
pub(crate) fn build_host_packages_discovery_config(
    topic_prefix: &str,
    tenant_id: Uuid,
    host_id: Uuid,
    friendly_name: &str,
    os_info: HostOsInfo<'_>,
) -> serde_json::Value {
    let uid = host_packages_unique_id(tenant_id, host_id);
    let default_entity_id = format!("update.uptrakit_{}_packages", super::slugify(friendly_name));

    serde_json::json!({
        "platform": "mqtt",
        "unique_id": uid,
        "name": null,
        "title": "Packages Update",
        "default_entity_id": default_entity_id,
        "enabled_by_default": false,
        "state_topic": host_packages_state_topic(topic_prefix, host_id),
        "latest_version_topic": host_packages_latest_version_topic(topic_prefix, host_id),
        "command_topic": host_packages_command_topic(topic_prefix, host_id),
        "payload_install": "install",
        "json_attributes_topic": host_packages_json_attributes_topic(topic_prefix, host_id),
        "availability": [
            {
                "topic": format!("{topic_prefix}/status"),
                "payload_available": "online",
                "payload_not_available": "offline"
            },
            {
                "topic": format!("{topic_prefix}/hosts/{host_id}/connectivity/state"),
                "payload_available": "online",
                "payload_not_available": "offline"
            }
        ],
        "availability_mode": "all",
        "device": build_device_block(tenant_id, host_id, friendly_name, os_info)
    })
}

/// Build the HA MQTT discovery JSON for a host's security updates `update`
/// entity.
///
/// This is a second `update` entity per host (alongside the all-packages entity)
/// that surfaces only packages with `update_category = "security"`. It is
/// **disabled by default** -- users opt in explicitly.
///
/// The device identifier is the same as the host packages entity
/// (`uptrakit_host_{tenant}_{host}`), so both entities appear under the same
/// HA device.
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::{build_host_security_discovery_config, HostOsInfo};
/// let v = build_host_security_discovery_config(
///     "uptrakit",
///     Uuid::nil(),
///     Uuid::nil(),
///     "myserver",
///     HostOsInfo::default(),
/// );
/// assert!(v["name"].is_null());
/// assert_eq!(v["platform"], "mqtt");
/// assert_eq!(v["payload_install"], "install");
/// assert_eq!(v["enabled_by_default"], false);
/// ```
pub(crate) fn build_host_security_discovery_config(
    topic_prefix: &str,
    tenant_id: Uuid,
    host_id: Uuid,
    friendly_name: &str,
    os_info: HostOsInfo<'_>,
) -> serde_json::Value {
    let uid = host_security_unique_id(tenant_id, host_id);
    let default_entity_id = format!(
        "update.uptrakit_{}_security_updates",
        super::slugify(friendly_name)
    );

    serde_json::json!({
        "platform": "mqtt",
        "unique_id": uid,
        "name": null,
        "title": "Security Updates",
        "default_entity_id": default_entity_id,
        "enabled_by_default": false,
        "state_topic": host_security_state_topic(topic_prefix, host_id),
        "latest_version_topic": host_security_latest_version_topic(topic_prefix, host_id),
        "command_topic": host_security_command_topic(topic_prefix, host_id),
        "payload_install": "install",
        "json_attributes_topic": host_security_json_attributes_topic(topic_prefix, host_id),
        "availability": [
            {
                "topic": format!("{topic_prefix}/status"),
                "payload_available": "online",
                "payload_not_available": "offline"
            },
            {
                "topic": format!("{topic_prefix}/hosts/{host_id}/connectivity/state"),
                "payload_available": "online",
                "payload_not_available": "offline"
            }
        ],
        "availability_mode": "all",
        "device": build_device_block(tenant_id, host_id, friendly_name, os_info)
    })
}

/// Build the HA MQTT discovery JSON for a host's connectivity `binary_sensor`
/// entity.
///
/// Creates one `binary_sensor` per host that surfaces whether the Uptrakit
/// agent is currently connected (`"online"`) or not (`"offline"`). The entity
/// is **enabled by default** because connectivity monitoring is core
/// operational value.
///
/// The device identifier is the same as the update entities for this host
/// (`uptrakit_host_{tenant}_{host}`), so the sensor appears under the same
/// HA device as the package and software item update entities.
///
/// When `os_info` fields are provided, the corresponding `model`, `sw_version`,
/// and `hw_version` fields are included in the HA device block. Home Assistant
/// merges device info from all entities sharing the same device identifier, so
/// enriching this one config is sufficient to populate the device card.
///
/// The returned JSON should be published (retained) on
/// [`super::host_connectivity_discovery_config_topic`].
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::{build_host_connectivity_discovery_config, HostOsInfo};
/// let v = build_host_connectivity_discovery_config(
///     "uptrakit",
///     Uuid::nil(),
///     Uuid::nil(),
///     "myserver",
///     HostOsInfo::default(),
/// );
/// assert_eq!(v["platform"], "mqtt");
/// assert_eq!(v["device_class"], "connectivity");
/// assert_eq!(v["payload_on"], "online");
/// assert_eq!(v["payload_off"], "offline");
/// assert_eq!(v["enabled_by_default"], true);
/// ```
pub(crate) fn build_host_connectivity_discovery_config(
    topic_prefix: &str,
    tenant_id: Uuid,
    host_id: Uuid,
    friendly_name: &str,
    os_info: HostOsInfo<'_>,
) -> serde_json::Value {
    let uid = host_connectivity_unique_id(tenant_id, host_id);
    let default_entity_id = format!(
        "binary_sensor.uptrakit_{}_agent",
        super::slugify(friendly_name)
    );

    serde_json::json!({
        "platform": "mqtt",
        "unique_id": uid,
        "name": format!("{friendly_name} agent"),
        "default_entity_id": default_entity_id,
        "device_class": "connectivity",
        "enabled_by_default": true,
        "state_topic": host_connectivity_state_topic(topic_prefix, host_id),
        "json_attributes_topic": host_connectivity_attributes_topic(topic_prefix, host_id),
        "payload_on": "online",
        "payload_off": "offline",
        "availability_topic": format!("{topic_prefix}/status"),
        "payload_available": "online",
        "payload_not_available": "offline",
        "device": build_device_block(tenant_id, host_id, friendly_name, os_info)
    })
}

#[cfg(test)]
mod tests {
    use super::super::topics::{
        host_packages_state_topic, host_security_command_topic, json_attributes_topic,
        latest_version_topic, state_topic,
    };
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
    // build_discovery_config
    // -------------------------------------------------------------------------

    #[test]
    fn build_discovery_config_platform_mqtt() {
        let v = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "myhost",
            ReleaseInfo::default(),
            HostOsInfo::default(),
        );
        assert_eq!(v["platform"], "mqtt");
    }

    #[test]
    fn build_discovery_config_unique_id_matches() {
        let v = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "myhost",
            ReleaseInfo::default(),
            HostOsInfo::default(),
        );
        let expected_uid = unique_id(tenant(), item(), host());
        assert_eq!(v["unique_id"], expected_uid.as_str());
    }

    #[test]
    fn build_discovery_config_name_is_item_name() {
        let v = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "MyApp",
            "server1",
            ReleaseInfo::default(),
            HostOsInfo::default(),
        );
        assert_eq!(v["name"], "MyApp");
    }

    #[test]
    fn build_discovery_config_title_embeds_item_name() {
        let v = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "My App",
            "myhost",
            ReleaseInfo::default(),
            HostOsInfo::default(),
        );
        assert_eq!(v["title"], "Software Update (My App)");
    }

    #[test]
    fn build_discovery_config_state_topic_correct() {
        let v = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "h",
            ReleaseInfo::default(),
            HostOsInfo::default(),
        );
        let expected = state_topic("uptrakit", item(), host());
        assert_eq!(v["state_topic"], expected.as_str());
    }

    #[test]
    fn build_discovery_config_latest_version_topic_correct() {
        let v = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "h",
            ReleaseInfo::default(),
            HostOsInfo::default(),
        );
        let expected = latest_version_topic("uptrakit", item(), host());
        assert_eq!(v["latest_version_topic"], expected.as_str());
    }

    #[test]
    fn build_discovery_config_command_topic_correct() {
        let v = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "h",
            ReleaseInfo::default(),
            HostOsInfo::default(),
        );
        let expected = command_topic("uptrakit", item(), host());
        assert_eq!(v["command_topic"], expected.as_str());
    }

    #[test]
    fn build_discovery_config_payload_install() {
        let v = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "h",
            ReleaseInfo::default(),
            HostOsInfo::default(),
        );
        assert_eq!(v["payload_install"], "install");
    }

    #[test]
    fn build_discovery_config_availability_topic() {
        let v = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "h",
            ReleaseInfo::default(),
            HostOsInfo::default(),
        );
        assert_eq!(v["availability"][0]["topic"], "uptrakit/status");
        assert_eq!(
            v["availability"][1]["topic"],
            format!("uptrakit/hosts/{}/connectivity/state", host())
        );
        assert_eq!(v["availability_mode"], "all");
    }

    #[test]
    fn build_discovery_config_availability_payloads() {
        let v = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "h",
            ReleaseInfo::default(),
            HostOsInfo::default(),
        );
        assert_eq!(v["availability"][0]["payload_available"], "online");
        assert_eq!(v["availability"][0]["payload_not_available"], "offline");
    }

    #[test]
    fn build_discovery_config_json_attributes_topic_correct() {
        let v = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "h",
            ReleaseInfo::default(),
            HostOsInfo::default(),
        );
        let expected = json_attributes_topic("uptrakit", item(), host());
        assert_eq!(v["json_attributes_topic"], expected.as_str());
    }

    #[test]
    fn build_discovery_config_device_identifiers() {
        let v = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "h",
            ReleaseInfo::default(),
            HostOsInfo::default(),
        );
        let tenant_simple = tenant().simple().to_string();
        let host_simple = host().simple().to_string();
        let expected_id = format!("uptrakit_host_{tenant_simple}_{host_simple}");
        assert_eq!(v["device"]["identifiers"][0], expected_id.as_str());
    }

    #[test]
    fn build_discovery_config_device_name_is_friendly_name() {
        let v = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "My App",
            "My Friendly Host",
            ReleaseInfo::default(),
            HostOsInfo::default(),
        );
        assert_eq!(v["device"]["name"], "My Friendly Host");
        assert_eq!(v["device"]["manufacturer"], "Uptrakit");
    }

    #[test]
    fn build_discovery_config_default_entity_id() {
        let v = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "uptrakit pangolin",
            "pangolin.uk.home.yantsen.su",
            ReleaseInfo::default(),
            HostOsInfo::default(),
        );
        assert_eq!(
            v["default_entity_id"],
            "update.uptrakit_pangolin_uk_home_yantsen_su_uptrakit_pangolin"
        );
    }

    #[test]
    fn build_discovery_config_default_entity_id_simple_names() {
        let v = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "MyApp",
            "server1",
            ReleaseInfo::default(),
            HostOsInfo::default(),
        );
        assert_eq!(v["default_entity_id"], "update.uptrakit_server1_myapp");
    }

    #[test]
    fn build_discovery_config_serializes_to_valid_json() {
        let v = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "h",
            ReleaseInfo::default(),
            HostOsInfo::default(),
        );
        let s = v.to_string();
        let reparsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(reparsed["platform"], "mqtt");
    }

    // -------------------------------------------------------------------------
    // build_discovery_config -- release metadata
    // -------------------------------------------------------------------------

    #[test]
    fn build_discovery_config_with_release_url() {
        let url = "https://github.com/owner/repo/releases/tag/v1.3.0";
        let v = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "h",
            ReleaseInfo {
                url: Some(url),
                icon_url: None,
            },
            HostOsInfo::default(),
        );
        assert_eq!(v["release_url"], url);
        assert!(v.get("release_summary").is_none());
    }

    #[test]
    fn build_discovery_config_no_release_metadata_omits_fields() {
        let v = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "h",
            ReleaseInfo::default(),
            HostOsInfo::default(),
        );
        assert!(v.get("release_url").is_none());
        assert!(v.get("release_summary").is_none());
    }

    #[test]
    fn build_discovery_config_with_icon_url() {
        let icon = "https://example.com/icon.png";
        let v = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "h",
            ReleaseInfo {
                url: None,
                icon_url: Some(icon),
            },
            HostOsInfo::default(),
        );
        assert_eq!(v["entity_picture"], icon);
    }

    #[test]
    fn build_discovery_config_no_icon_url_omits_entity_picture() {
        let v = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "h",
            ReleaseInfo::default(),
            HostOsInfo::default(),
        );
        assert!(v.get("entity_picture").is_none());
    }

    // -------------------------------------------------------------------------
    // build_host_connectivity_discovery_config
    // -------------------------------------------------------------------------

    #[test]
    fn build_host_connectivity_discovery_config_keeps_single_availability() {
        let v = build_host_connectivity_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "myhost",
            HostOsInfo::default(),
        );
        assert!(
            v.get("availability_topic").is_some(),
            "connectivity sensor must have availability_topic"
        );
        assert!(
            v.get("availability").is_none(),
            "connectivity sensor must NOT have availability array"
        );
    }

    // -------------------------------------------------------------------------
    // build_host_packages_discovery_config
    // -------------------------------------------------------------------------

    #[test]
    fn build_host_packages_discovery_config_platform_mqtt() {
        let v = build_host_packages_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "myserver",
            HostOsInfo::default(),
        );
        assert_eq!(v["platform"], "mqtt");
    }

    #[test]
    fn build_host_packages_discovery_config_payload_install() {
        let v = build_host_packages_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "myserver",
            HostOsInfo::default(),
        );
        assert_eq!(v["payload_install"], "install");
    }

    #[test]
    fn build_host_packages_discovery_config_name_is_null() {
        let v = build_host_packages_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "myserver",
            HostOsInfo::default(),
        );
        assert!(v["name"].is_null());
    }

    #[test]
    fn build_host_packages_discovery_config_title_is_packages_update() {
        let v = build_host_packages_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "myserver",
            HostOsInfo::default(),
        );
        assert_eq!(v["title"], "Packages Update");
    }

    #[test]
    fn build_host_packages_discovery_config_default_entity_id() {
        let v = build_host_packages_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "My Server",
            HostOsInfo::default(),
        );
        assert_eq!(v["default_entity_id"], "update.uptrakit_my_server_packages");
    }

    #[test]
    fn build_host_packages_discovery_config_state_topic_correct() {
        let v = build_host_packages_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "h",
            HostOsInfo::default(),
        );
        let expected = host_packages_state_topic("uptrakit", host());
        assert_eq!(v["state_topic"], expected.as_str());
    }

    #[test]
    fn build_host_packages_discovery_config_device_identifiers() {
        let v = build_host_packages_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "h",
            HostOsInfo::default(),
        );
        let tenant_simple = tenant().simple().to_string();
        let host_simple = host().simple().to_string();
        let expected_id = format!("uptrakit_host_{tenant_simple}_{host_simple}");
        assert_eq!(v["device"]["identifiers"][0], expected_id.as_str());
    }

    #[test]
    fn build_host_packages_discovery_config_device_name_is_friendly_name() {
        let v = build_host_packages_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "pangolin",
            HostOsInfo::default(),
        );
        assert_eq!(v["device"]["name"], "pangolin");
    }

    #[test]
    fn build_host_packages_discovery_config_disabled_by_default() {
        let v = build_host_packages_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "myserver",
            HostOsInfo::default(),
        );
        assert_eq!(v["enabled_by_default"], false);
    }

    // -------------------------------------------------------------------------
    // build_host_security_discovery_config
    // -------------------------------------------------------------------------

    #[test]
    fn build_host_security_discovery_config_platform_mqtt() {
        let v = build_host_security_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "myserver",
            HostOsInfo::default(),
        );
        assert_eq!(v["platform"], "mqtt");
    }

    #[test]
    fn build_host_security_discovery_config_name_is_null() {
        let v = build_host_security_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "myserver",
            HostOsInfo::default(),
        );
        assert!(v["name"].is_null());
    }

    #[test]
    fn build_host_security_discovery_config_title_is_security_updates() {
        let v = build_host_security_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "myserver",
            HostOsInfo::default(),
        );
        assert_eq!(v["title"], "Security Updates");
    }

    #[test]
    fn build_host_security_discovery_config_disabled_by_default() {
        let v = build_host_security_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "myserver",
            HostOsInfo::default(),
        );
        assert_eq!(v["enabled_by_default"], false);
    }

    #[test]
    fn build_host_security_discovery_config_payload_install() {
        let v = build_host_security_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "myserver",
            HostOsInfo::default(),
        );
        assert_eq!(v["payload_install"], "install");
    }

    #[test]
    fn build_host_security_discovery_config_default_entity_id() {
        let v = build_host_security_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "My Server",
            HostOsInfo::default(),
        );
        assert_eq!(
            v["default_entity_id"],
            "update.uptrakit_my_server_security_updates"
        );
    }

    #[test]
    fn build_host_security_discovery_config_uses_security_command_topic() {
        let v = build_host_security_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "h",
            HostOsInfo::default(),
        );
        let expected = host_security_command_topic("uptrakit", host());
        assert_eq!(v["command_topic"], expected.as_str());
    }

    #[test]
    fn build_host_security_discovery_config_device_same_as_packages() {
        let sec = build_host_security_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "h",
            HostOsInfo::default(),
        );
        let pkg = build_host_packages_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "h",
            HostOsInfo::default(),
        );
        assert_eq!(sec["device"]["identifiers"], pkg["device"]["identifiers"]);
    }

    #[test]
    fn build_discovery_config_device_same_as_host_packages() {
        let sw = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "h",
            ReleaseInfo::default(),
            HostOsInfo::default(),
        );
        let pkg = build_host_packages_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "h",
            HostOsInfo::default(),
        );
        assert_eq!(sw["device"]["identifiers"], pkg["device"]["identifiers"]);
    }

    // -------------------------------------------------------------------------
    // build_host_connectivity_discovery_config
    // -------------------------------------------------------------------------

    #[test]
    fn build_host_connectivity_discovery_config_platform_mqtt() {
        let v = build_host_connectivity_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "myserver",
            HostOsInfo::default(),
        );
        assert_eq!(v["platform"], "mqtt");
    }

    #[test]
    fn build_host_connectivity_discovery_config_device_class_connectivity() {
        let v = build_host_connectivity_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "myserver",
            HostOsInfo::default(),
        );
        assert_eq!(v["device_class"], "connectivity");
    }

    #[test]
    fn build_host_connectivity_discovery_config_enabled_by_default() {
        let v = build_host_connectivity_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "myserver",
            HostOsInfo::default(),
        );
        assert_eq!(v["enabled_by_default"], true);
    }

    #[test]
    fn build_host_connectivity_discovery_config_payloads() {
        let v = build_host_connectivity_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "myserver",
            HostOsInfo::default(),
        );
        assert_eq!(v["payload_on"], "online");
        assert_eq!(v["payload_off"], "offline");
    }

    #[test]
    fn build_host_connectivity_discovery_config_name_includes_agent() {
        let v = build_host_connectivity_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "myserver",
            HostOsInfo::default(),
        );
        assert_eq!(v["name"], "myserver agent");
    }

    #[test]
    fn build_host_connectivity_discovery_config_default_entity_id() {
        let v = build_host_connectivity_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "My Server",
            HostOsInfo::default(),
        );
        assert_eq!(
            v["default_entity_id"],
            "binary_sensor.uptrakit_my_server_agent"
        );
    }

    #[test]
    fn build_host_connectivity_discovery_config_device_same_as_packages() {
        let conn = build_host_connectivity_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "h",
            HostOsInfo::default(),
        );
        let pkg = build_host_packages_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "h",
            HostOsInfo::default(),
        );
        assert_eq!(conn["device"]["identifiers"], pkg["device"]["identifiers"]);
    }

    #[test]
    fn build_host_connectivity_discovery_config_device_model_from_os_type() {
        let v = build_host_connectivity_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "myserver",
            HostOsInfo {
                os_type: Some("Linux"),
                os_version: None,
                architecture: None,
            },
        );
        assert_eq!(v["device"]["model"], "Linux");
    }

    #[test]
    fn build_host_connectivity_discovery_config_device_sw_version_from_os_version() {
        let v = build_host_connectivity_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "myserver",
            HostOsInfo {
                os_type: None,
                os_version: Some("Ubuntu 24.04"),
                architecture: None,
            },
        );
        assert_eq!(v["device"]["sw_version"], "Ubuntu 24.04");
    }

    #[test]
    fn build_host_connectivity_discovery_config_device_hw_version_from_architecture() {
        let v = build_host_connectivity_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "myserver",
            HostOsInfo {
                os_type: None,
                os_version: None,
                architecture: Some("x86_64"),
            },
        );
        assert_eq!(v["device"]["hw_version"], "x86_64");
    }

    #[test]
    fn build_host_connectivity_discovery_config_device_model_absent_when_none() {
        let v = build_host_connectivity_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "myserver",
            HostOsInfo::default(),
        );
        assert!(v["device"].get("model").is_none());
    }

    // -------------------------------------------------------------------------
    // HostOsInfo device block enrichment
    // -------------------------------------------------------------------------

    #[test]
    fn build_discovery_config_device_includes_model_when_os_type_present() {
        let v = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "myhost",
            ReleaseInfo::default(),
            HostOsInfo {
                os_type: Some("Linux"),
                os_version: None,
                architecture: None,
            },
        );
        assert_eq!(v["device"]["model"], "Linux");
        assert!(v["device"].get("sw_version").is_none() || v["device"]["sw_version"].is_null());
    }

    #[test]
    fn build_discovery_config_device_no_model_when_os_type_none() {
        let v = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "myhost",
            ReleaseInfo::default(),
            HostOsInfo::default(),
        );
        assert!(v["device"].get("model").is_none());
    }

    #[test]
    fn build_host_packages_discovery_config_device_includes_os_info() {
        let v = build_host_packages_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "myserver",
            HostOsInfo {
                os_type: Some("Linux"),
                os_version: Some("Ubuntu 24.04"),
                architecture: Some("x86_64"),
            },
        );
        assert_eq!(v["device"]["model"], "Linux");
        assert_eq!(v["device"]["sw_version"], "Ubuntu 24.04");
        assert_eq!(v["device"]["hw_version"], "x86_64");
    }

    #[test]
    fn build_host_security_discovery_config_device_includes_os_info() {
        let v = build_host_security_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "myserver",
            HostOsInfo {
                os_type: Some("Linux"),
                os_version: Some("Ubuntu 24.04"),
                architecture: Some("x86_64"),
            },
        );
        assert_eq!(v["device"]["model"], "Linux");
        assert_eq!(v["device"]["sw_version"], "Ubuntu 24.04");
        assert_eq!(v["device"]["hw_version"], "x86_64");
    }
}
