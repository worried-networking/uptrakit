//! Pure helper functions for Home Assistant MQTT discovery.
//!
//! All functions are stateless and produce deterministic results from their
//! inputs. No async, no I/O.

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
/// # use uptrakit_mqtt::ha_discovery::discovery_config_topic;
/// let topic = discovery_config_topic("homeassistant", "uptrakit_abc_def_ghi");
/// assert_eq!(topic, "homeassistant/update/uptrakit/abc_def_ghi/config");
/// ```
pub fn discovery_config_topic(ha_prefix: &str, unique_id: &str) -> String {
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
/// # use uptrakit_mqtt::ha_discovery::host_topic_prefix;
/// let prefix = host_topic_prefix("uptrakit", Uuid::nil());
/// assert!(prefix.starts_with("uptrakit/hosts/"));
/// ```
pub fn host_topic_prefix(topic_prefix: &str, host_id: Uuid) -> String {
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
/// # use uptrakit_mqtt::ha_discovery::state_topic;
/// let item_id = Uuid::nil();
/// let host_id = Uuid::nil();
/// let topic = state_topic("uptrakit", item_id, host_id);
/// assert!(topic.ends_with("/state"));
/// ```
pub fn state_topic(topic_prefix: &str, item_id: Uuid, host_id: Uuid) -> String {
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
/// # use uptrakit_mqtt::ha_discovery::latest_version_topic;
/// let item_id = Uuid::nil();
/// let host_id = Uuid::nil();
/// let topic = latest_version_topic("uptrakit", item_id, host_id);
/// assert!(topic.ends_with("/latest_version"));
/// ```
pub fn latest_version_topic(topic_prefix: &str, item_id: Uuid, host_id: Uuid) -> String {
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
/// # use uptrakit_mqtt::ha_discovery::command_topic;
/// let item_id = Uuid::nil();
/// let host_id = Uuid::nil();
/// let topic = command_topic("uptrakit", item_id, host_id);
/// assert!(topic.ends_with("/set"));
/// ```
pub fn command_topic(topic_prefix: &str, item_id: Uuid, host_id: Uuid) -> String {
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
/// # use uptrakit_mqtt::ha_discovery::json_attributes_topic;
/// let item_id = Uuid::nil();
/// let host_id = Uuid::nil();
/// let topic = json_attributes_topic("uptrakit", item_id, host_id);
/// assert!(topic.ends_with("/attributes"));
/// ```
pub fn json_attributes_topic(topic_prefix: &str, item_id: Uuid, host_id: Uuid) -> String {
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
/// # use uptrakit_mqtt::ha_discovery::hostname_topic;
/// let topic = hostname_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/hostname"));
/// ```
pub fn hostname_topic(topic_prefix: &str, host_id: Uuid) -> String {
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
/// # use uptrakit_mqtt::ha_discovery::friendly_name_topic;
/// let topic = friendly_name_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/friendly_name"));
/// ```
pub fn friendly_name_topic(topic_prefix: &str, host_id: Uuid) -> String {
    let hp = host_topic_prefix(topic_prefix, host_id);
    format!("{hp}/friendly_name")
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
/// Optional fields are omitted from the JSON when `None`:
/// - `update_category` — classification of the pending update
///   (`"security"`, `"bugfix"`, `"feature"`, or `"unknown"`).
/// - `release_date` — ISO 8601 date string of the latest release (`"2025-01-15"`).
/// - `last_checked_at` — ISO 8601 datetime when the version was last detected.
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
pub fn build_attributes_payload(
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
    let h = host_id.simple();
    let i = item_id.simple();
    format!("uptrakit_{t}_{h}_{i}")
}

/// OS information for enriching HA device blocks.
///
/// All fields are optional. When `None`, the corresponding device block field
/// is omitted so that Home Assistant merges the info from whichever entity
/// provides it first.
#[derive(Debug, Default, Clone, Copy)]
pub struct HostOsInfo<'a> {
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
pub struct ReleaseInfo<'a> {
    /// URL to the upstream release page (e.g. a GitHub release).
    pub url: Option<&'a str>,
    /// Full release notes or changelog text.
    ///
    /// Truncated to 500 Unicode characters when written to the discovery
    /// config (`release_summary`).
    pub notes: Option<&'a str>,
}

/// Build the HA device block JSON shared across all entity builders.
///
/// Uses the host-centric device identifier `uptrakit_host_{tenant}_{host}` so
/// every entity for a host groups under a single HA device. When `os_info`
/// fields are present the `model`, `sw_version`, and `hw_version` fields are
/// included.
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
/// # use uptrakit_mqtt::ha_discovery::{build_discovery_config, ReleaseInfo, HostOsInfo};
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
#[allow(clippy::too_many_arguments)]
pub fn build_discovery_config(
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
        slugify(friendly_name),
        slugify(item_name)
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
        "availability_topic": format!("{topic_prefix}/status"),
        "payload_available": "online",
        "payload_not_available": "offline",
        "device": build_device_block(tenant_id, host_id, friendly_name, os_info)
    });

    if let Some(url) = release.url {
        config["release_url"] = serde_json::Value::String(url.to_string());
    }
    if let Some(notes) = release.notes {
        config["release_summary"] = serde_json::Value::String(truncate_str(notes, 500).to_string());
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
/// `{prefix}/hosts/{uuid}/items/{uuid}/set`.
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
/// assert!(parse_command_topic("uptrakit", "uptrakit/hosts/bad/items/set").is_none());
/// ```
pub fn parse_command_topic(topic_prefix: &str, topic: &str) -> Option<(Uuid, Uuid)> {
    // Expected: "{prefix}/hosts/{uuid}/items/{uuid}/set"
    let prefix = format!("{topic_prefix}/hosts/");
    let rest = topic.strip_prefix(prefix.as_str())?;
    let rest = rest.strip_suffix("/set")?;

    // rest should now be "{host_uuid}/items/{item_uuid}"
    let (host_str, rest) = rest.split_once('/')?;
    let (items_literal, item_str) = rest.split_once('/')?;
    if items_literal != "items" {
        return None;
    }
    // Make sure there are no further slashes in item_str.
    if item_str.contains('/') {
        return None;
    }

    let host_id = Uuid::parse_str(host_str).ok()?;
    let item_id = Uuid::parse_str(item_str).ok()?;
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
/// The published value is `"unknown"` when `pending_count > 0`, or
/// `"up-to-date"` when all packages are current.
/// See [`host_packages_state_string`].
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
/// See [`host_packages_latest_version_string`].
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
    let hp = host_topic_prefix(prefix, host_id);
    format!("{hp}/latest_version")
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
/// # use uptrakit_mqtt::ha_discovery::host_packages_command_topic;
/// let topic = host_packages_command_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/set"));
/// ```
pub fn host_packages_command_topic(prefix: &str, host_id: Uuid) -> String {
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
/// # use uptrakit_mqtt::ha_discovery::host_packages_discovery_config_topic;
/// let topic = host_packages_discovery_config_topic("homeassistant", Uuid::nil(), Uuid::nil());
/// assert!(topic.starts_with("homeassistant/update/uptrakit/pkgs_"));
/// assert!(topic.ends_with("/config"));
/// ```
pub fn host_packages_discovery_config_topic(
    ha_prefix: &str,
    tenant_id: Uuid,
    host_id: Uuid,
) -> String {
    let uid = host_packages_unique_id(tenant_id, host_id);
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
/// # use uptrakit_mqtt::ha_discovery::host_packages_unique_id;
/// let uid = host_packages_unique_id(Uuid::nil(), Uuid::nil());
/// assert!(uid.starts_with("uptrakit_"));
/// assert!(uid.ends_with("_pkgs"));
/// assert!(!uid.contains('-'));
/// ```
pub fn host_packages_unique_id(tenant_id: Uuid, host_id: Uuid) -> String {
    let t = tenant_id.simple();
    let h = host_id.simple();
    format!("uptrakit_{t}_{h}_pkgs")
}

/// Build the JSON attributes payload for a host's package entity.
///
/// Returns a [`serde_json::Value`] to be serialized and published (retained)
/// to [`host_packages_json_attributes_topic`].
///
/// Fields:
/// - `in_progress` — whether a batch update is pending/running.
/// - `pending_count` — number of packages with available updates.
/// - `total_count` — total number of tracked packages.
/// - `bugfix_count` — pending packages classified as `"bugfix"`.
/// - `feature_count` — pending packages classified as `"feature"`.
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
pub fn build_host_packages_attributes_payload(
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
/// to [`host_security_json_attributes_topic`].
///
/// # Examples
///
/// ```
/// # use uptrakit_mqtt::ha_discovery::build_host_security_attributes_payload;
/// let payload = build_host_security_attributes_payload(false, 2);
/// assert_eq!(payload["in_progress"], false);
/// assert_eq!(payload["pending_count"], 2u32);
/// ```
pub fn build_host_security_attributes_payload(
    in_progress: bool,
    security_pending_count: u32,
) -> serde_json::Value {
    serde_json::json!({
        "in_progress": in_progress,
        "pending_count": security_pending_count,
    })
}

/// Returns the state string published on the host packages state topic.
///
/// When `pending_count > 0`, returns `"unknown"` — there is no single
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
pub fn host_packages_state_string(pending_count: u32) -> String {
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
pub fn host_packages_latest_version_string(pending_count: u32) -> String {
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
pub fn host_security_state_string(security_pending_count: u32) -> String {
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
pub fn host_security_latest_version_string(security_pending_count: u32) -> String {
    if security_pending_count > 0 {
        format!("{security_pending_count} available")
    } else {
        "up-to-date".to_string()
    }
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
/// [`host_packages_discovery_config_topic`].
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt::ha_discovery::{build_host_packages_discovery_config, HostOsInfo};
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
pub fn build_host_packages_discovery_config(
    topic_prefix: &str,
    tenant_id: Uuid,
    host_id: Uuid,
    friendly_name: &str,
    os_info: HostOsInfo<'_>,
) -> serde_json::Value {
    let uid = host_packages_unique_id(tenant_id, host_id);
    let default_entity_id = format!("update.uptrakit_{}_packages", slugify(friendly_name));

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
        "availability_topic": format!("{topic_prefix}/status"),
        "payload_available": "online",
        "payload_not_available": "offline",
        "device": build_device_block(tenant_id, host_id, friendly_name, os_info)
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

// =============================================================================
// Security-only host package entity helpers
// =============================================================================

/// Returns the MQTT topic carrying the state string for a host's security
/// updates entity.
///
/// Format: `{prefix}/hosts/{host_id}/security/state`
///
/// The published value is `"unknown"` when `security_pending_count > 0`, or
/// `"up-to-date"` when all security packages are current.
/// See [`host_security_state_string`].
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt::ha_discovery::host_security_state_topic;
/// let topic = host_security_state_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/security/state"));
/// ```
pub fn host_security_state_topic(prefix: &str, host_id: Uuid) -> String {
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
/// See [`host_security_latest_version_string`].
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt::ha_discovery::host_security_latest_version_topic;
/// let topic = host_security_latest_version_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/security/latest_version"));
/// ```
pub fn host_security_latest_version_topic(prefix: &str, host_id: Uuid) -> String {
    let hp = host_topic_prefix(prefix, host_id);
    format!("{hp}/security/latest_version")
}

/// Returns the MQTT topic for HA JSON attributes of a host's security updates
/// entity.
///
/// Format: `{prefix}/hosts/{host_id}/security/attributes`
///
/// Published as a retained JSON payload with fields:
/// - `"in_progress"` (bool) — whether a security-only batch is pending/running
/// - `"pending_count"` (u32) — number of security packages with available updates
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt::ha_discovery::host_security_json_attributes_topic;
/// let topic = host_security_json_attributes_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/security/attributes"));
/// ```
pub fn host_security_json_attributes_topic(prefix: &str, host_id: Uuid) -> String {
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
/// # use uptrakit_mqtt::ha_discovery::host_security_command_topic;
/// let topic = host_security_command_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/security/set"));
/// ```
pub fn host_security_command_topic(prefix: &str, host_id: Uuid) -> String {
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
/// # use uptrakit_mqtt::ha_discovery::host_security_unique_id;
/// let uid = host_security_unique_id(Uuid::nil(), Uuid::nil());
/// assert!(uid.starts_with("uptrakit_"));
/// assert!(uid.ends_with("_sec"));
/// assert!(!uid.contains('-'));
/// ```
pub fn host_security_unique_id(tenant_id: Uuid, host_id: Uuid) -> String {
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
/// # use uptrakit_mqtt::ha_discovery::host_security_discovery_config_topic;
/// let topic = host_security_discovery_config_topic("homeassistant", Uuid::nil(), Uuid::nil());
/// assert!(topic.starts_with("homeassistant/update/uptrakit/sec_"));
/// assert!(topic.ends_with("/config"));
/// ```
pub fn host_security_discovery_config_topic(
    ha_prefix: &str,
    tenant_id: Uuid,
    host_id: Uuid,
) -> String {
    let uid = host_security_unique_id(tenant_id, host_id);
    let object_id = uid.strip_prefix("uptrakit_").unwrap_or(&uid);
    format!("{ha_prefix}/update/uptrakit/{object_id}/config")
}

/// Build the HA MQTT discovery JSON for a host's security updates `update`
/// entity.
///
/// This is a second `update` entity per host (alongside the all-packages entity)
/// that surfaces only packages with `update_category = "security"`. It is
/// **disabled by default** — users opt in explicitly.
///
/// The device identifier is the same as the host packages entity
/// (`uptrakit_host_{tenant}_{host}`), so both entities appear under the same
/// HA device.
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt::ha_discovery::{build_host_security_discovery_config, HostOsInfo};
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
pub fn build_host_security_discovery_config(
    topic_prefix: &str,
    tenant_id: Uuid,
    host_id: Uuid,
    friendly_name: &str,
    os_info: HostOsInfo<'_>,
) -> serde_json::Value {
    let uid = host_security_unique_id(tenant_id, host_id);
    let default_entity_id = format!(
        "update.uptrakit_{}_security_updates",
        slugify(friendly_name)
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
        "availability_topic": format!("{topic_prefix}/status"),
        "payload_available": "online",
        "payload_not_available": "offline",
        "device": build_device_block(tenant_id, host_id, friendly_name, os_info)
    })
}

// =============================================================================
// Host metadata and connectivity entity helpers
// =============================================================================

/// Returns the MQTT topic carrying the OS info JSON for a host.
///
/// Format: `{prefix}/hosts/{host_id}/info`
///
/// Published as a retained JSON payload. See [`build_host_info_payload`].
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt::ha_discovery::host_info_topic;
/// let topic = host_info_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/info"));
/// ```
pub fn host_info_topic(prefix: &str, host_id: Uuid) -> String {
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
/// # use uptrakit_mqtt::ha_discovery::host_tags_topic;
/// let topic = host_tags_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/tags"));
/// ```
pub fn host_tags_topic(prefix: &str, host_id: Uuid) -> String {
    let hp = host_topic_prefix(prefix, host_id);
    format!("{hp}/tags")
}

/// Returns the MQTT topic carrying the agent info JSON for a host.
///
/// Format: `{prefix}/hosts/{host_id}/agent`
///
/// Published as a retained JSON payload. See [`build_host_agent_payload`].
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt::ha_discovery::host_agent_topic;
/// let topic = host_agent_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/agent"));
/// ```
pub fn host_agent_topic(prefix: &str, host_id: Uuid) -> String {
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
/// # use uptrakit_mqtt::ha_discovery::host_connectivity_state_topic;
/// let topic = host_connectivity_state_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/connectivity/state"));
/// ```
pub fn host_connectivity_state_topic(prefix: &str, host_id: Uuid) -> String {
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
/// # use uptrakit_mqtt::ha_discovery::host_connectivity_attributes_topic;
/// let topic = host_connectivity_attributes_topic("uptrakit", Uuid::nil());
/// assert!(topic.ends_with("/connectivity/attributes"));
/// ```
pub fn host_connectivity_attributes_topic(prefix: &str, host_id: Uuid) -> String {
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
/// # use uptrakit_mqtt::ha_discovery::host_connectivity_unique_id;
/// let uid = host_connectivity_unique_id(Uuid::nil(), Uuid::nil());
/// assert!(uid.starts_with("uptrakit_"));
/// assert!(uid.ends_with("_conn"));
/// assert!(!uid.contains('-'));
/// ```
pub fn host_connectivity_unique_id(tenant_id: Uuid, host_id: Uuid) -> String {
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
/// # use uptrakit_mqtt::ha_discovery::host_connectivity_discovery_config_topic;
/// let topic = host_connectivity_discovery_config_topic("homeassistant", Uuid::nil(), Uuid::nil());
/// assert!(topic.starts_with("homeassistant/binary_sensor/uptrakit/"));
/// assert!(topic.ends_with("_conn/config"));
/// ```
pub fn host_connectivity_discovery_config_topic(
    ha_prefix: &str,
    tenant_id: Uuid,
    host_id: Uuid,
) -> String {
    let uid = host_connectivity_unique_id(tenant_id, host_id);
    let object_id = uid.strip_prefix("uptrakit_").unwrap_or(&uid);
    format!("{ha_prefix}/binary_sensor/uptrakit/{object_id}/config")
}

/// Build the JSON attributes payload for a host's connectivity entity.
///
/// Returns a [`serde_json::Value`] to be serialized and published (retained)
/// to [`host_connectivity_attributes_topic`].
///
/// Both fields are optional:
/// - `last_seen` — ISO 8601 datetime string of the agent's last contact.
/// - `version` — agent version string.
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
pub fn build_host_connectivity_attributes_payload(
    last_seen: Option<&str>,
    version: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "last_seen": last_seen,
        "version": version,
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
/// [`host_connectivity_discovery_config_topic`].
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt::ha_discovery::{build_host_connectivity_discovery_config, HostOsInfo};
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
pub fn build_host_connectivity_discovery_config(
    topic_prefix: &str,
    tenant_id: Uuid,
    host_id: Uuid,
    friendly_name: &str,
    os_info: HostOsInfo<'_>,
) -> serde_json::Value {
    let uid = host_connectivity_unique_id(tenant_id, host_id);
    let default_entity_id = format!("binary_sensor.uptrakit_{}_agent", slugify(friendly_name));

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

/// Build the JSON OS info payload for a host.
///
/// Returns a [`serde_json::Value`] to be serialized and published (retained)
/// to [`host_info_topic`].
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
pub fn build_host_info_payload(
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
/// to [`host_agent_topic`].
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
pub fn build_host_agent_payload(
    last_seen: Option<&str>,
    version: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "last_seen": last_seen,
        "version": version,
    })
}

/// Try to parse a host security command topic back to the `host_id`.
///
/// Returns `None` if the topic doesn't match
/// `{prefix}/hosts/{uuid}/security/set`.
///
/// This parser is unambiguous from [`parse_host_packages_command_topic`]:
/// the latter rejects any topic whose UUID segment contains a `/`, so
/// `{host_id}/security` cannot match the packages parser.
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt::ha_discovery::{host_security_command_topic, parse_host_security_command_topic};
/// let host_id = Uuid::nil();
/// let topic = host_security_command_topic("uptrakit", host_id);
/// let parsed = parse_host_security_command_topic("uptrakit", &topic);
/// assert_eq!(parsed, Some(host_id));
///
/// // Non-matching topic returns None.
/// assert!(parse_host_security_command_topic("uptrakit", "uptrakit/hosts/bad/set").is_none());
/// ```
pub fn parse_host_security_command_topic(topic_prefix: &str, topic: &str) -> Option<Uuid> {
    // Expected: "{prefix}/hosts/{uuid}/security/set"
    let prefix = format!("{topic_prefix}/hosts/");
    let rest = topic.strip_prefix(prefix.as_str())?;
    let rest = rest.strip_suffix("/security/set")?;

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
        // The leading "uptrakit_" is stripped from the object_id; the "uptrakit"
        // node_id level already provides the namespace.
        assert_eq!(
            discovery_config_topic("homeassistant", uid),
            "homeassistant/update/uptrakit/abc_def_ghi/config"
        );
    }

    #[test]
    fn discovery_config_topic_no_uptrakit_prefix_passthrough() {
        // IDs without an "uptrakit_" prefix pass through unchanged.
        assert_eq!(
            discovery_config_topic("ha", "uid123"),
            "ha/update/uptrakit/uid123/config"
        );
    }

    #[test]
    fn discovery_config_topic_custom_prefix() {
        let uid = unique_id(tenant(), item(), host());
        let topic = discovery_config_topic("ha", &uid);
        // Should contain the node_id but NOT the repeated "uptrakit_" prefix.
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
        // Format: uptrakit_{tenant}_{host}_{item}
        let zero = Uuid::nil();
        let uid = unique_id(zero, zero, zero);
        assert_eq!(
            uid,
            "uptrakit_00000000000000000000000000000000_00000000000000000000000000000000_00000000000000000000000000000000"
        );
    }

    #[test]
    fn unique_id_host_before_item() {
        // Verify host comes before item in the unique_id.
        let uid = unique_id(tenant(), item(), host());
        let host_simple = host().simple().to_string();
        let item_simple = item().simple().to_string();
        // host should appear before item in the string
        let host_pos = uid.find(&host_simple).unwrap();
        let item_pos = uid.find(&item_simple).unwrap();
        assert!(host_pos < item_pos);
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
            HostOsInfo::default(),
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
            ReleaseInfo {
                url: Some(url),
                notes: None,
            },
            HostOsInfo::default(),
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
            ReleaseInfo {
                url: None,
                notes: Some(notes),
            },
            HostOsInfo::default(),
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
            ReleaseInfo {
                url: None,
                notes: Some(&notes),
            },
            HostOsInfo::default(),
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
            HostOsInfo::default(),
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
        let topic = "uptrakit/hosts/not-a-uuid/items/not-a-uuid/set";
        assert!(parse_command_topic("uptrakit", topic).is_none());
    }

    #[test]
    fn parse_command_topic_too_many_segments() {
        let topic = format!("uptrakit/hosts/{}/items/{}/extra/set", host(), item());
        assert!(parse_command_topic("uptrakit", &topic).is_none());
    }

    #[test]
    fn parse_command_topic_missing_items_segment() {
        let topic = format!("uptrakit/hosts/{}/set", host());
        // This is a host packages command topic, not a software item one.
        assert!(parse_command_topic("uptrakit", &topic).is_none());
    }

    #[test]
    fn parse_command_topic_wrong_literal() {
        // "things" instead of "items"
        let topic = format!("uptrakit/hosts/{}/things/{}/set", host(), item());
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

    // -------------------------------------------------------------------------
    // host_packages_discovery_config_topic
    // -------------------------------------------------------------------------

    #[test]
    fn host_packages_discovery_config_topic_format() {
        let topic = host_packages_discovery_config_topic("homeassistant", tenant(), host());
        // The "uptrakit_" prefix is stripped; the node_id already carries the namespace.
        assert!(topic.starts_with("homeassistant/update/uptrakit/"));
        assert!(topic.ends_with("_pkgs/config"));
    }

    // -------------------------------------------------------------------------
    // build_host_packages_discovery_config — enabled_by_default: false
    // -------------------------------------------------------------------------

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
        // The "uptrakit_" prefix is stripped; the node_id already carries the namespace.
        assert!(topic.starts_with("homeassistant/update/uptrakit/"));
        assert!(topic.ends_with("_sec/config"));
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
        // Both entities must belong to the same HA device.
        assert_eq!(sec["device"]["identifiers"], pkg["device"]["identifiers"]);
    }

    #[test]
    fn build_discovery_config_device_same_as_host_packages() {
        // Software item entities should also share the same host-centric device.
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
    // parse_host_security_command_topic
    // -------------------------------------------------------------------------

    #[test]
    fn parse_host_security_command_topic_roundtrip() {
        let topic = host_security_command_topic("uptrakit", host());
        let parsed = parse_host_security_command_topic("uptrakit", &topic);
        assert_eq!(parsed, Some(host()));
    }

    #[test]
    fn parse_host_security_command_topic_wrong_prefix() {
        let topic = host_security_command_topic("uptrakit", host());
        assert!(parse_host_security_command_topic("other", &topic).is_none());
    }

    #[test]
    fn parse_host_security_command_topic_invalid_uuid() {
        let topic = "uptrakit/hosts/not-a-uuid/security/set";
        assert!(parse_host_security_command_topic("uptrakit", topic).is_none());
    }

    #[test]
    fn parse_host_security_command_topic_nil_uuid() {
        let zero = Uuid::nil();
        let topic = host_security_command_topic("pfx", zero);
        let parsed = parse_host_security_command_topic("pfx", &topic);
        assert_eq!(parsed, Some(zero));
    }

    #[test]
    fn parse_host_security_command_topic_does_not_match_packages_topic() {
        // A host-packages command topic ({prefix}/hosts/{uuid}/set) must NOT
        // match the security parser.
        let pkg_topic = host_packages_command_topic("uptrakit", host());
        assert!(parse_host_security_command_topic("uptrakit", &pkg_topic).is_none());
    }

    #[test]
    fn parse_host_packages_command_topic_does_not_match_security_topic() {
        // A security command topic ({prefix}/hosts/{uuid}/security/set) must
        // NOT match the packages parser (the UUID segment contains '/security').
        let sec_topic = host_security_command_topic("uptrakit", host());
        assert!(parse_host_packages_command_topic("uptrakit", &sec_topic).is_none());
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
