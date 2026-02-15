use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Status of a pending device authorization flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
pub enum DeviceAuthStatus {
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "pending"))]
    Pending,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "authorized"))]
    Authorized,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "expired"))]
    Expired,
}

impl DeviceAuthStatus {
    /// Returns the string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Authorized => "authorized",
            Self::Expired => "expired",
        }
    }
}

impl fmt::Display for DeviceAuthStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when parsing an invalid device auth status string.
#[derive(Debug)]
pub struct ParseDeviceAuthStatusError;

impl fmt::Display for ParseDeviceAuthStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid device auth status value")
    }
}

impl std::error::Error for ParseDeviceAuthStatusError {}

impl FromStr for DeviceAuthStatus {
    type Err = ParseDeviceAuthStatusError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "authorized" => Ok(Self::Authorized),
            "expired" => Ok(Self::Expired),
            _ => Err(ParseDeviceAuthStatusError),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip() {
        for variant in [
            DeviceAuthStatus::Pending,
            DeviceAuthStatus::Authorized,
            DeviceAuthStatus::Expired,
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            let deserialized: DeviceAuthStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, variant);
        }
    }

    #[test]
    fn display_matches_as_str() {
        for variant in [
            DeviceAuthStatus::Pending,
            DeviceAuthStatus::Authorized,
            DeviceAuthStatus::Expired,
        ] {
            assert_eq!(format!("{variant}"), variant.as_str());
        }
    }

    #[test]
    fn from_str_round_trip() {
        for variant in [
            DeviceAuthStatus::Pending,
            DeviceAuthStatus::Authorized,
            DeviceAuthStatus::Expired,
        ] {
            let s = variant.as_str();
            let parsed: DeviceAuthStatus = s.parse().unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn from_str_invalid_returns_err() {
        assert!("unknown".parse::<DeviceAuthStatus>().is_err());
        assert!("".parse::<DeviceAuthStatus>().is_err());
    }

    #[test]
    fn serde_values() {
        assert_eq!(
            serde_json::to_string(&DeviceAuthStatus::Pending).unwrap(),
            r#""pending""#
        );
        assert_eq!(
            serde_json::to_string(&DeviceAuthStatus::Authorized).unwrap(),
            r#""authorized""#
        );
        assert_eq!(
            serde_json::to_string(&DeviceAuthStatus::Expired).unwrap(),
            r#""expired""#
        );
    }
}
