#![expect(
    clippy::string_slice,
    reason = "string slices use byte positions derived from ASCII-only content or fixed-length pattern matching; UTF-8 boundary safety is guaranteed by construction"
)]
use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::{
    PluginConfig, PluginConfigValidationError, TypeSettings,
};

/// Validate a Snap channel string.
///
/// A valid channel is either:
/// - A bare risk: `stable`, `candidate`, `beta`, or `edge`.
/// - A track and risk separated by `/`: `<track>/<risk>` where `<track>` contains
///   alphanumeric characters, `.`, and `-` (max 30 chars), and `<risk>` is one of
///   the four valid risks.
///
/// The full channel string must not exceed 40 characters.
fn validate_channel(channel: &str) -> Result<(), PluginConfigValidationError> {
    if channel.is_empty() {
        return Err(PluginConfigValidationError::invalid_field(
            "channel",
            "must not be empty",
        ));
    }
    if channel.len() > 40 {
        return Err(PluginConfigValidationError::invalid_field(
            "channel",
            "must not exceed 40 characters",
        ));
    }

    const VALID_RISKS: &[&str] = &["stable", "candidate", "beta", "edge"];

    if let Some(slash) = channel.find('/') {
        let track = &channel[..slash];
        let risk = &channel[slash + 1..];

        if track.is_empty() {
            return Err(PluginConfigValidationError::invalid_field(
                "channel",
                "track must not be empty",
            ));
        }
        if track.len() > 30 {
            return Err(PluginConfigValidationError::invalid_field(
                "channel",
                "track must not exceed 30 characters",
            ));
        }
        for ch in track.chars() {
            if !ch.is_ascii_alphanumeric() && !matches!(ch, '.' | '-') {
                return Err(PluginConfigValidationError::invalid_field(
                    "channel",
                    format!("track contains invalid character: '{ch}'"),
                ));
            }
        }
        if !VALID_RISKS.contains(&risk) {
            return Err(PluginConfigValidationError::invalid_field(
                "channel",
                format!("risk must be one of: stable, candidate, beta, edge (found '{risk}')"),
            ));
        }
    } else if !VALID_RISKS.contains(&channel) {
        return Err(PluginConfigValidationError::invalid_field(
            "channel",
            format!(
                "must be one of: stable, candidate, beta, edge (or <track>/<risk>), found '{channel}'"
            ),
        ));
    }

    Ok(())
}

/// Configuration for the Snap package manager plugin.
///
/// No secrets — the `package_identifier` in `SoftwareItem` is the Snap package name
/// (e.g., `vlc`, `code`, `firefox`).
///
/// The `channel` field controls which Snap channel is tracked for version detection
/// and updates:
///
/// - `None` (default, serialises to `{}`) — uses `"latest/stable"` for release lookups.
/// - `Some("latest/stable")` — explicitly track the `latest/stable` channel.
/// - `Some("1.0/stable")` — track a specific track (for software with versioned tracks).
///
/// A [`DiscoveryTarget`] is always emitted per installed snap so the controller
/// can find-or-create plugin config and role assignments.
///
/// [`DiscoveryTarget`]: uptrakit_plugin_infrastructure_core::DiscoveryTarget
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapConfig {
    /// Snap channel to track. If `None` (the default when the config is `{}`),
    /// the plugin operates in discover-all mode and uses `"latest/stable"` for
    /// release queries. An explicit value such as `"1.0/stable"` or `"edge"` pins
    /// the plugin to that channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
}

impl PluginConfig for SnapConfig {
    fn validate_identifier(value: &str) -> Result<(), PluginConfigValidationError> {
        crate::validate_identifier(value)
    }

    fn validate(&self) -> Result<(), PluginConfigValidationError> {
        if let Some(channel) = &self.channel {
            validate_channel(channel)?;
        }
        Ok(())
    }
}

impl TypeSettings for SnapConfig {
    fn type_settings_form_schema()
    -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FormFieldDescriptor> {
        use uptrakit_plugin_infrastructure_core::form_schema::{
            FormFieldDescriptor, FormFieldType, FormSelectOptionDescriptor,
        };
        vec![
            FormFieldDescriptor::new("channel", "Channel")
                .with_type(FormFieldType::Select)
                .with_options(vec![
                    FormSelectOptionDescriptor::new("latest/stable", "Stable"),
                    FormSelectOptionDescriptor::new("latest/candidate", "Candidate"),
                    FormSelectOptionDescriptor::new("latest/beta", "Beta"),
                    FormSelectOptionDescriptor::new("latest/edge", "Edge"),
                ])
                .with_help_text("Default Snap channel to track for discovered packages"),
        ]
    }

    fn type_settings_sample() -> serde_json::Value {
        serde_json::json!({
            "channel": "latest/stable"
        })
    }
}

impl SnapConfig {
    /// Returns the effective channel to use for release queries.
    ///
    /// `None` (default config) behaves as `"latest/stable"`.
    pub(crate) fn effective_channel(&self) -> &str {
        self.channel.as_deref().unwrap_or("latest/stable")
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions use assert!(result.is_ok()) pattern"
    )]
    use super::*;

    // ── effective_channel ─────────────────────────────────────────────────────

    #[test]
    fn effective_channel_default_is_latest_stable() {
        assert_eq!(SnapConfig::default().effective_channel(), "latest/stable");
    }

    #[test]
    fn effective_channel_explicit() {
        let config = SnapConfig {
            channel: Some("1.0/stable".to_string()),
        };
        assert_eq!(config.effective_channel(), "1.0/stable");
    }

    #[test]
    fn effective_channel_bare_risk() {
        let config = SnapConfig {
            channel: Some("edge".to_string()),
        };
        assert_eq!(config.effective_channel(), "edge");
    }

    // ── deserialization ───────────────────────────────────────────────────────

    #[test]
    fn deserialize_empty_object_gives_none() {
        let config: SnapConfig = serde_json::from_str("{}").expect("deserialize");
        assert_eq!(config.channel, None);
    }

    #[test]
    fn deserialize_with_channel() {
        let config: SnapConfig =
            serde_json::from_str(r#"{"channel": "latest/stable"}"#).expect("deserialize");
        assert_eq!(config.channel, Some("latest/stable".to_string()));
    }

    // ── serialization ─────────────────────────────────────────────────────────

    #[test]
    fn serialization_none_gives_empty_object() {
        let config = SnapConfig::default();
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json, serde_json::json!({}));
        let deserialized: SnapConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    #[test]
    fn serialization_roundtrip_channel() {
        let config = SnapConfig {
            channel: Some("1.0/stable".to_string()),
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["channel"], "1.0/stable");
        let deserialized: SnapConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(deserialized, config);
    }

    // ── validate ──────────────────────────────────────────────────────────────

    #[test]
    fn validate_accepts_default_config() {
        assert!(SnapConfig::default().validate().is_ok());
    }

    #[test]
    fn validate_accepts_valid_bare_risk() {
        let config = SnapConfig {
            channel: Some("stable".to_string()),
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_accepts_valid_track_risk() {
        let config = SnapConfig {
            channel: Some("latest/stable".to_string()),
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_accepts_versioned_track() {
        let config = SnapConfig {
            channel: Some("1.0/stable".to_string()),
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_rejects_unknown_risk() {
        let config = SnapConfig {
            channel: Some("latest/nightly".to_string()),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_rejects_empty_channel() {
        let config = SnapConfig {
            channel: Some(String::new()),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_rejects_bare_invalid_risk() {
        let config = SnapConfig {
            channel: Some("nightly".to_string()),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_rejects_too_long_channel() {
        let config = SnapConfig {
            channel: Some("a".repeat(41)),
        };
        assert!(config.validate().is_err());
    }

    // ── validate_channel internal ─────────────────────────────────────────────

    #[test]
    fn validate_channel_bare_risks() {
        for risk in &["stable", "candidate", "beta", "edge"] {
            assert!(
                validate_channel(risk).is_ok(),
                "should accept bare risk '{risk}'"
            );
        }
    }

    #[test]
    fn validate_channel_track_slash_risk() {
        assert!(validate_channel("latest/stable").is_ok());
        assert!(validate_channel("1.0/stable").is_ok());
        assert!(validate_channel("2024.01/edge").is_ok());
    }

    #[test]
    fn validate_channel_invalid_risk_fails() {
        assert!(validate_channel("latest/nightly").is_err());
        assert!(validate_channel("latest/release").is_err());
    }

    #[test]
    fn validate_channel_invalid_track_char_fails() {
        assert!(validate_channel("my track/stable").is_err());
        assert!(validate_channel("my_track/stable").is_err());
    }

    #[test]
    fn validate_channel_empty_track_fails() {
        assert!(validate_channel("/stable").is_err());
    }

    #[test]
    fn validate_channel_empty_string_fails() {
        assert!(validate_channel("").is_err());
    }
}
