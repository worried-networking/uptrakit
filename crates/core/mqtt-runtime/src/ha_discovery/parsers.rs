//! Command topic parsers for Home Assistant MQTT discovery.
//!
//! Each parser extracts UUIDs from an incoming MQTT topic string,
//! returning `None` if the topic does not match the expected pattern.

use uuid::Uuid;

/// Try to parse a command topic back to `(item_id, host_id)`.
///
/// Returns `None` if the topic doesn't match
/// `{prefix}/hosts/{uuid}/items/{uuid}/set`.
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::{command_topic, parse_command_topic};
/// let item_id = Uuid::nil();
/// let host_id = Uuid::nil();
/// let topic = command_topic("uptrakit", item_id, host_id);
/// let parsed = parse_command_topic("uptrakit", &topic);
/// assert_eq!(parsed, Some((item_id, host_id)));
///
/// // Non-matching topic returns None.
/// assert!(parse_command_topic("uptrakit", "uptrakit/hosts/bad/items/set").is_none());
/// ```
pub(crate) fn parse_command_topic(topic_prefix: &str, topic: &str) -> Option<(Uuid, Uuid)> {
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

/// Try to parse a host packages command topic back to the `host_id`.
///
/// Returns `None` if the topic doesn't match `{prefix}/hosts/{uuid}/set`.
///
/// # Examples
///
/// ```
/// # use uuid::Uuid;
/// # use uptrakit_mqtt_runtime::ha_discovery::{host_packages_command_topic, parse_host_packages_command_topic};
/// let host_id = Uuid::nil();
/// let topic = host_packages_command_topic("uptrakit", host_id);
/// let parsed = parse_host_packages_command_topic("uptrakit", &topic);
/// assert_eq!(parsed, Some(host_id));
///
/// // Non-matching topic returns None.
/// assert!(parse_host_packages_command_topic("uptrakit", "uptrakit/update/bad/set").is_none());
/// ```
pub(crate) fn parse_host_packages_command_topic(topic_prefix: &str, topic: &str) -> Option<Uuid> {
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
/// # use uptrakit_mqtt_runtime::ha_discovery::{host_security_command_topic, parse_host_security_command_topic};
/// let host_id = Uuid::nil();
/// let topic = host_security_command_topic("uptrakit", host_id);
/// let parsed = parse_host_security_command_topic("uptrakit", &topic);
/// assert_eq!(parsed, Some(host_id));
///
/// // Non-matching topic returns None.
/// assert!(parse_host_security_command_topic("uptrakit", "uptrakit/hosts/bad/set").is_none());
/// ```
pub(crate) fn parse_host_security_command_topic(topic_prefix: &str, topic: &str) -> Option<Uuid> {
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
    use super::super::topics::{
        command_topic, host_packages_command_topic, host_packages_state_topic,
        host_security_command_topic, state_topic,
    };
    use super::*;

    fn item() -> Uuid {
        Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap()
    }
    fn host() -> Uuid {
        Uuid::parse_str("33333333-3333-3333-3333-333333333333").unwrap()
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
        assert!(parse_command_topic("uptrakit", &topic).is_none());
    }

    #[test]
    fn parse_command_topic_wrong_literal() {
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
        let topic = host_packages_state_topic("uptrakit", host());
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

    #[test]
    fn parse_host_packages_command_topic_does_not_match_software_item_topic() {
        let sw_topic = command_topic("uptrakit", item(), host());
        assert!(parse_host_packages_command_topic("uptrakit", &sw_topic).is_none());
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
        let pkg_topic = host_packages_command_topic("uptrakit", host());
        assert!(parse_host_security_command_topic("uptrakit", &pkg_topic).is_none());
    }

    #[test]
    fn parse_host_packages_command_topic_does_not_match_security_topic() {
        let sec_topic = host_security_command_topic("uptrakit", host());
        assert!(parse_host_packages_command_topic("uptrakit", &sec_topic).is_none());
    }
}
