// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
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
impl Serialize for UpdateCategory {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}
impl<'de> Deserialize<'de> for UpdateCategory {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(UpdateCategory::from)
    }
}
