use std::fmt;

use serde::{Deserialize, Serialize};

/// Type of service enrolling with the controller.
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
pub enum ServiceType {
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "agent"))]
    Agent,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "mqtt"))]
    Mqtt,
}

impl fmt::Display for ServiceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Agent => f.write_str("agent"),
            Self::Mqtt => f.write_str("mqtt"),
        }
    }
}

impl ServiceType {
    /// Returns the string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Mqtt => "mqtt",
        }
    }

    /// Parses a string into a `ServiceType` variant.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "agent" => Some(Self::Agent),
            "mqtt" => Some(Self::Mqtt),
            _ => None,
        }
    }
}

impl std::str::FromStr for ServiceType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| format!("unknown service type: {s}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip() {
        for variant in [ServiceType::Agent, ServiceType::Mqtt] {
            let json = serde_json::to_string(&variant).unwrap();
            let deserialized: ServiceType = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, variant);
        }
    }

    #[test]
    fn display_matches_as_str() {
        for variant in [ServiceType::Agent, ServiceType::Mqtt] {
            assert_eq!(format!("{variant}"), variant.as_str());
        }
    }

    #[test]
    fn parse_round_trip() {
        for variant in [ServiceType::Agent, ServiceType::Mqtt] {
            let s = variant.as_str();
            let parsed = ServiceType::parse(s);
            assert_eq!(parsed, Some(variant));
        }
    }

    #[test]
    fn parse_unknown_returns_none() {
        assert_eq!(ServiceType::parse("unknown"), None);
        assert_eq!(ServiceType::parse(""), None);
    }

    #[test]
    fn from_str_round_trip() {
        for variant in [ServiceType::Agent, ServiceType::Mqtt] {
            let s = variant.as_str();
            let parsed: ServiceType = s.parse().unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn from_str_unknown_returns_err() {
        assert!("unknown".parse::<ServiceType>().is_err());
    }
}
