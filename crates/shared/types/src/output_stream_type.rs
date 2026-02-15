use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Output stream source for update execution output lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(
    feature = "sea-orm",
    derive(strum::EnumIter, sea_orm::DeriveActiveEnum)
)]
#[cfg_attr(
    feature = "sea-orm",
    sea_orm(rs_type = "String", db_type = "Text")
)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum OutputStreamType {
    #[default]
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "stdout"))]
    Stdout,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "stderr"))]
    Stderr,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "pre_hook"))]
    PreHook,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "post_hook"))]
    PostHook,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "system"))]
    System,
}

impl OutputStreamType {
    /// Returns the string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
            Self::PreHook => "pre_hook",
            Self::PostHook => "post_hook",
            Self::System => "system",
        }
    }
}

impl fmt::Display for OutputStreamType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when parsing an invalid [`OutputStreamType`] string.
#[derive(Debug)]
pub struct ParseOutputStreamTypeError;

impl fmt::Display for ParseOutputStreamTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid output stream type value")
    }
}

impl std::error::Error for ParseOutputStreamTypeError {}

impl FromStr for OutputStreamType {
    type Err = ParseOutputStreamTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "stdout" => Ok(Self::Stdout),
            "stderr" => Ok(Self::Stderr),
            "pre_hook" => Ok(Self::PreHook),
            "post_hook" => Ok(Self::PostHook),
            "system" => Ok(Self::System),
            _ => Err(ParseOutputStreamTypeError),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip() {
        for variant in [
            OutputStreamType::Stdout,
            OutputStreamType::Stderr,
            OutputStreamType::PreHook,
            OutputStreamType::PostHook,
            OutputStreamType::System,
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            let deserialized: OutputStreamType = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, variant);
        }
    }

    #[test]
    fn display_matches_as_str() {
        for variant in [
            OutputStreamType::Stdout,
            OutputStreamType::Stderr,
            OutputStreamType::PreHook,
            OutputStreamType::PostHook,
            OutputStreamType::System,
        ] {
            assert_eq!(format!("{variant}"), variant.as_str());
        }
    }

    #[test]
    fn from_str_round_trip() {
        for variant in [
            OutputStreamType::Stdout,
            OutputStreamType::Stderr,
            OutputStreamType::PreHook,
            OutputStreamType::PostHook,
            OutputStreamType::System,
        ] {
            let s = variant.as_str();
            let parsed: OutputStreamType = s.parse().unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn from_str_invalid_returns_err() {
        assert!("unknown".parse::<OutputStreamType>().is_err());
        assert!("".parse::<OutputStreamType>().is_err());
    }

    #[test]
    fn default_is_stdout() {
        assert_eq!(OutputStreamType::default(), OutputStreamType::Stdout);
    }

    #[test]
    fn serde_values() {
        assert_eq!(
            serde_json::to_string(&OutputStreamType::Stdout).unwrap(),
            r#""stdout""#
        );
        assert_eq!(
            serde_json::to_string(&OutputStreamType::Stderr).unwrap(),
            r#""stderr""#
        );
        assert_eq!(
            serde_json::to_string(&OutputStreamType::PreHook).unwrap(),
            r#""pre_hook""#
        );
        assert_eq!(
            serde_json::to_string(&OutputStreamType::PostHook).unwrap(),
            r#""post_hook""#
        );
        assert_eq!(
            serde_json::to_string(&OutputStreamType::System).unwrap(),
            r#""system""#
        );
    }
}
