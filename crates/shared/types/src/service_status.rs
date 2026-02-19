use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Status of a service in the enrollment/approval workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "sea-orm",
    derive(strum::EnumIter, sea_orm::DeriveActiveEnum)
)]
#[cfg_attr(feature = "sea-orm", sea_orm(rs_type = "String", db_type = "Text"))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ServiceStatus {
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "pending"))]
    Pending,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "approved"))]
    Approved,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "rejected"))]
    Rejected,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "deactivated"))]
    Deactivated,
}

impl fmt::Display for ServiceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl ServiceStatus {
    /// Returns the string representation.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Deactivated => "deactivated",
        }
    }
}

/// Error returned when parsing an invalid service status string.
#[derive(Debug)]
pub struct ParseServiceStatusError;

impl fmt::Display for ParseServiceStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid service status value")
    }
}

impl std::error::Error for ParseServiceStatusError {}

impl FromStr for ServiceStatus {
    type Err = ParseServiceStatusError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "approved" => Ok(Self::Approved),
            "rejected" => Ok(Self::Rejected),
            "deactivated" => Ok(Self::Deactivated),
            _ => Err(ParseServiceStatusError),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip() {
        for variant in [
            ServiceStatus::Pending,
            ServiceStatus::Approved,
            ServiceStatus::Rejected,
            ServiceStatus::Deactivated,
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            let deserialized: ServiceStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, variant);
        }
    }

    #[test]
    fn display_matches_as_str() {
        for variant in [
            ServiceStatus::Pending,
            ServiceStatus::Approved,
            ServiceStatus::Rejected,
            ServiceStatus::Deactivated,
        ] {
            assert_eq!(format!("{variant}"), variant.as_str());
        }
    }

    #[test]
    fn from_str_round_trip() {
        for variant in [
            ServiceStatus::Pending,
            ServiceStatus::Approved,
            ServiceStatus::Rejected,
            ServiceStatus::Deactivated,
        ] {
            let s = variant.as_str();
            let parsed: ServiceStatus = s.parse().unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn from_str_invalid_returns_err() {
        assert!("unknown".parse::<ServiceStatus>().is_err());
        assert!("".parse::<ServiceStatus>().is_err());
    }
}
