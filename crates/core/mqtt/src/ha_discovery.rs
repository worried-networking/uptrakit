//! Pure helper functions for Home Assistant MQTT discovery.
//!
//! All functions are stateless and produce deterministic results from their
//! inputs. No async, no I/O.

use uuid::Uuid;

/// Returns the HA MQTT discovery config topic for an `update` entity.
///
/// Format: `{ha_prefix}/update/{unique_id}/config`
///
/// # Examples
///
/// ```
/// # use uptrakit_mqtt::ha_discovery::discovery_config_topic;
/// let topic = discovery_config_topic("homeassistant", "uptrakit_abc_def_ghi");
/// assert_eq!(topic, "homeassistant/update/uptrakit_abc_def_ghi/config");
/// ```
pub fn discovery_config_topic(ha_prefix: &str, unique_id: &str) -> String {
    format!("{ha_prefix}/update/{unique_id}/config")
}

/// Returns the topic carrying the installed version of a `(software_item, host)` pair.
///
/// Format: `{topic_prefix}/update/{item_id}/{host_id}/state`
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt::ha_discovery::state_topic;
/// let item_id = Uuid::nil();
/// let host_id = Uuid::nil();
/// let topic = state_topic("uptrakit", item_id, host_id);
/// assert!(topic.ends_with("/state"));
/// ```
pub fn state_topic(topic_prefix: &str, item_id: Uuid, host_id: Uuid) -> String {
    format!("{topic_prefix}/update/{item_id}/{host_id}/state")
}

/// Returns the topic carrying the latest available version.
///
/// Format: `{topic_prefix}/update/{item_id}/{host_id}/latest_version`
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt::ha_discovery::latest_version_topic;
/// let item_id = Uuid::nil();
/// let host_id = Uuid::nil();
/// let topic = latest_version_topic("uptrakit", item_id, host_id);
/// assert!(topic.ends_with("/latest_version"));
/// ```
pub fn latest_version_topic(topic_prefix: &str, item_id: Uuid, host_id: Uuid) -> String {
    format!("{topic_prefix}/update/{item_id}/{host_id}/latest_version")
}

/// Returns the MQTT command topic that HA publishes `"install"` to.
///
/// Format: `{topic_prefix}/update/{item_id}/{host_id}/set`
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt::ha_discovery::command_topic;
/// let item_id = Uuid::nil();
/// let host_id = Uuid::nil();
/// let topic = command_topic("uptrakit", item_id, host_id);
/// assert!(topic.ends_with("/set"));
/// ```
pub fn command_topic(topic_prefix: &str, item_id: Uuid, host_id: Uuid) -> String {
    format!("{topic_prefix}/update/{item_id}/{host_id}/set")
}

/// Returns a unique ID string for this `(tenant, software_item, host)` triple.
///
/// Format: `uptrakit_{tenant_id_no_dashes}_{item_id_no_dashes}_{host_id_no_dashes}`
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt::ha_discovery::unique_id;
/// let tenant_id = Uuid::nil();
/// let item_id   = Uuid::nil();
/// let host_id   = Uuid::nil();
/// let uid = unique_id(tenant_id, item_id, host_id);
/// assert!(uid.starts_with("uptrakit_"));
/// assert!(!uid.contains('-'));
/// ```
pub fn unique_id(tenant_id: Uuid, item_id: Uuid, host_id: Uuid) -> String {
    let t = tenant_id.simple();
    let i = item_id.simple();
    let h = host_id.simple();
    format!("uptrakit_{t}_{i}_{h}")
}

/// Build the HA MQTT discovery JSON for an `update` entity.
///
/// Each software item is represented as a distinct HA device (identified by
/// `uptrakit_{tenant_id}_{item_id}`), named after the software item. Entities
/// within that device are named after the hostname. An explicit
/// `default_entity_id` in the form `{item_slug}_on_{host_slug}` is included
/// so that HA uses a stable, human-readable entity ID on first registration,
/// independent of the entity name.
///
/// Returns a [`serde_json::Value`] that should be serialized with `to_string()`
/// and published (retained) on `discovery_config_topic(...)`.
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt::ha_discovery::build_discovery_config;
/// let v = build_discovery_config(
///     "homeassistant",
///     "uptrakit",
///     Uuid::nil(),
///     Uuid::nil(),
///     Uuid::nil(),
///     "My App",
///     "myhost",
/// );
/// assert_eq!(v["platform"], "mqtt");
/// assert_eq!(v["payload_install"], "install");
/// ```
pub fn build_discovery_config(
    _ha_prefix: &str,
    topic_prefix: &str,
    tenant_id: Uuid,
    item_id: Uuid,
    host_id: Uuid,
    item_name: &str,
    hostname: &str,
) -> serde_json::Value {
    let uid = unique_id(tenant_id, item_id, host_id);
    let tenant_simple = tenant_id.simple().to_string();
    let item_simple = item_id.simple().to_string();
    let default_entity_id = format!("uptrakit_{}_on_{}", slugify(item_name), slugify(hostname));

    serde_json::json!({
        "platform": "mqtt",
        "unique_id": uid,
        "name": hostname,
        "default_entity_id": default_entity_id,
        "state_topic": state_topic(topic_prefix, item_id, host_id),
        "latest_version_topic": latest_version_topic(topic_prefix, item_id, host_id),
        "command_topic": command_topic(topic_prefix, item_id, host_id),
        "payload_install": "install",
        "availability_topic": format!("{topic_prefix}/status"),
        "payload_available": "online",
        "payload_not_available": "offline",
        "device": {
            "identifiers": [format!("uptrakit_{tenant_simple}_{item_simple}")],
            "name": item_name,
            "manufacturer": "Uptrakit"
        }
    })
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
/// # use uptrakit_mqtt::ha_discovery::slugify;
/// assert_eq!(slugify("My App"), "my_app");
/// assert_eq!(slugify("pangolin.uk.home.yantsen.su"), "pangolin_uk_home_yantsen_su");
/// assert_eq!(slugify("foo--bar"), "foo_bar");
/// assert_eq!(slugify("  leading"), "leading");
/// ```
pub fn slugify(s: &str) -> String {
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

/// Try to parse a command topic back to `(item_id, host_id)`.
///
/// Returns `None` if the topic doesn't match
/// `{prefix}/update/{uuid}/{uuid}/set`.
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt::ha_discovery::{command_topic, parse_command_topic};
/// let item_id = Uuid::nil();
/// let host_id = Uuid::nil();
/// let topic = command_topic("uptrakit", item_id, host_id);
/// let parsed = parse_command_topic("uptrakit", &topic);
/// assert_eq!(parsed, Some((item_id, host_id)));
///
/// // Non-matching topic returns None.
/// assert!(parse_command_topic("uptrakit", "uptrakit/update/bad/set").is_none());
/// ```
pub fn parse_command_topic(topic_prefix: &str, topic: &str) -> Option<(Uuid, Uuid)> {
    // Expected: "{prefix}/update/{uuid}/{uuid}/set"
    let prefix = format!("{topic_prefix}/update/");
    let rest = topic.strip_prefix(prefix.as_str())?;
    let rest = rest.strip_suffix("/set")?;

    // rest should now be "{uuid}/{uuid}"
    let (item_str, host_str) = rest.split_once('/')?;
    // Make sure there are no further slashes (i.e. exactly two UUID segments).
    if host_str.contains('/') {
        return None;
    }

    let item_id = Uuid::parse_str(item_str).ok()?;
    let host_id = Uuid::parse_str(host_str).ok()?;
    Some((item_id, host_id))
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
            "homeassistant/update/uptrakit_abc_def_ghi/config"
        );
    }

    #[test]
    fn discovery_config_topic_custom_prefix() {
        assert_eq!(
            discovery_config_topic("ha", "uid123"),
            "ha/update/uid123/config"
        );
    }

    // -------------------------------------------------------------------------
    // state_topic
    // -------------------------------------------------------------------------

    #[test]
    fn state_topic_format() {
        let t = state_topic("uptrakit", item(), host());
        assert_eq!(
            t,
            "uptrakit/update/22222222-2222-2222-2222-222222222222/33333333-3333-3333-3333-333333333333/state"
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
            "uptrakit/update/22222222-2222-2222-2222-222222222222/33333333-3333-3333-3333-333333333333/latest_version"
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
            "uptrakit/update/22222222-2222-2222-2222-222222222222/33333333-3333-3333-3333-333333333333/set"
        );
    }

    #[test]
    fn command_topic_ends_with_set() {
        assert!(command_topic("pfx", item(), host()).ends_with("/set"));
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
        // The only underscores should be the separators; no hyphens from UUIDs.
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
        // All-zero UUIDs produce all-zero simple strings.
        let zero = Uuid::nil();
        let uid = unique_id(zero, zero, zero);
        assert_eq!(
            uid,
            "uptrakit_00000000000000000000000000000000_00000000000000000000000000000000_00000000000000000000000000000000"
        );
    }

    // -------------------------------------------------------------------------
    // build_discovery_config
    // -------------------------------------------------------------------------

    #[test]
    fn build_discovery_config_platform_mqtt() {
        let v = build_discovery_config(
            "homeassistant",
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "myhost",
        );
        assert_eq!(v["platform"], "mqtt");
    }

    #[test]
    fn build_discovery_config_unique_id_matches() {
        let v = build_discovery_config(
            "homeassistant",
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "myhost",
        );
        let expected_uid = unique_id(tenant(), item(), host());
        assert_eq!(v["unique_id"], expected_uid.as_str());
    }

    #[test]
    fn build_discovery_config_name_is_hostname() {
        let v = build_discovery_config(
            "homeassistant",
            "uptrakit",
            tenant(),
            item(),
            host(),
            "MyApp",
            "server1",
        );
        assert_eq!(v["name"], "server1");
    }

    #[test]
    fn build_discovery_config_state_topic_correct() {
        let v = build_discovery_config(
            "homeassistant",
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "h",
        );
        let expected = state_topic("uptrakit", item(), host());
        assert_eq!(v["state_topic"], expected.as_str());
    }

    #[test]
    fn build_discovery_config_latest_version_topic_correct() {
        let v = build_discovery_config(
            "homeassistant",
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "h",
        );
        let expected = latest_version_topic("uptrakit", item(), host());
        assert_eq!(v["latest_version_topic"], expected.as_str());
    }

    #[test]
    fn build_discovery_config_command_topic_correct() {
        let v = build_discovery_config(
            "homeassistant",
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "h",
        );
        let expected = command_topic("uptrakit", item(), host());
        assert_eq!(v["command_topic"], expected.as_str());
    }

    #[test]
    fn build_discovery_config_payload_install() {
        let v = build_discovery_config(
            "homeassistant",
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "h",
        );
        assert_eq!(v["payload_install"], "install");
    }

    #[test]
    fn build_discovery_config_availability_topic() {
        let v = build_discovery_config(
            "homeassistant",
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "h",
        );
        assert_eq!(v["availability_topic"], "uptrakit/status");
    }

    #[test]
    fn build_discovery_config_availability_payloads() {
        let v = build_discovery_config(
            "homeassistant",
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "h",
        );
        assert_eq!(v["payload_available"], "online");
        assert_eq!(v["payload_not_available"], "offline");
    }

    #[test]
    fn build_discovery_config_device_identifiers() {
        let v = build_discovery_config(
            "homeassistant",
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "h",
        );
        let tenant_simple = tenant().simple().to_string();
        let item_simple = item().simple().to_string();
        let expected_id = format!("uptrakit_{tenant_simple}_{item_simple}");
        assert_eq!(v["device"]["identifiers"][0], expected_id.as_str());
    }

    #[test]
    fn build_discovery_config_device_name_is_item_name() {
        let v = build_discovery_config(
            "homeassistant",
            "uptrakit",
            tenant(),
            item(),
            host(),
            "My App",
            "h",
        );
        assert_eq!(v["device"]["name"], "My App");
        assert_eq!(v["device"]["manufacturer"], "Uptrakit");
    }

    #[test]
    fn build_discovery_config_default_entity_id() {
        let v = build_discovery_config(
            "homeassistant",
            "uptrakit",
            tenant(),
            item(),
            host(),
            "uptrakit pangolin",
            "pangolin.uk.home.yantsen.su",
        );
        assert_eq!(
            v["default_entity_id"],
            "uptrakit_pangolin_on_pangolin_uk_home_yantsen_su"
        );
    }

    #[test]
    fn build_discovery_config_default_entity_id_simple_names() {
        let v = build_discovery_config(
            "homeassistant",
            "uptrakit",
            tenant(),
            item(),
            host(),
            "MyApp",
            "server1",
        );
        assert_eq!(v["default_entity_id"], "myapp_on_server1");
    }

    #[test]
    fn build_discovery_config_serializes_to_valid_json() {
        let v = build_discovery_config(
            "homeassistant",
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "h",
        );
        let s = v.to_string();
        let reparsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(reparsed["platform"], "mqtt");
    }

    // -------------------------------------------------------------------------
    // parse_command_topic
    // -------------------------------------------------------------------------

    #[test]
    fn parse_command_topic_roundtrip() {
        let topic = command_topic("uptrakit", item(), host());
        let parsed = parse_command_topic("uptrakit", &topic);
        assert_eq!(parsed, Some((item(), host())));
    }

    #[test]
    fn parse_command_topic_wrong_suffix() {
        let topic = state_topic("uptrakit", item(), host()); // ends with /state not /set
        assert!(parse_command_topic("uptrakit", &topic).is_none());
    }

    #[test]
    fn parse_command_topic_wrong_prefix() {
        let topic = command_topic("uptrakit", item(), host());
        assert!(parse_command_topic("other", &topic).is_none());
    }

    #[test]
    fn parse_command_topic_invalid_uuid() {
        let topic = "uptrakit/update/not-a-uuid/not-a-uuid/set";
        assert!(parse_command_topic("uptrakit", topic).is_none());
    }

    #[test]
    fn parse_command_topic_too_many_segments() {
        let topic = format!("uptrakit/update/{}/{}/extra/set", item(), host());
        assert!(parse_command_topic("uptrakit", &topic).is_none());
    }

    #[test]
    fn parse_command_topic_missing_host_segment() {
        let topic = format!("uptrakit/update/{}/set", item());
        assert!(parse_command_topic("uptrakit", &topic).is_none());
    }

    #[test]
    fn parse_command_topic_nil_uuids() {
        let zero = Uuid::nil();
        let topic = command_topic("pfx", zero, zero);
        let parsed = parse_command_topic("pfx", &topic);
        assert_eq!(parsed, Some((zero, zero)));
    }

    #[test]
    fn parse_command_topic_empty_string() {
        assert!(parse_command_topic("uptrakit", "").is_none());
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
        assert_eq!(slugify("pangolin.uk.home.yantsen.su"), "pangolin_uk_home_yantsen_su");
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
        assert_eq!(
            slugify("uptrakit pangolin"),
            "uptrakit_pangolin"
        );
        assert_eq!(
            slugify("pangolin.uk.home.yantsen.su"),
            "pangolin_uk_home_yantsen_su"
        );
    }
}
