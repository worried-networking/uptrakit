use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Classification of a software update by its nature.
///
/// Plugins that can determine the update type (e.g. APT can detect
/// security updates from the repository URL) set the category on
/// [`UpstreamRelease`]. Unknown is used when the plugin cannot classify.
///
/// # Wire forward-compatibility
///
/// `Other(String)` is a catch-all variant for category strings received from a
/// newer peer that this binary does not yet know about. Serde deserialization
/// is infallible: an unknown string becomes `Other(...)` rather than a parse
/// error, allowing older clients to survive rolling upgrades without dropping
/// entire messages.
///
/// `FromStr` remains strict for URL parameters and DB lookups where callers
/// need to distinguish known variants from unknown ones.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum UpdateCategory {
    /// A security patch or vulnerability fix.
    Security,
    /// A bug fix that does not address a security vulnerability.
    Bugfix,
    /// A new feature or enhancement.
    Feature,
    /// The update type could not be determined.
    #[default]
    Unknown,
    /// An unknown category received from a newer peer.
    ///
    /// The inner string is the raw snake_case value as it appeared on the wire.
    Other(String),
}

impl UpdateCategory {
    /// Returns the string representation.
    ///
    /// For [`UpdateCategory::Other`], returns the inner string as-is.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Security => "security",
            Self::Bugfix => "bugfix",
            Self::Feature => "feature",
            Self::Unknown => "unknown",
            Self::Other(s) => s.as_str(),
        }
    }
}

/// Error returned when parsing an invalid [`UpdateCategory`] string.
#[derive(Debug, Error)]
pub enum ParseUpdateCategoryError {
    /// The input string does not match any known category.
    #[error("invalid update category value")]
    Invalid,
}

impl fmt::Display for UpdateCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for UpdateCategory {
    type Err = ParseUpdateCategoryError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "security" => Ok(Self::Security),
            "bugfix" => Ok(Self::Bugfix),
            "feature" => Ok(Self::Feature),
            "unknown" => Ok(Self::Unknown),
            _ => Err(ParseUpdateCategoryError::Invalid),
        }
    }
}

impl From<String> for UpdateCategory {
    /// Converts a snake_case string to an update category.
    ///
    /// Unknown strings map to [`UpdateCategory::Other`] rather than failing.
    fn from(s: String) -> Self {
        match s.as_str() {
            "security" => Self::Security,
            "bugfix" => Self::Bugfix,
            "feature" => Self::Feature,
            "unknown" => Self::Unknown,
            _ => Self::Other(s),
        }
    }
}

// ── Serde: infallible string-based serialization ──────────────────────────────
//
// Custom Serialize/Deserialize are implemented manually rather than via derive
// so that unknown strings deserialize to `Other(String)` rather than failing.

impl Serialize for UpdateCategory {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for UpdateCategory {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Deserialize as a plain string, then convert via From<String>.
        // Unknown strings become Other(s) — this conversion is infallible.
        String::deserialize(deserializer).map(UpdateCategory::from)
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — is_ok/is_err provides readable failure messages"
    )]
    use super::*;

    #[test]
    fn serde_round_trip() {
        for variant in [
            UpdateCategory::Security,
            UpdateCategory::Bugfix,
            UpdateCategory::Feature,
            UpdateCategory::Unknown,
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            let deserialized: UpdateCategory = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, variant);
        }
    }

    #[test]
    fn display_values() {
        assert_eq!(UpdateCategory::Security.to_string(), "security");
        assert_eq!(UpdateCategory::Bugfix.to_string(), "bugfix");
        assert_eq!(UpdateCategory::Feature.to_string(), "feature");
        assert_eq!(UpdateCategory::Unknown.to_string(), "unknown");
    }

    #[test]
    fn from_str_valid() {
        assert_eq!(
            "security".parse::<UpdateCategory>().ok(),
            Some(UpdateCategory::Security)
        );
        assert_eq!(
            "bugfix".parse::<UpdateCategory>().ok(),
            Some(UpdateCategory::Bugfix)
        );
        assert_eq!(
            "feature".parse::<UpdateCategory>().ok(),
            Some(UpdateCategory::Feature)
        );
        assert_eq!(
            "unknown".parse::<UpdateCategory>().ok(),
            Some(UpdateCategory::Unknown)
        );
    }

    #[test]
    fn from_str_invalid() {
        assert!("".parse::<UpdateCategory>().is_err());
        assert!("Security".parse::<UpdateCategory>().is_err());
        assert!("SECURITY".parse::<UpdateCategory>().is_err());
        assert!("patch".parse::<UpdateCategory>().is_err());
    }

    #[test]
    fn unknown_category_deserializes_to_other() {
        let deserialized: UpdateCategory =
            serde_json::from_str(r#""future_category""#).expect("deserialize unknown");
        assert_eq!(
            deserialized,
            UpdateCategory::Other("future_category".to_string())
        );
    }

    #[test]
    fn other_serializes_to_inner_string() {
        let cat = UpdateCategory::Other("future_category".to_string());
        let json = serde_json::to_string(&cat).unwrap();
        assert_eq!(json, r#""future_category""#);
    }

    #[test]
    fn other_round_trip() {
        let original = r#""new_category""#;
        let deserialized: UpdateCategory = serde_json::from_str(original).expect("deserialize");
        assert_eq!(
            deserialized,
            UpdateCategory::Other("new_category".to_string())
        );
        let reserialized = serde_json::to_string(&deserialized).expect("serialize");
        assert_eq!(reserialized, original);
    }

    #[test]
    fn from_str_still_strict_for_unknown() {
        // FromStr must reject unknown strings even though Deserialize accepts them.
        assert!("future_category".parse::<UpdateCategory>().is_err());
        assert!("other".parse::<UpdateCategory>().is_err());
    }

    #[test]
    fn default_is_unknown() {
        assert_eq!(UpdateCategory::default(), UpdateCategory::Unknown);
    }
}
