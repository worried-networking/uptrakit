use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::form_schema::{
    FormFieldDescriptor, FormFieldType, FormSelectOptionDescriptor,
};
use uptrakit_plugin_infrastructure_core::{
    PluginConfig, PluginConfigValidationError, TypeSettings,
};

/// Valid RouterOS update channels.
const VALID_CHANNELS: &[&str] = &["stable", "long-term", "testing"];

/// Configuration for the RouterOS package manager plugin.
///
/// Controls which update channel is used and whether the router is allowed to
/// reboot automatically after downloading a new RouterOS version.
///
/// The `channel` field selects the RouterOS upgrade channel (`stable`,
/// `long-term`, or `testing`). When `None`, the channel configured on the
/// router is left unchanged.
///
/// The `reboot` field determines whether the plugin should issue
/// `/system package update install` (which downloads **and** reboots) or
/// `/system package update download` (download only, no reboot). This is a
/// soft preference — a hard gate (`allow_reboot` on [`RouterOsHostRuntime`])
/// must also be set for a reboot to happen.
///
/// [`RouterOsHostRuntime`]: uptrakit_plugin_infrastructure_core::RouterOsHostRuntime
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouterOsConfig {
    /// RouterOS update channel. `None` leaves the on-device channel unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    /// When `true` (and `allow_reboot` is granted by the host runtime),
    /// the plugin will issue `package install` (download + reboot) instead
    /// of `package download` (download only).
    #[serde(default)]
    pub reboot: bool,
}

impl PluginConfig for RouterOsConfig {
    fn validate(&self) -> std::result::Result<(), PluginConfigValidationError> {
        if let Some(ref ch) = self.channel
            && !VALID_CHANNELS.contains(&ch.as_str())
        {
            return Err(PluginConfigValidationError::invalid_field(
                "channel",
                format!("must be one of {VALID_CHANNELS:?}; got {ch:?}"),
            ));
        }
        Ok(())
    }

    fn form_schema() -> Vec<FormFieldDescriptor> {
        vec![
            FormFieldDescriptor::new("channel", "Update Channel")
                .with_type(FormFieldType::Select)
                .with_options(vec![
                    FormSelectOptionDescriptor::new("stable", "Stable"),
                    FormSelectOptionDescriptor::new("long-term", "Long-Term"),
                    FormSelectOptionDescriptor::new("testing", "Testing"),
                ])
                .with_help_text("RouterOS update channel (leave empty to keep device setting)"),
            FormFieldDescriptor::new("reboot", "Auto-Reboot")
                .with_type(FormFieldType::Toggle)
                .with_help_text(
                    "When enabled (and the router group policy allows reboots), \
                     the router will reboot automatically after downloading the update",
                ),
        ]
    }
}

impl TypeSettings for RouterOsConfig {
    fn type_settings_form_schema() -> Vec<FormFieldDescriptor> {
        Self::form_schema()
    }

    fn type_settings_sample() -> serde_json::Value {
        serde_json::json!({
            "channel": "stable",
            "reboot": false
        })
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions use assert!(result.is_ok()) / assert!(result.is_err()) pattern"
    )]

    use super::*;

    // ── validate ─────────────────────────────────────────────────────────────

    #[test]
    fn validate_accepts_no_channel() {
        let config = RouterOsConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_accepts_stable_channel() {
        let config = RouterOsConfig {
            channel: Some("stable".to_string()),
            reboot: false,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_accepts_long_term_channel() {
        let config = RouterOsConfig {
            channel: Some("long-term".to_string()),
            reboot: false,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_accepts_testing_channel() {
        let config = RouterOsConfig {
            channel: Some("testing".to_string()),
            reboot: true,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_rejects_invalid_channel() {
        let config = RouterOsConfig {
            channel: Some("nightly".to_string()),
            reboot: false,
        };
        let err = config
            .validate()
            .expect_err("invalid channel should be rejected");
        assert_eq!(err.field(), Some("channel"));
        assert!(err.to_string().contains("nightly"));
    }

    #[test]
    fn validate_rejects_empty_string_channel() {
        let config = RouterOsConfig {
            channel: Some(String::new()),
            reboot: false,
        };
        assert!(config.validate().is_err());
    }

    // ── deserialization ───────────────────────────────────────────────────────

    #[test]
    fn deserialize_empty_object_gives_default() {
        let config: RouterOsConfig = serde_json::from_str("{}").expect("deserialize");
        assert_eq!(config.channel, None);
        assert!(!config.reboot);
    }

    #[test]
    fn deserialize_with_channel_and_reboot() {
        let config: RouterOsConfig =
            serde_json::from_str(r#"{"channel": "stable", "reboot": true}"#).expect("deserialize");
        assert_eq!(config.channel, Some("stable".to_string()));
        assert!(config.reboot);
    }

    // ── serialization ─────────────────────────────────────────────────────────

    #[test]
    fn serialize_none_channel_is_omitted() {
        let config = RouterOsConfig::default();
        let json = serde_json::to_value(&config).expect("serialize");
        assert!(json.get("channel").is_none());
        assert_eq!(json["reboot"], false);
    }

    #[test]
    fn serialization_roundtrip_with_channel() {
        let config = RouterOsConfig {
            channel: Some("long-term".to_string()),
            reboot: true,
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["channel"], "long-term");
        assert_eq!(json["reboot"], true);
        let deserialized: RouterOsConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }
}
