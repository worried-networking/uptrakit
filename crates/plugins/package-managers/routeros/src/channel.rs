//! RouterOS update channel enum.
//!
//! Single source of truth for the three valid RouterOS update channels:
//! `stable`, `long-term`, `testing`. Used by [`RouterOsConfig`] for the
//! plugin's typed `channel` field, by the `Discoverer` impl to auto-fill
//! the channel during discovery from the router's `version: X.Y.Z (channel)`
//! suffix, and by the form schema to enumerate options.
//!
//! [`RouterOsConfig`]: crate::config::RouterOsConfig

use serde::{Deserialize, Serialize};

/// RouterOS update channel.
///
/// Wire form (JSON / YAML / form-schema): the kebab-case strings
/// `"stable"`, `"long-term"`, `"testing"` exactly — matches the values
/// RouterOS itself emits in `(channel)` suffixes and in `channel:` fields,
/// and is backward-compatible with existing `plugin_configs.config` JSON
/// blobs in the DB.
///
/// `#[non_exhaustive]` per project convention so a future RouterOS channel
/// addition is a non-breaking change. Internal matchers in this crate stay
/// exhaustive without wildcards because `non_exhaustive` only constrains
/// matchers in other crates — the routeros plugin owns this enum and
/// keeps full pattern coverage.
///
/// `strum::EnumIter` provides [`RouterOsChannel::iter()`] for the form
/// schema; `Other(String)` is intentionally absent (this is a
/// plugin-internal config enum, not a wire envelope variant), so the
/// strum derive is compatible.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, strum::EnumIter)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum RouterOsChannel {
    Stable,
    LongTerm,
    Testing,
}

impl RouterOsChannel {
    /// Wire-form canonical string for this channel.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::LongTerm => "long-term",
            Self::Testing => "testing",
        }
    }

    /// Parse the channel from RouterOS' `version: X.Y.Z (channel)` suffix.
    ///
    /// Returns `None` when the suffix is absent or unrecognized. The
    /// `Discoverer` impl uses this to auto-fill `channel` in the per-host
    /// plugin_config from the router's actual reported channel.
    pub fn from_resource_suffix(raw: &str) -> Option<Self> {
        let after_open = raw.split_once('(').map(|(_, after)| after)?;
        let label = after_open
            .split_once(')')
            .map_or(after_open, |(b, _)| b)
            .trim();
        match label {
            "stable" => Some(Self::Stable),
            "long-term" => Some(Self::LongTerm),
            "testing" => Some(Self::Testing),
            _ => None,
        }
    }
}

impl std::fmt::Display for RouterOsChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator as _;

    use super::*;

    #[test]
    fn from_resource_suffix_parses_each_known_channel() {
        assert_eq!(
            RouterOsChannel::from_resource_suffix("7.14.2 (stable)"),
            Some(RouterOsChannel::Stable)
        );
        assert_eq!(
            RouterOsChannel::from_resource_suffix("7.14.2 (long-term)"),
            Some(RouterOsChannel::LongTerm)
        );
        assert_eq!(
            RouterOsChannel::from_resource_suffix("7.14.2 (testing)"),
            Some(RouterOsChannel::Testing)
        );
    }

    #[test]
    fn from_resource_suffix_returns_none_for_bare_version() {
        assert_eq!(RouterOsChannel::from_resource_suffix("7.14.2"), None);
    }

    #[test]
    fn from_resource_suffix_returns_none_for_unknown_label() {
        assert_eq!(
            RouterOsChannel::from_resource_suffix("7.14.2 (development)"),
            None
        );
    }

    #[test]
    fn as_str_round_trips_via_serde() {
        // Catches drift between as_str() and the serde rename_all attribute.
        for variant in RouterOsChannel::iter() {
            let json = serde_json::to_string(&variant).expect("serialize");
            assert_eq!(json, format!("\"{}\"", variant.as_str()));
            let round_tripped: RouterOsChannel = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(round_tripped, variant);
        }
    }

    #[test]
    fn iter_covers_three_variants_in_declaration_order() {
        let variants: Vec<RouterOsChannel> = RouterOsChannel::iter().collect();
        assert_eq!(
            variants,
            vec![
                RouterOsChannel::Stable,
                RouterOsChannel::LongTerm,
                RouterOsChannel::Testing,
            ]
        );
    }

    #[test]
    fn display_matches_as_str() {
        for variant in RouterOsChannel::iter() {
            assert_eq!(variant.to_string(), variant.as_str());
        }
    }
}
