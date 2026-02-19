use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Type of service enrolling with the controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "sea-orm",
    derive(strum::EnumIter, sea_orm::DeriveActiveEnum)
)]
#[cfg_attr(feature = "sea-orm", sea_orm(rs_type = "String", db_type = "Text"))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ServiceType {
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "agent"))]
    Agent,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "mqtt"))]
    Mqtt,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "ssh_agent"))]
    SshAgent,
}

impl fmt::Display for ServiceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl ServiceType {
    /// Returns the string representation.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Mqtt => "mqtt",
            Self::SshAgent => "ssh_agent",
        }
    }
}

/// Error returned when parsing an invalid service type string.
#[derive(Debug)]
pub struct ParseServiceTypeError;

impl fmt::Display for ParseServiceTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid service type value")
    }
}

impl std::error::Error for ParseServiceTypeError {}

impl FromStr for ServiceType {
    type Err = ParseServiceTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "agent" => Ok(Self::Agent),
            "mqtt" => Ok(Self::Mqtt),
            "ssh_agent" => Ok(Self::SshAgent),
            _ => Err(ParseServiceTypeError),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip() {
        for variant in [ServiceType::Agent, ServiceType::Mqtt, ServiceType::SshAgent] {
            let json = serde_json::to_string(&variant).unwrap();
            let deserialized: ServiceType = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, variant);
        }
    }

    #[test]
    fn display_matches_as_str() {
        for variant in [ServiceType::Agent, ServiceType::Mqtt, ServiceType::SshAgent] {
            assert_eq!(format!("{variant}"), variant.as_str());
        }
    }

    #[test]
    fn from_str_round_trip() {
        for variant in [ServiceType::Agent, ServiceType::Mqtt, ServiceType::SshAgent] {
            let s = variant.as_str();
            let parsed: ServiceType = s.parse().unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn from_str_invalid_returns_err() {
        assert!("unknown".parse::<ServiceType>().is_err());
        assert!("".parse::<ServiceType>().is_err());
    }
}
