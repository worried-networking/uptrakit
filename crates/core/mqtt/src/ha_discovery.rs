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

/// Returns the MQTT topic for HA JSON attributes of a `(software_item, host)` pair.
///
/// Published as a retained JSON payload. The recognized attribute is
/// `"in_progress"` (bool): `true` while an update is pending or running,
/// `false` when idle.
///
/// Format: `{topic_prefix}/update/{item_id}/{host_id}/attributes`
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt::ha_discovery::json_attributes_topic;
/// let item_id = Uuid::nil();
/// let host_id = Uuid::nil();
/// let topic = json_attributes_topic("uptrakit", item_id, host_id);
/// assert!(topic.ends_with("/attributes"));
/// ```
pub fn json_attributes_topic(topic_prefix: &str, item_id: Uuid, host_id: Uuid) -> String {
    format!("{topic_prefix}/update/{item_id}/{host_id}/attributes")
}

/// Build the JSON attributes payload for a `(software_item, host)` entity.
///
/// Returns a [`serde_json::Value`] that should be serialized and published
/// (retained) to [`json_attributes_topic`].
///
/// The `in_progress` attribute is recognized by Home Assistant's `update`
/// entity:
/// - `false` — no update running (idle).
/// - `true` — an update is pending dispatch or executing on the agent
///   (displays a spinner in the HA UI).
///
/// # Examples
///
/// ```
/// # use uptrakit_mqtt::ha_discovery::build_attributes_payload;
/// let payload = build_attributes_payload(true);
/// assert_eq!(payload["in_progress"], true);
///
/// let payload = build_attributes_payload(false);
/// assert_eq!(payload["in_progress"], false);
/// ```
pub fn build_attributes_payload(in_progress: bool) -> serde_json::Value {
    serde_json::json!({ "in_progress": in_progress })
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

/// Optional upstream release metadata included in a discovery config.
///
/// Passed to [`build_discovery_config`] to include release page links and
/// changelog snippets in the HA MQTT discovery payload.
#[derive(Debug, Default, Clone)]
pub struct ReleaseInfo<'a> {
    /// URL to the upstream release page (e.g. a GitHub release).
    pub url: Option<&'a str>,
    /// Full release notes or changelog text.
    ///
    /// Truncated to 500 Unicode characters when written to the discovery
    /// config (`release_summary`).
    pub notes: Option<&'a str>,
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
/// When `release.url` is provided it is included verbatim as `release_url`.
/// When `release.notes` is provided, the first 500 characters are included
/// as `release_summary` (truncated at a character boundary to keep the MQTT
/// payload small).
///
/// Pass `ReleaseInfo::default()` (or `ReleaseInfo { url: None, notes: None }`)
/// when no release metadata is available.
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt::ha_discovery::{build_discovery_config, ReleaseInfo};
/// let v = build_discovery_config(
///     "uptrakit",
///     Uuid::nil(),
///     Uuid::nil(),
///     Uuid::nil(),
///     "My App",
///     "myhost",
///     ReleaseInfo::default(),
/// );
/// assert_eq!(v["platform"], "mqtt");
/// assert_eq!(v["payload_install"], "install");
/// ```
pub fn build_discovery_config(
    topic_prefix: &str,
    tenant_id: Uuid,
    item_id: Uuid,
    host_id: Uuid,
    item_name: &str,
    hostname: &str,
    release: ReleaseInfo<'_>,
) -> serde_json::Value {
    let uid = unique_id(tenant_id, item_id, host_id);
    let tenant_simple = tenant_id.simple().to_string();
    let item_simple = item_id.simple().to_string();
    let default_entity_id = format!("uptrakit_{}_on_{}", slugify(item_name), slugify(hostname));

    let mut config = serde_json::json!({
        "platform": "mqtt",
        "unique_id": uid,
        "name": hostname,
        "default_entity_id": default_entity_id,
        "state_topic": state_topic(topic_prefix, item_id, host_id),
        "latest_version_topic": latest_version_topic(topic_prefix, item_id, host_id),
        "command_topic": command_topic(topic_prefix, item_id, host_id),
        "payload_install": "install",
        "json_attributes_topic": json_attributes_topic(topic_prefix, item_id, host_id),
        "availability_topic": format!("{topic_prefix}/status"),
        "payload_available": "online",
        "payload_not_available": "offline",
        "device": {
            "identifiers": [format!("uptrakit_{tenant_simple}_{item_simple}")],
            "name": item_name,
            "manufacturer": "Uptrakit"
        }
    });

    if let Some(url) = release.url {
        config["release_url"] = serde_json::Value::String(url.to_string());
    }
    if let Some(notes) = release.notes {
        config["release_summary"] =
            serde_json::Value::String(truncate_str(notes, 500).to_string());
    }

    config
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

// =============================================================================
// Host package entity helpers
// =============================================================================

/// Returns the MQTT topic carrying the installed-version string (state) for a
/// host's package entity.
///
/// Format: `{prefix}/hosts/{host_id}/state`
///
/// The published value is `"{N} updates pending"` when `pending_count > 0`, or
/// `"up-to-date"` when all packages are current.
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt::ha_discovery::host_packages_state_topic;
/// let topic = host_packages_state_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/state"));
/// ```
pub fn host_packages_state_topic(prefix: &str, host_id: Uuid) -> String {
    format!("{prefix}/hosts/{host_id}/state")
}

/// Returns the MQTT topic carrying the latest-version string for a host's
/// package entity.
///
/// Format: `{prefix}/hosts/{host_id}/latest_version`
///
/// The published value is always `"up-to-date"`. Home Assistant compares this
/// against the state topic to determine whether an update badge is shown.
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt::ha_discovery::host_packages_latest_version_topic;
/// let topic = host_packages_latest_version_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/latest_version"));
/// ```
pub fn host_packages_latest_version_topic(prefix: &str, host_id: Uuid) -> String {
    format!("{prefix}/hosts/{host_id}/latest_version")
}

/// Returns the MQTT topic for HA JSON attributes of a host's package entity.
///
/// Format: `{prefix}/hosts/{host_id}/attributes`
///
/// Published as a retained JSON payload with fields:
/// - `"in_progress"` (bool) — whether a batch update is pending/running
/// - `"pending_count"` (u32) — number of packages with available updates
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt::ha_discovery::host_packages_json_attributes_topic;
/// let topic = host_packages_json_attributes_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/attributes"));
/// ```
pub fn host_packages_json_attributes_topic(prefix: &str, host_id: Uuid) -> String {
    format!("{prefix}/hosts/{host_id}/attributes")
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
/// # use uptrakit_mqtt::ha_discovery::host_packages_command_topic;
/// let topic = host_packages_command_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/set"));
/// ```
pub fn host_packages_command_topic(prefix: &str, host_id: Uuid) -> String {
    format!("{prefix}/hosts/{host_id}/set")
}

/// Returns the HA MQTT discovery config topic for a host's package `update`
/// entity.
///
/// Format: `{ha_prefix}/update/uptrakit_pkgs_{t}_{h}/config`
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt::ha_discovery::host_packages_discovery_config_topic;
/// let topic = host_packages_discovery_config_topic("homeassistant", Uuid::nil(), Uuid::nil());
/// assert!(topic.starts_with("homeassistant/update/"));
/// assert!(topic.ends_with("/config"));
/// ```
pub fn host_packages_discovery_config_topic(
    ha_prefix: &str,
    tenant_id: Uuid,
    host_id: Uuid,
) -> String {
    let uid = host_packages_unique_id(tenant_id, host_id);
    format!("{ha_prefix}/update/{uid}/config")
}

/// Returns a unique ID string for a host's package entity.
///
/// Format: `uptrakit_pkgs_{tenant_id_no_dashes}_{host_id_no_dashes}`
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt::ha_discovery::host_packages_unique_id;
/// let uid = host_packages_unique_id(Uuid::nil(), Uuid::nil());
/// assert!(uid.starts_with("uptrakit_pkgs_"));
/// assert!(!uid.contains('-'));
/// ```
pub fn host_packages_unique_id(tenant_id: Uuid, host_id: Uuid) -> String {
    let t = tenant_id.simple();
    let h = host_id.simple();
    format!("uptrakit_pkgs_{t}_{h}")
}

/// Build the JSON attributes payload for a host's package entity.
///
/// Returns a [`serde_json::Value`] to be serialized and published (retained)
/// to [`host_packages_json_attributes_topic`].
///
/// # Examples
///
/// ```
/// # use uptrakit_mqtt::ha_discovery::build_host_packages_attributes_payload;
/// let payload = build_host_packages_attributes_payload(true, 3);
/// assert_eq!(payload["in_progress"], true);
/// assert_eq!(payload["pending_count"], 3u32);
/// ```
pub fn build_host_packages_attributes_payload(
    in_progress: bool,
    pending_count: u32,
) -> serde_json::Value {
    serde_json::json!({
        "in_progress": in_progress,
        "pending_count": pending_count
    })
}

/// Build the HA MQTT discovery JSON for a host's package `update` entity.
///
/// Publishes a single entity per host that represents the overall package update
/// status. The device is identified by the host (not the software item). Home
/// Assistant displays an update badge when `installed_version != latest_version`,
/// i.e. when `pending_count > 0`.
///
/// The returned JSON should be published (retained) on
/// [`host_packages_discovery_config_topic`].
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt::ha_discovery::build_host_packages_discovery_config;
/// let v = build_host_packages_discovery_config(
///     "uptrakit",
///     Uuid::nil(),
///     Uuid::nil(),
///     "myserver",
/// );
/// assert_eq!(v["name"], "myserver packages");
/// assert_eq!(v["platform"], "mqtt");
/// assert_eq!(v["payload_install"], "install");
/// ```
pub fn build_host_packages_discovery_config(
    topic_prefix: &str,
    tenant_id: Uuid,
    host_id: Uuid,
    hostname: &str,
) -> serde_json::Value {
    let uid = host_packages_unique_id(tenant_id, host_id);
    let host_simple = host_id.simple().to_string();
    let tenant_simple = tenant_id.simple().to_string();
    let default_entity_id = format!("{}_packages", slugify(hostname));

    serde_json::json!({
        "platform": "mqtt",
        "unique_id": uid,
        "name": format!("{hostname} packages"),
        "default_entity_id": default_entity_id,
        "state_topic": host_packages_state_topic(topic_prefix, host_id),
        "latest_version_topic": host_packages_latest_version_topic(topic_prefix, host_id),
        "command_topic": host_packages_command_topic(topic_prefix, host_id),
        "payload_install": "install",
        "json_attributes_topic": host_packages_json_attributes_topic(topic_prefix, host_id),
        "availability_topic": format!("{topic_prefix}/status"),
        "payload_available": "online",
        "payload_not_available": "offline",
        "device": {
            "identifiers": [format!("uptrakit_host_{tenant_simple}_{host_simple}")],
            "name": hostname,
            "manufacturer": "Uptrakit"
        }
    })
}

/// Try to parse a host packages command topic back to the `host_id`.
///
/// Returns `None` if the topic doesn't match `{prefix}/hosts/{uuid}/set`.
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt::ha_discovery::{host_packages_command_topic, parse_host_packages_command_topic};
/// let host_id = Uuid::nil();
/// let topic = host_packages_command_topic("uptrakit", host_id);
/// let parsed = parse_host_packages_command_topic("uptrakit", &topic);
/// assert_eq!(parsed, Some(host_id));
///
/// // Non-matching topic returns None.
/// assert!(parse_host_packages_command_topic("uptrakit", "uptrakit/update/bad/set").is_none());
/// ```
pub fn parse_host_packages_command_topic(topic_prefix: &str, topic: &str) -> Option<Uuid> {
    // Expected: "{prefix}/hosts/{uuid}/set"
    let prefix = format!("{topic_prefix}/hosts/");
    let rest = topic.strip_prefix(prefix.as_str())?;
    let rest = rest.strip_suffix("/set")?;

    // rest should now be just a UUID with no slashes.
    if rest.contains('/') {
        return None;
    }

    Uuid::parse_str(rest).ok()
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
    // json_attributes_topic
    // -------------------------------------------------------------------------

    #[test]
    fn json_attributes_topic_format() {
        let t = json_attributes_topic("uptrakit", item(), host());
        assert_eq!(
            t,
            "uptrakit/update/22222222-2222-2222-2222-222222222222/33333333-3333-3333-3333-333333333333/attributes"
        );
    }

    #[test]
    fn json_attributes_topic_ends_with_attributes() {
        assert!(json_attributes_topic("pfx", item(), host()).ends_with("/attributes"));
    }

    #[test]
    fn json_attributes_topic_custom_prefix() {
        let t = json_attributes_topic("home/uptrakit", item(), host());
        assert!(t.starts_with("home/uptrakit/update/"));
        assert!(t.ends_with("/attributes"));
    }

    // -------------------------------------------------------------------------
    // build_attributes_payload
    // -------------------------------------------------------------------------

    #[test]
    fn build_attributes_payload_in_progress_true() {
        let payload = build_attributes_payload(true);
        assert_eq!(payload["in_progress"], true);
    }

    #[test]
    fn build_attributes_payload_in_progress_false() {
        let payload = build_attributes_payload(false);
        assert_eq!(payload["in_progress"], false);
    }

    #[test]
    fn build_attributes_payload_is_valid_json() {
        let payload = build_attributes_payload(true);
        let s = payload.to_string();
        let reparsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(reparsed["in_progress"], true);
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
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "myhost",
            ReleaseInfo::default(),
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
        );
        let expected_uid = unique_id(tenant(), item(), host());
        assert_eq!(v["unique_id"], expected_uid.as_str());
    }

    #[test]
    fn build_discovery_config_name_is_hostname() {
        let v = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "MyApp",
            "server1",
            ReleaseInfo::default(),
        );
        assert_eq!(v["name"], "server1");
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
        );
        assert_eq!(v["availability_topic"], "uptrakit/status");
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
        );
        assert_eq!(v["payload_available"], "online");
        assert_eq!(v["payload_not_available"], "offline");
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
        );
        let tenant_simple = tenant().simple().to_string();
        let item_simple = item().simple().to_string();
        let expected_id = format!("uptrakit_{tenant_simple}_{item_simple}");
        assert_eq!(v["device"]["identifiers"][0], expected_id.as_str());
    }

    #[test]
    fn build_discovery_config_device_name_is_item_name() {
        let v = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "My App",
            "h",
            ReleaseInfo::default(),
        );
        assert_eq!(v["device"]["name"], "My App");
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
        );
        assert_eq!(
            v["default_entity_id"],
            "uptrakit_uptrakit_pangolin_on_pangolin_uk_home_yantsen_su"
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
        );
        assert_eq!(v["default_entity_id"], "uptrakit_myapp_on_server1");
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
        );
        let s = v.to_string();
        let reparsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(reparsed["platform"], "mqtt");
    }

    // -------------------------------------------------------------------------
    // build_discovery_config — release metadata
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
            ReleaseInfo { url: Some(url), notes: None },
        );
        assert_eq!(v["release_url"], url);
        assert!(v.get("release_summary").is_none());
    }

    #[test]
    fn build_discovery_config_with_release_summary() {
        let notes = "## What's New\n- Feature A\n- Bug fix B";
        let v = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "h",
            ReleaseInfo { url: None, notes: Some(notes) },
        );
        assert_eq!(v["release_summary"], notes);
        assert!(v.get("release_url").is_none());
    }

    #[test]
    fn build_discovery_config_release_summary_truncated_at_500_chars() {
        // Build a string of 600 ASCII characters.
        let notes: String = "a".repeat(600);
        let v = build_discovery_config(
            "uptrakit",
            tenant(),
            item(),
            host(),
            "App",
            "h",
            ReleaseInfo { url: None, notes: Some(&notes) },
        );
        let summary = v["release_summary"].as_str().unwrap();
        assert_eq!(summary.len(), 500);
        assert_eq!(summary, &notes[..500]);
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
        );
        assert!(v.get("release_url").is_none());
        assert!(v.get("release_summary").is_none());
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
        assert_eq!(
            t,
            "uptrakit/hosts/33333333-3333-3333-3333-333333333333/set"
        );
    }

    // -------------------------------------------------------------------------
    // host_packages_unique_id
    // -------------------------------------------------------------------------

    #[test]
    fn host_packages_unique_id_starts_with_uptrakit_pkgs() {
        assert!(host_packages_unique_id(tenant(), host()).starts_with("uptrakit_pkgs_"));
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
    // build_host_packages_attributes_payload
    // -------------------------------------------------------------------------

    #[test]
    fn build_host_packages_attributes_payload_in_progress() {
        let payload = build_host_packages_attributes_payload(true, 5);
        assert_eq!(payload["in_progress"], true);
        assert_eq!(payload["pending_count"], 5u32);
    }

    #[test]
    fn build_host_packages_attributes_payload_idle() {
        let payload = build_host_packages_attributes_payload(false, 0);
        assert_eq!(payload["in_progress"], false);
        assert_eq!(payload["pending_count"], 0u32);
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
        );
        assert_eq!(v["payload_install"], "install");
    }

    #[test]
    fn build_host_packages_discovery_config_name_includes_packages() {
        let v = build_host_packages_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "myserver",
        );
        assert_eq!(v["name"], "myserver packages");
    }

    #[test]
    fn build_host_packages_discovery_config_default_entity_id() {
        let v = build_host_packages_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "My Server",
        );
        assert_eq!(v["default_entity_id"], "my_server_packages");
    }

    #[test]
    fn build_host_packages_discovery_config_state_topic_correct() {
        let v = build_host_packages_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "h",
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
        );
        let tenant_simple = tenant().simple().to_string();
        let host_simple = host().simple().to_string();
        let expected_id = format!("uptrakit_host_{tenant_simple}_{host_simple}");
        assert_eq!(v["device"]["identifiers"][0], expected_id.as_str());
    }

    #[test]
    fn build_host_packages_discovery_config_device_name_is_hostname() {
        let v = build_host_packages_discovery_config(
            "uptrakit",
            tenant(),
            host(),
            "pangolin",
        );
        assert_eq!(v["device"]["name"], "pangolin");
    }

    // -------------------------------------------------------------------------
    // parse_host_packages_command_topic
    // -------------------------------------------------------------------------

    #[test]
    fn parse_host_packages_command_topic_roundtrip() {
        let topic = host_packages_command_topic("uptrakit", host());
        let parsed = parse_host_packages_command_topic("uptrakit", &topic);
        assert_eq!(parsed, Some(host()));
    }

    #[test]
    fn parse_host_packages_command_topic_wrong_suffix() {
        let topic = host_packages_state_topic("uptrakit", host()); // ends with /state not /set
        assert!(parse_host_packages_command_topic("uptrakit", &topic).is_none());
    }

    #[test]
    fn parse_host_packages_command_topic_wrong_prefix() {
        let topic = host_packages_command_topic("uptrakit", host());
        assert!(parse_host_packages_command_topic("other", &topic).is_none());
    }

    #[test]
    fn parse_host_packages_command_topic_invalid_uuid() {
        let topic = "uptrakit/hosts/not-a-uuid/set";
        assert!(parse_host_packages_command_topic("uptrakit", topic).is_none());
    }

    #[test]
    fn parse_host_packages_command_topic_too_many_segments() {
        let topic = format!("uptrakit/hosts/{}/extra/set", host());
        assert!(parse_host_packages_command_topic("uptrakit", &topic).is_none());
    }

    #[test]
    fn parse_host_packages_command_topic_nil_uuid() {
        let zero = Uuid::nil();
        let topic = host_packages_command_topic("pfx", zero);
        let parsed = parse_host_packages_command_topic("pfx", &topic);
        assert_eq!(parsed, Some(zero));
    }

    #[test]
    fn parse_host_packages_command_topic_empty_string() {
        assert!(parse_host_packages_command_topic("uptrakit", "").is_none());
    }

    // host packages and software items use distinct prefixes — no confusion
    #[test]
    fn parse_host_packages_command_topic_does_not_match_software_item_topic() {
        // A software item command topic uses /update/ not /hosts/
        let sw_topic = command_topic("uptrakit", item(), host());
        assert!(parse_host_packages_command_topic("uptrakit", &sw_topic).is_none());
    }
}
