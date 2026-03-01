use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Classification of a software update by its nature.
///
/// Plugins that can determine the update type (e.g. APT can detect
/// security updates from the repository URL) set the category on
/// [`UpstreamRelease`]. Unknown is used when the plugin cannot classify.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "sea-orm",
    derive(strum::EnumIter, sea_orm::DeriveActiveEnum)
)]
#[cfg_attr(feature = "sea-orm", sea_orm(rs_type = "String", db_type = "Text"))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum UpdateCategory {
    /// A security patch or vulnerability fix.
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "security"))]
    Security,
    /// A bug fix that does not address a security vulnerability.
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "bugfix"))]
    Bugfix,
    /// A new feature or enhancement.
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "feature"))]
    Feature,
    /// The update type could not be determined.
    #[default]
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "unknown"))]
    Unknown,
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
        f.write_str(match self {
            Self::Security => "security",
            Self::Bugfix => "bugfix",
            Self::Feature => "feature",
            Self::Unknown => "unknown",
        })
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

#[cfg(test)]
mod tests {
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
}
