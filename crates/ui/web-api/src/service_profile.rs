//! Behavioral profiles derived from a service's persisted capability set.
//!
//! [`ServiceProfile`] is a derived enum — never stored in the database. It is
//! computed from `BTreeSet<Capability>` via [`ServiceProfile::from_capabilities`]
//! and drives controller-side behavioral defaults (ping interval, shutdown
//! timeout, human-readable label).

use std::collections::BTreeSet;

use uptrakit_internal_wire::Capability;

/// Behavioral profile derived from a service's capability set.
///
/// Two profiles cover all current service kinds:
///
/// | Profile | Key capability | Services |
/// | --- | --- | --- |
/// | `MqttBridge` | `Capability::MqttBridge` | MQTT service |
/// | `Agent` | `Capability::SoftwareDiscovery` | Local agent, SSH agent |
///
/// `Unknown` is the fallback for unrecognized capability combinations.
/// `ServiceProfile` is never persisted — it is always derived from capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ServiceProfile {
    /// MQTT bridge service (has `MqttBridge` capability).
    MqttBridge,
    /// Agent service — local or SSH-backed (has `SoftwareDiscovery` capability).
    Agent,
    /// Unrecognized capability combination.
    Unknown,
}

impl ServiceProfile {
    /// Derive the behavioral profile from a capability set.
    ///
    /// `MqttBridge` takes precedence if both `MqttBridge` and `SoftwareDiscovery`
    /// are present (this combination is not expected in practice).
    pub fn from_capabilities(caps: &BTreeSet<Capability>) -> Self {
        if caps.contains(&Capability::MqttBridge) {
            Self::MqttBridge
        } else if caps.contains(&Capability::SoftwareDiscovery) {
            Self::Agent
        } else {
            Self::Unknown
        }
    }

    /// Default ping interval in seconds for this profile.
    ///
    /// - `MqttBridge`: 15 seconds (MQTT lease heartbeat).
    /// - `Agent` / `Unknown`: 300 seconds (5 minutes).
    pub const fn default_ping_interval_secs(&self) -> u32 {
        match self {
            Self::MqttBridge => 15,
            Self::Agent | Self::Unknown => 300,
        }
    }

    /// Shutdown timeout in seconds, if applicable.
    ///
    /// - `MqttBridge`: `None` (no graceful shutdown timeout).
    /// - `Agent` / `Unknown`: `Some(120)` (2 minutes).
    pub const fn shutdown_timeout_secs(&self) -> Option<u32> {
        match self {
            Self::MqttBridge => None,
            Self::Agent | Self::Unknown => Some(120),
        }
    }

    /// Human-readable label for display in the API and UI.
    ///
    /// For `Agent` profiles, pass `has_ssh_remote = true` to distinguish
    /// SSH-backed agents from local agents.
    pub const fn service_label(&self, has_ssh_remote: bool) -> &'static str {
        match self {
            Self::MqttBridge => "MQTT Bridge",
            Self::Agent if has_ssh_remote => "SSH Agent",
            Self::Agent => "Agent",
            Self::Unknown => "Unknown",
        }
    }
}

/// Parse a JSON array string into a capability set.
///
/// The JSON is expected to be an array of snake_case strings
/// (e.g. `["software_discovery","update_hooks","graceful_shutdown"]`).
/// Returns an empty set on parse failure.
pub fn parse_capabilities(json: &str) -> BTreeSet<Capability> {
    serde_json::from_str::<Vec<Capability>>(json)
        .unwrap_or_default()
        .into_iter()
        .collect()
}

/// Serialize a capability set into a JSON array string.
///
/// Produces a sorted JSON array of snake_case strings
/// (e.g. `["graceful_shutdown","software_discovery","update_hooks"]`).
/// `BTreeSet` iteration order guarantees deterministic output.
pub fn serialize_capabilities(caps: &BTreeSet<Capability>) -> String {
    let vec: Vec<&Capability> = caps.iter().collect();
    serde_json::to_string(&vec).unwrap_or_else(|_| "[]".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(items: &[Capability]) -> BTreeSet<Capability> {
        items.iter().cloned().collect()
    }

    // ---------------------------------------------------------------
    // ServiceProfile::from_capabilities
    // ---------------------------------------------------------------

    #[test]
    fn profile_mqtt_bridge() {
        let c = caps(&[Capability::MqttBridge, Capability::GracefulShutdown]);
        assert_eq!(ServiceProfile::from_capabilities(&c), ServiceProfile::MqttBridge);
    }

    #[test]
    fn profile_agent() {
        let c = caps(&[
            Capability::SoftwareDiscovery,
            Capability::UpdateHooks,
            Capability::GracefulShutdown,
        ]);
        assert_eq!(ServiceProfile::from_capabilities(&c), ServiceProfile::Agent);
    }

    #[test]
    fn profile_ssh_agent() {
        let c = caps(&[
            Capability::SoftwareDiscovery,
            Capability::SshRemote,
            Capability::UpdateHooks,
            Capability::GracefulShutdown,
        ]);
        assert_eq!(ServiceProfile::from_capabilities(&c), ServiceProfile::Agent);
    }

    #[test]
    fn profile_unknown_empty() {
        assert_eq!(
            ServiceProfile::from_capabilities(&BTreeSet::new()),
            ServiceProfile::Unknown
        );
    }

    #[test]
    fn profile_unknown_only_graceful_shutdown() {
        let c = caps(&[Capability::GracefulShutdown]);
        assert_eq!(ServiceProfile::from_capabilities(&c), ServiceProfile::Unknown);
    }

    #[test]
    fn mqtt_bridge_takes_precedence() {
        let c = caps(&[Capability::MqttBridge, Capability::SoftwareDiscovery]);
        assert_eq!(ServiceProfile::from_capabilities(&c), ServiceProfile::MqttBridge);
    }

    // ---------------------------------------------------------------
    // Ping intervals
    // ---------------------------------------------------------------

    #[test]
    fn ping_interval_mqtt() {
        assert_eq!(ServiceProfile::MqttBridge.default_ping_interval_secs(), 15);
    }

    #[test]
    fn ping_interval_agent() {
        assert_eq!(ServiceProfile::Agent.default_ping_interval_secs(), 300);
    }

    #[test]
    fn ping_interval_unknown() {
        assert_eq!(ServiceProfile::Unknown.default_ping_interval_secs(), 300);
    }

    // ---------------------------------------------------------------
    // Shutdown timeouts
    // ---------------------------------------------------------------

    #[test]
    fn shutdown_timeout_mqtt() {
        assert_eq!(ServiceProfile::MqttBridge.shutdown_timeout_secs(), None);
    }

    #[test]
    fn shutdown_timeout_agent() {
        assert_eq!(ServiceProfile::Agent.shutdown_timeout_secs(), Some(120));
    }

    #[test]
    fn shutdown_timeout_unknown() {
        assert_eq!(ServiceProfile::Unknown.shutdown_timeout_secs(), Some(120));
    }

    // ---------------------------------------------------------------
    // Service labels
    // ---------------------------------------------------------------

    #[test]
    fn label_agent() {
        assert_eq!(ServiceProfile::Agent.service_label(false), "Agent");
    }

    #[test]
    fn label_ssh_agent() {
        assert_eq!(ServiceProfile::Agent.service_label(true), "SSH Agent");
    }

    #[test]
    fn label_mqtt_bridge() {
        assert_eq!(ServiceProfile::MqttBridge.service_label(false), "MQTT Bridge");
    }

    #[test]
    fn label_unknown() {
        assert_eq!(ServiceProfile::Unknown.service_label(false), "Unknown");
    }

    // ---------------------------------------------------------------
    // Capability JSON round-trip
    // ---------------------------------------------------------------

    #[test]
    fn serialize_empty_set() {
        assert_eq!(serialize_capabilities(&BTreeSet::new()), "[]");
    }

    #[test]
    fn serialize_and_parse_round_trip() {
        let original = caps(&[
            Capability::GracefulShutdown,
            Capability::SoftwareDiscovery,
            Capability::UpdateHooks,
        ]);
        let json = serialize_capabilities(&original);
        let parsed = parse_capabilities(&json);
        assert_eq!(parsed, original);
    }

    #[test]
    fn parse_invalid_json_returns_empty() {
        assert!(parse_capabilities("not json").is_empty());
    }

    #[test]
    fn parse_empty_array() {
        assert!(parse_capabilities("[]").is_empty());
    }

    #[test]
    fn parse_preserves_all_known_capabilities() {
        let json =
            r#"["graceful_shutdown","mqtt_bridge","software_discovery","ssh_remote","update_hooks"]"#;
        let parsed = parse_capabilities(json);
        assert_eq!(parsed.len(), 5);
        assert!(parsed.contains(&Capability::GracefulShutdown));
        assert!(parsed.contains(&Capability::MqttBridge));
        assert!(parsed.contains(&Capability::SoftwareDiscovery));
        assert!(parsed.contains(&Capability::SshRemote));
        assert!(parsed.contains(&Capability::UpdateHooks));
    }

    #[test]
    fn parse_unknown_capabilities_become_other() {
        let json = r#"["software_discovery","future_cap"]"#;
        let parsed = parse_capabilities(json);
        assert_eq!(parsed.len(), 2);
        assert!(parsed.contains(&Capability::SoftwareDiscovery));
        assert!(parsed.contains(&Capability::Other("future_cap".to_string())));
    }

    #[test]
    fn serialize_produces_sorted_output() {
        let c = caps(&[
            Capability::UpdateHooks,
            Capability::GracefulShutdown,
            Capability::SoftwareDiscovery,
        ]);
        let json = serialize_capabilities(&c);
        // BTreeSet sorts by Ord impl; verify output is deterministic.
        let parsed: Vec<String> = serde_json::from_str(&json).unwrap_or_default();
        let mut sorted = parsed.clone();
        sorted.sort();
        assert_eq!(parsed, sorted);
    }
}
